use decoders::*;
use decoders::tiff::*;
use decoders::basics::*;
use decoders::ljpeg::*;
use std::f32::NAN;
use std::cmp;

#[derive(Debug, Clone)]
pub struct DngDecoder<'a> {
  buffer: &'a [u8],
  rawloader: &'a RawLoader,
  tiff: TiffIFD<'a>,
}

impl<'a> DngDecoder<'a> {
  pub fn new(buf: &'a [u8], tiff: TiffIFD<'a>, rawloader: &'a RawLoader) -> DngDecoder<'a> {
    DngDecoder {
      buffer: buf,
      tiff: tiff,
      rawloader: rawloader,
    }
  }
}

impl<'a> Decoder for DngDecoder<'a> {
  fn image(&self) -> Result<Image,String> {
    let camera = try!(self.rawloader.check_supported(&self.tiff));
    let ifds = self.tiff.find_ifds_with_tag(Tag::Compression).into_iter().filter(|ifd| {
      let compression = (**ifd).find_entry(Tag::Compression).unwrap().get_u32(0);
      let subsampled = match (**ifd).find_entry(Tag::NewSubFileType) {
        Some(e) => e.get_u32(0) & 1 != 0,
        None => false,
      };
      !subsampled && (compression == 7 || compression == 1 || compression == 0x884c)
    }).collect::<Vec<&TiffIFD>>();
    let raw = ifds[0];
    let width = fetch_tag!(raw, Tag::ImageWidth).get_u32(0);
    let height = fetch_tag!(raw, Tag::ImageLength).get_u32(0);

    let image = match fetch_tag!(raw, Tag::Compression).get_u32(0) {
      1 => try!(self.decode_uncompressed(raw, width as usize, height as usize)),
      7 => try!(self.decode_compressed(raw, width as usize, height as usize)),
      c => return Err(format!("Don't know how to read DNGs with compression {}", c).to_string()),
    };

    ok_image(camera, width, height, try!(self.get_wb()), image)
  }
}

impl<'a> DngDecoder<'a> {
  fn get_wb(&self) -> Result<[f32;4], String> {
    let levels = fetch_tag!(self.tiff, Tag::AsShotNeutral);
    Ok([1.0/levels.get_f32(0),1.0/levels.get_f32(1),1.0/levels.get_f32(2),NAN])
  }

  pub fn decode_uncompressed(&self, raw: &TiffIFD, width: usize, height: usize) -> Result<Vec<u16>,String> {
    let offset = fetch_tag!(raw, Tag::StripOffsets).get_u32(0) as usize;
    let src = &self.buffer[offset..];

    match fetch_tag!(raw, Tag::BitsPerSample).get_u32(0) {
      16  => Ok(decode_16le(src, width, height)),
      bps => Err(format!("DNG: Don't know about {} bps images", bps).to_string()),
    }
  }

  pub fn decode_compressed(&self, raw: &TiffIFD, width: usize, height: usize) -> Result<Vec<u16>,String> {
    if raw.has_entry(Tag::StripOffsets) { // We're in a normal offset situation
      let offsets = fetch_tag!(raw, Tag::StripOffsets);
      if offsets.count() != 1 {
        return Err("DNG: files with more than one slice not supported yet".to_string())
      }
      let offset = offsets.get_u32(0) as usize;
      let src = &self.buffer[offset..];
      let mut out = vec![0 as u16; width*height];
      let decompressor = try!(LjpegDecompressor::new(src, true));
      try!(decompressor.decode(&mut out, 0, width, width, height));
      Ok(out)
    } else if raw.has_entry(Tag::TileOffsets) { // They've gone with tiling
      let twidth = fetch_tag!(raw, Tag::TileWidth).get_u32(0) as usize;
      let tlength = fetch_tag!(raw, Tag::TileLength).get_u32(0) as usize;
      let offsets = fetch_tag!(raw, Tag::TileOffsets);
      let coltiles = (width-1)/twidth + 1;
      let rowtiles = (height-1)/tlength + 1;
      if coltiles*rowtiles != offsets.count() as usize {
        return Err(format!("DNG: trying to decode {} tiles from {} offsets",
                           coltiles*rowtiles, offsets.count()).to_string())
      }

      Ok(decode_threaded_multiline(width, height, tlength, &(|strip: &mut [u16], row| {
        let row = row / tlength;
        for col in 0..coltiles {
          let offset = offsets.get_u32(row*coltiles+col) as usize;
          let src = &self.buffer[offset..];
          // We don't use bigtable here as the tiles are two small to amortize the setup cost
          let decompressor = LjpegDecompressor::new(src, true).unwrap();
          let bwidth = cmp::min(width, (col+1)*twidth) - col*twidth;
          let blength = cmp::min(height, (row+1)*tlength) - row*tlength;
          decompressor.decode(strip, col*twidth, width, bwidth, blength).unwrap();
        }
      })))
    } else {
      Err("DNG: didn't find tiles or strips".to_string())
    }
  }
}
