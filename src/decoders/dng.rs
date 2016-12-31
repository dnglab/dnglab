use decoders::*;
use decoders::tiff::*;
use decoders::basics::*;
use decoders::ljpeg::*;
use decoders::cfa::*;
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

    let (make, model, canonical_make, canonical_model) = {
      match self.rawloader.check_supported(&self.tiff) {
        Ok(cam) => {
          (cam.make.clone(), cam.model.clone(),
           cam.canonical_make.clone(),cam.canonical_model.clone())
        },
        Err(_) => {
          let make = fetch_tag!(self.tiff, Tag::Make).get_str();
          let model = fetch_tag!(self.tiff, Tag::Model).get_str();
          (make.to_string(), model.to_string(), make.to_string(), model.to_string())
        },
      }
    };

    Ok(Image {
      make: make,
      model: model,
      canonical_make: canonical_make,
      canonical_model: canonical_model,
      width: width as usize,
      height: height as usize,
      wb_coeffs: try!(self.get_wb()),
      data: image.into_boxed_slice(),
      blacklevels: try!(self.get_blacklevels(raw)),
      whitelevels: try!(self.get_whitelevels(raw)),
      xyz_to_cam: try!(self.get_color_matrix()),
      cfa: try!(self.get_cfa(raw)),
      crops: try!(self.get_crops(raw, width as usize, height as usize)),
    })
  }
}

impl<'a> DngDecoder<'a> {
  fn get_wb(&self) -> Result<[f32;4], String> {
    let levels = fetch_tag!(self.tiff, Tag::AsShotNeutral);
    Ok([1.0/levels.get_f32(0),1.0/levels.get_f32(1),1.0/levels.get_f32(2),NAN])
  }

  fn get_blacklevels(&self, raw: &TiffIFD) -> Result<[u16;4], String> {
    if let Some(levels) = raw.find_entry(Tag::BlackLevels) {
      if levels.count() < 4 {
        let black = levels.get_u32(0) as u16;
        Ok([black, black, black, black])
      } else {
        Ok([levels.get_u32(0) as u16,levels.get_u32(1) as u16,
            levels.get_u32(2) as u16,levels.get_u32(3) as u16])
      }
    } else {
      Ok([0,0,0,0])
    }
  }

  fn get_whitelevels(&self, raw: &TiffIFD) -> Result<[u16;4], String> {
    let level = fetch_tag!(raw, Tag::WhiteLevel).get_u32(0) as u16;
    Ok([level,level,level,level])
  }

  fn get_cfa(&self, raw: &TiffIFD) -> Result<CFA,String> {
    let pattern = fetch_tag!(raw, Tag::CFAPattern);
    Ok(CFA::new_from_tag(pattern))
  }

  fn get_crops(&self, raw: &TiffIFD, width: usize, height: usize) -> Result<[usize;4],String> {
    if let Some(crops) = raw.find_entry(Tag::ActiveArea) {
      Ok([crops.get_u32(0) as usize, width - crops.get_u32(3) as usize,
          height - crops.get_u32(2) as usize, crops.get_u32(1) as usize])
    } else {
      // Ignore missing crops, at least some pentax DNGs don't have it
      Ok([0,0,0,0])
    }
  }

  fn get_color_matrix(&self) -> Result<[[f32;3];4],String> {
    let mut matrix: [[f32;3];4] = [[0.0;3];4];
    let cmatrix = {
      if let Some(c) = self.tiff.find_entry(Tag::ColorMatrix2) {
        c
      } else {
        fetch_tag!(self.tiff, Tag::ColorMatrix1)
      }
    };
    if cmatrix.count() > 12 {
      Err(format!("color matrix supposedly has {} components",cmatrix.count()).to_string())
    } else {
      for i in 0..(cmatrix.count() as usize) {
        matrix[i/3][i%3] = cmatrix.get_f32(i);
      }
      Ok(matrix)
    }
  }

  pub fn decode_uncompressed(&self, raw: &TiffIFD, width: usize, height: usize) -> Result<Vec<u16>,String> {
    let offset = fetch_tag!(raw, Tag::StripOffsets).get_u32(0) as usize;
    let src = &self.buffer[offset..];

    match fetch_tag!(raw, Tag::BitsPerSample).get_u32(0) {
      16  => Ok(decode_16le(src, width, height)),
      12  => Ok(decode_12be(src, width, height)),
      8   => {
        // It's 8 bit so there will be linearization involved surely!
        let linearization = fetch_tag!(self.tiff, Tag::Linearization);
        let curve = {
          let mut points = vec![0 as u16; 256];
          for i in 0..256 {
            points[i] = linearization.get_u32(i) as u16;
          }
          LookupTable::new(&points)
        };
        Ok(decode_8bit_wtable(src, &curve, width, height))
      },
      bps => Err(format!("DNG: Don't know about {} bps images", bps).to_string()),
    }
  }

  pub fn decode_compressed(&self, raw: &TiffIFD, width: usize, height: usize) -> Result<Vec<u16>,String> {
    if let Some(offsets) = raw.find_entry(Tag::StripOffsets) { // We're in a normal offset situation
      if offsets.count() != 1 {
        return Err("DNG: files with more than one slice not supported yet".to_string())
      }
      let offset = offsets.get_u32(0) as usize;
      let src = &self.buffer[offset..];
      let mut out = vec![0 as u16; width*height];
      let decompressor = try!(LjpegDecompressor::new(src, true));
      try!(decompressor.decode(&mut out, 0, width, width, height));
      Ok(out)
    } else if let Some(offsets) = raw.find_entry(Tag::TileOffsets) {
      // They've gone with tiling
      let twidth = fetch_tag!(raw, Tag::TileWidth).get_u32(0) as usize;
      let tlength = fetch_tag!(raw, Tag::TileLength).get_u32(0) as usize;
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
