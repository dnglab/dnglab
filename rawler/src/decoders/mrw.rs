use crate::analyze::FormatDump;
use crate::bits::BEu16;
use crate::bits::BEu32;
use crate::exif::Exif;
use crate::formats::tiff::IFD;
use crate::packed::decode_12be;
use crate::packed::decode_12be_unpacked;
use crate::RawFile;
use crate::RawImage;
use crate::RawLoader;
use crate::Result;
use std::io::Cursor;

use super::ok_cfa_image;
use super::Camera;
use super::Decoder;
use super::RawDecodeParams;
use super::RawMetadata;

const MRW_MAGIC: u32 = 0x004D524D;

pub fn is_mrw(file: &mut RawFile) -> bool {
  match file.subview(0, 4) {
    Ok(buf) => BEu32(&buf, 0) == MRW_MAGIC,
    Err(_) => false,
  }
}

#[derive(Debug, Clone)]
pub struct MrwDecoder<'a> {
  #[allow(unused)]
  rawloader: &'a RawLoader,
  data_offset: usize,
  raw_width: usize,
  raw_height: usize,
  packed: bool,
  wb_vals: [u16; 4],
  #[allow(unused)]
  //tiff: GenericTiffReader,
  tiff: IFD,
  camera: Camera,
}

impl<'a> MrwDecoder<'a> {
  pub fn new(file: &mut RawFile, rawloader: &'a RawLoader) -> Result<MrwDecoder<'a>> {
    let full = file.as_vec().unwrap();
    let buf = &full;
    let data_offset: usize = (BEu32(buf, 4) + 8) as usize;
    let mut raw_height: usize = 0;
    let mut raw_width: usize = 0;
    let mut packed = false;
    let mut wb_vals: [u16; 4] = [0; 4];
    let mut tiffpos: usize = 0;

    let mut currpos: usize = 8;
    // At most we read 20 bytes from currpos so check we don't step outside that
    while currpos + 20 < data_offset {
      let tag: u32 = BEu32(buf, currpos);
      let len: u32 = BEu32(buf, currpos + 4);

      match tag {
        0x505244 => {
          // PRD
          raw_height = BEu16(buf, currpos + 16) as usize;
          raw_width = BEu16(buf, currpos + 18) as usize;
          packed = buf[currpos + 24] == 12;
        }
        0x574247 => {
          // WBG
          for i in 0..4 {
            wb_vals[i] = BEu16(buf, currpos + 12 + i * 2);
          }
        }
        0x545457 => {
          // TTW
          // Base value for offsets needs to be at the beginning of the
          // TIFF block, not the file
          tiffpos = currpos + 8;
        }
        _ => {}
      }
      currpos += (len + 8) as usize;
    }

    let tiff_data = file.subview_until_eof(tiffpos as u64).unwrap();
    let tiff = IFD::new(&mut Cursor::new(tiff_data), 8, 0, 0, crate::bits::Endian::Big, &[]).unwrap();

    let camera = rawloader.check_supported(&tiff)?;

    Ok(MrwDecoder {
      data_offset,
      raw_width,
      raw_height,
      packed,
      wb_vals,
      tiff,
      rawloader,
      camera,
    })
  }
}

impl<'a> Decoder for MrwDecoder<'a> {
  fn raw_image(&self, file: &mut RawFile, _params: RawDecodeParams, dummy: bool) -> Result<RawImage> {
    let src = file.subview_until_eof(self.data_offset as u64).unwrap();

    let buffer = if self.packed {
      decode_12be(&src, self.raw_width, self.raw_height, dummy)
    } else {
      decode_12be_unpacked(&src, self.raw_width, self.raw_height, dummy)
    };

    let wb_coeffs = if self.camera.find_hint("swapped_wb") {
      [self.wb_vals[2] as f32, self.wb_vals[0] as f32, self.wb_vals[0] as f32, self.wb_vals[1] as f32]
    } else {
      [self.wb_vals[0] as f32, self.wb_vals[1] as f32, self.wb_vals[2] as f32, self.wb_vals[3] as f32]
    };
    let cpp = 1;
    ok_cfa_image(self.camera.clone(), cpp, normalize_wb(wb_coeffs), buffer, dummy)
  }

  fn format_dump(&self) -> FormatDump {
    todo!()
  }

  fn raw_metadata(&self, _file: &mut RawFile, _params: RawDecodeParams) -> Result<RawMetadata> {
    let exif = Exif::new(&self.tiff)?;
    let mdata = RawMetadata::new(&self.camera, exif);
    Ok(mdata)
  }
}

fn normalize_wb(raw_wb: [f32; 4]) -> [f32; 4] {
  log::debug!("MRW raw wb: {:?}", raw_wb);
  let div = raw_wb[1]; // G1 should be 1024 and we use this as divisor
  let mut norm = raw_wb;
  norm.iter_mut().for_each(|v| {
    if v.is_normal() {
      *v /= div
    }
  });
  [norm[0], (norm[1] + norm[2]) / 2.0, norm[3], f32::NAN]
}
