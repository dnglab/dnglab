// SPDX-License-Identifier: LGPL-2.1
// Copyright by image-tiff authors (see https://github.com/image-rs/image-tiff)
// Copyright 2021 Daniel Vogelbacher <daniel@chaospixel.com>

use std::io::Write;

use super::writer::DngWriter;
use super::{bytecast, error::DngError, tags::Type, DngResult};

/// Trait for types that can be encoded in a tiff file
pub trait TiffValue {
    const BYTE_LEN: u8;
    const FIELD_TYPE: Type;
    fn count(&self) -> usize;
    fn bytes(&self) -> usize {
        self.count() * usize::from(Self::BYTE_LEN)
    }
    fn write<W: Write>(&self, writer: &mut DngWriter<W>) -> DngResult<()>;
}

impl TiffValue for [u8] {
    const BYTE_LEN: u8 = 1;
    const FIELD_TYPE: Type = Type::BYTE;

    fn count(&self) -> usize {
        self.len()
    }

    fn write<W: Write>(&self, writer: &mut DngWriter<W>) -> DngResult<()> {
        writer.write_bytes(self)?;
        Ok(())
    }
}

impl TiffValue for [i8] {
    const BYTE_LEN: u8 = 1;
    const FIELD_TYPE: Type = Type::SBYTE;

    fn count(&self) -> usize {
        self.len()
    }

    fn write<W: Write>(&self, writer: &mut DngWriter<W>) -> DngResult<()> {
        let slice = bytecast::i8_as_ne_bytes(self);
        writer.write_bytes(slice)?;
        Ok(())
    }
}

impl TiffValue for [u16] {
    const BYTE_LEN: u8 = 2;
    const FIELD_TYPE: Type = Type::SHORT;

    fn count(&self) -> usize {
        self.len()
    }

    fn write<W: Write>(&self, writer: &mut DngWriter<W>) -> DngResult<()> {
        let slice = bytecast::u16_as_ne_bytes(self);
        writer.write_bytes(slice)?;
        Ok(())
    }
}

impl TiffValue for [i16] {
    const BYTE_LEN: u8 = 2;
    const FIELD_TYPE: Type = Type::SSHORT;

    fn count(&self) -> usize {
        self.len()
    }

    fn write<W: Write>(&self, writer: &mut DngWriter<W>) -> DngResult<()> {
        let slice = bytecast::i16_as_ne_bytes(self);
        writer.write_bytes(slice)?;
        Ok(())
    }
}

impl TiffValue for [u32] {
    const BYTE_LEN: u8 = 4;
    const FIELD_TYPE: Type = Type::LONG;

    fn count(&self) -> usize {
        self.len()
    }

    fn write<W: Write>(&self, writer: &mut DngWriter<W>) -> DngResult<()> {
        let slice = bytecast::u32_as_ne_bytes(self);
        writer.write_bytes(slice)?;
        Ok(())
    }
}

impl TiffValue for [i32] {
    const BYTE_LEN: u8 = 4;
    const FIELD_TYPE: Type = Type::SLONG;

    fn count(&self) -> usize {
        self.len()
    }

    fn write<W: Write>(&self, writer: &mut DngWriter<W>) -> DngResult<()> {
        let slice = bytecast::i32_as_ne_bytes(self);
        writer.write_bytes(slice)?;
        Ok(())
    }
}

impl TiffValue for [u64] {
    const BYTE_LEN: u8 = 8;
    const FIELD_TYPE: Type = Type::LONG8;

    fn count(&self) -> usize {
        self.len()
    }

    fn write<W: Write>(&self, writer: &mut DngWriter<W>) -> DngResult<()> {
        let slice = bytecast::u64_as_ne_bytes(self);
        writer.write_bytes(slice)?;
        Ok(())
    }
}

impl TiffValue for [i64] {
    const BYTE_LEN: u8 = 8;
    const FIELD_TYPE: Type = Type::SLONG8;

    fn count(&self) -> usize {
        self.len()
    }

    fn write<W: Write>(&self, writer: &mut DngWriter<W>) -> DngResult<()> {
        let slice = bytecast::i64_as_ne_bytes(self);
        writer.write_bytes(slice)?;
        Ok(())
    }
}

impl TiffValue for [f32] {
    const BYTE_LEN: u8 = 4;
    const FIELD_TYPE: Type = Type::FLOAT;

    fn count(&self) -> usize {
        self.len()
    }

    fn write<W: Write>(&self, writer: &mut DngWriter<W>) -> DngResult<()> {
        // We write using nativeedian so this sould be safe
        let slice = bytecast::f32_as_ne_bytes(self);
        writer.write_bytes(slice)?;
        Ok(())
    }
}

impl TiffValue for [f64] {
    const BYTE_LEN: u8 = 8;
    const FIELD_TYPE: Type = Type::DOUBLE;

    fn count(&self) -> usize {
        self.len()
    }

    fn write<W: Write>(&self, writer: &mut DngWriter<W>) -> DngResult<()> {
        // We write using nativeedian so this sould be safe
        let slice = bytecast::f64_as_ne_bytes(self);
        writer.write_bytes(slice)?;
        Ok(())
    }
}

impl TiffValue for [Ifd] {
    const BYTE_LEN: u8 = 4;
    const FIELD_TYPE: Type = Type::IFD;

    fn count(&self) -> usize {
        self.len()
    }

    fn write<W: Write>(&self, writer: &mut DngWriter<W>) -> DngResult<()> {
        for x in self {
            x.write(writer)?;
        }
        Ok(())
    }
}

impl TiffValue for [Ifd8] {
    const BYTE_LEN: u8 = 8;
    const FIELD_TYPE: Type = Type::IFD8;

    fn count(&self) -> usize {
        self.len()
    }

    fn write<W: Write>(&self, writer: &mut DngWriter<W>) -> DngResult<()> {
        for x in self {
            x.write(writer)?;
        }
        Ok(())
    }
}

impl TiffValue for [Rational] {
    const BYTE_LEN: u8 = 8;
    const FIELD_TYPE: Type = Type::RATIONAL;

    fn count(&self) -> usize {
        self.len()
    }

    fn write<W: Write>(&self, writer: &mut DngWriter<W>) -> DngResult<()> {
        for x in self {
            x.write(writer)?;
        }
        Ok(())
    }
}

impl TiffValue for [SRational] {
    const BYTE_LEN: u8 = 8;
    const FIELD_TYPE: Type = Type::SRATIONAL;

    fn count(&self) -> usize {
        self.len()
    }

    fn write<W: Write>(&self, writer: &mut DngWriter<W>) -> DngResult<()> {
        for x in self {
            x.write(writer)?;
        }
        Ok(())
    }
}

impl TiffValue for u8 {
    const BYTE_LEN: u8 = 1;
    const FIELD_TYPE: Type = Type::BYTE;

    fn count(&self) -> usize {
        1
    }

    fn write<W: Write>(&self, writer: &mut DngWriter<W>) -> DngResult<()> {
        writer.write_u8(*self)?;
        Ok(())
    }
}

impl TiffValue for i8 {
    const BYTE_LEN: u8 = 1;
    const FIELD_TYPE: Type = Type::SBYTE;

    fn count(&self) -> usize {
        1
    }

    fn write<W: Write>(&self, writer: &mut DngWriter<W>) -> DngResult<()> {
        writer.write_i8(*self)?;
        Ok(())
    }
}

impl TiffValue for u16 {
    const BYTE_LEN: u8 = 2;
    const FIELD_TYPE: Type = Type::SHORT;

    fn count(&self) -> usize {
        1
    }

    fn write<W: Write>(&self, writer: &mut DngWriter<W>) -> DngResult<()> {
        writer.write_u16(*self)?;
        Ok(())
    }
}

impl TiffValue for i16 {
    const BYTE_LEN: u8 = 2;
    const FIELD_TYPE: Type = Type::SSHORT;

    fn count(&self) -> usize {
        1
    }

    fn write<W: Write>(&self, writer: &mut DngWriter<W>) -> DngResult<()> {
        writer.write_i16(*self)?;
        Ok(())
    }
}

impl TiffValue for u32 {
    const BYTE_LEN: u8 = 4;
    const FIELD_TYPE: Type = Type::LONG;

    fn count(&self) -> usize {
        1
    }

    fn write<W: Write>(&self, writer: &mut DngWriter<W>) -> DngResult<()> {
        writer.write_u32(*self)?;
        Ok(())
    }
}

impl TiffValue for i32 {
    const BYTE_LEN: u8 = 4;
    const FIELD_TYPE: Type = Type::SLONG;

    fn count(&self) -> usize {
        1
    }

    fn write<W: Write>(&self, writer: &mut DngWriter<W>) -> DngResult<()> {
        writer.write_i32(*self)?;
        Ok(())
    }
}

impl TiffValue for u64 {
    const BYTE_LEN: u8 = 8;
    const FIELD_TYPE: Type = Type::LONG8;

    fn count(&self) -> usize {
        1
    }

    fn write<W: Write>(&self, writer: &mut DngWriter<W>) -> DngResult<()> {
        writer.write_u64(*self)?;
        Ok(())
    }
}

impl TiffValue for i64 {
    const BYTE_LEN: u8 = 8;
    const FIELD_TYPE: Type = Type::SLONG8;

    fn count(&self) -> usize {
        1
    }

    fn write<W: Write>(&self, writer: &mut DngWriter<W>) -> DngResult<()> {
        writer.write_i64(*self)?;
        Ok(())
    }
}

impl TiffValue for f32 {
    const BYTE_LEN: u8 = 4;
    const FIELD_TYPE: Type = Type::FLOAT;

    fn count(&self) -> usize {
        1
    }

    fn write<W: Write>(&self, writer: &mut DngWriter<W>) -> DngResult<()> {
        writer.write_f32(*self)?;
        Ok(())
    }
}

impl TiffValue for f64 {
    const BYTE_LEN: u8 = 8;
    const FIELD_TYPE: Type = Type::DOUBLE;

    fn count(&self) -> usize {
        1
    }

    fn write<W: Write>(&self, writer: &mut DngWriter<W>) -> DngResult<()> {
        writer.write_f64(*self)?;
        Ok(())
    }
}

impl TiffValue for Ifd {
    const BYTE_LEN: u8 = 4;
    const FIELD_TYPE: Type = Type::IFD;

    fn count(&self) -> usize {
        1
    }

    fn write<W: Write>(&self, writer: &mut DngWriter<W>) -> DngResult<()> {
        writer.write_u32(self.0)?;
        Ok(())
    }
}

impl TiffValue for Ifd8 {
    const BYTE_LEN: u8 = 8;
    const FIELD_TYPE: Type = Type::IFD8;

    fn count(&self) -> usize {
        1
    }

    fn write<W: Write>(&self, writer: &mut DngWriter<W>) -> DngResult<()> {
        writer.write_u64(self.0)?;
        Ok(())
    }
}

impl TiffValue for Rational {
    const BYTE_LEN: u8 = 8;
    const FIELD_TYPE: Type = Type::RATIONAL;

    fn count(&self) -> usize {
        1
    }

    fn write<W: Write>(&self, writer: &mut DngWriter<W>) -> DngResult<()> {
        writer.write_u32(self.n)?;
        writer.write_u32(self.d)?;
        Ok(())
    }
}

impl TiffValue for SRational {
    const BYTE_LEN: u8 = 8;
    const FIELD_TYPE: Type = Type::SRATIONAL;

    fn count(&self) -> usize {
        1
    }

    fn write<W: Write>(&self, writer: &mut DngWriter<W>) -> DngResult<()> {
        writer.write_i32(self.n)?;
        writer.write_i32(self.d)?;
        Ok(())
    }
}

impl TiffValue for str {
    const BYTE_LEN: u8 = 1;
    const FIELD_TYPE: Type = Type::ASCII;

    fn count(&self) -> usize {
        self.len() + 1
    }

    fn write<W: Write>(&self, writer: &mut DngWriter<W>) -> DngResult<()> {
        if self.is_ascii() && !self.bytes().any(|b| b == 0) {
            writer.write_bytes(self.as_bytes())?;
            writer.write_u8(0)?;
            Ok(())
        } else {
            Err(DngError::InvalidValue)
        }
    }
}

impl<'a, T: TiffValue + ?Sized> TiffValue for &'a T {
    const BYTE_LEN: u8 = T::BYTE_LEN;
    const FIELD_TYPE: Type = T::FIELD_TYPE;

    fn count(&self) -> usize {
        (*self).count()
    }

    fn write<W: Write>(&self, writer: &mut DngWriter<W>) -> DngResult<()> {
        (*self).write(writer)
    }
}

/// Type to represent tiff values of type `IFD`
#[derive(Clone)]
pub struct Ifd(pub u32);

/// Type to represent tiff values of type `IFD8`
#[derive(Clone)]
pub struct Ifd8(pub u64);

/// Type to represent tiff values of type `RATIONAL`
#[derive(Clone)]
pub struct Rational {
    pub n: u32,
    pub d: u32,
}

/// Type to represent tiff values of type `SRATIONAL`
#[derive(Clone)]
pub struct SRational {
    pub n: i32,
    pub d: i32,
}
