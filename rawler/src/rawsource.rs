use std::{
  fmt::Debug,
  fs::File,
  io::Cursor,
  iter::repeat,
  ops::Deref,
  path::{Path, PathBuf},
  sync::Arc,
};

use md5::Digest;
use memmap2::MmapOptions;

use crate::buffer::PaddedBuf;

pub struct RawSource {
  path: PathBuf,
  inner: RawSourceImpl,
}

enum RawSourceImpl {
  Memmap(memmap2::Mmap),
  Memory(Arc<Vec<u8>>),
}

impl RawSource {
  pub fn new(path: &Path) -> std::io::Result<Self> {
    let file = File::open(path)?;
    let mmap = unsafe { MmapOptions::new().populate().map(&file)? };
    #[cfg(unix)]
    {
      mmap.advise(memmap2::Advice::WillNeed)?;
      mmap.advise(memmap2::Advice::Sequential)?;
    }
    Ok(Self {
      path: path.canonicalize().unwrap_or_else(|_| path.to_owned()),
      inner: RawSourceImpl::Memmap(mmap),
    })
  }

  pub fn new_from_shared_vec(buf: Arc<Vec<u8>>) -> Self {
    Self {
      path: PathBuf::default(),
      inner: RawSourceImpl::Memory(buf),
    }
  }

  pub fn with_path(self, path: impl AsRef<Path>) -> Self {
    Self {
      path: path.as_ref().to_owned(),
      inner: self.inner,
    }
  }

  pub fn new_from_slice(buf: &[u8]) -> Self {
    Self::new_from_shared_vec(Arc::new(Vec::from(buf)))
  }

  /// Calculate digest for file
  pub fn digest(&self) -> Digest {
    md5::compute(self.buf())
  }

  pub fn path(&self) -> &Path {
    &self.path
  }

  pub fn buf(&self) -> &[u8] {
    self.deref()
  }

  pub fn subview(&self, offset: u64, size: u64) -> std::io::Result<&[u8]> {
    self.buf().get(offset as usize..(offset + size) as usize).ok_or(std::io::Error::new(
      std::io::ErrorKind::UnexpectedEof,
      format!("subview(): Offset {}+{} is behind EOF", offset, size),
    ))
  }

  pub fn subview_padded(&self, offset: u64, size: u64) -> std::io::Result<PaddedBuf<'_>> {
    if offset + size <= self.len() as u64 {
      if offset + size + 16 <= self.len() as u64 {
        self.subview(offset, size + 16).map(|buf| PaddedBuf::new_ref(buf, size as usize))
      } else {
        let mut buf = Vec::with_capacity((size + 16) as usize);
        buf.extend_from_slice(self.subview(offset, size)?);
        buf.extend(repeat(0).take(16));
        Ok(PaddedBuf::new_owned(buf, size as usize))
      }
    } else {
      Err(std::io::Error::new(
        std::io::ErrorKind::UnexpectedEof,
        format!("subview_padded(): Offset {}+{} is behind EOF", offset, size),
      ))
    }
  }

  pub fn subview_until_eof(&self, offset: u64) -> std::io::Result<&[u8]> {
    self.buf().get(offset as usize..).ok_or(std::io::Error::new(
      std::io::ErrorKind::UnexpectedEof,
      format!("subview_until_eof(): Offset {} is behind EOF", offset),
    ))
  }

  pub fn subview_until_eof_padded(&self, offset: u64) -> std::io::Result<PaddedBuf<'_>> {
    if offset < self.len() as u64 {
      let mut buf = Vec::with_capacity((self.len() - offset as usize + 16) as usize);
      buf.extend_from_slice(self.subview_until_eof(offset)?);
      buf.extend(repeat(0).take(16));
      Ok(PaddedBuf::new_owned(buf, self.len() - offset as usize))
    } else {
      Err(std::io::Error::new(
        std::io::ErrorKind::UnexpectedEof,
        format!("subview_until_eof_padded(): Offset {} is behind EOF", offset),
      ))
    }
  }

  pub fn reader(&self) -> Cursor<&[u8]> {
    Cursor::new(self.buf())
  }

  pub fn as_vec(&self) -> std::io::Result<Vec<u8>> {
    Ok(self.buf().to_vec())
  }

  pub fn stream_len(&mut self) -> u64 {
    self.buf().len() as u64
  }
}

impl Deref for RawSource {
  type Target = [u8];

  fn deref(&self) -> &Self::Target {
    match &self.inner {
      RawSourceImpl::Memmap(mmap) => mmap.deref(),
      RawSourceImpl::Memory(mem) => mem.deref(),
    }
  }
}

impl Debug for RawSource {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    f.debug_struct("RawSource").field("path", &self.path).finish()
  }
}
