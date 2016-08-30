use decoders::*;
use decoders::tiff::*;
use decoders::basics::*;
use std::mem::transmute;
extern crate itertools;
use self::itertools::Itertools;
use std::f32;

#[derive(Debug, Clone)]
pub struct ArwDecoder<'a> {
  buffer: &'a [u8],
  rawloader: &'a RawLoader,
  tiff: TiffIFD<'a>,
}

impl<'a> ArwDecoder<'a> {
  pub fn new(buf: &'a [u8], tiff: TiffIFD<'a>, rawloader: &'a RawLoader) -> ArwDecoder<'a> {
    ArwDecoder {
      buffer: buf,
      tiff: tiff,
      rawloader: rawloader,
    }
  }
}

impl<'a> Decoder for ArwDecoder<'a> {
  fn identify(&self) -> Result<&Camera, String> {
    let make = fetch_tag!(self.tiff, Tag::MAKE, "ARW: Couldn't find Make").get_str();
    let model = fetch_tag!(self.tiff, Tag::MODEL, "ARW: Couldn't find Model").get_str();
    self.rawloader.check_supported(make, model)
  }

  fn image(&self) -> Result<Image,String> {
    let camera = try!(self.identify());
    let data = self.tiff.find_ifds_with_tag(Tag::STRIPOFFSETS);
    if data.len() == 0 {
      return Err("ARW: Couldn't find the data IFD!".to_string())
    }
    let raw = data[0];
    let compression = fetch_tag!(raw, Tag::COMPRESSION, "ARW: Couldn't find Compression").get_u16(0);
    match compression {
      1 => self.decode_uncompressed(camera, raw),
      x => Err(format!("ARW: Don't know how to decode type {}", x).to_string())
    }
  }
}

impl<'a> ArwDecoder<'a> {
  fn decode_uncompressed(&self, camera: &Camera, raw: &TiffIFD) -> Result<Image,String> {
    let width = fetch_tag!(raw, Tag::IMAGEWIDTH, "ARW: Couldn't find width").get_u16(0) as u32;
    let height = fetch_tag!(raw, Tag::IMAGELENGTH, "ARW: Couldn't find height").get_u16(0) as u32;
    let offset = fetch_tag!(raw, Tag::STRIPOFFSETS, "ARW: Couldn't find offset").get_u32(0) as usize;

    let src = &self.buffer[offset .. self.buffer.len()];
    let image = decode_16le(src, width as usize, height as usize);

    Ok(Image {
      width: width,
      height: height,
      wb_coeffs: try!(self.get_wb()),
      data: image.into_boxed_slice(),
      blacklevels: camera.blacklevels,
      whitelevels: camera.whitelevels,
      color_matrix: camera.color_matrix,
      dcraw_filters: camera.dcraw_filters,
      crops: camera.crops,
    })
  }

  fn get_wb(&self) -> Result<[f32;4], String> {
    let priv_offset = fetch_tag!(self.tiff, Tag::DNGPRIVATEDATA, "ARW: Couldn't find private offset").get_u32(0);
    let priv_tiff = TiffIFD::new(self.buffer, priv_offset as usize, 0, LITTLE_ENDIAN);
    let sony_offset = fetch_tag!(priv_tiff, Tag::SONY_OFFSET, "ARW: Couldn't find sony offset").get_u32(0) as usize;
    let sony_length = fetch_tag!(priv_tiff, Tag::SONY_LENGTH, "ARW: Couldn't find sony length").get_u32(0) as usize;
    let sony_key = fetch_tag!(priv_tiff, Tag::SONY_KEY, "ARW: Couldn't find sony key").get_u32(0);
    let mut clone = self.buffer.to_vec();
    ArwDecoder::sony_decrypt(& mut clone, sony_offset, sony_length, sony_key);
    let decrypted_tiff = TiffIFD::new(&clone, sony_offset, 0, LITTLE_ENDIAN);
    let grgb_levels = decrypted_tiff.find_entry(Tag::SONYGRBGLEVELS);
    let rggb_levels = decrypted_tiff.find_entry(Tag::SONYRGGBLEVELS);
    if grgb_levels.is_some() {
      let levels = grgb_levels.unwrap();
      Ok([levels.get_u16(1) as f32, levels.get_u16(0) as f32, levels.get_u16(2) as f32, f32::NAN])
    } else if rggb_levels.is_some() {
      let levels = rggb_levels.unwrap();
      Ok([levels.get_u16(0) as f32, levels.get_u16(1) as f32, levels.get_u16(3) as f32, f32::NAN])
    } else {
      Err("ARW: Couldn't find GRGB or RGGB levels".to_string())
    }
  }

  fn sony_decrypt(buf: & mut [u8], offset: usize, length: usize, key: u32) {
    let mut pad: [u32; 128] = [0 as u32; 128];
    let mut mkey = key;
    // Initialize the decryption pad from the key
    for p in 0..4 {
      mkey = mkey.wrapping_mul(48828125).wrapping_add(1);
      pad[p] = mkey;
    }
    pad[3] = pad[3] << 1 | (pad[0]^pad[2]) >> 31;
    for p in 4..127 {
      pad[p] = (pad[p-4]^pad[p-2]) << 1 | (pad[p-3]^pad[p-1]) >> 31;
    }
    for p in 0..127 {
      pad[p] = u32::from_be(pad[p]);
    }

    for i in (0..length).step(4) {
      let p = i/4 + 127;
      pad[p & 127] = pad[(p+1) & 127] ^ pad[(p+1+64) & 127];
      let output = LEu32(buf, offset+i) ^ pad[p & 127];
      let bytes: [u8; 4] = unsafe { transmute(output.to_le()) };
      buf[offset+i]   = bytes[0];
      buf[offset+i+1] = bytes[1];
      buf[offset+i+2] = bytes[2];
      buf[offset+i+3] = bytes[3];
    }
  }
}
