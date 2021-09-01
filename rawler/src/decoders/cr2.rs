use core::panic;
use std::convert::TryFrom;
use std::f32::NAN;

use image::DynamicImage;

use crate::RawImage;
use crate::alloc_image_plain;
use crate::bits::Endian;
use crate::bits::LookupTable;
use crate::bits::clampbits;
use crate::decoders::*;
use crate::formats::tiff_legacy::*;
use crate::decompressors::ljpeg::*;
use crate::tags::LegacyTiffRootTag;
use crate::tags::TiffTagEnum;

#[derive(Debug, Clone)]
pub struct Cr2Decoder<'a> {
  buffer: &'a [u8],
  rawloader: &'a RawLoader,
  tiff: LegacyTiffIFD<'a>,
  exif: Option<LegacyTiffIFD<'a>>,
  makernotes: Option<LegacyTiffIFD<'a>>,
}

impl<'a> Cr2Decoder<'a> {
  pub fn new(buf: &'a [u8], tiff: LegacyTiffIFD<'a>, rawloader: &'a RawLoader) -> Cr2Decoder<'a> {
    Cr2Decoder {
      buffer: buf,
      tiff: tiff,
      rawloader: rawloader,
      exif: None,
      makernotes: None,
    }
  }
}

impl<'a> Decoder for Cr2Decoder<'a> {

  fn decode_metadata(&mut self) -> Result<()> {
    // TODO: ExifIFD tag should be save and can be moved to general parser?
    let tiff = LegacyTiffIFD::new_file(self.buffer, &vec![LegacyTiffRootTag::ExifIFDPointer.into()])?;
    self.tiff = tiff;

    if let Some(entry) = self.tiff.find_entry(LegacyTiffRootTag::Makernote) {

      let ifd = Self::new_makernote(self.buffer, entry.data_offset(), self.tiff.start_offset, self.tiff.chain_level, self.tiff.get_endian());
      match ifd {
        Ok(val) => {
          println!("{}", dump_ifd_entries::<Cr2MakernoteTag>(&val)); // fmt_with_tags
          self.makernotes = Some(val);

        },
        Err(_) => {
          panic!("Unable to parse makernotes for CR2");
        },
      }
    } else {
      panic!("No makernotes found");
    }

    Ok(())
  }

  fn raw_image(&self, _params: RawDecodeParams, dummy: bool) -> Result<RawImage> {
    let camera = self.rawloader.check_supported(&self.tiff)?;
    let (raw, offset) = {
      if let Some(raw) = self.tiff.find_first_ifd(LegacyTiffRootTag::Cr2Id) {
        (raw, fetch_tag!(raw, LegacyTiffRootTag::StripOffsets).get_usize(0))
      } else if let Some(raw) = self.tiff.find_first_ifd(LegacyTiffRootTag::CFAPattern) {
        (raw, fetch_tag!(raw, LegacyTiffRootTag::StripOffsets).get_usize(0))
      } else if let Some(off) = self.tiff.find_entry(LegacyTiffRootTag::Cr2OldOffset) {
        (&self.tiff, off.get_usize(0))
      } else {
        return Err(RawlerError::General("CR2: Couldn't find raw info".to_string()))
      }
    };
    let src = &self.buffer[offset..];

    let (width, height, cpp, image) = {
      let decompressor = LjpegDecompressor::new(src)?;
      let ljpegwidth = decompressor.width();
      let mut width = ljpegwidth;
      let mut height = decompressor.height();
      let cpp = if decompressor.super_h() == 2 {3} else {1};
      let mut ljpegout = alloc_image_plain!(width, height, dummy);

      decompressor.decode(&mut ljpegout, 0, width, width, height, dummy)?;

      // Linearize the output (applies only to D2000 as far as I can tell)
      if camera.find_hint("linearization") {
        let table = {
          let linearization = fetch_tag!(self.tiff, LegacyTiffRootTag::GrayResponse);
          let mut t = [0 as u16;4096];
          for i in 0..t.len() {
            t[i] = linearization.get_u32(i) as u16;
          }
          LookupTable::new(&t)
        };

        let mut random = ljpegout[0] as u32;
        for o in ljpegout.chunks_exact_mut(1) {
          o[0] = table.dither(o[0], &mut random);
        }
      }

      // Convert the YUV in sRAWs to RGB
      if cpp == 3 {
        self.convert_to_rgb(&camera, &mut ljpegout, dummy)?;
        if raw.has_entry(LegacyTiffRootTag::ImageWidth) {
          width = fetch_tag!(raw, LegacyTiffRootTag::ImageWidth).get_usize(0) * cpp;
          height = fetch_tag!(raw, LegacyTiffRootTag::ImageLength).get_usize(0) ;
        } else if width/cpp < height {
          let temp = width/cpp;
          width = height*cpp;
          height = temp;
        }
      } else if camera.find_hint("double_line") {
        width /= 2;
        height *= 2;
      }

      // Take each of the vertical fields and put them into the right location
      // FIXME: Doing this at the decode would reduce about 5% in runtime but I haven't
      //        been able to do it without hairy code
      if let Some(canoncol) = raw.find_entry(LegacyTiffRootTag::Cr2StripeWidths) {
        if canoncol.get_usize(0) == 0 {
          (width, height, cpp, ljpegout)
        } else {
          let mut out = alloc_image_plain!(width, height, dummy);
          if !dummy {
            let mut fieldwidths = Vec::new();
            for _ in 0..canoncol.get_usize(0) {
              fieldwidths.push(canoncol.get_usize(1));
            }
            fieldwidths.push(canoncol.get_usize(2));

            if decompressor.super_v() == 2 {
              // We've decoded 2 lines at a time so we also need to copy two strips at a time
              let nfields = fieldwidths.len();
              let fieldwidth = fieldwidths[0];
              let mut fieldstart = 0;
              let mut inpos = 0;
              for _ in 0..nfields {
                for row in (0..height).step_by(2) {
                  for col in (0..fieldwidth).step_by(3) {
                    let outpos = row*width+fieldstart+col;
                    out[outpos..outpos+3].copy_from_slice(&ljpegout[inpos..inpos+3]);
                    let outpos = (row+1)*width+fieldstart+col;
                    let inpos2 = inpos+ljpegwidth;
                    out[outpos..outpos+3].copy_from_slice(&ljpegout[inpos2..inpos2+3]);
                    inpos += 3;
                    if inpos % ljpegwidth == 0 {
                      // we've used a full input line and we're reading 2 by 2 so skip one
                      inpos += ljpegwidth;
                    }
                  }
                }
                fieldstart += fieldwidth;
              }
            } else {
              let sh = decompressor.super_h();
              let mut fieldstart = 0;
              let mut fieldpos = 0;
              for fieldwidth in fieldwidths {
                let fieldwidth = fieldwidth/sh*cpp;
                for row in 0..height {
                  let outpos = row*width+fieldstart;
                  let inpos = fieldpos+row*fieldwidth;
                  let outb = &mut out[outpos..outpos+fieldwidth];
                  let inb = &ljpegout[inpos..inpos+fieldwidth];
                  outb.copy_from_slice(inb);
                }
                fieldstart += fieldwidth;
                fieldpos += fieldwidth*height;
              }
            }
          }

          (width, height, cpp, out)
        }
      } else {
        (width, height, cpp, ljpegout)
      }
    };

    let wb = self.get_wb(&camera)?;
    let mut img = RawImage::new(camera, width, height, wb, image, dummy);
    if cpp == 3 {
      img.cpp = 3;
      img.width /= 3;
      img.crops = [0,0,0,0];
      img.blacklevels = [0,0,0,0];
      img.whitelevels = [65535,65535,65535,65535];
    }
    Ok(img)
  }


  fn full_image(&self) -> Result<DynamicImage> {
    // For CR2, there is a full resolution image in IFD0.
    // This is compressed with old-JPEG compression (Compression = 6)
    let root_ifd = &self.tiff.chained_ifds[0];
    let buf = root_ifd.singlestrip_data().unwrap();
    let img = image::load_from_memory_with_format(buf, image::ImageFormat::Jpeg).unwrap();
    Ok(img)
  }
}

impl<'a> Cr2Decoder<'a> {
  pub fn new_makernote(buf: &'a[u8], offset: usize, base_offset: usize, chain_level: isize, e: Endian) -> Result<LegacyTiffIFD<'a>> {
    let mut off = 0;
    let data = &buf[offset..];
    let mut endian = e;

    // Some have MM or II to indicate endianness - read that
    if data[off..off+2] == b"II"[..] {
      off +=2;
      endian = Endian::Little;
    } if data[off..off+2] == b"MM"[..] {
      off +=2;
      endian = Endian::Big;
    }

    Ok(LegacyTiffIFD::new(buf, offset+off, base_offset, 0, chain_level+1, endian, &vec![])?)
  }


  fn get_wb(&self, cam: &Camera) -> Result<[f32;4]> {
    if let Some(makernotes) = self.makernotes.as_ref() {
      if let Some(levels) = makernotes.find_entry(Cr2MakernoteTag::ColorData) {
        let offset = if cam.wb_offset != 0 {cam.wb_offset} else {63};
        return Ok([levels.get_force_u16(offset) as f32, levels.get_force_u16(offset+1) as f32,
            levels.get_force_u16(offset+3) as f32, NAN]);
      }
    }
    // TODO: check if these tags belongs to RootIFD or makernote
    if let Some(levels) = self.tiff.find_entry(LegacyTiffRootTag::Cr2PowerShotWB) {
      Ok([levels.get_force_u32(3) as f32, levels.get_force_u32(2) as f32,
          levels.get_force_u32(4) as f32, NAN])
    } else if let Some(levels) = self.tiff.find_entry(LegacyTiffRootTag::Cr2OldWB) {
      Ok([levels.get_f32(0), levels.get_f32(1), levels.get_f32(2), NAN])
    } else {
      // At least the D2000 has no WB
      Ok([NAN,NAN,NAN,NAN])
    }
  }

  fn convert_to_rgb(&self, cam: &Camera, image: &mut [u16], dummy: bool) -> Result<()>{
    let coeffs = self.get_wb(cam)?;
    if dummy {
      return Ok(())
    }

    let c1 = (1024.0*1024.0/coeffs[0]) as i32;
    let c2 = coeffs[1] as i32;
    let c3 = (1024.0*1024.0/coeffs[2]) as i32;

    let yoffset = if cam.find_hint("40d_yuv") { 512 } else { 0 };

    for pix in image.chunks_exact_mut(3) {
      let y = pix[0] as i32 - yoffset;
      let cb = pix[1] as i32 - 16383;
      let cr = pix[2] as i32 - 16383;

      let r = c1 * (y + cr);
      let g = c2 * (y + ((-778*cb - (cr<<11)) >> 12));
      let b = c3 * (y + cb);

      pix[0] = clampbits(r >> 8, 16);
      pix[1] = clampbits(g >> 8, 16);
      pix[2] = clampbits(b >> 8, 16);
    }
    Ok(())
  }
}




#[derive(Debug, Copy, Clone, PartialEq, enumn::N)]
#[repr(u16)]
pub enum Cr2MakernoteTag {
  CameraSettings   = 0x0001,
  FocusInfo   = 0x0002,
  FlashInfo   = 0x0003,
  ShotInfo    = 0x0004,
  Panorama    = 0x0005,
  ImageType   = 0x0006,
  FirmareVer  = 0x0007,
  FileNumber  = 0x0008,
  OwnerName   = 0x0009,
  UnknownD30  = 0x000a,
  SerialNum   = 0x000c,
  CameraInfo  = 0x000d,
  FileLen     = 0x000e,
  CustomFunc  = 0x000f,
  ModelId     = 0x0010,
  MovieInfo   = 0x0011,
  AFInfo      = 0x0012,
  ThumbArea   = 0x0013,
  SerialFormat = 0x0014,
  SuperMacro    = 0x001a,
  DateStampMode = 0x001c,
  MyColors      = 0x001d,
  FirmwareRev   = 0x001e,
  Categories    = 0x0023,
  FaceDetect1   = 0x0024,
  FaceDetect2   = 0x0025,
  AFInfo2       = 0x0026,
  ContrastInfo  = 0x0027,
  ImgUniqueID   = 0x0028,
  WBInfo        = 0x0029,
  FaceDetect3   = 0x002f,
  TimeInfo      = 0x0035,
  BatteryType   = 0x0038,
  AFInfo3       = 0x003c,
  RawDataOffset = 0x0081,
  OrigDecisionDataOffset = 0x0083,
  CustomFunc1D  = 0x0090,
  PersFunc  = 0x0091,
  PersFuncValues  = 0x0092,
  FileInfo    = 0x0093,
  AFPointsInFocus1D = 0x0094,
  LensModel   = 0x0095,
  InternalSerial = 0x0096,
  DustRemovalData = 0x0097,
  CropInfo = 0x0098,
  CustomFunc2 = 0x0099,
  AspectInfo  = 0x009a,
  ProcessingInfo  = 0x00a0,
  ToneCurveTable = 0x00a1,
  SharpnessTable = 0x00a2,
  SharpnessFreqTable = 0x00a3,
  WhiteBalanceTable = 0x00a4,
  ColorBalance  = 0x00a9,
  MeasuredColor = 0x00aa,
  ColorTemp     = 0x00ae,
  CanonFlags    = 0x00b0,
  ModifiedInfo  = 0x00b1,
  TnoeCurveMatching = 0x00b2,
  WhiteBalanceMatching = 0x00b3,
  ColorSpace   = 0x00b4,
  PreviewImageInfo = 0x00b6,
  VRDOffset = 0x00d0,
  SensorInfo  = 0x00e0,
  ColorData     = 0x4001,
  CRWParam  = 0x4002,
  ColorInfo = 0x4003,
  Flavor = 0x4005,
  PictureStyleUserDef = 0x4008,
  PictureStylePC = 0x4009,
  CustomPictureStyleFileName = 0x4010,
  AFMicroAdj = 0x4013,
  VignettingCorr = 0x4015,
  VignettingCorr2 = 0x4016,
  LightningOpt = 0x4018,
  LensInfo = 0x4019,
  AmbienceInfo = 0x4020,
  MultiExp = 0x4021,
  FilterInfo = 0x4024,
  HDRInfo = 0x4025,
  AFConfig = 0x4028,
  RawBurstModeRoll = 0x403f,
}

impl Into<u16> for Cr2MakernoteTag {
    fn into(self) -> u16 {
        self as u16
    }
}

impl TryFrom<u16> for Cr2MakernoteTag {
  type Error = String;

  fn try_from(value: u16) -> std::result::Result<Self, Self::Error> {
      Self::n(value).ok_or(format!("Unable to convert tag: {}, not defined in enum", value))
  }
}

impl TiffTagEnum for Cr2MakernoteTag {}
