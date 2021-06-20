// SPDX-License-Identifier: LGPL-2.1
// Copyright 2021 Daniel Vogelbacher <daniel@chaospixel.com>

use libflate::zlib::{EncodeOptions, Encoder};
use rayon::prelude::*;
use std::io::{Cursor, Seek, SeekFrom, Write};

// DNG requires this block size
const COMPRESS_BLOCK_SIZE: u32 = 65536;

/// Calculate digest for original file, DNG uses MD5 for that
pub fn original_digest(data: &[u8]) -> [u8; 16] {
  md5::compute(&data).into()
}

/// Compress an original file for embedding into DNG
pub fn original_compress(uncomp_data: &[u8]) -> Result<Vec<u8>, std::io::Error> {
  let pool = rayon::ThreadPoolBuilder::new()
    .num_threads(rayon::current_num_threads() / 2 + 1)
    .build()
    .unwrap();

  let result = pool.install(move || {
    let mut compr_data = Cursor::new(Vec::<u8>::new());

    let raw_fork_size: u32 = uncomp_data.len() as u32;
    let raw_fork_blocks: u32 = ((raw_fork_size + (COMPRESS_BLOCK_SIZE - 1)) / COMPRESS_BLOCK_SIZE) as u32;

    compr_data.write_all(&raw_fork_size.to_be_bytes())?; // Fork 1
    compr_data.seek(SeekFrom::Current((raw_fork_blocks + 1) as i64 * 4))?; // skip index

    let mut index_list: Vec<u32> = Vec::with_capacity(raw_fork_blocks as usize + 1);

    let compr_chunks: Vec<Result<ComprChunk, String>> = uncomp_data
      .par_chunks(COMPRESS_BLOCK_SIZE as usize)
      .map(|chunk| compress_chunk(chunk))
      .collect();

    for chunk in &compr_chunks {
      index_list.push(compr_data.stream_position()? as u32);
      match chunk {
        Ok(chunk) => {
          compr_data.write_all(&chunk.chunk)?;
        }
        Err(_) => {
          panic!("Compression failed!")
        }
      }
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
  });

  result
}

/// Single chunk for compressed data
struct ComprChunk {
  chunk: Vec<u8>,
}

/// Compress a buffer to ComprChunk
fn compress_chunk(buf: &[u8]) -> Result<ComprChunk, String> {
  let mut encoder = Encoder::with_options(Vec::new(), EncodeOptions::new().block_size(COMPRESS_BLOCK_SIZE as usize)).unwrap();
  encoder.write_all(&buf).unwrap();
  Ok(ComprChunk {
    chunk: encoder.finish().into_result().unwrap(),
  })
}
