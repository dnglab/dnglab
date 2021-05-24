// SPDX-License-Identifier: LGPL-2.1
// Copyright by image-tiff authors (see https://github.com/image-rs/image-tiff)
// Copyright 2021 Daniel Vogelbacher <daniel@chaospixel.com>

use super::error::DngResult;
use std::io::{self, Seek, SeekFrom, Write};

pub fn write_tiff_header<W: Write>(writer: &mut DngWriter<W>) -> DngResult<()> {
    #[cfg(target_endian = "little")]
    let boi: u8 = 0x49;
    #[cfg(not(target_endian = "little"))]
    let boi: u8 = 0x4d;

    writer.writer.write_all(&[boi, boi])?;
    writer.writer.write_all(&42_u16.to_ne_bytes())?;
    writer.offset += 4;

    Ok(())
}

/// Writes a BigTiff header, excluding the IFD offset field.
///
/// Writes the byte order, version number, offset byte size, and zero constant fields. Does
// _not_ write the offset to the first IFD, this should be done by the caller.
pub fn write_bigtiff_header<W: Write>(writer: &mut DngWriter<W>) -> DngResult<()> {
    #[cfg(target_endian = "little")]
    let boi: u8 = 0x49;
    #[cfg(not(target_endian = "little"))]
    let boi: u8 = 0x4d;

    // byte order indication
    writer.writer.write_all(&[boi, boi])?;
    // version number
    writer.writer.write_all(&43_u16.to_ne_bytes())?;
    // bytesize of offsets (pointer size)
    writer.writer.write_all(&8_u16.to_ne_bytes())?;
    // always 0
    writer.writer.write_all(&0_u16.to_ne_bytes())?;

    // we wrote 8 bytes, so set the internal offset accordingly
    writer.offset += 8;

    Ok(())
}

pub struct DngWriter<W> {
    writer: W,
    offset: u64,
}

impl<W: Write> DngWriter<W> {
    pub fn new(writer: W) -> Self {
        Self { writer, offset: 0 }
    }

    pub fn offset(&self) -> u64 {
        self.offset
    }

    pub fn write_bytes(&mut self, bytes: &[u8]) -> Result<(), io::Error> {
        self.writer.write_all(bytes)?;
        self.offset += bytes.len() as u64;
        Ok(())
    }

    pub fn write_u8(&mut self, n: u8) -> Result<(), io::Error> {
        self.writer.write_all(&n.to_ne_bytes())?;
        self.offset += 1;
        Ok(())
    }

    pub fn write_i8(&mut self, n: i8) -> Result<(), io::Error> {
        self.writer.write_all(&n.to_ne_bytes())?;
        self.offset += 1;
        Ok(())
    }

    pub fn write_u16(&mut self, n: u16) -> Result<(), io::Error> {
        self.writer.write_all(&n.to_ne_bytes())?;
        self.offset += 2;

        Ok(())
    }

    pub fn write_i16(&mut self, n: i16) -> Result<(), io::Error> {
        self.writer.write_all(&n.to_ne_bytes())?;
        self.offset += 2;

        Ok(())
    }

    pub fn write_u32(&mut self, n: u32) -> Result<(), io::Error> {
        self.writer.write_all(&n.to_ne_bytes())?;
        self.offset += 4;

        Ok(())
    }

    pub fn write_i32(&mut self, n: i32) -> Result<(), io::Error> {
        self.writer.write_all(&n.to_ne_bytes())?;
        self.offset += 4;

        Ok(())
    }

    pub fn write_u64(&mut self, n: u64) -> Result<(), io::Error> {
        self.writer.write_all(&n.to_ne_bytes())?;
        self.offset += 8;

        Ok(())
    }

    pub fn write_i64(&mut self, n: i64) -> Result<(), io::Error> {
        self.writer.write_all(&n.to_ne_bytes())?;
        self.offset += 8;

        Ok(())
    }

    pub fn write_f32(&mut self, n: f32) -> Result<(), io::Error> {
        self.writer.write_all(&u32::to_ne_bytes(n.to_bits()))?;
        self.offset += 4;

        Ok(())
    }

    pub fn write_f64(&mut self, n: f64) -> Result<(), io::Error> {
        self.writer.write_all(&u64::to_ne_bytes(n.to_bits()))?;
        self.offset += 8;

        Ok(())
    }

    pub fn pad_word_boundary(&mut self) -> Result<(), io::Error> {
        if self.offset % 4 != 0 {
            let padding = [0, 0, 0];
            let padd_len = 4 - (self.offset % 4);
            self.writer.write_all(&padding[..padd_len as usize])?;
            self.offset += padd_len;
        }

        Ok(())
    }
}

impl<W: Seek> DngWriter<W> {
    pub fn goto_offset(&mut self, offset: u64) -> Result<(), io::Error> {
        self.offset = offset;
        self.writer.seek(SeekFrom::Start(offset as u64))?;
        Ok(())
    }
}
