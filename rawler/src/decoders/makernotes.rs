// SPDX-License-Identifier: LGPL-2.1
// Copyright 2026 Daniel Vogelbacher <daniel@chaospixel.com>

//! Virtual makernotes, a container IFD for all makernote data of a raw file
//!
//! Most cameras store a single makernote IFD in the EXIF data, which
//! [`WellKnownIFD::Makernote`](super::WellKnownIFD::Makernote) returns. Other
//! formats spread the makernote data over several places, like the CMT boxes and
//! the CTMD records of Canon CR3 files.
//! [`WellKnownIFD::VirtualMakernotes`](super::WellKnownIFD::VirtualMakernotes)
//! collects them into a single IFD with one section per source:
//!
//! - A section with a contiguous payload is an UNDEFINED entry with the
//!   original bytes. If the payload is a TIFF structure, the sub-IFD list of the
//!   same tag holds the parsed IFD.
//! - A section without a contiguous payload, like an IFD that is referenced by
//!   an offset in the TIFF structure, is a LONG entry with the original offset,
//!   and the sub-IFD list of the same tag holds the parsed IFD.
//!
//! The sections are the entries of the container, their tags are defined in
//! [`MakernoteSection`].

use crate::bits::Endian;
use crate::formats::tiff::{Entry, IFD, Value};
use crate::tags::ExifTag;

use super::FormatHint;

crate::tags::tiff_tag_enum!(MakernoteSection);

/// Tags of the virtual makernotes container
///
/// Sections from a TIFF structure keep the tag number of the original tag.
#[derive(Debug, Copy, Clone, PartialEq, enumn::N)]
#[repr(u16)]
pub enum MakernoteSection {
  /// Version of the container layout, BYTE\[4\]
  LayoutVersion = 0x0001,
  /// Name of the raw format the container was built from, ASCII
  SourceFormat = 0x0002,
  /// Byte order of the TIFF structure that contains [`Self::ExifMakerNote`], ASCII `II` or `MM`
  ExifMakerNoteByteOrder = 0x0010,
  /// Offset of [`Self::ExifMakerNote`] from the start of its TIFF structure, LONG
  ///
  /// Offsets in some makernotes are relative to the start of the TIFF
  /// structure, so this is needed to parse them.
  ExifMakerNoteOriginalOffset = 0x0011,
  /// Panasonic camera IFD from the RW2 root IFD
  PanasonicCameraIFD = 0x0120,
  /// CMT1 box of CR3 files, a TIFF structure with the basic EXIF tags
  Cr3Cmt1 = 0x1001,
  /// CMT2 box of CR3 files, a TIFF structure with the EXIF tags
  Cr3Cmt2 = 0x1002,
  /// CMT3 box of CR3 files, a TIFF structure with the makernote tags
  Cr3Cmt3 = 0x1003,
  /// CMT4 box of CR3 files, a TIFF structure with the GPS tags
  Cr3Cmt4 = 0x1004,
  /// CTMD records of CR3 files for the decoded image. The entry holds the
  /// record payload. For records with TIFF blocks the sub-IFD list holds a
  /// nested container with the blocks as [`Self::Cr3CtmdExifIFD`] and [`Self::ExifMakerNote`].
  Cr3CtmdRecord1 = 0x1101,
  Cr3CtmdRecord2 = 0x1102,
  Cr3CtmdRecord3 = 0x1103,
  Cr3CtmdRecord4 = 0x1104,
  Cr3CtmdRecord5 = 0x1105,
  Cr3CtmdRecord6 = 0x1106,
  Cr3CtmdRecord7 = 0x1107,
  Cr3CtmdRecord8 = 0x1108,
  Cr3CtmdRecord9 = 0x1109,
  Cr3CtmdRecord10 = 0x110A,
  Cr3CtmdRecord11 = 0x110B,
  Cr3CtmdRecord12 = 0x110C,
  /// Kodak IFD of DCR files
  KodakIFD = 0x8290,
  /// EXIF IFD, used for the EXIF blocks of CR3 CTMD records
  Cr3CtmdExifIFD = 0x8769,
  /// The makernote from the EXIF IFD, as stored
  ExifMakerNote = 0x927C,
  /// Kodak IFD of KDC files
  KdcIFD = 0xFE00,
}

/// First tag of the CR3 CTMD record sections, the record type is added to it
pub const CR3_CTMD_RECORD_BASE: u16 = 0x1100;

/// Version of the container layout, increased for incompatible changes
const LAYOUT_VERSION: [u8; 4] = [1, 0, 0, 0];

/// Builder for a virtual makernotes container
pub(crate) struct VirtualMakernotes {
  ifd: IFD,
  sections: usize,
}

impl VirtualMakernotes {
  /// Container for the top level, with the layout version and the source format
  pub(crate) fn new(format: FormatHint) -> Self {
    let mut container = Self::nested();
    container.insert(MakernoteSection::LayoutVersion, Value::Byte(LAYOUT_VERSION.to_vec()));
    container.insert(MakernoteSection::SourceFormat, Value::from(format!("{:?}", format)));
    container
  }

  /// Container without header tags, for a section that holds several parts
  pub(crate) fn nested() -> Self {
    Self {
      ifd: IFD::default(),
      sections: 0,
    }
  }

  fn insert(&mut self, tag: impl Into<u16>, value: Value) {
    let tag = tag.into();
    self.ifd.entries.insert(tag, Entry { tag, value, embedded: None });
  }

  /// Add a section with the original bytes and the parsed IFD, if the bytes are a TIFF structure
  pub(crate) fn add_blob(&mut self, tag: impl Into<u16>, bytes: Vec<u8>, parsed: Option<IFD>) {
    let tag = tag.into();
    self.insert(tag, Value::Undefined(bytes));
    if let Some(parsed) = parsed {
      self.ifd.sub.insert(tag, vec![parsed]);
    }
    self.sections += 1;
  }

  /// Add a section for an IFD that is referenced by an offset in the TIFF structure
  pub(crate) fn add_pointer_ifd(&mut self, tag: impl Into<u16>, offset: u32, ifd: IFD) {
    let tag = tag.into();
    self.insert(tag, Value::Long(vec![offset]));
    self.ifd.sub.insert(tag, vec![ifd]);
    self.sections += 1;
  }

  /// Add the makernote of an EXIF IFD with its byte order and offset.
  /// `parsed` is the makernote IFD as the decoder parsed it.
  pub(crate) fn add_exif_makernote(&mut self, exif: Option<&IFD>, parsed: Option<&IFD>) {
    let Some(exif) = exif else { return };
    let Some(entry) = exif.get_entry(ExifTag::MakerNotes) else { return };
    let Ok(bytes) = entry.value.get_data() else {
      log::warn!("EXIF makernote has an unexpected {} value", entry.value.value_type_name());
      return;
    };
    let offset = entry.offset().and_then(|offset| u32::try_from(offset).ok());
    self.add_makernote(exif.endian, offset, bytes.to_vec(), parsed.cloned());
  }

  /// Add a makernote with the byte order of its TIFF structure and its offset from the start of it
  pub(crate) fn add_makernote(&mut self, endian: Endian, offset: Option<u32>, bytes: Vec<u8>, parsed: Option<IFD>) {
    let byte_order = match endian {
      Endian::Little => "II",
      Endian::Big => "MM",
    };
    self.insert(MakernoteSection::ExifMakerNoteByteOrder, Value::from(byte_order));
    if let Some(offset) = offset {
      self.insert(MakernoteSection::ExifMakerNoteOriginalOffset, Value::Long(vec![offset]));
    }
    self.add_blob(MakernoteSection::ExifMakerNote, bytes, parsed);
  }

  /// The container, `None` if no section was added
  pub(crate) fn build(self) -> Option<IFD> {
    (self.sections > 0).then_some(self.ifd)
  }
}

/// Container with the makernote of `exif`, `None` if it has none
pub(crate) fn from_exif(format: FormatHint, exif: Option<&IFD>, parsed: Option<&IFD>) -> Option<IFD> {
  let mut container = VirtualMakernotes::new(format);
  container.add_exif_makernote(exif, parsed);
  container.build()
}
