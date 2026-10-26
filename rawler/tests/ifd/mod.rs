use rawler::decoders::{Decoder, WellKnownIFD};
use rawler::devtools::rawdb::get_rawdb_cache;
use rawler::devtools::rawdb::rawdb_ensure_file;
use rawler::formats::tiff::Value;
use rawler::formats::tiff::ifd::OffsetMode;
use rawler::rawsource::RawSource;
use rawler::tags::{ExifTag, TiffCommonTag};

/// Check that the MakerNotes entry of the EXIF IFD points to its data
/// inside the file and can be parsed. Canon CameraSettings is tag 0x0001.
fn check_makernote(decoder: &dyn Decoder, rawfile: &RawSource) -> std::result::Result<(), Box<dyn std::error::Error>> {
  let exif = decoder.ifd(WellKnownIFD::Exif)?.ok_or("no EXIF IFD")?;
  let makernote = exif.get_entry(ExifTag::MakerNotes).ok_or("no makernote")?;
  let offset = makernote.offset().ok_or("no makernote offset")?;
  let Value::Undefined(data) = &makernote.value else {
    return Err("makernote has unexpected type".into());
  };
  assert!(!data.is_empty());
  assert_eq!(rawfile.subview(exif.base as u64 + offset as u64, data.len() as u64)?, &data[..]);

  let makernote_ifd = exif.parse_makernote(&mut rawfile.reader(), OffsetMode::Absolute, &[])?.ok_or("makernote not parsed")?;
  assert!(makernote_ifd.get_entry(0x0001_u16).is_some());
  Ok(())
}

#[test]
fn cr2_ifd_exif_contains_makernote() -> std::result::Result<(), Box<dyn std::error::Error>> {
  let path = rawdb_ensure_file(&get_rawdb_cache(), "Canon", "EOS 100D", "raw_modes/Canon EOS 100D_ISO_100_RAW.CR2")?;
  let rawfile = RawSource::new(&path)?;
  let decoder = rawler::get_decoder(&rawfile)?;

  assert!(decoder.ifd(WellKnownIFD::Root)?.is_some());
  let raw = decoder.ifd(WellKnownIFD::Raw)?.ok_or("no raw IFD")?;
  assert!(raw.get_entry(TiffCommonTag::StripOffsets).is_some());
  check_makernote(decoder.as_ref(), &rawfile)?;

  for wk_ifd in [WellKnownIFD::Preview, WellKnownIFD::VirtualDngRootTags, WellKnownIFD::VirtualDngRawTags] {
    assert!(decoder.ifd(wk_ifd)?.is_none(), "{:?}", wk_ifd);
  }
  Ok(())
}

#[test]
fn cr3_ifd_returns_cmt_boxes() -> std::result::Result<(), Box<dyn std::error::Error>> {
  let path = rawdb_ensure_file(&get_rawdb_cache(), "Canon", "EOS 250D", "raw_modes/Canon EOS 250D_RAW_ISO_100_nocrop_nodual.CR3")?;
  let rawfile = RawSource::new(&path)?;
  let decoder = rawler::get_decoder(&rawfile)?;

  let root = decoder.ifd(WellKnownIFD::Root)?.ok_or("no root IFD")?;
  // Entry offsets must resolve against the file, not the CMT box buffer.
  let make = root.get_entry_raw(TiffCommonTag::Make, &mut rawfile.reader())?.ok_or("no make")?;
  assert!(make.data.starts_with(b"Canon"));
  let exif = decoder.ifd(WellKnownIFD::Exif)?.ok_or("no EXIF IFD")?;
  assert!(exif.get_entry(ExifTag::ExposureTime).is_some());
  check_makernote(decoder.as_ref(), &rawfile)?;
  assert!(decoder.ifd(WellKnownIFD::ExifGps)?.is_some());

  for wk_ifd in [WellKnownIFD::Raw, WellKnownIFD::Preview, WellKnownIFD::VirtualDngRootTags, WellKnownIFD::VirtualDngRawTags] {
    assert!(decoder.ifd(wk_ifd)?.is_none(), "{:?}", wk_ifd);
  }
  Ok(())
}
