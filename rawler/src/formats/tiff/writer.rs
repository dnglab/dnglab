// SPDX-License-Identifier: MIT
// Copyright 2021 Daniel Vogelbacher <daniel@chaospixel.com>

use std::{
  collections::BTreeMap,
  io::{Seek, SeekFrom, Write},
};

use byteorder::{LittleEndian, NativeEndian, WriteBytesExt};

use crate::tags::TiffTag;

use super::{Entry, Result, TiffError, Value, TIFF_MAGIC};

pub struct TiffWriter<'w> {
  ifd_location: u64,
  pub writer: &'w mut (dyn WriteAndSeek + Send),
}

pub trait WriteAndSeek: Write + Seek {}

impl<T> WriteAndSeek for T where T: Write + Seek {}

impl<'w> TiffWriter<'w> {
  pub fn new<T: WriteAndSeek + Send>(writer: &'w mut T) -> Result<Self> {
    let mut tmp = Self { writer, ifd_location: 0 };
    tmp.write_header()?;
    Ok(tmp)
  }

  pub fn new_directory(&mut self) -> DirectoryWriter<'_, 'w> {
    DirectoryWriter::new(self)
  }

  fn write_header(&mut self) -> Result<()> {
    #[cfg(target_endian = "little")]
    let boi: u8 = 0x49;
    #[cfg(not(target_endian = "little"))]
    let boi: u8 = 0x4d;

    self.writer.write_all(&[boi, boi])?;
    self.writer.write_u16::<NativeEndian>(TIFF_MAGIC)?;
    self.ifd_location = self.writer.stream_position()?;
    self.writer.write_u32::<NativeEndian>(0_u32)?;

    Ok(())
  }

  pub(crate) fn pad_word_boundary(&mut self) -> Result<()> {
    if self.position()? % 4 != 0 {
      let padding = [0, 0, 0];
      let padd_len = 4 - (self.position()? % 4);
      self.writer.write_all(&padding[..padd_len as usize])?;
    }
    Ok(())
  }

  pub fn build(self, ifd0_offset: u32) -> Result<()> {
    self.writer.seek(SeekFrom::Start(self.ifd_location))?;
    self.writer.write_u32::<NativeEndian>(ifd0_offset)?;
    Ok(())
  }

  pub fn position(&mut self) -> Result<u32> {
    Ok(self.writer.stream_position().map(|v| v as u32)?) // TODO: try_from?
  }
}

pub struct DirectoryWriter<'a, 'w> {
  pub tiff: &'a mut TiffWriter<'w>,
  // We use BTreeMap to make sure tags are written in correct order
  entries: BTreeMap<u16, Entry>,
  next_ifd: u32,
}

impl<'a, 'w> DirectoryWriter<'a, 'w> {
  pub fn new(tiff: &'a mut TiffWriter<'w>) -> Self {
    Self {
      tiff,
      entries: BTreeMap::new(),
      next_ifd: 0,
    }
  }

  pub fn new_directory(&mut self) -> DirectoryWriter<'_, 'w> {
    DirectoryWriter::new(self.tiff)
  }

  pub fn entry_count(&self) -> u16 {
    self.entries.len() as u16
  }

  pub fn build(mut self) -> Result<u32> {
    if self.entries.is_empty() {
      return Err(TiffError::General("IFD is empty, not allowed by TIFF specification".to_string()));
    }
    for &mut Entry {
      ref mut value,
      ref mut embedded,
      ..
    } in self.entries.values_mut()
    {
      let data_bytes = 4;

      if value.byte_size() > data_bytes {
        self.tiff.pad_word_boundary()?;
        let offset = self.tiff.position()?;
        value.write(self.tiff.writer)?;
        embedded.replace(offset);
      } else {
        embedded.replace(value.as_embedded()?);
      }
    }

    self.tiff.pad_word_boundary()?;
    let offset = self.tiff.position()?;

    self.tiff.writer.write_all(&self.entry_count().to_ne_bytes())?;

    for (tag, entry) in self.entries {
      self.tiff.writer.write_u16::<NativeEndian>(tag)?;
      self.tiff.writer.write_u16::<NativeEndian>(entry.value_type())?;
      self.tiff.writer.write_u32::<NativeEndian>(entry.count())?;
      self.tiff.writer.write_u32::<NativeEndian>(entry.embedded.unwrap())?;
    }
    self.tiff.writer.write_u32::<NativeEndian>(self.next_ifd)?; // Next IFD

    Ok(offset)
  }

  pub fn add_tag<T: TiffTag, V: Into<Value>>(&mut self, tag: T, value: V) -> Result<()> {
    let tag: u16 = tag.into();
    self.entries.insert(
      tag,
      Entry {
        tag,
        value: value.into(),
        embedded: None,
      },
    );
    Ok(())
  }

  pub fn add_untyped_tag<V: Into<Value>>(&mut self, tag: u16, value: V) -> Result<()> {
    self.entries.insert(
      tag,
      Entry {
        tag,
        value: value.into(),
        embedded: None,
      },
    );
    Ok(())
  }

  pub fn add_tag_undefined<T: TiffTag>(&mut self, tag: T, data: Vec<u8>) -> Result<()> {
    let tag: u16 = tag.into();
    //let data = data.as_ref();
    //let offset = self.write_data(data)?;
    self.entries.insert(
      tag,
      Entry {
        tag,
        value: Value::Undefined(data),
        embedded: None,
      },
    );
    Ok(())
  }

  pub fn add_value<T: TiffTag>(&mut self, tag: T, value: Value) -> Result<()> {
    let tag: u16 = tag.into();
    self.entries.insert(tag, Entry { tag, value, embedded: None });
    Ok(())
  }

  /*
  pub fn add_entry(&mut self, entry: Entry) {
    self.ifd.insert(tag.into(), entry);
  }
   */

  pub fn write_data(&mut self, data: &[u8]) -> Result<u32> {
    self.tiff.pad_word_boundary()?;
    let offset = self.tiff.position()?;
    self.tiff.writer.write_all(data)?;
    Ok(offset)
  }

  pub fn write_data_u16_be(&mut self, data: &[u16]) -> Result<u32> {
    self.tiff.pad_word_boundary()?;
    let offset = self.tiff.position()?;
    for v in data {
      self.tiff.writer.write_u16::<LittleEndian>(*v)?; // TODO bug?
    }
    Ok(offset)
  }
}
