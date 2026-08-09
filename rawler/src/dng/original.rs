// SPDX-License-Identifier: LGPL-2.1
// Copyright 2021 Daniel Vogelbacher <daniel@chaospixel.com>

use byteorder::{BigEndian, ReadBytesExt};
use libflate::zlib::{Decoder, EncodeOptions, Encoder};
use log::debug;
use rayon::prelude::*;
use std::{
  io::{self, Cursor, Read, Seek, SeekFrom, Write},
  mem::size_of,
  ops::Neg,
};

use crate::{
  RawlerError,
  formats::tiff::{GenericTiffReader, Value, reader::TiffReader},
  rawsource::RawSource,
  tags::DngTag,
};

// DNG requires this block size
const COMPRESS_BLOCK_SIZE: u32 = 65536;

pub type OriginalDigest = [u8; 16];

pub struct OriginalCompressed {
  raw_fork_size: u32,
  chunks: Vec<ForkBlock>,
}

impl OriginalCompressed {
  pub fn new<T>(stream: &mut T) -> io::Result<Self>
  where
    T: Read + Seek,
  {
    let start = stream.stream_position()?;

    let raw_fork_size: u32 = stream.read_u32::<BigEndian>()?;
    let raw_fork_blocks: u32 = raw_fork_size.div_ceil(COMPRESS_BLOCK_SIZE); // (raw_fork_size + (COMPRESS_BLOCK_SIZE - 1)) / COMPRESS_BLOCK_SIZE

    let mut index_list: Vec<u32> = Vec::with_capacity(raw_fork_blocks as usize + 1);

    for _ in 0..raw_fork_blocks + 1 {
      let idx = stream.read_u32::<BigEndian>()?;
      index_list.push(idx);
    }

    let mut chunks = Vec::with_capacity(index_list.len());
    let mut iter = index_list.into_iter().map(u64::from);

    if let Some(mut offset) = iter.next() {
      stream.seek(SeekFrom::Start(start + offset))?;
      for end in iter {
        let len = end
          .checked_sub(offset)
          .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "Offset underflow"))?;
        let mut chunk = vec![0; len as usize];
        stream.read_exact(&mut chunk)?;
        chunks.push(ForkBlock::new(chunk));
        offset = end;
      }
    }

    Ok(Self { chunks, raw_fork_size })
  }

  pub fn decompress<T>(&self, stream: &mut T) -> io::Result<usize>
  where
    T: Write,
  {
    let mut total = 0;
    for chunk in self.chunks.iter().map(ForkBlock::decompress) {
      let buf = chunk?;
      stream.write_all(&buf)?;
      total += buf.len();
    }
    Ok(total)
  }

  /// Read bytes from stream until EOF, split into chunks
  /// and compress each one.
  pub fn compress<T>(stream: &mut T) -> io::Result<Self>
  where
    T: Seek + Read,
  {
    let pos = stream.stream_position()?;
    stream.seek(SeekFrom::End(0))?;
    let uncomp_len = stream.stream_position()? - pos;
    stream.seek(SeekFrom::Current((uncomp_len as i64).neg()))?;

    let raw_fork_size = u32::try_from(uncomp_len).map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;
    let raw_fork_blocks = raw_fork_size.div_ceil(COMPRESS_BLOCK_SIZE); // (raw_fork_size + (COMPRESS_BLOCK_SIZE - 1)) / COMPRESS_BLOCK_SIZE
    let mut forks = Vec::with_capacity(raw_fork_blocks as usize);

    loop {
      let mut buf = Vec::with_capacity(COMPRESS_BLOCK_SIZE as usize);
      stream.take(COMPRESS_BLOCK_SIZE as u64).read_to_end(&mut buf)?;
      if buf.is_empty() {
        break;
      }
      forks.push(buf);
      //chunks.push(ForkBlock::compress(&buf)?);
    }
    let chunks = forks.par_iter().flat_map(ForkBlock::compress).collect();
    Ok(Self { raw_fork_size, chunks })
  }

  /// Write compressed chunks to output stream.
  ///
  /// Returns the MD5 digest of everything written to `stream`.
  pub fn write_to_stream<T>(&self, stream: &mut T) -> io::Result<md5::Digest>
  where
    T: Write,
  {
    // Tee every byte into the digest as it is written to the stream.
    let mut stream = DigestWriter::new(stream);
    stream.write_all(&self.raw_fork_size.to_be_bytes())?; // Fork 1
    let chunks_start: u32 = (size_of::<u32>() + (self.chunks.len() + 1) * size_of::<u32>()) as u32;
    // Offset of first chunk
    stream.write_all(&chunks_start.to_be_bytes())?;
    // Write all other end offsets.
    for end in self.chunks.iter().map(ForkBlock::len).scan(chunks_start, |end, len| {
      *end += len as u32;
      Some(*end)
    }) {
      stream.write_all(&end.to_be_bytes())?;
    }
    for chunk in self.chunks.iter() {
      stream.write_all(&chunk.chunk)?;
    }
    stream.write_all(&0u32.to_be_bytes())?;
    stream.write_all(&0u32.to_be_bytes())?;
    stream.write_all(&0u32.to_be_bytes())?;
    stream.write_all(&0u32.to_be_bytes())?;
    stream.write_all(&0u32.to_be_bytes())?;
    stream.write_all(&0u32.to_be_bytes())?;
    stream.write_all(&0u32.to_be_bytes())?;
    Ok(stream.finalize())
  }
}

/// A [`Write`] adapter that forwards all bytes to an inner writer while
/// feeding them into an MD5 context, so the digest of the written stream is
/// available without a second pass over the data.
struct DigestWriter<'a, T> {
  inner: &'a mut T,
  context: md5::Context,
}

impl<'a, T: Write> DigestWriter<'a, T> {
  fn new(inner: &'a mut T) -> Self {
    Self {
      inner,
      context: md5::Context::new(),
    }
  }

  /// Consume the adapter and return the digest of everything written so far.
  fn finalize(self) -> md5::Digest {
    self.context.finalize()
  }
}

impl<T: Write> Write for DigestWriter<'_, T> {
  fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
    let written = self.inner.write(buf)?;
    // Only hash the bytes actually accepted by the inner writer.
    self.context.consume(&buf[..written]);
    Ok(written)
  }

  fn flush(&mut self) -> io::Result<()> {
    self.inner.flush()
  }
}

/// Extract the original raw file embedded in a DNG and write it to `target`.
///
/// Reads the `OriginalRawFileData` tag from `dng`, decompresses the embedded
/// payload and writes the reconstructed original file to `target`. When
/// `verify_digest` is `true`, the decompressed data is checked against the
/// `OriginalRawFileDigest` stored in the DNG.
///
/// Returns the number of bytes written to `target`.
///
/// # Errors
///
/// Fails if `dng` is not a valid DNG, does not embed an original raw file, or
/// if digest verification fails.
pub fn extract_original<W: Write + Seek + Send>(dng: &RawSource, target: &mut W, verify_digest: bool) -> crate::Result<usize> {
  //let mut in_file = BufReader::new(dng_file);
  let file = GenericTiffReader::new(&mut dng.reader(), 0, 0, None, &[])?;

  if !file.has_entry(DngTag::DNGVersion) {
    return Err(RawlerError::DecoderFailed(format!(
      "Input file '{}' is not a valid DNG file",
      dng.path().display()
    )));
  }
  if let Some(orig_data) = file.get_entry(DngTag::OriginalRawFileData) {
    if let Value::Undefined(val) = &orig_data.value {
      let digest = file
        .get_entry(DngTag::OriginalRawFileDigest)
        .map(|entry| entry.value.get_data().as_slice())
        .and_then(|data| OriginalDigest::try_from(data).ok());
      let new_digest = md5::compute(val);

      debug!("Encoded calculated original data digest: {:x?}", digest);
      debug!("New calculated original data digest: {:x?}", new_digest);

      if digest.ne(&Some(new_digest.into())) {
        if verify_digest {
          return Err(RawlerError::DecoderFailed(
            "Embedded original digest and output digest mismatch, data may be corrupt".into(),
          ));
        } else {
          log::warn!("Embedded original digest and output digest mismatch, data may be corrupt, but verify checks are disabled");
        }
      }

      let original = OriginalCompressed::new(&mut Cursor::new(val))?;
      match original.decompress(target) {
        Ok(size) => Ok(size),
        Err(error) => Err(RawlerError::with_io_error("extract_original", dng.path(), error)),
      }
    } else {
      Err(RawlerError::DecoderFailed("DNG file contains OriginalRawFileData tag, but is invalid".into()))
    }
  } else {
    Err(RawlerError::DecoderFailed(
      "No embedded raw file found (missing OriginalRawFileData tag)".into(),
    ))
  }
}

/// Single chunk for compressed data
struct ForkBlock {
  /// Compressed data for block
  chunk: Vec<u8>,
}

impl ForkBlock {
  fn new(chunk: Vec<u8>) -> Self {
    Self { chunk }
  }

  fn len(&self) -> usize {
    self.chunk.len()
  }

  fn compress(buf: impl AsRef<[u8]>) -> io::Result<Self> {
    let mut encoder = Encoder::with_options(
      Vec::with_capacity(COMPRESS_BLOCK_SIZE as usize),
      EncodeOptions::new().block_size(COMPRESS_BLOCK_SIZE as usize),
    )
    .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;
    encoder.write_all(buf.as_ref()).map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;
    Ok(ForkBlock {
      chunk: encoder.finish().into_result().map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?,
    })
  }

  fn decompress(&self) -> io::Result<Vec<u8>> {
    let mut decoder = Decoder::new(self.chunk.as_slice()).map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;
    let mut chunk = Vec::new();
    decoder.read_to_end(&mut chunk).map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;
    Ok(chunk)
  }
}

#[cfg(test)]
mod tests {

  use std::io::Cursor;

  use super::*;

  #[test]
  fn empty_data() -> std::result::Result<(), Box<dyn std::error::Error>> {
    //let data = [0x00, 0xFF, 0xDD];
    let data = [];
    let mut file = Cursor::new(data);
    // Compress
    let orig = OriginalCompressed::compress(&mut file)?;
    let mut out = Cursor::new(Vec::new());
    let _digest = orig.write_to_stream(&mut out)?;
    out.seek(SeekFrom::Start(0))?;
    // Reload
    let comp = OriginalCompressed::new(&mut out)?;
    // Decompress
    let mut restored = Cursor::new(Vec::new());
    comp.decompress(&mut restored)?;
    // Compare
    let unpacked = restored.into_inner();
    assert_eq!(unpacked, data);
    Ok(())
  }

  #[test]
  fn dummy_data() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let data = [0x00, 0xFF, 0xDD, 0x00, 0x00];
    let mut file = Cursor::new(data);
    // Compress
    let orig = OriginalCompressed::compress(&mut file)?;
    let mut out = Cursor::new(Vec::new());
    let _digest = orig.write_to_stream(&mut out)?;
    out.seek(SeekFrom::Start(0))?;
    // Reload
    let comp = OriginalCompressed::new(&mut out)?;
    // Decompress
    let mut restored = Cursor::new(Vec::new());
    comp.decompress(&mut restored)?;
    // Compare
    let unpacked = restored.into_inner();
    assert_eq!(unpacked, data);
    Ok(())
  }
}
