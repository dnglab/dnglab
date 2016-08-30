use decoders::*;
use decoders::tiff::*;
use decoders::basics::*;

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
      wb_coeffs: [0.0,0.0,0.0,0.0],
      data: image.into_boxed_slice(),
      blacklevels: camera.blacklevels,
      whitelevels: camera.whitelevels,
      color_matrix: camera.color_matrix,
      dcraw_filters: camera.dcraw_filters,
      crops: camera.crops,
    })
  }
}
