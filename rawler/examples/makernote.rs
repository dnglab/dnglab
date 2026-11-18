//! Print the makernote of a raw file and the sections of its virtual makernotes
//!
//! Without an argument, it reads a Canon EOS 5D Mark III CR2 of rawdb.dnglab.org.
//! The makernote is an IFD with tags defined by the camera maker, here
//! [`Cr2MakernoteTag`]. The virtual makernotes collect all makernote data of
//! the file, like the CMT boxes and CTMD records of CR3 files.
//!
//! ```sh
//! cargo run -p rawler --release --example makernote [-- <raw file>]
//! ```

use rawler::{
  decoders::{
    FormatHint, RawDecodeParams, WellKnownIFD,
    cr2::Cr2MakernoteTag,
    makernotes::{CR3_CTMD_RECORD_BASE, MakernoteSection},
  },
  formats::tiff::IFD,
  rawdb,
  rawsource::RawSource,
};
use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
  let path = match std::env::args_os().nth(1) {
    Some(path) => PathBuf::from(path),
    None => rawdb::sample("Canon", "EOS 5D Mark III", "raw_modes/Canon EOS 5D Mark III_RAW_ISO_200.CR2")?,
  };
  println!("Sample: {}", path.display());

  let source = RawSource::new(&path)?;
  let decoder = rawler::get_decoder(&source)?;
  let params = RawDecodeParams::default();

  if let Some(makernote) = decoder.ifd(&source, &params, WellKnownIFD::Makernote)? {
    // All entries, arrays are shortened to 8 values
    let dump = if decoder.format_hint() == FormatHint::CR2 {
      let string = |tag| makernote.get_entry(tag).and_then(|entry| entry.as_string()).map(|s| s.trim_end_matches('\0'));
      let model_id = makernote.get_entry(Cr2MakernoteTag::ModelId).and_then(|entry| entry.get_u32(0).ok().flatten());
      println!("Firmware: {}", string(Cr2MakernoteTag::FirmwareVer).unwrap_or("unknown"));
      println!("Lens:     {}", string(Cr2MakernoteTag::LensModel).unwrap_or("unknown"));
      println!("Model ID: {}", model_id.map_or_else(|| "unknown".to_string(), |id| format!("{:#010x}", id)));
      println!();
      makernote.dump::<Cr2MakernoteTag>(8)
    } else {
      makernote.dump::<u16>(8)
    };
    for line in dump {
      println!("{}", line.trim_end());
    }
  } else {
    println!("No makernote");
  }
  println!();

  match decoder.ifd(&source, &params, WellKnownIFD::VirtualMakernotes)? {
    Some(container) => {
      println!("Virtual makernote sections:");
      print_sections(&container, 1);
    }
    None => println!("No virtual makernotes"),
  }
  Ok(())
}

/// Print the sections of a virtual makernotes container and of its nested containers
fn print_sections(container: &IFD, depth: usize) {
  let indent = "  ".repeat(depth);
  for (tag, entry) in container.entries() {
    let name = MakernoteSection::try_from(*tag).map_or_else(|_| format!("{:#06x}", tag), |section| format!("{:?}", section));
    let parsed = container.get_sub_ifd(*tag);
    let parsed_info = parsed.map_or_else(String::new, |ifd| format!(", parsed: {} entries", ifd.entries().len()));
    println!("{}{:<28} {:<9} {:>8}{}", indent, name, entry.type_name(), entry.count(), parsed_info);
    // CTMD records hold a nested container with their TIFF blocks
    if let Some(nested) = parsed.filter(|_| (CR3_CTMD_RECORD_BASE..CR3_CTMD_RECORD_BASE + 0x100).contains(tag)) {
      print_sections(nested, depth + 1);
    }
  }
}
