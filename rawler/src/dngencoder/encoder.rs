// SPDX-License-Identifier: LGPL-2.1
// Copyright by image-tiff authors (see https://github.com/image-rs/image-tiff)
// Copyright 2021 Daniel Vogelbacher <daniel@chaospixel.com>

use std::{
    collections::BTreeMap,
    convert::{TryFrom, TryInto},
    io::{Seek, Write},
    marker::PhantomData,
    mem,
    num::TryFromIntError,
};

pub use super::tiff_value::*;

use super::tags::Tag;
use super::{error::DngResult, tags};

use super::cfa_image::CfaImageEncoder;
use super::preview_image::PreviewImageEncoder;

use super::colortype::*;
use super::writer::*;

/// Encoder for Tiff and BigTiff files.
///
/// With this type you can get a `DirectoryEncoder` or a `ImageEncoder`
/// to encode Tiff/BigTiff ifd directories with images.
///
/// See `DirectoryEncoder` and `ImageEncoder`.
pub struct DngEncoder<W, K: TiffKind = TiffKindStandard> {
    writer: DngWriter<W>,
    kind: PhantomData<K>,
}

/// Constructor functions to create standard Tiff files.
impl<W: Write + Seek> DngEncoder<W> {
    /// Creates a new encoder for standard Tiff files.
    ///
    /// To create BigTiff files, use [`new_big`][DngEncoder::new_big] or
    /// [`new_generic`][DngEncoder::new_generic].
    pub fn new(writer: W) -> DngResult<DngEncoder<W, TiffKindStandard>> {
        DngEncoder::new_generic(writer)
    }

    pub fn writer_mut(&mut self) -> &mut DngWriter<W> {
        &mut self.writer
    }
}

/// Constructor functions to create BigTiff files.
impl<W: Write + Seek> DngEncoder<W, TiffKindBig> {
    /// Creates a new encoder for BigTiff files.
    ///
    /// To create standard Tiff files, use [`new`][DngEncoder::new] or
    /// [`new_generic`][DngEncoder::new_generic].
    pub fn new_big(writer: W) -> DngResult<Self> {
        DngEncoder::new_generic(writer)
    }
}

/// Generic functions that are available for both Tiff and BigTiff encoders.
impl<W: Write + Seek, K: TiffKind> DngEncoder<W, K> {
    /// Creates a new Tiff or BigTiff encoder, inferred from the return type.
    pub fn new_generic(writer: W) -> DngResult<Self> {
        let mut encoder = DngEncoder {
            writer: DngWriter::new(writer),
            kind: PhantomData,
        };

        K::write_header(&mut encoder.writer)?;
        K::write_offset(&mut encoder.writer, 0)?; // IFD0 Placeholder
        Ok(encoder)
    }

    /// TODO: doc
    pub fn update_ifd0_offset(&mut self, offset: K::OffsetType) -> DngResult<()> {
        K::update_ifd0_offset(&mut self.writer, offset)?;
        Ok(())
    }

    /// Create a [`DirectoryEncoder`] to encode an ifd directory.
    pub fn new_directory(&mut self) -> DngResult<DirectoryEncoder<W, K>> {
        DirectoryEncoder::new(&mut self.writer)
    }

    /// Create an [`PlainImageEncoder`] to encode an image one slice at a time.
    pub fn new_plain_image<C: ColorType>(
        &mut self,
        width: u32,
        height: u32,
    ) -> DngResult<CfaImageEncoder<W, C, K>> {
        let encoder = DirectoryEncoder::new(&mut self.writer)?;
        CfaImageEncoder::new(encoder, width, height)
    }

    /// Create an [`ImageEncoder`] to encode an image one slice at a time.
    pub fn new_preview_image<C: ColorType>(
        &mut self,
        width: u32,
        height: u32,
    ) -> DngResult<PreviewImageEncoder<W, C, K>> {
        let encoder = DirectoryEncoder::new(&mut self.writer)?;
        PreviewImageEncoder::new(encoder, width, height)
    }

    /// Convenience function to write an entire image from memory.
    pub fn write_image<C: ColorType>(
        &mut self,
        width: u32,
        height: u32,
        data: &[C::Inner],
    ) -> DngResult<K::OffsetType>
    where
        [C::Inner]: TiffValue,
    {
        let encoder = DirectoryEncoder::new(&mut self.writer)?;
        let image: CfaImageEncoder<W, C, K> = CfaImageEncoder::new(encoder, width, height)?;
        Ok(image.write_data(data)?)
    }
}

/// Low level interface to encode ifd directories.
///
/// You should call `finish` on this when you are finished with it.
/// Encoding can silently fail while this is dropping.
pub struct DirectoryEncoder<'a, W: 'a + Write + Seek, K: TiffKind> {
    pub writer: &'a mut DngWriter<W>,
    // We use BTreeMap to make sure tags are written in correct order
    pub ifd_pointer_pos: u64,
    ifd: BTreeMap<u16, DirectoryEntry<K::OffsetType>>,
}

impl<'a, W: 'a + Write + Seek, K: TiffKind> DirectoryEncoder<'a, W, K> {
    pub fn new(writer: &'a mut DngWriter<W>) -> DngResult<Self> {
        // the previous word is the IFD offset position
        let ifd_pointer_pos = writer.offset() - mem::size_of::<K::OffsetType>() as u64;
        writer.pad_word_boundary()?; // TODO: Do we need to adjust this for BigTiff?
        Ok(DirectoryEncoder {
            writer,
            ifd_pointer_pos,
            ifd: BTreeMap::new(),
        })
    }

    /// Write a single ifd tag.
    pub fn write_tag<T: TiffValue>(&mut self, tag: Tag, value: T) -> DngResult<()> {
        let mut bytes = Vec::with_capacity(value.bytes());
        {
            let mut writer = DngWriter::new(&mut bytes);
            value.write(&mut writer)?;
        }

        self.ifd.insert(
            tag.to_u16(),
            DirectoryEntry {
                data_type: <T>::FIELD_TYPE.to_u16(),
                count: value.count().try_into()?,
                data: bytes,
            },
        );

        Ok(())
    }

    /// Write a single ifd tag.
    pub fn write_tag_undefined<T: TiffValue>(&mut self, tag: Tag, value: T) -> DngResult<()> {
        let mut bytes = Vec::with_capacity(value.bytes());
        {
            let mut writer = DngWriter::new(&mut bytes);
            value.write(&mut writer)?;
        }

        self.ifd.insert(
            tag.to_u16(),
            DirectoryEntry {
                data_type: tags::Type::UNDEFINED.to_u16(),
                count: value.count().try_into()?,
                data: bytes,
            },
        );

        Ok(())
    }

    pub fn write_directory(&mut self) -> DngResult<K::OffsetType> {
        // Start by writing out all values
        for &mut DirectoryEntry {
            data: ref mut bytes,
            ..
        } in self.ifd.values_mut()
        {
            let data_bytes = mem::size_of::<K::OffsetType>();

            if bytes.len() > data_bytes {
                let offset = self.writer.offset();
                self.writer.write_bytes(bytes)?;
                *bytes = vec![0; data_bytes];
                let mut writer = DngWriter::new(bytes as &mut [u8]);
                K::write_offset(&mut writer, offset)?;
            } else {
                while bytes.len() < data_bytes {
                    bytes.push(0);
                }
            }
        }

        let offset = self.writer.offset();

        K::write_entry_count(&mut self.writer, self.ifd.len())?;
        for (
            tag,
            &DirectoryEntry {
                data_type: ref field_type,
                ref count,
                data: ref offset,
            },
        ) in self.ifd.iter()
        {
            self.writer.write_u16(*tag)?;
            self.writer.write_u16(*field_type)?;
            (*count).write(&mut self.writer)?;
            self.writer.write_bytes(offset)?;
        }

        Ok(K::convert_offset(offset)?)
    }

    /// Write some data to the tiff file, the offset of the data is returned.
    ///
    /// This could be used to write tiff strips.
    pub fn write_data<T: TiffValue>(&mut self, value: T) -> DngResult<K::OffsetType> {
        let offset = self.writer.offset();
        value.write(&mut self.writer)?;
        Ok(K::convert_offset(offset)?)
    }

    pub fn finish(mut self) -> DngResult<K::OffsetType> {
        // TODO: new by myself
        let ifd_pointer = self.write_directory()?;
        K::write_offset(&mut self.writer, 0)?; // 0 ptr IFD

        Ok(ifd_pointer)
    }
}

pub struct DirectoryEntry<S> {
    data_type: u16,
    count: S,
    data: Vec<u8>,
}

/// Trait to abstract over Tiff/BigTiff differences.
///
/// Implemented for [`TiffKindStandard`] and [`TiffKindBig`].
pub trait TiffKind {
    /// The type of offset fields, `u32` for normal Tiff, `u64` for BigTiff.
    type OffsetType: TryFrom<usize, Error = TryFromIntError> + Into<u64> + TiffValue;

    /// Needed for the `convert_slice` method.
    type OffsetArrayType: ?Sized + TiffValue;

    /// Write the (Big)Tiff header.
    fn write_header<W: Write>(writer: &mut DngWriter<W>) -> DngResult<()>;

    /// Convert a file offset to `Self::OffsetType`.
    ///
    /// This returns an error for normal Tiff if the offset is larger than `u32::MAX`.
    fn convert_offset(offset: u64) -> DngResult<Self::OffsetType>;

    /// Write an offset value to the given writer.
    ///
    /// Like `convert_offset`, this errors if `offset > u32::MAX` for normal Tiff.
    fn write_offset<W: Write>(writer: &mut DngWriter<W>, offset: u64) -> DngResult<()>;

    // TODO doc
    fn update_ifd0_offset<W: Write + Seek>(
        writer: &mut DngWriter<W>,
        offset: Self::OffsetType,
    ) -> DngResult<()>;

    /// Write the IFD entry count field with the given `count` value.
    ///
    /// The entry count field is an `u16` for normal Tiff and `u64` for BigTiff. Errors
    /// if the given `usize` is larger than the representable values.
    fn write_entry_count<W: Write>(writer: &mut DngWriter<W>, count: usize) -> DngResult<()>;

    /// Internal helper method for satisfying Rust's type checker.
    ///
    /// The `TiffValue` trait is implemented for both primitive values (e.g. `u8`, `u32`) and
    /// slices of primitive values (e.g. `[u8]`, `[u32]`). However, this is not represented in
    /// the type system, so there is no guarantee that that for all `T: TiffValue` there is also
    /// an implementation of `TiffValue` for `[T]`. This method works around that problem by
    /// providing a conversion from `[T]` to some value that implements `TiffValue`, thereby
    /// making all slices of `OffsetType` usable with `write_tag` and similar methods.
    ///
    /// Implementations of this trait should always set `OffsetArrayType` to `[OffsetType]`.
    fn convert_slice(slice: &[Self::OffsetType]) -> &Self::OffsetArrayType;
}

/// Create a standard Tiff file.
pub struct TiffKindStandard;

impl TiffKind for TiffKindStandard {
    type OffsetType = u32;
    type OffsetArrayType = [u32];

    fn write_header<W: Write>(writer: &mut DngWriter<W>) -> DngResult<()> {
        write_tiff_header(writer)?;
        // blank the IFD offset location
        writer.write_u32(0)?;

        Ok(())
    }

    fn convert_offset(offset: u64) -> DngResult<Self::OffsetType> {
        Ok(Self::OffsetType::try_from(offset)?)
    }

    fn write_offset<W: Write>(writer: &mut DngWriter<W>, offset: u64) -> DngResult<()> {
        writer.write_u32(u32::try_from(offset)?)?;
        Ok(())
    }

    fn update_ifd0_offset<W: Write + Seek>(
        writer: &mut DngWriter<W>,
        offset: Self::OffsetType,
    ) -> DngResult<()> {
        let curr_offset = writer.offset();
        writer.goto_offset(4)?;
        writer.write_u32(offset)?;
        writer.goto_offset(curr_offset)?;
        Ok(())
    }

    fn write_entry_count<W: Write>(writer: &mut DngWriter<W>, count: usize) -> DngResult<()> {
        writer.write_u16(u16::try_from(count)?)?;

        Ok(())
    }

    fn convert_slice(slice: &[Self::OffsetType]) -> &Self::OffsetArrayType {
        slice
    }
}

/// Create a BigTiff file.
pub struct TiffKindBig;

impl TiffKind for TiffKindBig {
    type OffsetType = u64;
    type OffsetArrayType = [u64];

    fn write_header<W: Write>(writer: &mut DngWriter<W>) -> DngResult<()> {
        write_bigtiff_header(writer)?;
        // blank the IFD offset location
        writer.write_u64(0)?;

        Ok(())
    }

    fn convert_offset(offset: u64) -> DngResult<Self::OffsetType> {
        Ok(offset)
    }

    fn write_offset<W: Write>(writer: &mut DngWriter<W>, offset: u64) -> DngResult<()> {
        writer.write_u64(offset)?;
        Ok(())
    }

    fn update_ifd0_offset<W: Write + Seek>(
        writer: &mut DngWriter<W>,
        offset: Self::OffsetType,
    ) -> DngResult<()> {
        let curr_offset = writer.offset();
        writer.goto_offset(4)?; // TODO correct offset?
        writer.write_u64(offset)?;
        writer.goto_offset(curr_offset)?;
        Ok(())
    }

    fn write_entry_count<W: Write>(writer: &mut DngWriter<W>, count: usize) -> DngResult<()> {
        writer.write_u64(u64::try_from(count)?)?;
        Ok(())
    }

    fn convert_slice(slice: &[Self::OffsetType]) -> &Self::OffsetArrayType {
        slice
    }
}
