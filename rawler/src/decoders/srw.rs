use log::warn;
use std::cmp;

use crate::RawImage;
use crate::RawLoader;
use crate::RawlerError;
use crate::Result;
use crate::alloc_image;
use crate::analyze::FormatDump;
use crate::bits::LEu32;
use crate::bits::clampbits;
use crate::exif::Exif;
use crate::formats::tiff::GenericTiffReader;
use crate::formats::tiff::IFD;
use crate::formats::tiff::ifd::OffsetMode;
use crate::formats::tiff::reader::TiffReader;
use crate::lens::LensDescription;
use crate::lens::LensResolver;
use crate::packed::decode_12be;
use crate::packed::decode_12le;
use crate::packed::decode_12le_unpacked;
use crate::packed::decode_14le_unpacked;
use crate::pixarray::PixU16;
use crate::pumps::BitPump;
use crate::pumps::BitPumpMSB;
use crate::pumps::BitPumpMSB32;
use crate::rawsource::RawSource;
use crate::tags::ExifTag;
use crate::tags::TiffCommonTag;

use super::Camera;
use super::Decoder;
use super::FormatHint;
use super::RawDecodeParams;
use super::RawMetadata;
use super::ok_cfa_image_with_blacklevels;

const NX_MOUNT: &str = "NX-mount";

#[derive(Debug, Clone)]
pub struct SrwDecoder<'a> {
  #[allow(unused)]
  rawloader: &'a RawLoader,
  tiff: GenericTiffReader,
  makernote: IFD,
  camera: Camera,
}

impl<'a> SrwDecoder<'a> {
  pub fn new(file: &RawSource, tiff: GenericTiffReader, rawloader: &'a RawLoader) -> Result<SrwDecoder<'a>> {
    let camera = rawloader.check_supported(tiff.root_ifd())?;

    let makernote = if let Some(exif) = tiff.find_first_ifd_with_tag(ExifTag::MakerNotes) {
      exif.parse_makernote(&mut file.reader(), OffsetMode::RelativeToIFD, &[])?
    } else {
      warn!("SRW makernote not found");
      None
    }
    .ok_or("File has not makernotes")?;

    Ok(SrwDecoder {
      tiff,
      rawloader,
      camera,
      makernote,
    })
  }
}

impl<'a> Decoder for SrwDecoder<'a> {
  fn raw_image(&self, file: &RawSource, _params: &RawDecodeParams, dummy: bool) -> Result<RawImage> {
    let raw = self
      .tiff
      .find_first_ifd_with_tag(TiffCommonTag::StripOffsets)
      .ok_or_else(|| RawlerError::DecoderFailed(format!("Failed to find a IFD with StripOffsets tag")))?;
    let width = fetch_tiff_tag!(raw, TiffCommonTag::ImageWidth).force_usize(0);
    let height = fetch_tiff_tag!(raw, TiffCommonTag::ImageLength).force_usize(0);
    let offset = fetch_tiff_tag!(raw, TiffCommonTag::StripOffsets).force_usize(0);
    let compression = fetch_tiff_tag!(raw, TiffCommonTag::Compression).force_u32(0);
    let bits = fetch_tiff_tag!(raw, TiffCommonTag::BitsPerSample).force_u32(0);
    let src = file.subview_until_eof_padded(offset as u64)?;

    let image = match compression {
      32769 => match bits {
        12 => decode_12le_unpacked(&src, width, height, dummy),
        14 => decode_14le_unpacked(&src, width, height, dummy),
        x => return Err(RawlerError::unsupported(&self.camera, format!("SRW: Don't know how to handle bps {}", x))),
      },
      32770 => match raw.get_entry(TiffCommonTag::SrwSensorAreas) {
        None => match bits {
          12 => {
            if self.camera.find_hint("little_endian") {
              decode_12le(&src, width, height, dummy)
            } else {
              decode_12be(&src, width, height, dummy)
            }
          }
          14 => decode_14le_unpacked(&src, width, height, dummy),
          x => return Err(RawlerError::unsupported(&self.camera, format!("SRW: Don't know how to handle bps {}", x))),
        },
        Some(x) => {
          let coffset = x.force_usize(0);
          assert!(coffset > 0, "Surely this can't be the start of the file");
          let loffsets = file.subview_until_eof(coffset as u64)?;
          SrwDecoder::decode_srw1(&src, loffsets, width, height, dummy)
        }
      },
      32772 => SrwDecoder::decode_srw2(&src, width, height, dummy),
      32773 => SrwDecoder::decode_srw3(&src, width, height, dummy),
      x => {
        return Err(RawlerError::unsupported(
          &self.camera,
          format!("SRW: Don't know how to handle compression {}", x),
        ));
      }
    };
    let cpp = 1;
    ok_cfa_image_with_blacklevels(self.camera.clone(), cpp, self.get_wb()?, self.get_blacklevel()?, image, dummy)
  }

  fn format_dump(&self) -> FormatDump {
    todo!()
  }

  fn raw_metadata(&self, _file: &RawSource, _params: &RawDecodeParams) -> Result<RawMetadata> {
    let exif = Exif::new(self.tiff.root_ifd())?;
    let mdata = RawMetadata::new_with_lens(&self.camera, exif, self.get_lens_description()?.cloned());
    Ok(mdata)
  }

  fn format_hint(&self) -> FormatHint {
    FormatHint::SRW
  }
}

impl<'a> SrwDecoder<'a> {
  pub fn decode_srw1(buf: &[u8], loffsets: &[u8], width: usize, height: usize, dummy: bool) -> PixU16 {
    let mut out = alloc_image!(width, height, dummy);

    for row in 0..height {
      let mut len: [u32; 4] = [if row < 2 { 7 } else { 4 }; 4];
      let loffset = LEu32(loffsets, row * 4) as usize;
      let mut pump = BitPumpMSB32::new(&buf[loffset..]);

      let img = width * row;
      let img_up = width * (cmp::max(1, row) - 1);
      let img_up2 = width * (cmp::max(2, row) - 2);

      // Image is arranged in groups of 16 pixels horizontally
      for col in (0..width).step_by(16) {
        let dir = pump.get_bits(1) == 1;

        let ops = [pump.get_bits(2), pump.get_bits(2), pump.get_bits(2), pump.get_bits(2)];
        for (i, op) in ops.iter().enumerate() {
          match *op {
            3 => len[i] = pump.get_bits(4),
            2 => len[i] -= 1,
            1 => len[i] += 1,
            _ => {}
          }
        }

        // First decode even pixels
        for c in (0..16).step_by(2) {
          let l = len[c >> 3];
          let adj = pump.get_ibits_sextended(l);
          let predictor = if dir {
            // Upward prediction
            out[img_up + col + c]
          } else {
            // Left to right prediction
            if col == 0 { 128 } else { out[img + col - 2] }
          };
          if col + c < width {
            // No point in decoding pixels outside the image
            out[img + col + c] = ((predictor as i32) + adj) as u16;
          }
        }
        // Now decode odd pixels
        for c in (1..16).step_by(2) {
          let l = len[2 | (c >> 3)];
          let adj = pump.get_ibits_sextended(l);
          let predictor = if dir {
            // Upward prediction
            out[img_up2 + col + c]
          } else {
            // Left to right prediction
            if col == 0 { 128 } else { out[img + col - 1] }
          };
          if col + c < width {
            // No point in decoding pixels outside the image
            out[img + col + c] = ((predictor as i32) + adj) as u16;
          }
        }
      }
    }

    // SRW1 apparently has red and blue swapped, just changing the CFA pattern to
    // match causes color fringing in high contrast areas because the actual pixel
    // locations would not match the CFA pattern
    for row in (0..height).step_by(2) {
      for col in (0..width).step_by(2) {
        out.pixels_mut().swap(row * width + col + 1, (row + 1) * width + col);
      }
    }

    out
  }

  pub fn decode_srw2(buf: &[u8], width: usize, height: usize, dummy: bool) -> PixU16 {
    let mut out = alloc_image!(width, height, dummy);

    // This format has a variable length encoding of how many bits are needed
    // to encode the difference between pixels, we use a table to process it
    // that has two values, the first the number of bits that were used to
    // encode, the second the number of bits that come after with the difference
    // The table has 14 entries because the difference can have between 0 (no
    // difference) and 13 bits (differences between 12 bits numbers can need 13)
    let tab: [[u32; 2]; 14] = [
      [3, 4],
      [3, 7],
      [2, 6],
      [2, 5],
      [4, 3],
      [6, 0],
      [7, 9],
      [8, 10],
      [9, 11],
      [10, 12],
      [10, 13],
      [5, 1],
      [4, 8],
      [4, 2],
    ];

    // We generate a 1024 entry table (to be addressed by reading 10 bits) by
    // consecutively filling in 2^(10-N) positions where N is the variable number of
    // bits of the encoding. So for example 4 is encoded with 3 bits so the first
    // 2^(10-3)=128 positions are set with 3,4 so that any time we read 000 we
    // know the next 4 bits are the difference. We read 10 bits because that is
    // the maximum number of bits used in the variable encoding (for the 12 and
    // 13 cases)
    let mut tbl: [[u32; 2]; 1024] = [[0, 0]; 1024];
    let mut n: usize = 0;
    for i in 0..14 {
      let mut c = 0;
      while c < (1024 >> tab[i][0]) {
        tbl[n][0] = tab[i][0];
        tbl[n][1] = tab[i][1];
        n += 1;
        c += 1;
      }
    }

    let mut vpred: [[i32; 2]; 2] = [[0, 0], [0, 0]];
    let mut hpred: [i32; 2] = [0, 0];
    let mut pump = BitPumpMSB::new(buf);
    for row in 0..height {
      for col in 0..width {
        let diff = SrwDecoder::srw2_diff(&mut pump, &tbl);
        if col < 2 {
          vpred[row & 1][col] += diff;
          hpred[col] = vpred[row & 1][col];
        } else {
          hpred[col & 1] += diff;
        }
        out[row * width + col] = hpred[col & 1] as u16;
      }
    }

    out
  }

  pub fn srw2_diff(pump: &mut BitPumpMSB, tbl: &[[u32; 2]; 1024]) -> i32 {
    // We read 10 bits to index into our table
    let c = pump.peek_bits(10);
    // Skip the bits that were used to encode this case
    pump.consume_bits(tbl[c as usize][0]);
    // Read the number of bits the table tells me
    let len = tbl[c as usize][1];
    let mut diff = pump.get_bits(len) as i32;
    // If the first bit is 0 we need to turn this into a negative number
    if len != 0 && (diff & (1 << (len - 1))) == 0 {
      diff -= (1 << len) - 1;
    }
    diff
  }

  pub fn decode_srw3(buf: &[u8], width: usize, height: usize, dummy: bool) -> PixU16 {
    // Decoder for third generation compressed SRW files (NX1)
    // Seriously Samsung just use lossless jpeg already, it compresses better too :)

    // Thanks to Michael Reichmann (Luminous Landscape) for putting me in contact
    // and Loring von Palleske (Samsung) for pointing to the open-source code of
    // Samsung's DNG converter at http://opensource.samsung.com/

    let mut out = alloc_image!(width, height, dummy);
    let mut pump = BitPumpMSB32::new(buf);

    // Process the initial metadata bits, we only really use initVal, width and
    // height (the last two match the TIFF values anyway)
    pump.get_bits(16); // NLCVersion
    pump.get_bits(4); // ImgFormat
    let bit_depth = pump.get_bits(4) + 1;
    pump.get_bits(4); // NumBlkInRCUnit
    pump.get_bits(4); // CompressionRatio
    pump.get_bits(16); // Width;
    pump.get_bits(16); // Height;
    pump.get_bits(16); // TileWidth
    pump.get_bits(4); // reserved

    // The format includes an optimization code that sets 3 flags to change the
    // decoding parameters
    let optflags = pump.get_bits(4);
    static OPT_SKIP: u32 = 1; // Skip checking if we need differences from previous line
    static OPT_MV: u32 = 2; // Simplify motion vector definition
    static OPT_QP: u32 = 4; // Don't scale the diff values

    pump.get_bits(8); // OverlapWidth
    pump.get_bits(8); // reserved
    pump.get_bits(8); // Inc
    pump.get_bits(2); // reserved
    let init_val = pump.get_bits(14) as u16;

    // The format is relatively straightforward. Each line gets encoded as a set
    // of differences from pixels from another line. Pixels are grouped in blocks
    // of 16 (8 green, 8 red or blue). Each block is encoded in three sections.
    // First 1 or 4 bits to specify which reference pixels to use, then a section
    // that specifies for each pixel the number of bits in the difference, then
    // the actual difference bits
    let mut line_offset = 0;
    for row in 0..height {
      line_offset += pump.get_pos();
      // Align pump to 16byte boundary
      if (line_offset & 0x0f) != 0 {
        line_offset += 16 - (line_offset & 0xf);
      }
      pump = BitPumpMSB32::new(&buf[line_offset..]);

      let img = width * row;
      let img_up = width * (cmp::max(1, row) - 1);
      let img_up2 = width * (cmp::max(2, row) - 2);

      // Initialize the motion and diff modes at the start of the line
      let mut motion: usize = 7;
      // By default we are not scaling values at all
      let mut scale: i32 = 0;
      let mut diff_bits_mode: [[u32; 2]; 3] = [[0; 2]; 3];
      for i in 0..3 {
        let init: u32 = if row < 2 { 7 } else { 4 };
        diff_bits_mode[i][0] = init;
        diff_bits_mode[i][1] = init;
      }

      for col in (0..width).step_by(16) {
        // Calculate how much scaling the final values will need
        scale = if (optflags & OPT_QP) == 0 && (col & 63) == 0 {
          let scalevals: [i32; 3] = [0, -2, 2];
          let i = pump.get_bits(2) as usize;
          if i < 3 { scale + scalevals[i] } else { pump.get_bits(12) as i32 }
        } else {
          scale // Keep value from previous iteration
        };

        // First we figure out which reference pixels mode we're in
        if (optflags & OPT_MV) != 0 {
          motion = if pump.get_bits(1) != 0 { 3 } else { 7 };
        } else if pump.get_bits(1) == 0 {
          motion = pump.get_bits(3) as usize;
        }

        if row < 2 && motion != 7 {
          panic!("SRW Decoder: At start of image and motion isn't 7. File corrupted?")
        }

        if motion == 7 {
          // The base case, just set all pixels to the previous ones on the same line
          // If we're at the left edge we just start at the initial value
          for i in 0..16 {
            out[img + col + i] = if col == 0 { init_val } else { out[img + col + i - 2] };
          }
        } else {
          // The complex case, we now need to actually lookup one or two lines above
          if row < 2 {
            panic!("SRW: Got a previous line lookup on first two lines. File corrupted?");
          }
          let motion_offset: [isize; 7] = [-4, -2, -2, 0, 0, 2, 4];
          let motion_average: [i32; 7] = [0, 0, 1, 0, 1, 0, 0];
          let slide_offset = motion_offset[motion];

          for i in 0..16 {
            let refpixel: usize = if ((row + i) & 0x1) != 0 {
              // Red or blue pixels use same color two lines up
              ((img_up2 + col + i) as isize + slide_offset) as usize
            } else {
              // Green pixel N uses Green pixel N from row above (top left or top right)
              if (i % 2) != 0 {
                ((img_up + col + i - 1) as isize + slide_offset) as usize
              } else {
                ((img_up + col + i + 1) as isize + slide_offset) as usize
              }
            };
            // In some cases we use as reference interpolation of this pixel and the next
            out[img + col + i] = if motion_average[motion] != 0 {
              (out[refpixel] + out[refpixel + 2] + 1) >> 1
            } else {
              out[refpixel]
            }
          }
        }

        // Figure out how many difference bits we have to read for each pixel
        let mut diff_bits: [u32; 4] = [0; 4];
        if (optflags & OPT_SKIP) != 0 || pump.get_bits(1) == 0 {
          let flags: [u32; 4] = [pump.get_bits(2), pump.get_bits(2), pump.get_bits(2), pump.get_bits(2)];
          for i in 0..4 {
            // The color is 0-Green 1-Blue 2-Red
            let colornum: usize = if row % 2 != 0 { i >> 1 } else { ((i >> 1) + 2) % 3 };
            match flags[i] {
              0 => {
                diff_bits[i] = diff_bits_mode[colornum][0];
              }
              1 => {
                diff_bits[i] = diff_bits_mode[colornum][0] + 1;
              }
              2 => {
                diff_bits[i] = diff_bits_mode[colornum][0] - 1;
              }
              3 => {
                diff_bits[i] = pump.get_bits(4);
              }
              _ => {}
            }
            diff_bits_mode[colornum][0] = diff_bits_mode[colornum][1];
            diff_bits_mode[colornum][1] = diff_bits[i];
            if diff_bits[i] > bit_depth + 1 {
              panic!("SRW Decoder: Too many difference bits. File corrupted?");
            }
          }
        }

        // Actually read the differences and write them to the pixels
        for i in 0..16 {
          let len = diff_bits[i >> 2];
          let mut diff = pump.get_ibits_sextended(len);
          diff = diff * (scale * 2 + 1) + scale;

          // Apply the diff to pixels 0 2 4 6 8 10 12 14 1 3 5 7 9 11 13 15
          let pos = if row % 2 != 0 {
            ((i & 0x7) << 1) + 1 - (i >> 3)
          } else {
            ((i & 0x7) << 1) + (i >> 3)
          } + img
            + col;
          out[pos] = clampbits((out[pos] as i32) + diff, bit_depth);
        }
      }
    }

    out
  }

  /// Get lens description by analyzing TIFF tags and makernotes
  fn get_lens_description(&self) -> Result<Option<&'static LensDescription>> {
    if let Some(lens_id) = self.makernote.get_entry(SrwMakernote::LensModel) {
      let lens_id = lens_id.force_u16(0);
      let resolver = LensResolver::new()
        .with_lens_id((lens_id.into(), 0))
        .with_camera(&self.camera)
        .with_mounts(&[NX_MOUNT.into()]);
      return Ok(resolver.resolve());
    }
    Ok(None)
  }

  fn get_wb(&self) -> Result<[f32; 4]> {
    let rggb_levels = fetch_tiff_tag!(self.makernote, SrwMakernote::SrwRGGBLevels);
    let rggb_blacks = fetch_tiff_tag!(self.makernote, SrwMakernote::SrwRGGBBlacks);

    if rggb_levels.count() != 4 || rggb_blacks.count() != 4 {
      Err(RawlerError::DecoderFailed("SRW: RGGB Levels and Blacks don't have 4 elements".to_string()))
    } else {
      Ok([
        (rggb_levels.force_u32(0) as f32 - rggb_blacks.force_u32(0) as f32) / 4096.0,
        (rggb_levels.force_u32(1) as f32 - rggb_blacks.force_u32(1) as f32) / 4096.0,
        (rggb_levels.force_u32(3) as f32 - rggb_blacks.force_u32(3) as f32) / 4096.0,
        f32::NAN,
      ])
    }
  }

  /// Extract blacklevel
  /// Ironically, the data is already black level subtracted, but the
  /// WB coeffs are not. So we can return 0 here. The black level
  /// is subtracted in the get_wb() function.
  fn get_blacklevel(&self) -> Result<[u32; 4]> {
    Ok([0, 0, 0, 0])
    /*
     let rggb_blacks = fetch_tiff_tag!(self.makernote, SrwMakernote::SrwRGGBBlacks);
     if rggb_blacks.count() != 4 {
       Err(RawlerError::General("SRW: RGGB Blacks don't have 4 elements".to_string()))
     } else {
       Ok([
         rggb_blacks.force_u16(0),
         rggb_blacks.force_u16(1),
         rggb_blacks.force_u16(2),
         rggb_blacks.force_u16(3),
       ])
     }
    */
  }
}

crate::tags::tiff_tag_enum!(SrwMakernote);

#[allow(non_camel_case_types)]
#[derive(Debug, Copy, Clone, PartialEq, enumn::N)]
#[repr(u16)]
pub enum SrwMakernote {
  LensModel = 0xA003,
  SrwRGGBLevels = 0xA021,
  SrwRGGBBlacks = 0xA028,
}
