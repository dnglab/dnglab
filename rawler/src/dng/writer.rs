use std::{
  borrow::Cow,
  io::{self, Seek, Write},
  mem::size_of,
  time::Instant,
};

use image::{DynamicImage, codecs::jpeg::JpegEncoder, imageops::FilterType};
use log::debug;
use rayon::prelude::*;

use crate::{
  CFA, RawImage, RawImageData,
  decoders::{Camera, RawMetadata},
  dng::rect_to_dng_area,
  formats::tiff::{
    CompressionMethod, PhotometricInterpretation, PreviewColorSpace, Rational, TiffError, Value,
    writer::{DirectoryWriter, TiffWriter, transfer_entry},
  },
  imgop::{Dim2, Point, Rect},
  ljpeg92::LjpegCompressor,
  pixarray::PixU16,
  rawimage::{BlackLevel, RawPhotometricInterpretation, WhiteLevel},
  tags::ExifTag,
  tiles::ImageTiler,
};
use crate::{
  formats::tiff::SRational,
  imgop::xyz::Illuminant,
  tags::{DngTag, TiffCommonTag},
};

use super::{CropMode, DNG_VERSION_V1_6, DngCompression, DngPhotometricConversion, original::OriginalCompressed};

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
  ifd: Option<DirectoryWriter>,
}

impl<'w, B> SubFrameWriter<'w, B>
where
  B: Write + Seek,
{
  pub fn new(writer: &'w mut DngWriter<B>, subtype: u32, use_root: bool) -> Self {
    let ifd = if use_root {
      writer.root_ifd_mut().add_tag(TiffCommonTag::NewSubFileType, subtype);
      None
    } else {
      let mut ifd = DirectoryWriter::new();
      ifd.add_tag(TiffCommonTag::NewSubFileType, subtype);
      Some(ifd)
    };
    Self { ifd, writer }
  }

  pub fn ifd(&mut self) -> &DirectoryWriter {
    self.ifd.as_ref().unwrap_or(self.writer.root_ifd())
  }

  pub fn ifd_mut(&mut self) -> &mut DirectoryWriter {
    self.ifd.as_mut().unwrap_or(self.writer.root_ifd_mut())
  }

  pub fn rgb_image_u8(&mut self, data: &[u8], width: usize, height: usize, compression: DngCompression, predictor: u8) -> Result<()> {
    let cpp = 3;
    let rawimagedata = PixU16::new_with(data.iter().copied().map(u16::from).collect(), width * cpp, height);
    let mut cam = Camera::new();
    cam.cfa = CFA::new("RGGB");

    let wb_coeffs = [1.0, 1.0, 1.0, 1.0];
    let blacklevel = Some(BlackLevel::new(&[0_u32, 0, 0], 1, 1, 3));
    let whitelevel = Some(WhiteLevel::new_bits(8, cpp));
    let photometric = RawPhotometricInterpretation::LinearRaw;
    let rawimage = RawImage::new(cam, rawimagedata, cpp, wb_coeffs, photometric, blacklevel, whitelevel, false);
    self.raw_image(&rawimage, CropMode::None, compression, DngPhotometricConversion::Original, predictor)
  }

  pub fn rgb_image_u16(&mut self, data: &[u16], width: usize, height: usize, compression: DngCompression, predictor: u8) -> Result<()> {
    let cpp = 3;
    let rawimagedata = PixU16::new_with(data.to_vec(), width * cpp, height);
    let mut cam = Camera::new();
    cam.cfa = CFA::new("RGGB");

    let wb_coeffs = [1.0, 1.0, 1.0, 1.0];
    let blacklevel = Some(BlackLevel::new(&[0_u32, 0, 0], 1, 1, 3));
    let whitelevel = Some(WhiteLevel::new_bits(16, cpp));
    let photometric = RawPhotometricInterpretation::LinearRaw;
    let rawimage = RawImage::new(cam, rawimagedata, cpp, wb_coeffs, photometric, blacklevel, whitelevel, false);
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
      DngPhotometricConversion::Original => self.write_rawimage(Cow::Borrowed(rawimage), cropmode, compression, predictor)?,

      DngPhotometricConversion::Linear => {
        if rawimage.cpp == 3 {
          self.write_rawimage(Cow::Borrowed(rawimage), cropmode, compression, predictor)?;
        } else {
          let rawimage = rawimage.linearize().unwrap(); // TODO: implement me
          self.write_rawimage(Cow::Borrowed(&rawimage), cropmode, compression, predictor)?;
        }
      }
    }

    /*
    for (tag, value) in rawimage.dng_tags.iter() {
      self.ifd.add_untyped_tag(*tag, value.clone())?;
    }
     */

    Ok(())
  }

  fn write_rawimage(&mut self, mut rawimage: Cow<RawImage>, cropmode: CropMode, compression: DngCompression, predictor: u8) -> Result<()> {
    if compression == DngCompression::Lossless && matches!(rawimage.data, RawImageData::Float(_)) {
      // Lossless (LJPEG92) can only be used for 16 bit integer data.
      // If we have floats, convert them.
      rawimage.to_mut().data.force_integer();
      rawimage.to_mut().whitelevel.0.iter_mut().for_each(|x| *x = u16::MAX as u32);
      rawimage.to_mut().bps = 16; // Reset bps as intgers are scaled to u16 range.
    }

    if rawimage.cpp > 1 || matches!(rawimage.photometric, RawPhotometricInterpretation::Cfa(_)) {
      self.writer.as_shot_neutral(wbcoeff_to_tiff_value(&rawimage));
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
    self.ifd_mut().add_tag(TiffCommonTag::ImageWidth, rawimage.width as u32);
    self.ifd_mut().add_tag(TiffCommonTag::ImageLength, rawimage.height as u32);

    self.ifd_mut().add_tag(DngTag::ActiveArea, rect_to_dng_area(&active_area));

    match cropmode {
      CropMode::ActiveArea => {
        let crop = active_area;
        assert!(crop.p.x >= active_area.p.x);
        assert!(crop.p.y >= active_area.p.y);
        self.ifd_mut().add_tag(
          DngTag::DefaultCropOrigin,
          [(crop.p.x - active_area.p.x) as u16, (crop.p.y - active_area.p.y) as u16],
        );
        self.ifd_mut().add_tag(DngTag::DefaultCropSize, [crop.d.w as u16, crop.d.h as u16]);
      }
      CropMode::Best => {
        let crop = rawimage.crop_area.unwrap_or(active_area);
        assert!(crop.p.x >= active_area.p.x);
        assert!(crop.p.y >= active_area.p.y);
        self.ifd_mut().add_tag(
          DngTag::DefaultCropOrigin,
          [(crop.p.x - active_area.p.x) as u16, (crop.p.y - active_area.p.y) as u16],
        );
        self.ifd_mut().add_tag(DngTag::DefaultCropSize, [crop.d.w as u16, crop.d.h as u16]);
      }
      CropMode::None => {}
    }

    self.ifd_mut().add_tag(ExifTag::PlanarConfiguration, 1_u16);

    self.ifd_mut().add_tag(
      DngTag::DefaultScale,
      [
        Rational::new(rawimage.camera.default_scale.0[0][0], rawimage.camera.default_scale.0[0][1]),
        Rational::new(rawimage.camera.default_scale.0[1][0], rawimage.camera.default_scale.0[1][1]),
      ],
    );
    self.ifd_mut().add_tag(
      DngTag::BestQualityScale,
      Rational::new(rawimage.camera.best_quality_scale.0[0], rawimage.camera.best_quality_scale.0[1]),
    );

    // Whitelevel
    assert_eq!(rawimage.whitelevel.0.len(), rawimage.cpp, "Whitelevel sample count must match cpp");

    if rawimage.whitelevel.0.iter().all(|x| *x <= (u16::MAX as u32)) {
      // Add as u16
      self
        .ifd_mut()
        .add_tag(DngTag::WhiteLevel, &rawimage.whitelevel.0.iter().map(|x| *x as u16).collect::<Vec<u16>>());
    } else {
      self.ifd_mut().add_tag(DngTag::WhiteLevel, &rawimage.whitelevel.0);
    }

    // Blacklevel
    let blacklevel = rawimage.blacklevel.shift(active_area.p.x, active_area.p.y);

    self
      .ifd_mut()
      .add_tag(DngTag::BlackLevelRepeatDim, [blacklevel.height as u16, blacklevel.width as u16]);

    if blacklevel.levels.iter().all(|x| x.d == 1) {
      let payload: Vec<u32> = blacklevel.levels.iter().map(|x| x.n as u32).collect();
      if payload.iter().all(|x| *x <= (u16::MAX as u32)) {
        // Add as u16
        self
          .ifd_mut()
          .add_tag(DngTag::BlackLevel, &payload.into_iter().map(|x| x as u16).collect::<Vec<u16>>());
      } else {
        // Add as u32
        self.ifd_mut().add_tag(DngTag::BlackLevel, &payload);
      }
    } else {
      // Add as RATIONAL
      self.ifd_mut().add_tag(DngTag::BlackLevel, blacklevel.levels.as_slice());
    }

    if !rawimage.blackareas.is_empty() {
      let data: Vec<u16> = rawimage.blackareas.iter().flat_map(rect_to_dng_area).collect();
      self.ifd_mut().add_tag(DngTag::MaskedAreas, &data);
    }

    self.ifd_mut().add_tag(TiffCommonTag::SamplesPerPixel, rawimage.cpp as u16);

    match &rawimage.photometric {
      RawPhotometricInterpretation::BlackIsZero => {
        assert_eq!(rawimage.cpp, 1);
        self.ifd_mut().add_tag(TiffCommonTag::PhotometricInt, PhotometricInterpretation::BlackIsZero);
      }
      RawPhotometricInterpretation::Cfa(config) => {
        assert!(config.cfa.is_valid());
        assert_eq!(rawimage.cpp, 1);
        let cfa = config.cfa.shift(active_area.p.x, active_area.p.y);
        self
          .ifd_mut()
          .add_tag(TiffCommonTag::CFARepeatPatternDim, [cfa.width as u16, cfa.height as u16]);
        self.ifd_mut().add_tag(TiffCommonTag::CFAPattern, &cfa.flat_pattern()[..]);
        self.ifd_mut().add_tag(TiffCommonTag::PhotometricInt, PhotometricInterpretation::CFA);
        self.ifd_mut().add_tag(DngTag::CFAPlaneColor, &config.colors);
        self.ifd_mut().add_tag(DngTag::CFALayout, 1_u16); // Square layout
      }
      RawPhotometricInterpretation::LinearRaw => {
        self.ifd_mut().add_tag(TiffCommonTag::PhotometricInt, PhotometricInterpretation::LinearRaw);
      }
    }

    match compression {
      DngCompression::Uncompressed => {
        self.ifd_mut().add_tag(TiffCommonTag::Compression, CompressionMethod::None);
        dng_put_raw_uncompressed(self, &rawimage)?;
      }
      DngCompression::Lossless => {
        self.ifd_mut().add_tag(TiffCommonTag::Compression, CompressionMethod::ModernJPEG);
        dng_put_raw_ljpeg(self, &rawimage, predictor)?;
      }
    }

    /*
    for (tag, value) in rawimage.dng_tags.iter() {
      self.ifd.add_untyped_tag(*tag, value.clone())?;
    }
     */

    Ok(())
  }

  pub fn preview(&mut self, img: &DynamicImage, quality: f32) -> Result<()> {
    let now = Instant::now();
    let preview_img = DynamicImage::ImageRgb8(img.resize(1024, 768, FilterType::Nearest).to_rgb8());
    debug!("preview downscale: {} s", now.elapsed().as_secs_f32());

    self.ifd_mut().add_tag(TiffCommonTag::ImageWidth, Value::long(preview_img.width()));
    self.ifd_mut().add_tag(TiffCommonTag::ImageLength, Value::long(preview_img.height()));
    self.ifd_mut().add_tag(TiffCommonTag::Compression, CompressionMethod::ModernJPEG);
    self.ifd_mut().add_tag(TiffCommonTag::BitsPerSample, [8_u16, 8, 8]);
    self.ifd_mut().add_tag(TiffCommonTag::SampleFormat, [1_u16, 1, 1]);
    self.ifd_mut().add_tag(TiffCommonTag::PhotometricInt, PhotometricInterpretation::YCbCr);
    self.ifd_mut().add_tag(TiffCommonTag::RowsPerStrip, Value::long(preview_img.height()));
    self.ifd_mut().add_tag(TiffCommonTag::SamplesPerPixel, 3_u16);
    self.ifd_mut().add_tag(DngTag::PreviewColorSpace, PreviewColorSpace::SRgb); // ??

    //ifd.add_tag(TiffRootTag::XResolution, Rational { n: 1, d: 1 })?;
    //ifd.add_tag(TiffRootTag::YResolution, Rational { n: 1, d: 1 })?;
    //ifd.add_tag(TiffRootTag::ResolutionUnit, ResolutionUnit::None.to_u16())?;

    let now = Instant::now();
    let offset = self.writer.dng.position()?;
    // TODO: improve offsets?
    let jpeg_encoder = JpegEncoder::new_with_quality(&mut self.writer.dng.writer, (quality * 100.0).max(100.0) as u8);
    preview_img
      .write_with_encoder(jpeg_encoder)
      .map_err(|err| io::Error::new(io::ErrorKind::Other, format!("Failed to write jpeg preview: {:?}", err)))?;
    let data_len = self.writer.dng.position()? - offset;
    debug!("writing preview: {} s", now.elapsed().as_secs_f32());

    self.ifd_mut().add_value(TiffCommonTag::StripOffsets, Value::Long(vec![offset]));
    self.ifd_mut().add_tag(TiffCommonTag::StripByteCounts, Value::Long(vec![data_len]));

    Ok(())
  }

  pub fn finalize(self) -> Result<()> {
    if let Some(ifd) = self.ifd {
      let offset = ifd.build(&mut self.writer.dng)?;
      self.writer.subs.push(offset);
    }
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
    exif_ifd.add_tag_undefined(ExifTag::ExifVersion, vec![48, 50, 50, 48]);

    Ok(Self {
      dng,
      root_ifd,
      exif_ifd,
      subs: Vec::new(),
    })
  }

  pub fn as_shot_neutral(&mut self, wb: impl AsRef<[Rational]>) {
    // Only write tag if wb is valid
    if wb.as_ref()[0].n != 0 {
      self.root_ifd.add_tag(DngTag::AsShotNeutral, wb.as_ref());
    }
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
    metadata.write_exif_tags(&mut self.dng, &mut self.root_ifd, &mut self.exif_ifd)?;

    // DNG has a lens info tag that is identical to the LensSpec tag in EXIF IFD
    transfer_entry(&mut self.root_ifd, DngTag::LensInfo, &metadata.exif.lens_spec)?;

    if let Some(id) = &metadata.unique_image_id {
      self.root_ifd.add_tag(DngTag::RawDataUniqueID, id.to_le_bytes());
    }
    Ok(())
  }

  pub fn xpacket(&mut self, xpacket: impl AsRef<[u8]>) -> Result<()> {
    self.root_ifd.add_tag(ExifTag::ApplicationNotes, xpacket.as_ref());

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

    self.root_ifd.add_tag_undefined(DngTag::OriginalRawFileData, buf.into_inner());
    self.root_ifd.add_tag(DngTag::OriginalRawFileName, filename.as_ref());
    if let Some(digest) = original.digest() {
      self.root_ifd.add_tag(DngTag::OriginalRawFileDigest, digest);
    }
    Ok(())
  }

  pub fn subframe(&mut self, id: u32) -> SubFrameWriter<B> {
    SubFrameWriter::new(self, id, false)
  }

  pub fn subframe_on_root(&mut self, id: u32) -> SubFrameWriter<B> {
    SubFrameWriter::new(self, id, true)
  }

  /// Write thumbnail image into DNG
  pub fn thumbnail(&mut self, img: &DynamicImage) -> Result<()> {
    let thumb_img = img.resize(240, 120, FilterType::Nearest).to_rgb8();
    self.root_ifd.add_tag(TiffCommonTag::NewSubFileType, 1_u32);
    self.root_ifd.add_tag(TiffCommonTag::ImageWidth, thumb_img.width() as u32);
    self.root_ifd.add_tag(TiffCommonTag::ImageLength, thumb_img.height() as u32);
    self.root_ifd.add_tag(TiffCommonTag::Compression, CompressionMethod::None);
    self.root_ifd.add_tag(TiffCommonTag::BitsPerSample, [8_u16, 8, 8]);
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

  pub fn exif_ifd(&mut self) -> &DirectoryWriter {
    &self.exif_ifd
  }

  pub fn exif_ifd_mut(&mut self) -> &mut DirectoryWriter {
    &mut self.exif_ifd
  }
}

/// DNG requires the WB values to be the reciprocal
fn wbcoeff_to_tiff_value(rawimage: &RawImage) -> Vec<Rational> {
  let wb = &rawimage.wb_coeffs;
  match &rawimage.photometric {
    RawPhotometricInterpretation::BlackIsZero => {
      vec![Rational::new(1, 1)] // TODO: is this useful?
    }
    RawPhotometricInterpretation::Cfa(config) => {
      assert!([1, 3, 4].contains(&config.cfa.unique_colors()));

      let mut values = Vec::with_capacity(4);

      values.push(Rational::new_f32(1.0 / wb[0], 100000));
      values.push(Rational::new_f32(1.0 / wb[1], 100000));
      values.push(Rational::new_f32(1.0 / wb[2], 100000));

      if config.cfa.unique_colors() == 4 {
        values.push(Rational::new_f32(1.0 / wb[3], 100000));
      }
      values
    }
    RawPhotometricInterpretation::LinearRaw => {
      //assert_eq!(rawimage.cpp, 3);

      match rawimage.cpp {
        1 => {
          vec![Rational::new(1, 1)]
        }
        3 => {
          let mut values = Vec::with_capacity(3);
          values.push(Rational::new_f32(1.0 / wb[0], 100000));
          values.push(Rational::new_f32(1.0 / wb[1], 100000));
          values.push(Rational::new_f32(1.0 / wb[2], 100000));
          values
        }
        _ => todo!(),
      }
    }
  }
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
fn dng_put_raw_ljpeg<W>(subframe: &mut SubFrameWriter<W>, rawimage: &RawImage, predictor: u8) -> Result<()>
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

      let tiled_data: Vec<Vec<u16>> = ImageTiler::new(data, rawimage.width, rawimage.height, rawimage.cpp, tile_w, tile_h).collect();

      let (j_width, j_height, components, realign) = match &rawimage.photometric {
        RawPhotometricInterpretation::BlackIsZero => {
          assert_eq!(rawimage.cpp, 1);
          (tile_w, tile_h, 1, 1)
        }
        RawPhotometricInterpretation::Cfa(config) => {
          assert_eq!(rawimage.cpp, 1);
          let realign = if (4..=7).contains(&predictor) && config.cfa.width == 2 && config.cfa.height == 2 {
            2
          } else {
            1
          };
          (tile_w / 2, tile_h, 2, realign)
        }
        RawPhotometricInterpretation::LinearRaw => {
          (tile_w, tile_h, rawimage.cpp, 1) /* RGB LinearRaw */
        }
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
    let offs = subframe.writer.dng.write_data(tile).unwrap();
    tile_offsets.push(offs);
    tile_sizes.push((tile.len() * size_of::<u8>()) as u32);
  });

  subframe
    .ifd_mut()
    .add_tag(TiffCommonTag::BitsPerSample, &vec![rawimage.bps as u16; rawimage.cpp]);
  subframe.ifd_mut().add_tag(TiffCommonTag::SampleFormat, &vec![1_u16; rawimage.cpp]);
  subframe.ifd_mut().add_tag(TiffCommonTag::TileOffsets, &tile_offsets);
  subframe.ifd_mut().add_tag(TiffCommonTag::TileByteCounts, &tile_sizes);
  subframe.ifd_mut().add_tag(TiffCommonTag::TileWidth, tile_w as u16);
  subframe.ifd_mut().add_tag(TiffCommonTag::TileLength, tile_h as u16);

  Ok(())
}

/// Write RAW uncompressed into DNG
///
/// This uses unsigned 16 bit values for storage
/// Data is split into multiple strips
fn dng_put_raw_uncompressed<W>(subframe: &mut SubFrameWriter<W>, rawimage: &RawImage) -> Result<()>
where
  W: Write + Seek,
{
  let mut strip_offsets: Vec<u32> = Vec::new();
  let mut strip_sizes: Vec<u32> = Vec::new();
  let mut strip_rows: Vec<u32> = Vec::new();

  let rows_per_strip = if rawimage.height > 1000 { 256 } else { rawimage.height };

  match rawimage.data {
    RawImageData::Integer(ref data) => {
      for strip in data.chunks(rows_per_strip * rawimage.width * rawimage.cpp) {
        let offset = subframe.writer.dng.write_data_u16_le(strip)?;
        strip_offsets.push(offset);
        strip_sizes.push(std::mem::size_of_val(strip) as u32);
        strip_rows.push((strip.len() / (rawimage.width * rawimage.cpp)) as u32);
      }
      subframe.ifd_mut().add_tag(TiffCommonTag::SampleFormat, &vec![1_u16; rawimage.cpp]); // Unsigned Integer
      subframe.ifd_mut().add_tag(TiffCommonTag::BitsPerSample, &vec![16_u16; rawimage.cpp]);
    }
    RawImageData::Float(ref data) => {
      for strip in data.chunks(rows_per_strip * rawimage.width * rawimage.cpp) {
        let offset = subframe.writer.dng.write_data_f32_le(strip)?;
        strip_offsets.push(offset);
        strip_sizes.push(std::mem::size_of_val(strip) as u32);
        strip_rows.push((strip.len() / (rawimage.width * rawimage.cpp)) as u32);
      }
      subframe.ifd_mut().add_tag(TiffCommonTag::SampleFormat, &vec![3_u16; rawimage.cpp]); // Unsigned Integer// IEEE Float
      subframe.ifd_mut().add_tag(TiffCommonTag::BitsPerSample, &vec![32_u16; rawimage.cpp]);
    }
  };
  subframe.ifd_mut().add_tag(TiffCommonTag::StripOffsets, &strip_offsets);
  subframe.ifd_mut().add_tag(TiffCommonTag::StripByteCounts, &strip_sizes);
  subframe.ifd_mut().add_tag(TiffCommonTag::RowsPerStrip, &strip_rows);

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
      rawsource::RawSource,
    };
    use std::{
      fs::File,
      io::{BufReader, BufWriter},
      path::PathBuf,
    };

    let mut rawdb = PathBuf::from(std::env::var("RAWLER_RAWDB").expect("RAWLER_RAWDB variable must be set in order to run RAW test!"));
    rawdb.push("cameras/Canon/EOS R6/raw_modes/Canon EOS R6_RAW_ISO_100_nocrop_nodual.CR3");

    let rawfile = RawSource::new(&rawdb)?;

    let original_thread = std::thread::spawn(|| OriginalCompressed::compress(&mut BufReader::new(File::open(rawdb).unwrap())));

    let decoder = crate::get_decoder(&rawfile)?;

    let rawimage = decoder.raw_image(&rawfile, &RawDecodeParams::default(), false)?;

    let full_image = decoder.full_image(&rawfile, &RawDecodeParams::default())?.unwrap();

    let metadata = decoder.raw_metadata(&rawfile, &RawDecodeParams::default())?;

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

    if let Some(xpacket) = decoder.xpacket(&rawfile, &RawDecodeParams::default())? {
      dng.xpacket(xpacket)?;
    }

    let original = original_thread.join().unwrap()?;

    dng.original_file(&original, "test.CR3")?;

    dng.close()?;

    Ok(())
  }
}
