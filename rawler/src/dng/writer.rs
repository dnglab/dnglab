use std::{
  io::{self, Seek, Write},
  mem::size_of,
  time::Instant,
};

use image::{imageops::FilterType, DynamicImage};
use log::debug;
use rayon::prelude::*;

use crate::{
  decoders::{Camera, RawMetadata},
  dng::rect_to_dng_area,
  exif::Exif,
  formats::tiff::{
    writer::{DirectoryWriter, TiffWriter},
    CompressionMethod, PhotometricInterpretation, PreviewColorSpace, Rational, TiffError, Value,
  },
  imgop::{Dim2, Point, Rect},
  ljpeg92::LjpegCompressor,
  pixarray::PixU16,
  rawimage::BlackLevel,
  tags::{ExifGpsTag, ExifTag, TiffTag},
  tiles::ImageTiler,
  RawImage, RawImageData, CFA,
};
use crate::{
  formats::tiff::SRational,
  imgop::xyz::Illuminant,
  tags::{DngTag, TiffCommonTag},
};

use super::{original::OriginalCompressed, CropMode, DngCompression, DngPhotometricConversion, DNG_VERSION_V1_6};

pub type DngError = TiffError;

pub type Result<T> = std::result::Result<T, DngError>;

pub struct DngWriter<B>
where
  B: Write + Seek,
{
  pub dng: TiffWriter<B>,
  root_ifd: DirectoryWriter,
  //raw_ifd: DirectoryWriter,
  //preview_ifd: DirectoryWriter,
  exif_ifd: DirectoryWriter,
  subs: Vec<u32>,
}

pub struct SubFrameWriter<'w, B>
where
  B: Write + Seek,
{
  writer: &'w mut DngWriter<B>,
  ifd: DirectoryWriter,
}

impl<'w, B> SubFrameWriter<'w, B>
where
  B: Write + Seek,
{
  pub fn new(writer: &'w mut DngWriter<B>, subtype: u16) -> Self {
    let mut ifd = DirectoryWriter::new();

    ifd.add_tag(TiffCommonTag::NewSubFileType, subtype);
    Self { ifd, writer }
  }

  pub fn ifd(&mut self) -> &DirectoryWriter {
    &self.ifd
  }

  pub fn ifd_mut(&mut self) -> &mut DirectoryWriter {
    &mut self.ifd
  }

  pub fn rgb_image_u8(&mut self, data: &[u8], width: usize, height: usize, compression: DngCompression, predictor: u8) -> Result<()> {
    let cpp = 3;
    let rawimagedata = PixU16::new_with(data.iter().copied().map(u16::from).collect(), width * cpp, height);
    let mut cam = Camera::new();
    cam.cfa = CFA::new("RGGB");

    let wb_coeffs = [1.0, 1.0, 1.0, 1.0];
    let blacklevel = Some(BlackLevel::new(&[0, 0, 0], 1, 1, 3));
    let whitelevel = Some(vec![0xFF, 0xFF, 0xFF]);
    let rawimage = RawImage::new(cam, rawimagedata, cpp, wb_coeffs, blacklevel, whitelevel, false);
    self.raw_image(&rawimage, CropMode::None, compression, DngPhotometricConversion::Original, predictor)
  }

  pub fn rgb_image_u16(&mut self, data: &[u16], width: usize, height: usize, compression: DngCompression, predictor: u8) -> Result<()> {
    let cpp = 3;
    let rawimagedata = PixU16::new_with(data.iter().copied().map(u16::from).collect(), width * cpp, height);
    let mut cam = Camera::new();
    cam.cfa = CFA::new("RGGB");

    let wb_coeffs = [1.0, 1.0, 1.0, 1.0];
    let blacklevel = Some(BlackLevel::new(&[0, 0, 0], 1, 1, 3));
    let whitelevel = Some(vec![0xFFFF, 0xFFFF, 0xFFFF]);
    let rawimage = RawImage::new(cam, rawimagedata, cpp, wb_coeffs, blacklevel, whitelevel, false);
    self.raw_image(&rawimage, CropMode::None, compression, DngPhotometricConversion::Original, predictor)
  }

  pub fn image(&mut self, _image: &RawImageData, _width: u16, _height: u16) -> Result<()> {
    todo!()
  }

  pub fn raw_image(
    &mut self,
    rawimage: &RawImage,
    cropmode: CropMode,
    compression: DngCompression,
    photometric_conversion: DngPhotometricConversion,
    predictor: u8,
  ) -> Result<()> {
    match photometric_conversion {
      DngPhotometricConversion::Original => self.write_rawimage(rawimage, cropmode, compression, predictor)?,

      DngPhotometricConversion::Linear => {
        if rawimage.cpp == 3 {
          self.write_rawimage(rawimage, cropmode, compression, predictor)?;
        } else {
          let rawimage = rawimage.linearize().unwrap(); // TODO: implement me
          self.write_rawimage(&rawimage, cropmode, compression, predictor)?;
        }
      }
    }

    for (tag, value) in rawimage.dng_tags.iter() {
      self.ifd.add_untyped_tag(*tag, value.clone())?;
    }

    Ok(())
  }

  fn write_rawimage(&mut self, rawimage: &RawImage, cropmode: CropMode, compression: DngCompression, predictor: u8) -> Result<()> {
    self.writer.as_shot_neutral(&wbcoeff_to_tiff_value(rawimage));

    // Add matrix and illumninant
    let mut available_matrices = rawimage.color_matrix.clone();
    if let Some(first_key) = available_matrices.keys().next().cloned() {
      let first_matrix = available_matrices
        .remove_entry(&Illuminant::A)
        .or_else(|| available_matrices.remove_entry(&Illuminant::A))
        .or_else(|| available_matrices.remove_entry(&first_key))
        .expect("No matrix found");
      self
        .writer
        .color_matrix(1, first_matrix.0, matrix_to_tiff_value(&first_matrix.1, 10_000).as_slice());
      if let Some(second_matrix) = available_matrices
        .remove_entry(&Illuminant::D65)
        .or_else(|| available_matrices.remove_entry(&Illuminant::D50))
      {
        self
          .writer
          .color_matrix(2, second_matrix.0, matrix_to_tiff_value(&second_matrix.1, 10_000).as_slice());
      }
    }

    let full_size = Rect::new(Point::new(0, 0), Dim2::new(rawimage.width, rawimage.height));

    // Active area or uncropped
    let active_area: Rect = match cropmode {
      CropMode::ActiveArea | CropMode::Best => rawimage.active_area.unwrap_or(full_size),
      CropMode::None => full_size,
    };

    assert!(active_area.p.x + active_area.d.w <= rawimage.width);
    assert!(active_area.p.y + active_area.d.h <= rawimage.height);

    //self.ifd.add_tag(TiffCommonTag::NewSubFileType, 0_u16)?; // Raw
    self.ifd.add_tag(TiffCommonTag::ImageWidth, rawimage.width as u32);
    self.ifd.add_tag(TiffCommonTag::ImageLength, rawimage.height as u32);

    self.ifd.add_tag(DngTag::ActiveArea, rect_to_dng_area(&active_area));

    match cropmode {
      CropMode::ActiveArea => {
        let crop = active_area;
        assert!(crop.p.x >= active_area.p.x);
        assert!(crop.p.y >= active_area.p.y);
        self.ifd.add_tag(
          DngTag::DefaultCropOrigin,
          [(crop.p.x - active_area.p.x) as u16, (crop.p.y - active_area.p.y) as u16],
        );
        self.ifd.add_tag(DngTag::DefaultCropSize, [crop.d.w as u16, crop.d.h as u16]);
      }
      CropMode::Best => {
        let crop = rawimage.crop_area.unwrap_or(active_area);
        assert!(crop.p.x >= active_area.p.x);
        assert!(crop.p.y >= active_area.p.y);
        self.ifd.add_tag(
          DngTag::DefaultCropOrigin,
          [(crop.p.x - active_area.p.x) as u16, (crop.p.y - active_area.p.y) as u16],
        );
        self.ifd.add_tag(DngTag::DefaultCropSize, [crop.d.w as u16, crop.d.h as u16]);
      }
      CropMode::None => {}
    }

    self.ifd.add_tag(ExifTag::PlanarConfiguration, 1_u16);

    self.ifd.add_tag(
      DngTag::DefaultScale,
      [
        Rational::new(rawimage.camera.default_scale.0[0][0], rawimage.camera.default_scale.0[0][1]),
        Rational::new(rawimage.camera.default_scale.0[1][0], rawimage.camera.default_scale.0[1][1]),
      ],
    );
    self.ifd.add_tag(
      DngTag::BestQualityScale,
      Rational::new(rawimage.camera.best_quality_scale.0[0], rawimage.camera.best_quality_scale.0[1]),
    );

    // Whitelevel
    assert_eq!(rawimage.whitelevel.len(), rawimage.cpp, "Whitelevel sample count must match cpp");
    self.ifd.add_tag(DngTag::WhiteLevel, &rawimage.whitelevel);

    // Blacklevel
    let blacklevel = rawimage.blacklevel.shift(active_area.p.x, active_area.p.y);

    self
      .ifd
      .add_tag(DngTag::BlackLevelRepeatDim, [blacklevel.height as u16, blacklevel.width as u16]);

    assert!(blacklevel.sample_count() == rawimage.cpp || blacklevel.sample_count() == rawimage.cfa.width * rawimage.cfa.height * rawimage.cpp);
    if blacklevel.levels.iter().all(|x| x.d == 1) {
      let payload: Vec<u16> = blacklevel.levels.iter().map(|x| x.n as u16).collect();
      self.ifd.add_tag(DngTag::BlackLevel, &payload);
    } else {
      self.ifd.add_tag(DngTag::BlackLevel, blacklevel.levels.as_slice());
    }

    match rawimage.cpp {
      1 => {
        if !rawimage.blackareas.is_empty() {
          let data: Vec<u16> = rawimage.blackareas.iter().flat_map(rect_to_dng_area).collect();
          self.ifd.add_tag(DngTag::MaskedAreas, &data);
        }
        self.ifd.add_tag(TiffCommonTag::PhotometricInt, PhotometricInterpretation::CFA);
        self.ifd.add_tag(TiffCommonTag::SamplesPerPixel, 1_u16);
        self.ifd.add_tag(TiffCommonTag::BitsPerSample, [16_u16]);

        let cfa = rawimage.cfa.shift(active_area.p.x, active_area.p.y);

        self.ifd.add_tag(TiffCommonTag::CFARepeatPatternDim, [cfa.width as u16, cfa.height as u16]);
        self.ifd.add_tag(TiffCommonTag::CFAPattern, &cfa.flat_pattern()[..]);

        //raw_ifd.add_tag(DngTag::CFAPlaneColor, [0u8, 1u8, 2u8])?; // RGB

        //raw_ifd.add_tag(DngTag::CFAPlaneColor, [1u8, 4u8, 3u8, 5u8])?; // RGB

        self.ifd.add_tag(DngTag::CFALayout, 1_u16); // Square layout

        //raw_ifd.add_tag(LegacyTiffRootTag::CFAPattern, [0u8, 1u8, 1u8, 2u8])?; // RGGB
        //raw_ifd.add_tag(LegacyTiffRootTag::CFARepeatPatternDim, [2u16, 2u16])?;
        //raw_ifd.add_tag(DngTag::CFAPlaneColor, [0u8, 1u8, 2u8])?; // RGGB
      }
      3 => {
        self.ifd.add_tag(TiffCommonTag::PhotometricInt, PhotometricInterpretation::LinearRaw);
        self.ifd.add_tag(TiffCommonTag::SamplesPerPixel, 3_u16);
        self.ifd.add_tag(TiffCommonTag::BitsPerSample, [16_u16, 16_u16, 16_u16]);

        //raw_ifd.add_tag(DngTag::CFAPlaneColor, [1u8, 2u8, 0u8])?; //
      }
      cpp => {
        panic!("Unsupported cpp: {}", cpp);
      }
    }

    match compression {
      DngCompression::Uncompressed => {
        self.ifd.add_tag(TiffCommonTag::Compression, CompressionMethod::None);
        dng_put_raw_uncompressed(&mut self.ifd, self.writer, rawimage)?;
      }
      DngCompression::Lossless => {
        self.ifd.add_tag(TiffCommonTag::Compression, CompressionMethod::ModernJPEG);
        dng_put_raw_ljpeg(&mut self.ifd, self.writer, rawimage, predictor)?;
      }
    }

    for (tag, value) in rawimage.dng_tags.iter() {
      self.ifd.add_untyped_tag(*tag, value.clone())?;
    }

    Ok(())
  }

  pub fn preview(&mut self, img: &DynamicImage, quality: f32) -> Result<()> {
    let now = Instant::now();
    let preview_img = DynamicImage::ImageRgb8(img.resize(1024, 768, FilterType::Nearest).to_rgb8());
    debug!("preview downscale: {} s", now.elapsed().as_secs_f32());

    self.ifd.add_tag(TiffCommonTag::ImageWidth, Value::long(preview_img.width()));
    self.ifd.add_tag(TiffCommonTag::ImageLength, Value::long(preview_img.height()));
    self.ifd.add_tag(TiffCommonTag::Compression, CompressionMethod::ModernJPEG);
    self.ifd.add_tag(TiffCommonTag::BitsPerSample, 8_u16);
    self.ifd.add_tag(TiffCommonTag::SampleFormat, [1_u16, 1, 1]);
    self.ifd.add_tag(TiffCommonTag::PhotometricInt, PhotometricInterpretation::YCbCr);
    self.ifd.add_tag(TiffCommonTag::RowsPerStrip, Value::long(preview_img.height()));
    self.ifd.add_tag(TiffCommonTag::SamplesPerPixel, 3_u16);
    self.ifd.add_tag(DngTag::PreviewColorSpace, PreviewColorSpace::SRgb); // ??

    //ifd.add_tag(TiffRootTag::XResolution, Rational { n: 1, d: 1 })?;
    //ifd.add_tag(TiffRootTag::YResolution, Rational { n: 1, d: 1 })?;
    //ifd.add_tag(TiffRootTag::ResolutionUnit, ResolutionUnit::None.to_u16())?;

    let now = Instant::now();
    let offset = self.writer.dng.position()?;
    // TODO: improve offsets?
    preview_img
      .write_to(&mut self.writer.dng.writer, image::ImageOutputFormat::Jpeg((quality * u8::MAX as f32) as u8))
      .map_err(|err| io::Error::new(io::ErrorKind::Other, format!("Failed to write jpeg preview: {:?}", err)))?;
    let data_len = self.writer.dng.position()? - offset;
    debug!("writing preview: {} s", now.elapsed().as_secs_f32());

    self.ifd.add_value(TiffCommonTag::StripOffsets, Value::Long(vec![offset]));
    self.ifd.add_tag(TiffCommonTag::StripByteCounts, Value::Long(vec![data_len]));

    Ok(())
  }

  pub fn finalize(self) -> Result<()> {
    let offset = self.ifd.build(&mut self.writer.dng)?;
    self.writer.subs.push(offset);
    Ok(())
  }
}

impl<B> DngWriter<B>
where
  B: Write + Seek,
{
  pub fn new(buf: B, backward_version: [u8; 4]) -> Result<Self> {
    let dng = TiffWriter::new(buf)?;

    let mut root_ifd = DirectoryWriter::new();
    let mut exif_ifd = DirectoryWriter::new();
    root_ifd.add_tag(DngTag::DNGBackwardVersion, backward_version);
    root_ifd.add_tag(DngTag::DNGVersion, DNG_VERSION_V1_6);
    // Add EXIF version 0220
    exif_ifd
      .add_tag_undefined(ExifTag::ExifVersion, vec![48, 50, 50, 48])
      .expect("Failed to add EXIF version tag");

    Ok(Self {
      dng,
      root_ifd,
      exif_ifd,
      subs: Vec::new(),
    })
  }

  pub fn as_shot_neutral(&mut self, wb: impl AsRef<[Rational]>) {
    self.root_ifd.add_tag(DngTag::AsShotNeutral, wb.as_ref());
  }

  pub fn color_matrix(&mut self, slot: usize, illu: Illuminant, matrix: impl AsRef<[SRational]>) {
    match slot {
      1 => {
        self.root_ifd.add_tag(DngTag::CalibrationIlluminant1, u16::from(illu));
        self.root_ifd.add_tag(DngTag::ColorMatrix1, matrix.as_ref());
      }
      2 => {
        self.root_ifd.add_tag(DngTag::CalibrationIlluminant2, u16::from(illu));
        self.root_ifd.add_tag(DngTag::ColorMatrix2, matrix.as_ref());
      }
      _ => todo!(),
    }
  }

  pub fn load_metadata(&mut self, metadata: &RawMetadata) -> Result<()> {
    self.fill_exif_root(&metadata.exif)?;

    if let Some(id) = &metadata.unique_image_id {
      self.root_ifd.add_tag(DngTag::RawDataUniqueID, id.to_le_bytes());
    }

    fill_exif_ifd(&mut self.exif_ifd, &metadata.exif)?;
    Ok(())
  }

  pub fn xpacket(&mut self, xpacket: impl AsRef<[u8]>) -> Result<()> {
    self.root_ifd.add_tag(ExifTag::ApplicationNotes, xpacket.as_ref());

    Ok(())
  }

  fn fill_exif_root(&mut self, exif: &Exif) -> Result<()> {
    transfer_entry(&mut self.root_ifd, ExifTag::Orientation, &exif.orientation)?;
    transfer_entry(&mut self.root_ifd, ExifTag::ModifyDate, &exif.modify_date)?;
    transfer_entry(&mut self.root_ifd, ExifTag::Copyright, &exif.copyright)?;
    transfer_entry(&mut self.root_ifd, ExifTag::Artist, &exif.artist)?;

    // DNG has a lens info tag that is identical to the LensSpec tag in EXIF IFD
    transfer_entry(&mut self.root_ifd, DngTag::LensInfo, &exif.lens_spec)?;

    if let Some(gps) = &exif.gps {
      let gps_offset = {
        let mut gps_ifd = DirectoryWriter::new();
        transfer_entry(&mut gps_ifd, ExifGpsTag::GPSVersionID, &gps.gps_version_id)?;
        transfer_entry(&mut gps_ifd, ExifGpsTag::GPSLatitudeRef, &gps.gps_latitude_ref)?;
        transfer_entry(&mut gps_ifd, ExifGpsTag::GPSLatitude, &gps.gps_latitude)?;
        transfer_entry(&mut gps_ifd, ExifGpsTag::GPSLongitudeRef, &gps.gps_longitude_ref)?;
        transfer_entry(&mut gps_ifd, ExifGpsTag::GPSLongitude, &gps.gps_longitude)?;
        transfer_entry(&mut gps_ifd, ExifGpsTag::GPSAltitudeRef, &gps.gps_altitude_ref)?;
        transfer_entry(&mut gps_ifd, ExifGpsTag::GPSAltitude, &gps.gps_altitude)?;
        transfer_entry(&mut gps_ifd, ExifGpsTag::GPSTimeStamp, &gps.gps_timestamp)?;
        transfer_entry(&mut gps_ifd, ExifGpsTag::GPSSatellites, &gps.gps_satellites)?;
        transfer_entry(&mut gps_ifd, ExifGpsTag::GPSStatus, &gps.gps_status)?;
        transfer_entry(&mut gps_ifd, ExifGpsTag::GPSMeasureMode, &gps.gps_measure_mode)?;
        transfer_entry(&mut gps_ifd, ExifGpsTag::GPSDOP, &gps.gps_dop)?;
        transfer_entry(&mut gps_ifd, ExifGpsTag::GPSSpeedRef, &gps.gps_speed_ref)?;
        transfer_entry(&mut gps_ifd, ExifGpsTag::GPSSpeed, &gps.gps_speed)?;
        transfer_entry(&mut gps_ifd, ExifGpsTag::GPSTrackRef, &gps.gps_track_ref)?;
        transfer_entry(&mut gps_ifd, ExifGpsTag::GPSTrack, &gps.gps_track)?;
        transfer_entry(&mut gps_ifd, ExifGpsTag::GPSImgDirectionRef, &gps.gps_img_direction_ref)?;
        transfer_entry(&mut gps_ifd, ExifGpsTag::GPSImgDirection, &gps.gps_img_direction)?;
        transfer_entry(&mut gps_ifd, ExifGpsTag::GPSMapDatum, &gps.gps_map_datum)?;
        transfer_entry(&mut gps_ifd, ExifGpsTag::GPSDestLatitudeRef, &gps.gps_dest_latitude_ref)?;
        transfer_entry(&mut gps_ifd, ExifGpsTag::GPSDestLatitude, &gps.gps_dest_latitude)?;
        transfer_entry(&mut gps_ifd, ExifGpsTag::GPSDestLongitudeRef, &gps.gps_dest_longitude_ref)?;
        transfer_entry(&mut gps_ifd, ExifGpsTag::GPSDestLongitude, &gps.gps_dest_longitude)?;
        transfer_entry(&mut gps_ifd, ExifGpsTag::GPSDestBearingRef, &gps.gps_dest_bearing_ref)?;
        transfer_entry(&mut gps_ifd, ExifGpsTag::GPSDestBearing, &gps.gps_dest_bearing)?;
        transfer_entry(&mut gps_ifd, ExifGpsTag::GPSDestDistanceRef, &gps.gps_dest_distance_ref)?;
        transfer_entry(&mut gps_ifd, ExifGpsTag::GPSDestDistance, &gps.gps_dest_distance)?;
        transfer_entry_undefined(&mut gps_ifd, ExifGpsTag::GPSProcessingMethod, &gps.gps_processing_method)?;
        transfer_entry_undefined(&mut gps_ifd, ExifGpsTag::GPSAreaInformation, &gps.gps_area_information)?;
        transfer_entry(&mut gps_ifd, ExifGpsTag::GPSDateStamp, &gps.gps_date_stamp)?;
        transfer_entry(&mut gps_ifd, ExifGpsTag::GPSDifferential, &gps.gps_differential)?;
        transfer_entry(&mut gps_ifd, ExifGpsTag::GPSHPositioningError, &gps.gps_h_positioning_error)?;
        if gps_ifd.entry_count() > 0 {
          Some(gps_ifd.build(&mut self.dng)?)
        } else {
          None
        }
      };
      if let Some(gps_offset) = gps_offset {
        self.root_ifd.add_tag(ExifTag::GPSInfo, [gps_offset]);
      }
    }

    Ok(())
  }

  pub fn load_base_tags(&mut self, rawimage: &RawImage) -> Result<()> {
    self.root_ifd.add_tag(TiffCommonTag::Make, rawimage.clean_make.as_str());
    self.root_ifd.add_tag(TiffCommonTag::Model, rawimage.clean_model.as_str());
    let uq_model = format!("{} {}", rawimage.clean_make, rawimage.clean_model);
    self.root_ifd.add_tag(DngTag::UniqueCameraModel, uq_model.as_str());
    Ok(())
  }

  pub fn close(mut self) -> Result<()> {
    if !self.exif_ifd.is_empty() {
      let exif_ifd_offset = self.exif_ifd.build(&mut self.dng)?;
      self.root_ifd.add_tag(TiffCommonTag::ExifIFDPointer, exif_ifd_offset);
    }

    // Add SubIFDs
    if !self.subs.is_empty() {
      self.root_ifd.add_tag(TiffCommonTag::SubIFDs, &self.subs);
    }

    self.dng.build(self.root_ifd)?;
    Ok(())
  }

  pub fn original_file(&mut self, original: &OriginalCompressed, filename: impl AsRef<str>) -> Result<()> {
    let mut buf = std::io::Cursor::new(Vec::new());
    original.write_to_stream(&mut buf)?;

    self.root_ifd.add_tag_undefined(DngTag::OriginalRawFileData, buf.into_inner())?;
    self.root_ifd.add_tag(DngTag::OriginalRawFileName, filename.as_ref());
    if let Some(digest) = original.digest() {
      self.root_ifd.add_tag(DngTag::OriginalRawFileDigest, digest);
    }
    Ok(())
  }

  pub fn subframe(&mut self, id: u16) -> SubFrameWriter<B> {
    SubFrameWriter::new(self, id)
  }

  /// Write thumbnail image into DNG
  pub fn thumbnail(&mut self, img: &DynamicImage) -> Result<()> {
    let thumb_img = img.resize(240, 120, FilterType::Nearest).to_rgb8();
    self.root_ifd.add_tag(TiffCommonTag::NewSubFileType, 1_u16);
    self.root_ifd.add_tag(TiffCommonTag::ImageWidth, thumb_img.width() as u32);
    self.root_ifd.add_tag(TiffCommonTag::ImageLength, thumb_img.height() as u32);
    self.root_ifd.add_tag(TiffCommonTag::Compression, CompressionMethod::None);
    self.root_ifd.add_tag(TiffCommonTag::BitsPerSample, 8_u16);
    self.root_ifd.add_tag(TiffCommonTag::SampleFormat, [1_u16, 1, 1]);
    self.root_ifd.add_tag(TiffCommonTag::PhotometricInt, PhotometricInterpretation::RGB);
    self.root_ifd.add_tag(TiffCommonTag::SamplesPerPixel, 3_u16);
    //ifd.add_tag(TiffRootTag::XResolution, Rational { n: 1, d: 1 })?;
    //ifd.add_tag(TiffRootTag::YResolution, Rational { n: 1, d: 1 })?;
    //ifd.add_tag(TiffRootTag::ResolutionUnit, ResolutionUnit::None.to_u16())?;

    let offset = self.dng.write_data(&thumb_img)?;

    self.root_ifd.add_tag(TiffCommonTag::StripOffsets, offset);
    self.root_ifd.add_tag(TiffCommonTag::StripByteCounts, thumb_img.len() as u32);
    self.root_ifd.add_tag(TiffCommonTag::RowsPerStrip, thumb_img.height() as u32);

    Ok(())
  }

  pub fn root_ifd(&mut self) -> &DirectoryWriter {
    &self.root_ifd
  }

  pub fn root_ifd_mut(&mut self) -> &mut DirectoryWriter {
    &mut self.root_ifd
  }
}

/// DNG requires the WB values to be the reciprocal
fn wbcoeff_to_tiff_value(rawimage: &RawImage) -> Vec<Rational> {
  assert!([3, 4].contains(&rawimage.cfa.unique_colors()));
  let wb = &rawimage.wb_coeffs;
  let mut values = Vec::with_capacity(4);

  values.push(Rational::new_f32(1.0 / wb[0], 100000));
  values.push(Rational::new_f32(1.0 / wb[1], 100000));
  values.push(Rational::new_f32(1.0 / wb[2], 100000));

  if rawimage.cfa.unique_colors() == 4 {
    values.push(Rational::new_f32(1.0 / wb[3], 100000));
  }
  values
}

fn matrix_to_tiff_value(xyz_to_cam: &[f32], d: i32) -> Vec<SRational> {
  xyz_to_cam.iter().map(|a| SRational::new((a * d as f32) as i32, d)).collect()
}

/// Compress RAW image with LJPEG-92
///
/// Data is split into multiple tiles, each tile is compressed seperately
///
/// Predictor mode 4,5,6,7 is best for images where two images
/// lines are merged, because then the image bayer pattern is:
/// RGRGGBGB
/// RGRGGBGB
/// Instead of the default:
/// RGRG
/// GBGB
/// RGRG
/// GBGB
fn dng_put_raw_ljpeg<W>(raw_ifd: &mut DirectoryWriter, writer: &mut DngWriter<W>, rawimage: &RawImage, predictor: u8) -> Result<()>
where
  W: Seek + Write,
{
  let tile_w = 256 & !0b111; // ensure div 16
  let tile_h = 256 & !0b111;

  let lj92_data = match rawimage.data {
    RawImageData::Integer(ref data) => {
      // Inject black pixel data for testing purposes.
      // let data = vec![0x0000; data.len()];
      //let tiled_data = TiledData::new(&data, rawimage.width, rawimage.height, rawimage.cpp);

      // Only merge two lines into one for higher predictors, if image is CFA
      let realign = if (4..=7).contains(&predictor) && rawimage.cfa.width == 2 && rawimage.cfa.height == 2 {
        2
      } else {
        1
      };

      let tiled_data: Vec<Vec<u16>> = ImageTiler::new(data, rawimage.width, rawimage.height, rawimage.cpp, tile_w, tile_h).collect();

      let j_height = tile_h;
      let (j_width, components) = if rawimage.cpp == 3 {
        (tile_w, 3) /* RGB LinearRaw */
      } else {
        (tile_w / 2, 2) /* CFA */
      };

      debug!("LJPEG compression: bit depth: {}", rawimage.bps);

      let tiles_compr: Vec<Vec<u8>> = tiled_data
        .par_iter()
        .map(|tile| {
          //assert_eq!((tile_w * rawimage.cpp) % components, 0);
          //assert_eq!((tile_w * rawimage.cpp) % 2, 0);
          //assert_eq!(tile_h % 2, 0);
          let state = LjpegCompressor::new(tile, j_width * realign, j_height / realign, components, rawimage.bps as u8, predictor, 0, 0).unwrap();
          state.encode().unwrap()
        })
        .collect();
      tiles_compr
    }
    RawImageData::Float(ref _data) => {
      panic!("invalid format");
    }
  };

  let mut tile_offsets: Vec<u32> = Vec::new();
  let mut tile_sizes: Vec<u32> = Vec::new();

  lj92_data.iter().for_each(|tile| {
    let offs = writer.dng.write_data(tile).unwrap();
    tile_offsets.push(offs);
    tile_sizes.push((tile.len() * size_of::<u8>()) as u32);
  });

  //let offs = raw_ifd.write_data(&lj92_data)?;
  raw_ifd.add_tag(TiffCommonTag::TileOffsets, &tile_offsets);
  raw_ifd.add_tag(TiffCommonTag::TileByteCounts, &tile_sizes);
  //raw_ifd.add_tag(LegacyTiffRootTag::TileWidth, lj92_data.1 as u16)?; // FIXME
  //raw_ifd.add_tag(LegacyTiffRootTag::TileLength, lj92_data.2 as u16)?;
  raw_ifd.add_tag(TiffCommonTag::TileWidth, tile_w as u16); // FIXME
  raw_ifd.add_tag(TiffCommonTag::TileLength, tile_h as u16);

  Ok(())
}

/// Write RAW uncompressed into DNG
///
/// This uses unsigned 16 bit values for storage
/// Data is split into multiple strips
fn dng_put_raw_uncompressed<W>(raw_ifd: &mut DirectoryWriter, writer: &mut DngWriter<W>, rawimage: &RawImage) -> Result<()>
where
  W: Write + Seek,
{
  match rawimage.data {
    RawImageData::Integer(ref data) => {
      let mut strip_offsets: Vec<u32> = Vec::new();
      let mut strip_sizes: Vec<u32> = Vec::new();
      let mut strip_rows: Vec<u32> = Vec::new();

      // 8 Strips
      let rows_per_strip = rawimage.height / 8;

      for strip in data.chunks(rows_per_strip * rawimage.width * rawimage.cpp) {
        let offset = writer.dng.write_data_u16_be(strip)?;
        strip_offsets.push(offset);
        strip_sizes.push(std::mem::size_of_val(strip) as u32);
        strip_rows.push((strip.len() / (rawimage.width * rawimage.cpp)) as u32);
      }

      raw_ifd.add_tag(TiffCommonTag::StripOffsets, &strip_offsets);
      raw_ifd.add_tag(TiffCommonTag::StripByteCounts, &strip_sizes);
      raw_ifd.add_tag(TiffCommonTag::RowsPerStrip, &strip_rows);
    }
    RawImageData::Float(ref _data) => {
      panic!("invalid format");
    }
  };

  Ok(())
}

fn transfer_entry<T, V>(raw_ifd: &mut DirectoryWriter, tag: T, entry: &Option<V>) -> Result<()>
where
  T: TiffTag,
  V: Into<Value> + Clone,
{
  if let Some(entry) = entry {
    raw_ifd.add_tag(tag, entry.clone());
  }
  Ok(())
}

fn transfer_entry_undefined<T>(raw_ifd: &mut DirectoryWriter, tag: T, entry: &Option<Vec<u8>>) -> Result<()>
where
  T: TiffTag,
{
  if let Some(entry) = entry {
    raw_ifd.add_tag_undefined(tag, entry.clone())?;
  }
  Ok(())
}

fn fill_exif_ifd(exif_ifd: &mut DirectoryWriter, exif: &Exif) -> Result<()> {
  transfer_entry(exif_ifd, ExifTag::FNumber, &exif.fnumber)?;
  transfer_entry(exif_ifd, ExifTag::ApertureValue, &exif.aperture_value)?;
  transfer_entry(exif_ifd, ExifTag::BrightnessValue, &exif.brightness_value)?;
  transfer_entry(exif_ifd, ExifTag::RecommendedExposureIndex, &exif.recommended_exposure_index)?;
  transfer_entry(exif_ifd, ExifTag::ExposureTime, &exif.exposure_time)?;
  transfer_entry(exif_ifd, ExifTag::ISOSpeedRatings, &exif.iso_speed_ratings)?;
  transfer_entry(exif_ifd, ExifTag::ISOSpeed, &exif.iso_speed)?;
  transfer_entry(exif_ifd, ExifTag::SensitivityType, &exif.sensitivity_type)?;
  transfer_entry(exif_ifd, ExifTag::ExposureProgram, &exif.exposure_program)?;
  transfer_entry(exif_ifd, ExifTag::TimeZoneOffset, &exif.timezone_offset)?;
  transfer_entry(exif_ifd, ExifTag::DateTimeOriginal, &exif.date_time_original)?;
  transfer_entry(exif_ifd, ExifTag::CreateDate, &exif.create_date)?;
  transfer_entry(exif_ifd, ExifTag::OffsetTime, &exif.offset_time)?;
  transfer_entry(exif_ifd, ExifTag::OffsetTimeOriginal, &exif.offset_time_original)?;
  transfer_entry(exif_ifd, ExifTag::OffsetTimeDigitized, &exif.offset_time_digitized)?;
  transfer_entry(exif_ifd, ExifTag::SubSecTime, &exif.sub_sec_time)?;
  transfer_entry(exif_ifd, ExifTag::SubSecTimeOriginal, &exif.sub_sec_time_original)?;
  transfer_entry(exif_ifd, ExifTag::SubSecTimeDigitized, &exif.sub_sec_time_digitized)?;
  transfer_entry(exif_ifd, ExifTag::ShutterSpeedValue, &exif.shutter_speed_value)?;
  transfer_entry(exif_ifd, ExifTag::MaxApertureValue, &exif.max_aperture_value)?;
  transfer_entry(exif_ifd, ExifTag::SubjectDistance, &exif.subject_distance)?;
  transfer_entry(exif_ifd, ExifTag::MeteringMode, &exif.metering_mode)?;
  transfer_entry(exif_ifd, ExifTag::LightSource, &exif.light_source)?;
  transfer_entry(exif_ifd, ExifTag::Flash, &exif.flash)?;
  transfer_entry(exif_ifd, ExifTag::FocalLength, &exif.focal_length)?;
  transfer_entry(exif_ifd, ExifTag::ImageNumber, &exif.image_number)?;
  transfer_entry(exif_ifd, ExifTag::ColorSpace, &exif.color_space)?;
  transfer_entry(exif_ifd, ExifTag::FlashEnergy, &exif.flash_energy)?;
  transfer_entry(exif_ifd, ExifTag::ExposureMode, &exif.exposure_mode)?;
  transfer_entry(exif_ifd, ExifTag::WhiteBalance, &exif.white_balance)?;
  transfer_entry(exif_ifd, ExifTag::SceneCaptureType, &exif.scene_capture_type)?;
  transfer_entry(exif_ifd, ExifTag::SubjectDistanceRange, &exif.subject_distance_range)?;
  transfer_entry(exif_ifd, ExifTag::OwnerName, &exif.owner_name)?;
  transfer_entry(exif_ifd, ExifTag::SerialNumber, &exif.serial_number)?;
  transfer_entry(exif_ifd, ExifTag::LensSerialNumber, &exif.lens_serial_number)?;
  transfer_entry(exif_ifd, ExifTag::LensSpecification, &exif.lens_spec)?;
  transfer_entry(exif_ifd, ExifTag::LensMake, &exif.lens_make)?;
  transfer_entry(exif_ifd, ExifTag::LensModel, &exif.lens_model)?;

  Ok(())
}

#[cfg(test)]
mod tests {

  use std::io::Cursor;

  use crate::dng::DNG_VERSION_V1_4;

  use super::*;

  #[test]
  fn build_empty_dng() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let mut buf = Cursor::new(Vec::new());
    let mut dng = DngWriter::new(&mut buf, DNG_VERSION_V1_4)?;
    dng.root_ifd_mut().add_tag(TiffCommonTag::Artist, "Test");
    dng.close()?;
    let expected_output = [
      73, 73, 42, 0, 36, 0, 0, 0, 1, 0, 0, 144, 7, 0, 4, 0, 0, 0, 48, 50, 50, 48, 0, 0, 0, 0, 0, 0, 84, 101, 115, 116, 0, 0, 0, 0, 4, 0, 59, 1, 2, 0, 5, 0, 0,
      0, 28, 0, 0, 0, 105, 135, 4, 0, 1, 0, 0, 0, 8, 0, 0, 0, 18, 198, 1, 0, 4, 0, 0, 0, 1, 6, 0, 0, 19, 198, 1, 0, 4, 0, 0, 0, 1, 4, 0, 0, 0, 0, 0, 0,
    ];
    assert_eq!(expected_output, buf.into_inner().as_slice());
    Ok(())
  }

  #[cfg(feature = "samplecheck")]
  #[test]
  fn convert_canon_cr3_to_dng() -> std::result::Result<(), Box<dyn std::error::Error>> {
    use crate::{
      decoders::RawDecodeParams,
      dng::{DNG_VERSION_V1_4, PREVIEW_JPEG_QUALITY},
      RawFile,
    };
    use std::{
      fs::File,
      io::{BufReader, BufWriter},
      path::PathBuf,
    };

    let mut rawdb = PathBuf::from(std::env::var("RAWLER_RAWDB").expect("RAWLER_RAWDB variable must be set in order to run RAW test!"));
    rawdb.push("cameras/Canon/EOS R5/raw_modes/Canon EOS R5_RAW_ISO_100_nocrop_nodual.CR3");

    let mut rawfile = RawFile::new(rawdb.clone(), File::open(rawdb.clone()).unwrap());

    let original_thread = std::thread::spawn(|| OriginalCompressed::compress(&mut BufReader::new(File::open(rawdb).unwrap())));

    let decoder = crate::get_decoder(&mut rawfile)?;

    let rawimage = decoder.raw_image(&mut rawfile, RawDecodeParams::default(), false)?;

    let full_image = decoder.full_image(&mut rawfile)?.unwrap();

    let metadata = decoder.raw_metadata(&mut rawfile, RawDecodeParams::default())?;

    let predictor = 1;

    let buf = BufWriter::new(Cursor::new(Vec::new()));
    //let buf = BufWriter::new(File::create("/tmp/dng_writer_simple_test.dng")?);
    let mut dng = DngWriter::new(buf, DNG_VERSION_V1_4)?;
    let mut raw = dng.subframe(0);
    raw.raw_image(
      &rawimage,
      CropMode::Best,
      DngCompression::Lossless,
      DngPhotometricConversion::Original,
      predictor,
    )?;
    raw.finalize()?;
    dng.thumbnail(&full_image)?;
    let mut preview = dng.subframe(1);
    preview.preview(&full_image, PREVIEW_JPEG_QUALITY)?;
    preview.finalize()?;
    dng.load_base_tags(&rawimage)?;

    dng.load_metadata(&metadata)?;

    if let Some(xpacket) = decoder.xpacket(&mut rawfile, RawDecodeParams::default())? {
      dng.xpacket(xpacket)?;
    }

    let original = original_thread.join().unwrap()?;

    dng.original_file(&original, "test.CR3")?;

    dng.close()?;

    Ok(())
  }
}
