use std::io::Read;

use crate::RawlerError;
use crate::Result;

/// Buffer to hold an image in memory with enough extra space at the end for speed optimizations
#[derive(Debug, Clone)]
pub struct Buffer {
  // TODO: delete buffer impl
  pub buf: Vec<u8>,
  size: usize,
}

impl Buffer {
  /// Creates a new buffer from anything that can be read
  pub fn new(reader: &mut dyn Read) -> Result<Buffer> {
    let mut buffer = Vec::new();
    if let Err(err) = reader.read_to_end(&mut buffer) {
      return Err(RawlerError::with_io_error("Buffer::new()", "<internal_buf>", err));
    }
    let size = buffer.len();
    //buffer.extend([0;16].iter().cloned());
    Ok(Buffer { buf: buffer, size })
  }

  pub fn raw_buf(&self) -> &[u8] {
    &self.buf[..self.size]
  }

  pub fn get_range(&self, offset: usize, len: usize) -> &[u8] {
    &self.buf[offset..offset + len]
  }

  pub fn size(&self) -> usize {
    self.size
  }
}

impl From<Vec<u8>> for Buffer {
  fn from(buf: Vec<u8>) -> Self {
    let size = buf.len();
    Self { buf, size }
  }
}
