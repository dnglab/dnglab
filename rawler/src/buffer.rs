use std::ops::Deref;

/// Buffer to hold an image in memory with enough extra space at the end for speed optimizations
pub struct PaddedBuf<'a> {
  buf: PaddedBufImpl<'a>,
  size: usize,
}

pub enum PaddedBufImpl<'a> {
  Owned(Vec<u8>),
  Ref(&'a [u8]),
}

impl<'a> PaddedBuf<'a> {
  pub fn new_owned(buf: Vec<u8>, size: usize) -> Self {
    Self {
      buf: PaddedBufImpl::Owned(buf),
      size,
    }
  }

  pub fn new_ref(buf: &'a [u8], size: usize) -> Self {
    Self {
      buf: PaddedBufImpl::Ref(buf),
      size,
    }
  }

  pub fn size(&self) -> usize {
    self.size
  }

  pub fn buf(&'a self) -> &'a [u8] {
    self
  }

  pub fn real_size(&self) -> usize {
    self.buf().len()
  }
}

impl<'a> Deref for PaddedBuf<'a> {
  type Target = [u8];

  fn deref(&self) -> &Self::Target {
    match &self.buf {
      PaddedBufImpl::Owned(x) => x.as_ref(),
      PaddedBufImpl::Ref(x) => x,
    }
  }
}
