use std::f32::NAN;

use crate::analyze::FormatDump;
use crate::bits::LEu16;
use crate::decoders::decode_threaded_multiline;
use crate::exif::Exif;
use crate::formats::tiff::reader::TiffReader;
use crate::formats::tiff::GenericTiffReader;
use crate::packed::decode_12le_unpacked_left_aligned;
use crate::packed::decode_12le_wcontrol;
use crate::pixarray::PixU16;
use crate::pumps::BitPump;
use crate::tags::TiffCommonTag;
use crate::OptBuffer;
use crate::RawFile;
use crate::RawImage;
use crate::RawLoader;
use crate::RawlerError;
use crate::Result;

use super::ok_image;
use super::Camera;
use super::Decoder;
use super::RawDecodeParams;
use super::RawMetadata;

#[derive(Debug, Clone)]
pub struct Rw2Decoder<'a> {
  #[allow(unused)]
  rawloader: &'a RawLoader,
  tiff: GenericTiffReader,
  camera: Camera,
}

impl<'a> Rw2Decoder<'a> {
  pub fn new(_file: &mut RawFile, tiff: GenericTiffReader, rawloader: &'a RawLoader) -> Result<Rw2Decoder<'a>> {
    let width;
    let height;
    let data = tiff.find_ifds_with_tag(TiffCommonTag::PanaOffsets);
    if !data.is_empty() {
      let raw = data[0];
      width = fetch_tiff_tag!(raw, TiffCommonTag::PanaWidth).force_usize(0);
      height = fetch_tiff_tag!(raw, TiffCommonTag::PanaLength).force_usize(0);
    } else {
      let raw = tiff.find_first_ifd_with_tag(TiffCommonTag::StripOffsets).unwrap();
      width = fetch_tiff_tag!(raw, TiffCommonTag::PanaWidth).force_usize(0);
      height = fetch_tiff_tag!(raw, TiffCommonTag::PanaLength).force_usize(0);
    }

    let mode = {
      let ratio = width * 100 / height;
      if ratio < 125 {
        "1:1"
      } else if ratio < 145 {
        "4:3"
      } else if ratio < 165 {
        "3:2"
      } else {
        "16:9"
      }
    };
    let camera = rawloader.check_supported_with_mode(tiff.root_ifd(), mode)?;

    Ok(Rw2Decoder { rawloader, tiff, camera })
  }
}

impl<'a> Decoder for Rw2Decoder<'a> {
  fn raw_image(&self, file: &mut RawFile, _params: RawDecodeParams, dummy: bool) -> Result<RawImage> {
    let width;
    let height;
    let image = {
      let data = self.tiff.find_ifds_with_tag(TiffCommonTag::PanaOffsets);
      if !data.is_empty() {
        let raw = data[0];
        width = fetch_tiff_tag!(raw, TiffCommonTag::PanaWidth).force_usize(0);
        height = fetch_tiff_tag!(raw, TiffCommonTag::PanaLength).force_usize(0);
        let offset = fetch_tiff_tag!(raw, TiffCommonTag::PanaOffsets).force_usize(0);
        let src: OptBuffer = file.subview_until_eof(offset as u64).unwrap().into(); // TODO add size and check all samples
        Rw2Decoder::decode_panasonic(&src, width, height, true, dummy)
      } else {
        let raw = self.tiff.find_first_ifd_with_tag(TiffCommonTag::StripOffsets).unwrap();
        width = fetch_tiff_tag!(raw, TiffCommonTag::PanaWidth).force_usize(0);
        height = fetch_tiff_tag!(raw, TiffCommonTag::PanaLength).force_usize(0);
        let offset = fetch_tiff_tag!(raw, TiffCommonTag::StripOffsets).force_usize(0);
        let src: OptBuffer = file.subview_until_eof(offset as u64).unwrap().into(); // TODO add size and check all samples

        if src.len() >= width * height * 2 {
          decode_12le_unpacked_left_aligned(&src, width, height, dummy)
        } else if src.len() >= width * height * 3 / 2 {
          decode_12le_wcontrol(&src, width, height, dummy)
        } else {
          Rw2Decoder::decode_panasonic(&src, width, height, false, dummy)
        }
      }
    };

    let cpp = 1;
    ok_image(self.camera.clone(), width, height, cpp, self.get_wb()?, image.into_inner())
  }

  fn format_dump(&self) -> FormatDump {
    todo!()
  }

  fn raw_metadata(&self, _file: &mut RawFile, _params: RawDecodeParams) -> Result<RawMetadata> {
    let exif = Exif::new(self.tiff.root_ifd())?;
    let mdata = RawMetadata::new(&self.camera, exif);
    Ok(mdata)
  }
}

impl<'a> Rw2Decoder<'a> {
  fn get_wb(&self) -> Result<[f32; 4]> {
    if self.tiff.has_entry(TiffCommonTag::PanaWBsR) && self.tiff.has_entry(TiffCommonTag::PanaWBsB) {
      let r = fetch_tiff_tag!(self.tiff, TiffCommonTag::PanaWBsR).force_u32(0) as f32;
      let b = fetch_tiff_tag!(self.tiff, TiffCommonTag::PanaWBsB).force_u32(0) as f32;
      Ok([r, 256.0, b, NAN])
    } else if self.tiff.has_entry(TiffCommonTag::PanaWBs2R) && self.tiff.has_entry(TiffCommonTag::PanaWBs2G) && self.tiff.has_entry(TiffCommonTag::PanaWBs2B) {
      let r = fetch_tiff_tag!(self.tiff, TiffCommonTag::PanaWBs2R).force_u32(0) as f32;
      let g = fetch_tiff_tag!(self.tiff, TiffCommonTag::PanaWBs2G).force_u32(0) as f32;
      let b = fetch_tiff_tag!(self.tiff, TiffCommonTag::PanaWBs2B).force_u32(0) as f32;
      Ok([r, g, b, NAN])
    } else {
      Err(RawlerError::General("Couldn't find WB".to_string()))
    }
  }

  pub(crate) fn decode_panasonic(buf: &[u8], width: usize, height: usize, split: bool, dummy: bool) -> PixU16 {
    decode_threaded_multiline(
      width,
      height,
      5,
      dummy,
      &(|out: &mut [u16], row| {
        let skip = ((width * row * 9) + (width / 14 * 2 * row)) / 8;
        let blocks = skip / 0x4000;
        let src = &buf[blocks * 0x4000..];
        let mut pump = BitPumpPanasonic::new(src, split);
        for _ in 0..(skip % 0x4000) {
          pump.get_bits(8);
        }

        let mut sh: i32 = 0;
        for out in out.chunks_exact_mut(14) {
          let mut pred: [i32; 2] = [0, 0];
          let mut nonz: [i32; 2] = [0, 0];

          for i in 0..14 {
            if (i % 3) == 2 {
              sh = 4 >> (3 - pump.get_bits(2));
            }
            if nonz[i & 1] != 0 {
              let j = pump.get_bits(8) as i32;
              if j != 0 {
                pred[i & 1] -= 0x80 << sh;
                if pred[i & 1] < 0 || sh == 4 {
                  pred[i & 1] &= !(-1 << sh);
                }
                pred[i & 1] += j << sh;
              }
            } else {
              nonz[i & 1] = pump.get_bits(8) as i32;
              if nonz[i & 1] != 0 || i > 11 {
                pred[i & 1] = nonz[i & 1] << 4 | (pump.get_bits(4) as i32);
              }
            }
            out[i] = pred[i & 1] as u16;
          }
        }
      }),
    )
  }
}

pub struct BitPumpPanasonic<'a> {
  buffer: &'a [u8],
  pos: usize,
  nbits: u32,
  split: bool,
}

impl<'a> BitPumpPanasonic<'a> {
  pub fn new(src: &'a [u8], split: bool) -> BitPumpPanasonic {
    BitPumpPanasonic {
      buffer: src,
      pos: 0,
      nbits: 0,
      split,
    }
  }
}

impl<'a> BitPump for BitPumpPanasonic<'a> {
  fn peek_bits(&mut self, num: u32) -> u32 {
    if num > self.nbits {
      self.nbits += 0x4000 * 8;
      self.pos += 0x4000;
    }
    let mut byte = (self.nbits - num) >> 3 ^ 0x3ff0;
    if self.split {
      byte = (byte + 0x4000 - 0x2008) % 0x4000;
    }
    let bits = LEu16(self.buffer, byte as usize + self.pos - 0x4000) as u32;
    (bits >> ((self.nbits - num) & 7)) & (0x0ffffffffu32 >> (32 - num))
  }

  fn consume_bits(&mut self, num: u32) {
    self.nbits -= num;
  }
}
