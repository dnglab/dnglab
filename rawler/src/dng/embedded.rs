// SPDX-License-Identifier: LGPL-2.1
// Copyright 2021 Daniel Vogelbacher <daniel@chaospixel.com>

use crate::Result;
use byteorder::{BigEndian, ReadBytesExt};
use libflate::zlib::{Decoder, EncodeOptions, Encoder};
use log::debug;
use rayon::prelude::*;
use std::io::{Cursor, Read, Seek, SeekFrom, Write};

// DNG requires this block size
const COMPRESS_BLOCK_SIZE: u32 = 65536;

/// Calculate digest for original file, DNG uses MD5 for that
pub fn original_digest(data: &[u8]) -> [u8; 16] {
  md5::compute(data).into()
}

/// Compress an original file for embedding into DNG
pub fn original_compress(uncomp_data: &[u8]) -> Result<Vec<u8>> {
  let threads = rayon::current_num_threads() / 2 + 1;
  debug!("Compress original raw using {} threads", threads);
  let pool = rayon::ThreadPoolBuilder::new()
    .num_threads(threads)
    .build()
    .expect("Failed to build thread pool");

  pool.install(move || {
    let mut compr_data = Cursor::new(Vec::<u8>::with_capacity(uncomp_data.len()));

    let raw_fork_size: u32 = uncomp_data.len() as u32;
    let raw_fork_blocks: u32 = (raw_fork_size + (COMPRESS_BLOCK_SIZE - 1)) / COMPRESS_BLOCK_SIZE;

    compr_data.write_all(&raw_fork_size.to_be_bytes())?; // Fork 1
    compr_data.seek(SeekFrom::Current((raw_fork_blocks + 1) as i64 * 4))?; // skip index

    let mut index_list: Vec<u32> = Vec::with_capacity(raw_fork_blocks as usize + 1);

    let compr_chunks = uncomp_data
      .par_chunks(COMPRESS_BLOCK_SIZE as usize)
      .map(compress_chunk)
      .collect::<Result<Vec<ComprChunk>>>()?;

    for chunk in &compr_chunks {
      index_list.push(compr_data.stream_position()? as u32);
      compr_data.write_all(&chunk.chunk)?;
    }
    index_list.push(compr_data.stream_position()? as u32); // end index

    assert!(index_list.len() == raw_fork_blocks as usize + 1);

    compr_data.write_all(&0u32.to_be_bytes())?;
    compr_data.write_all(&0u32.to_be_bytes())?;
    compr_data.write_all(&0u32.to_be_bytes())?;
    compr_data.write_all(&0u32.to_be_bytes())?;
    compr_data.write_all(&0u32.to_be_bytes())?;
    compr_data.write_all(&0u32.to_be_bytes())?;
    compr_data.write_all(&0u32.to_be_bytes())?;

    // Write chunk pointers into reserved area
    compr_data.seek(SeekFrom::Start(4))?;
    for idx in index_list {
      compr_data.write_all(&idx.to_be_bytes())?;
    }

    Ok(compr_data.into_inner())
  })
}

/// Decompress an original file from DNG
pub fn original_decompress(comp_data: &[u8]) -> Result<Vec<u8>> {
  let pool = rayon::ThreadPoolBuilder::new()
    .num_threads(rayon::current_num_threads() / 2 + 1)
    .build()
    .expect("Failed to build thread pool");

  pool.install(move || {
    let mut comp_data = Cursor::new(comp_data);

    let raw_fork_size: u32 = comp_data.read_u32::<BigEndian>()?;
    let raw_fork_blocks: u32 = (raw_fork_size + (COMPRESS_BLOCK_SIZE - 1)) / COMPRESS_BLOCK_SIZE;

    let mut index_list: Vec<usize> = Vec::with_capacity(raw_fork_blocks as usize + 1);

    for _ in 0..raw_fork_blocks + 1 {
      let idx: usize = comp_data.read_u32::<BigEndian>()? as usize;
      index_list.push(idx);
    }

    let comp_data = comp_data.into_inner();
    let mut chunks: Vec<&[u8]> = Vec::new();
    let mut prev_idx = index_list.first().expect("Failed to get first element");
    for idx in index_list.iter().skip(1) {
      let chunk = &comp_data[*prev_idx..*idx];
      chunks.push(chunk);
      prev_idx = idx;
    }

    let uncompr_chunks: Vec<Result<UncomprChunk>> = chunks.par_iter().map(|chunk| decompress_chunk(chunk)).collect();
    let mut data: Vec<u8> = Vec::new();
    for chunk in uncompr_chunks {
      data.extend_from_slice(&chunk.unwrap().chunk);
    }
    Ok(data)
  })
}

/// Single chunk for compressed data
struct ComprChunk {
  chunk: Vec<u8>,
}

/// Compress a buffer to ComprChunk
fn compress_chunk(buf: &[u8]) -> Result<ComprChunk> {
  let mut encoder = Encoder::with_options(
    Vec::with_capacity(COMPRESS_BLOCK_SIZE as usize),
    EncodeOptions::new().block_size(COMPRESS_BLOCK_SIZE as usize),
  )
  .unwrap();
  encoder.write_all(buf).unwrap();
  Ok(ComprChunk {
    chunk: encoder.finish().into_result().unwrap(),
  })
}

/// Single chunk for uncompressed data
struct UncomprChunk {
  chunk: Vec<u8>,
}

/// Compress a buffer to ComprChunk
fn decompress_chunk(buf: &[u8]) -> Result<UncomprChunk> {
  let mut decoder = Decoder::new(buf).unwrap();
  let mut chunk = Vec::new();
  decoder.read_to_end(&mut chunk).unwrap();
  Ok(UncomprChunk { chunk })
}
