//! Print the makernote of a Canon EOS 5D Mark III CR2 of rawdb.dnglab.org
//!
//! The makernote is an IFD in the EXIF data with tags defined by the camera
//! maker, here [`Cr2MakernoteTag`].
//!
//! ```sh
//! cargo run -p rawler --release --example makernote
//! ```

use rawler::{
  decoders::cr2::Cr2MakernoteTag,
  formats::tiff::{GenericTiffReader, ifd::OffsetMode, reader::TiffReader},
  rawdb,
  rawsource::RawSource,
  tags::ExifTag,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
  let path = rawdb::sample("Canon", "EOS 5D Mark III", "raw_modes/Canon EOS 5D Mark III_RAW_ISO_200.CR2")?;
  println!("Sample: {}", path.display());

  let source = RawSource::new(&path)?;
  let tiff = GenericTiffReader::new(&mut source.reader(), 0, 0, None, &[])?;
  let exif = tiff.find_first_ifd_with_tag(ExifTag::MakerNotes).ok_or("file has no makernote")?;
  // Offsets in Canon makernotes are relative to the start of the file
  let makernote = exif
    .parse_makernote(&mut source.reader(), OffsetMode::Absolute, &[])?
    .ok_or("file has no makernote")?;

  let string = |tag| makernote.get_entry(tag).and_then(|entry| entry.as_string()).map(|s| s.trim_end_matches('\0'));
  let model_id = makernote.get_entry(Cr2MakernoteTag::ModelId).and_then(|entry| entry.get_u32(0).ok().flatten());
  println!("Firmware: {}", string(Cr2MakernoteTag::FirmareVer).unwrap_or("unknown"));
  println!("Lens:     {}", string(Cr2MakernoteTag::LensModel).unwrap_or("unknown"));
  println!("Model ID: {}", model_id.map_or_else(|| "unknown".to_string(), |id| format!("{:#010x}", id)));
  println!();

  // All entries, arrays are shortened to 8 values
  for line in makernote.dump::<Cr2MakernoteTag>(8) {
    println!("{}", line.trim_end());
  }
  Ok(())
}
