use std::cell::UnsafeCell;

use rayon::prelude::*;

use crate::imgop::{Dim2, Rect};

pub struct Pix2D<T> {
  pub width: usize,
  pub height: usize,
  pub data: Vec<T>,
  pub initialized: bool,
}

pub type PixU16 = Pix2D<u16>;

impl<T> Pix2D<T>
where
  T: Copy + Default + Send,
{
  pub fn new_with(data: Vec<T>, width: usize, height: usize) -> Self {
    assert_eq!(data.len(), height * width);
    Self {
      data,
      width,
      height,
      initialized: true,
    }
  }

  pub fn new(width: usize, height: usize) -> Self {
    let data = vec![T::default(); width * height];
    Self {
      data,
      width,
      height,
      initialized: true,
    }
  }

  pub fn new_uninit(width: usize, height: usize) -> Self {
    let data = Vec::with_capacity(width * height);
    Self {
      data,
      width,
      height,
      initialized: false,
    }
  }

  pub fn into_inner(self) -> Vec<T> {
    self.data
  }

  pub fn dim(&self) -> Dim2 {
    Dim2::new(self.width, self.height)
  }

  pub fn pixels(&self) -> &[T] {
    debug_assert!(self.initialized);
    &self.data
  }

  pub fn pixels_mut(&mut self) -> &mut [T] {
    debug_assert!(self.initialized);
    &mut self.data
  }

  pub fn pixel_rows(&self) -> std::slice::ChunksExact<T> {
    debug_assert!(self.initialized);
    self.data.chunks_exact(self.width)
  }

  pub fn pixel_rows_mut(&mut self) -> std::slice::ChunksExactMut<T> {
    debug_assert!(self.initialized);
    self.data.chunks_exact_mut(self.width)
  }

  #[inline(always)]
  pub fn at(&self, row: usize, col: usize) -> &T {
    debug_assert!(self.initialized);
    #[cfg(debug_assertions)]
    {
      &self.data[row * self.width + col]
    }
    #[cfg(not(debug_assertions))]
    unsafe {
      self.data.get_unchecked(row * self.width + col)
    }
  }

  #[inline(always)]
  pub fn at_mut(&mut self, row: usize, col: usize) -> &mut T {
    debug_assert!(self.initialized);
    #[cfg(debug_assertions)]
    {
      &mut self.data[row * self.width + col]
    }
    #[cfg(not(debug_assertions))]
    unsafe {
      self.data.get_unchecked_mut(row * self.width + col)
    }
  }

  #[inline(always)]
  pub fn for_each<F>(&mut self, op: F)
  where
    F: Fn(T) -> T + Send + Sync,
  {
    debug_assert!(self.initialized);
    self.data.par_iter_mut().for_each(|v| *v = op(*v));
  }

  #[inline(always)]
  pub fn for_each_index<F>(&mut self, op: F)
  where
    F: Fn(T, usize, usize) -> T,
  {
    debug_assert!(self.initialized);
    self
      .pixel_rows_mut()
      .enumerate()
      .for_each(|(row, rowbuf)| rowbuf.iter_mut().enumerate().for_each(|(col, v)| *v = op(*v, row, col)));
  }

  pub fn crop(&self, area: Rect) -> Self {
    debug_assert!(self.initialized);
    let mut output = Vec::with_capacity(area.d.h * area.d.w);
    output.extend(
      self
        .pixels()
        .chunks_exact(self.width)
        .skip(area.p.y)
        .take(area.d.h)
        .flat_map(|row| row[area.p.x..area.p.x + area.d.w].iter())
        .cloned(),
    );
    Self::new_with(output, area.d.w, area.d.h)
  }
}

/*
impl<T> Index<usize> for Pix2D<T> {
  type Output = T;
  fn index<'a>(&'a self, i: usize) -> &'a T {
    &self.data[i]
  }
}
 */

impl<I, T> std::ops::Index<I> for Pix2D<T>
where
  I: std::slice::SliceIndex<[T]>,
{
  type Output = I::Output;

  fn index(&self, index: I) -> &Self::Output {
    &self.data[index]
  }
}

impl<I, T> std::ops::IndexMut<I> for Pix2D<T>
where
  I: std::slice::SliceIndex<[T]>,
{
  fn index_mut<'a>(&mut self, index: I) -> &mut Self::Output {
    &mut self.data[index]
  }
}

/*
impl<T> Default for Pix2D<T>
where
  T: Default,
{
  fn default() -> Self {
    Self {
      width: 0,
      height: 0,
      data: Default::default(),
      initialized: false,
    }
  }
}
 */

/// An ugly hack to get multiple mutable references to Pix2D
pub struct SharedPix2D<T> {
  pub inner: UnsafeCell<Pix2D<T>>,
}

impl<T> SharedPix2D<T> {
  pub fn new(inner: Pix2D<T>) -> Self {
    Self { inner: inner.into() }
  }

  /// Get inner Pix2D<> reference
  ///
  /// # Safety
  /// Only use this inside Rayon parallel iterators.
  #[allow(clippy::mut_from_ref)]
  pub unsafe fn inner_mut(&self) -> &mut Pix2D<T> {
    &mut *self.inner.get()
  }

  pub fn into_inner(self) -> Pix2D<T> {
    self.inner.into_inner()
  }
}

unsafe impl<T> Sync for SharedPix2D<T> where T: Copy + Default + Send {}

#[derive(Clone)]
pub struct Rgb2D<T> {
  pub width: usize,
  pub height: usize,
  pub data: Vec<[T; 3]>,
}

pub type RgbF32 = Rgb2D<f32>;

impl<T> Rgb2D<T>
where
  T: Copy + Clone + Default + Send,
{
  pub fn new_with(data: Vec<[T; 3]>, width: usize, height: usize) -> Self {
    debug_assert_eq!(data.len(), height * width);
    Self { data, width, height }
  }

  pub fn new(width: usize, height: usize) -> Self {
    let data = vec![<[T; 3]>::default(); width * height];
    Self { data, width, height }
  }

  pub fn into_inner(self) -> Vec<[T; 3]> {
    self.data
  }

  pub fn data_ptr(&self) -> Rgb2DPtr<T> {
    Rgb2DPtr::new(self)
  }

  pub fn pixels(&self) -> &[[T; 3]] {
    &self.data
  }

  pub fn pixels_mut(&mut self) -> &mut [[T; 3]] {
    &mut self.data
  }

  pub fn pixel_rows(&self) -> std::slice::ChunksExact<[T; 3]> {
    self.data.chunks_exact(self.width)
  }

  pub fn pixel_rows_mut(&mut self) -> std::slice::ChunksExactMut<[T; 3]> {
    self.data.chunks_exact_mut(self.width)
  }

  #[inline(always)]
  pub fn at(&self, row: usize, col: usize) -> &[T; 3] {
    #[cfg(debug_assertions)]
    {
      &self.data[row * self.width + col]
    }
    #[cfg(not(debug_assertions))]
    unsafe {
      self.data.get_unchecked(row * self.width + col)
    }
  }

  #[inline(always)]
  pub fn at_mut(&mut self, row: usize, col: usize) -> &mut [T; 3] {
    #[cfg(debug_assertions)]
    {
      &mut self.data[row * self.width + col]
    }
    #[cfg(not(debug_assertions))]
    unsafe {
      self.data.get_unchecked_mut(row * self.width + col)
    }
  }

  #[inline(always)]
  pub fn for_each<F>(&mut self, op: F)
  where
    F: Fn([T; 3]) -> [T; 3] + Send + Sync,
  {
    self.data.par_iter_mut().for_each(|v| *v = op(*v));
  }

  // TODO: use par_iterator
  #[inline(always)]
  pub fn for_each_index<F>(&mut self, op: F)
  where
    F: Fn([T; 3], usize, usize) -> [T; 3],
  {
    self
      .pixel_rows_mut()
      .enumerate()
      .for_each(|(row, rowbuf)| rowbuf.iter_mut().enumerate().for_each(|(col, v)| *v = op(*v, row, col)));
  }

  pub fn crop(&self, area: Rect) -> Self {
    let mut output = Vec::with_capacity(area.d.h * area.d.w);
    output.extend(
      self
        .pixels()
        .chunks_exact(self.width)
        .skip(area.p.y)
        .take(area.d.h)
        .flat_map(|row| row[area.p.x..area.p.x + area.d.w].iter())
        .cloned(),
    );
    Self::new_with(output, area.d.w, area.d.h)
  }
}

impl<T> Default for Rgb2D<T>
where
  T: Default,
{
  fn default() -> Self {
    Self {
      width: 0,
      height: 0,
      data: Default::default(),
    }
  }
}

#[derive(Clone, Debug)]
pub struct Rgb2DPtr<T> {
  ptr: *const [T; 3],
  pub width: usize,
  pub height: usize,
}
impl<T> Rgb2DPtr<T>
where
  T: Copy + Clone,
{
  fn new(orig: &Rgb2D<T>) -> Self {
    Self {
      ptr: orig.data.as_slice().as_ptr(),
      width: orig.width,
      height: orig.height,
    }
  }

  /// Get a pixel from raw pointer
  /// # Safety
  /// TODO
  #[inline(always)]
  pub unsafe fn at(&self, row: usize, col: usize) -> &[T; 3] {
    debug_assert!(row * col < self.height * self.width);
    &*self.ptr.add(row * self.width + col)
  }
}

unsafe impl<T> Sync for Rgb2DPtr<T> {}

#[macro_export]
macro_rules! alloc_image_plain {
  ($width:expr, $height:expr, $dummy: expr) => {{
    if $width * $height > 500000000 || $width > 50000 || $height > 50000 {
      panic!("rawler: surely there's no such thing as a >500MP or >50000 px wide/tall image!");
    }
    if $dummy {
      crate::pixarray::PixU16::new_uninit($width, $height)
    } else {
      crate::pixarray::PixU16::new($width, $height)
    }
  }};
}

#[macro_export]
macro_rules! alloc_image {
  ($width:expr, $height:expr, $dummy: expr) => {{
    if $dummy {
      return crate::pixarray::PixU16::new_uninit($width, $height);
    } else {
      crate::alloc_image_plain!($width, $height, $dummy)
    }
  }};
}

#[macro_export]
macro_rules! alloc_image_ok {
  ($width:expr, $height:expr, $dummy: expr) => {{
    if $dummy {
      return Ok(crate::pixarray::PixU16::new_uninit($width, $height));
    } else {
      crate::alloc_image_plain!($width, $height, $dummy)
    }
  }};
}
