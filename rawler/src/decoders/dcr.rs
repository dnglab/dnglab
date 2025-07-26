use std::cmp;

use crate::RawImage;
use crate::RawLoader;
use crate::RawlerError;
use crate::Result;
use crate::alloc_image;
use crate::analyze::FormatDump;
use crate::bits::BEu16;
use crate::bits::Endian;
use crate::bits::LookupTable;
use crate::exif::Exif;
use crate::formats::tiff::GenericTiffReader;
use crate::formats::tiff::IFD;
use crate::formats::tiff::reader::TiffReader;
use crate::pixarray::PixU16;
use crate::pumps::ByteStream;
use crate::rawsource::RawSource;
use crate::tags::TiffCommonTag;

use super::Camera;
use super::Decoder;
use super::FormatHint;
use super::RawDecodeParams;
use super::RawMetadata;
use super::ok_cfa_image;

#[derive(Debug, Clone)]
pub struct DcrDecoder<'a> {
  #[allow(unused)]
  rawloader: &'a RawLoader,
  tiff: GenericTiffReader,
  makernote: IFD,
  camera: Camera,
}

impl<'a> DcrDecoder<'a> {
  pub fn new(file: &RawSource, tiff: GenericTiffReader, rawloader: &'a RawLoader) -> Result<DcrDecoder<'a>> {
    let camera = rawloader.check_supported(tiff.root_ifd())?;

    let kodak_ifd = fetch_tiff_tag!(tiff, TiffCommonTag::KodakIFD);
    let makernote = IFD::new(&mut file.reader(), kodak_ifd.force_u32(0), 0, 0, tiff.get_endian(), &[])?;

    Ok(DcrDecoder {
      tiff,
      rawloader,
      camera,
      makernote,
    })
  }
}

impl<'a> Decoder for DcrDecoder<'a> {
  fn raw_image(&self, file: &RawSource, _params: &RawDecodeParams, dummy: bool) -> Result<RawImage> {
    let raw = self
      .tiff
      .find_first_ifd_with_tag(TiffCommonTag::CFAPattern)
      .ok_or_else(|| RawlerError::DecoderFailed(format!("Failed to find a IFD with CFAPattern tag")))?;
    let width = fetch_tiff_tag!(raw, TiffCommonTag::ImageWidth).force_usize(0);
    let height = fetch_tiff_tag!(raw, TiffCommonTag::ImageLength).force_usize(0);
    let offset = fetch_tiff_tag!(raw, TiffCommonTag::StripOffsets).force_usize(0);

    let src = file.subview_until_eof_padded(offset as u64)?; // TODO add size and check all samples

    let linearization = fetch_tiff_tag!(self.makernote, TiffCommonTag::DcrLinearization);
    let curve = {
      let mut points = Vec::new();
      for i in 0..linearization.count() {
        points.push(linearization.force_u32(i) as u16);
      }
      LookupTable::new(&points)
    };

    let image = DcrDecoder::decode_kodak65000(&src, &curve, width, height, dummy);

    let cpp = 1;
    ok_cfa_image(self.camera.clone(), cpp, self.get_wb()?, image, dummy)
  }

  fn format_dump(&self) -> FormatDump {
    todo!()
  }

  fn raw_metadata(&self, _file: &RawSource, __params: &RawDecodeParams) -> Result<RawMetadata> {
    let exif = Exif::new(self.tiff.root_ifd())?;
    let mdata = RawMetadata::new(&self.camera, exif);
    Ok(mdata)
  }

  fn format_hint(&self) -> FormatHint {
    FormatHint::DCR
  }
}

impl<'a> DcrDecoder<'a> {
  fn get_wb(&self) -> Result<[f32; 4]> {
    let dcrwb = fetch_tiff_tag!(self.makernote, TiffCommonTag::DcrWB);
    if dcrwb.count() >= 46 {
      let levels = dcrwb.get_data();
      Ok([
        2048.0 / BEu16(levels, 40) as f32,
        2048.0 / BEu16(levels, 42) as f32,
        2048.0 / BEu16(levels, 44) as f32,
        f32::NAN,
      ])
    } else {
      Ok([f32::NAN, f32::NAN, f32::NAN, f32::NAN])
    }
  }

  pub(crate) fn decode_kodak65000(buf: &[u8], curve: &LookupTable, width: usize, height: usize, dummy: bool) -> PixU16 {
    let mut out = alloc_image!(width, height, dummy);
    let mut input = ByteStream::new(buf, Endian::Little);

    let mut random: u32 = 0;
    for row in 0..height {
      for col in (0..width).step_by(256) {
        let mut pred: [i32; 2] = [0; 2];
        let buf = DcrDecoder::decode_segment(&mut input, cmp::min(256, width - col));
        for (i, val) in buf.iter().enumerate() {
          pred[i & 1] += *val;
          if pred[i & 1] < 0 {
            panic!("Found a negative pixel!");
          }
          out[row * width + col + i] = curve.dither(pred[i & 1] as u16, &mut random);
        }
      }
    }

    out
  }

  fn decode_segment(input: &mut ByteStream, size: usize) -> Vec<i32> {
    let mut out: Vec<i32> = vec![0; size];

    let mut lens: [usize; 256] = [0; 256];
    for i in (0..size).step_by(2) {
      lens[i] = (input.peek_u8() & 15) as usize;
      lens[i + 1] = (input.get_u8() >> 4) as usize;
    }

    let mut bitbuf: u64 = 0;
    let mut bits: usize = 0;
    if (size & 7) == 4 {
      bitbuf = ((input.get_u8() as u64) << 8) | (input.get_u8() as u64);
      bits = 16;
    }

    for i in 0..size {
      let len = lens[i];
      if bits < len {
        for j in (0..32).step_by(8) {
          bitbuf += (input.get_u8() as u64) << (bits + (j ^ 8));
        }
        bits += 32;
      }
      out[i] = (bitbuf & (0xffff >> (16 - len))) as i32;
      bitbuf >>= len;
      bits -= len;
      if len != 0 && (out[i] & (1 << (len - 1))) == 0 {
        out[i] -= (1 << len) - 1;
      }
    }

    out
  }
}
