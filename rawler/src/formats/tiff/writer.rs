// SPDX-License-Identifier: MIT
// Copyright 2021 Daniel Vogelbacher <daniel@chaospixel.com>

use std::{
  collections::BTreeMap,
  io::{Seek, SeekFrom, Write},
};

use byteorder::{LittleEndian, NativeEndian, WriteBytesExt};

use crate::tags::TiffTag;

use super::{Entry, Result, TiffError, Value, TIFF_MAGIC};

pub struct TiffWriter<W> {
  ifd_location: u64,
  pub writer: W,
}

impl<W> TiffWriter<W>
where
  W: Write + Seek,
{
  pub fn new(writer: W) -> Result<Self> {
    let mut tmp = Self { writer, ifd_location: 0 };
    tmp.write_header()?;
    Ok(tmp)
  }

  pub fn new_directory(&self) -> DirectoryWriter {
    DirectoryWriter::new()
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

  pub fn write_data(&mut self, data: &[u8]) -> Result<u32>
  where
    W: Seek + Write,
  {
    self.pad_word_boundary()?;
    let offset = self.position()?;
    self.writer.write_all(data)?;
    Ok(offset)
  }

  pub fn write_data_u16_le(&mut self, data: &[u16]) -> Result<u32>
  where
    W: Seek + Write,
  {
    self.pad_word_boundary()?;
    let offset = self.position()?;
    for v in data {
      self.writer.write_u16::<LittleEndian>(*v)?;
    }
    Ok(offset)
  }

  pub fn write_data_f32_le(&mut self, data: &[f32]) -> Result<u32>
  where
    W: Seek + Write,
  {
    self.pad_word_boundary()?;
    let offset = self.position()?;
    for v in data {
      self.writer.write_f32::<LittleEndian>(*v)?;
    }
    Ok(offset)
  }

  pub(crate) fn pad_word_boundary(&mut self) -> Result<()> {
    if self.position()? % 4 != 0 {
      let padding = [0, 0, 0];
      let padd_len = 4 - (self.position()? % 4);
      self.writer.write_all(&padding[..padd_len as usize])?;
    }
    Ok(())
  }

  pub fn build(mut self, root_ifd: DirectoryWriter) -> Result<()> {
    let ifd0_offset = root_ifd.build(&mut self)?;
    self.writer.seek(SeekFrom::Start(self.ifd_location))?;
    self.writer.write_u32::<NativeEndian>(ifd0_offset)?;
    Ok(())
  }
}

impl<W> TiffWriter<W>
where
  W: Seek,
{
  pub fn position(&mut self) -> Result<u32> {
    Ok(self.writer.stream_position().map(|v| v as u32)?) // TODO: try_from?
  }
}

#[derive(Default)]
pub struct DirectoryWriter {
  // We use BTreeMap to make sure tags are written in correct order
  entries: BTreeMap<u16, Entry>,
  next_ifd: u32,
}

impl DirectoryWriter {
  pub fn remove_tag<T: TiffTag>(&mut self, tag: T) {
    let tag: u16 = tag.into();
    self.entries.remove(&tag);
  }

  pub fn add_tag<T: TiffTag, V: Into<Value>>(&mut self, tag: T, value: V) {
    let tag: u16 = tag.into();
    self.entries.insert(
      tag,
      Entry {
        tag,
        value: value.into(),
        embedded: None,
      },
    );
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

  pub fn add_value<T: TiffTag>(&mut self, tag: T, value: Value) {
    let tag: u16 = tag.into();
    self.entries.insert(tag, Entry { tag, value, embedded: None });
  }

  pub fn entry_count(&self) -> u16 {
    self.entries.len() as u16
  }
}

impl DirectoryWriter {
  pub fn new() -> Self {
    Self {
      entries: BTreeMap::new(),
      next_ifd: 0,
    }
  }

  pub fn is_empty(&self) -> bool {
    self.entries.is_empty()
  }

  pub fn build<W>(mut self, tiff: &mut TiffWriter<W>) -> Result<u32>
  where
    W: Seek + Write,
  {
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
        tiff.pad_word_boundary()?;
        let offset = tiff.position()?;
        value.write(&mut tiff.writer)?;
        embedded.replace(offset as u32);
      } else {
        embedded.replace(value.as_embedded()?);
      }
    }

    tiff.pad_word_boundary()?;
    let offset = tiff.position()?;

    tiff.writer.write_all(&self.entry_count().to_ne_bytes())?;

    for (tag, entry) in self.entries {
      tiff.writer.write_u16::<NativeEndian>(tag)?;
      tiff.writer.write_u16::<NativeEndian>(entry.value_type())?;
      tiff.writer.write_u32::<NativeEndian>(entry.count())?;
      tiff.writer.write_u32::<NativeEndian>(entry.embedded.unwrap())?;
    }
    tiff.writer.write_u32::<NativeEndian>(self.next_ifd)?; // Next IFD

    Ok(offset)
  }

  /*
  pub fn add_entry(&mut self, entry: Entry) {
    self.ifd.insert(tag.into(), entry);
  }
   */
}
