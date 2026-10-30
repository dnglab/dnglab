//! Develop a sample of rawdb.dnglab.org to a JPEG image with [`plan`] and
//! [`execute`]
//!
//! This is the develop example without
//! [`RawDevelop`](rawler::imgop::develop::RawDevelop): [`plan`] decides the
//! processing operations from the metadata of the raw image and the
//! [`RenderOptions`], [`execute`] applies them to the image data. The options
//! are the default values, `StepSet::all().without(ProcessingStep::...)` leaves
//! out a processing step. The image is written as JPEG with EXIF data and the
//! ICC profile of its color space. Run with:
//!
//! ```sh
//! cargo run -p rawler --release --example full_render [-- <output.jpg>]
//! ```

use image::{ImageEncoder, codecs::jpeg::JpegEncoder};
use rawler::{
  decoders::{Orientation, RawDecodeParams, RawMetadata},
  formats::tiff::{DirectoryWriter, TiffWriter},
  imgop::{
    color::{OutputSpace, TransferFunction},
    render::{DemosaicQuality, RenderOptions, RenderTarget, RenderedImage, StepSet, WhiteBalancePlacement, execute, plan},
  },
  rawdb,
  rawsource::RawSource,
  tags::{ExifTag, TiffCommonTag},
};
use std::{
  error::Error,
  fs::File,
  io::{BufWriter, Cursor},
  path::{Path, PathBuf},
};

fn main() -> Result<(), Box<dyn Error>> {
  let output = std::env::args_os().nth(1).map_or_else(|| PathBuf::from("full_render.jpg"), PathBuf::from);

  // Path of the sample in the cache, it is downloaded if missing
  let path = rawdb::sample("Canon", "EOS 5D Mark III", "raw_modes/Canon EOS 5D Mark III_RAW_ISO_200.CR2")?;
  println!("Sample: {}", path.display());

  // Decode the raw image and its metadata
  let source = RawSource::new(&path)?;
  let decoder = rawler::get_decoder(&source)?;
  let params = RawDecodeParams::default();
  let rawimage = decoder.raw_image(&source, &params, false)?;
  let metadata = decoder.raw_metadata(&source, &params)?;
  // Camera and orientation for the EXIF data, rendering consumes the raw image
  let (make, model, orientation) = (rawimage.clean_make.clone(), rawimage.clean_model.clone(), rawimage.orientation);

  let options = RenderOptions {
    // An image of the processing steps, not LinearRaw data like in a DNG file
    target: RenderTarget::Image,
    // All processing steps
    steps: StepSet::all(),
    // CFA data is white balanced before it is demosaiced
    white_balance: WhiteBalancePlacement::BeforeDemosaic,
    // PPG for Bayer, Markesteijn for X-Trans data
    demosaic: DemosaicQuality::Quality,
    // Color space of the image, with its transfer function
    color_space: OutputSpace::Srgb,
  };

  // Plan the processing operations for this image
  let plan = plan(&rawimage, &options)?;
  println!("{:#?}", plan);

  // Apply the operations to the image data
  let image = execute(rawimage, &plan)?;
  println!(
    "Rendered image: {}x{}, {} channels, {:?}, {:?} transfer function",
    image.width, image.height, image.channels, image.color_space, image.transfer_function
  );

  write_jpeg(&image, &make, &model, orientation, &metadata, &output)?;
  println!("Developed image: {}", output.display());
  Ok(())
}

/// Write `image` as JPEG with the EXIF data of `metadata` and the ICC profile of
/// its color space, like [`RawDevelop::develop_jpeg`](rawler::imgop::develop::RawDevelop::develop_jpeg)
fn write_jpeg(image: &RenderedImage, make: &str, model: &str, orientation: Orientation, metadata: &RawMetadata, path: &Path) -> Result<(), Box<dyn Error>> {
  // EXIF data is a TIFF structure without image data
  let mut exif = Cursor::new(Vec::new());
  let mut tiff = TiffWriter::new(&mut exif)?;
  let mut root_ifd = DirectoryWriter::new();
  let mut exif_ifd = DirectoryWriter::new();
  // EXIF version 0220
  exif_ifd.add_tag_undefined(ExifTag::ExifVersion, vec![48, 50, 50, 48]);
  metadata.write_exif_tags(&mut tiff, &mut root_ifd, &mut exif_ifd)?;
  root_ifd.add_tag(TiffCommonTag::Make, make);
  root_ifd.add_tag(TiffCommonTag::Model, model);
  // The color space of the image, not of the camera: sRGB or uncalibrated
  let srgb = image.color_space == Some(OutputSpace::Srgb) && image.transfer_function == TransferFunction::Srgb;
  exif_ifd.add_tag(ExifTag::ColorSpace, if srgb { 1_u16 } else { 0xffff });
  let exif_offset = exif_ifd.build(&mut tiff)?;
  root_ifd.add_tag(TiffCommonTag::ExifIFDPointer, exif_offset);
  root_ifd.add_tag(TiffCommonTag::Orientation, orientation.to_u16());
  tiff.build(root_ifd)?;
  let exif = exif.into_inner();

  let mut encoder = JpegEncoder::new_with_quality(BufWriter::new(File::create(path)?), 90);
  if let Some(icc) = image.icc_profile() {
    encoder.set_icc_profile(icc)?;
  }
  // The APP1 segment holds 65533 bytes, including the "Exif\0\0" header
  if exif.len() <= 65527 {
    encoder.set_exif_metadata(exif)?;
  } else {
    eprintln!("EXIF data of {} bytes doesn't fit into JPEG, it is not written", exif.len());
  }
  let image = image.to_dynamic_image().ok_or("failed to convert the rendered image")?;
  image.write_with_encoder(encoder)?;
  Ok(())
}
