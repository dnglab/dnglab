use std::cell::UnsafeCell;

use rayon::prelude::*;

use crate::imgop::{Dim2, Point, Rect};

/// Trait for types that can be used as subpixels in image processing
pub trait SubPixel: Copy + Default + Send + Sync + 'static {
  fn as_f32(self) -> f32;
  fn as_u16(self) -> u16;
}

impl SubPixel for u16 {
  fn as_f32(self) -> f32 {
    self as f32
  }
  fn as_u16(self) -> u16 {
    self
  }
}

impl SubPixel for f32 {
  fn as_f32(self) -> f32 {
    self
  }
  fn as_u16(self) -> u16 {
    self as u16
  }
}

impl SubPixel for u8 {
  fn as_f32(self) -> f32 {
    self as f32
  }
  fn as_u16(self) -> u16 {
    self as u16
  }
}

/// Type alias for mutable line access
pub type LineMut<'a, T> = &'a mut [T];

#[derive(Clone)]
pub struct Pix2D<T> {
  pub width: usize,
  pub height: usize,
  pub data: Vec<T>,
  pub initialized: bool,
}

pub type PixU16 = Pix2D<u16>;
pub type PixF32 = Pix2D<f32>;

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

  pub fn is_initialized(&self) -> bool {
    self.initialized
  }

  pub fn into_inner(self) -> Vec<T> {
    self.data
  }

  pub fn dim(&self) -> Dim2 {
    Dim2::new(self.width, self.height)
  }

  pub fn rect(&self) -> Rect {
    Rect::new(Point::default(), Dim2::new(self.width, self.height))
  }

  pub fn update_dimension(&mut self, dim: Dim2) {
    if self.width * self.height == dim.w * dim.h {
      self.width = dim.w;
      self.height = dim.h;
    } else {
      panic!("Can not change dimension: mismatch with old dimension: {:?} vs. {:?}", self.dim(), dim);
    }
  }

  pub fn pixels(&self) -> &[T] {
    debug_assert!(self.initialized);
    &self.data
  }

  pub fn pixels_mut(&mut self) -> &mut [T] {
    debug_assert!(self.initialized);
    &mut self.data
  }

  pub fn pixel_rows(&self) -> std::slice::ChunksExact<'_, T> {
    debug_assert!(self.initialized);
    self.data.chunks_exact(self.width)
  }

  pub fn pixel_rows_mut(&mut self) -> std::slice::ChunksExactMut<'_, T> {
    debug_assert!(self.initialized);
    self.data.chunks_exact_mut(self.width)
  }

  pub fn par_pixel_rows_mut(&mut self) -> rayon::slice::ChunksExactMut<'_, T> {
    debug_assert!(self.initialized);
    self.data.par_chunks_exact_mut(self.width)
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

  pub fn into_crop(self, area: Rect) -> Self {
    self.crop(area)
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
    unsafe { &mut *self.inner.get() }
  }

  pub fn into_inner(self) -> Pix2D<T> {
    self.inner.into_inner()
  }
}

unsafe impl<T> Sync for SharedPix2D<T> where T: Copy + Default + Send {}

#[derive(Clone)]
pub struct Color2D<T, const N: usize> {
  pub width: usize,
  pub height: usize,
  pub data: Vec<[T; N]>,
}

pub type RgbF32 = Color2D<f32, 3>;
pub type Ch4F32 = Color2D<f32, 4>;

impl<T, const N: usize> Color2D<T, N>
where
  T: Copy + Clone + Default + Send,
  [T; N]: Default,
{
  pub fn new_with(data: Vec<[T; N]>, width: usize, height: usize) -> Self {
    debug_assert_eq!(data.len(), height * width);
    Self { data, width, height }
  }

  pub fn new(width: usize, height: usize) -> Self {
    let data = vec![<[T; N]>::default(); width * height];
    Self { data, width, height }
  }

  pub fn into_inner(self) -> Vec<[T; N]> {
    self.data
  }

  pub fn dim(&self) -> Dim2 {
    Dim2::new(self.width, self.height)
  }

  pub fn rect(&self) -> Rect {
    Rect::new(Point::default(), Dim2::new(self.width, self.height))
  }

  pub fn flatten(&self) -> Vec<T> {
    self.data.iter().flatten().copied().collect::<Vec<T>>()
  }

  pub fn data_ptr(&self) -> Color2DPtr<T, N> {
    Color2DPtr::new(self)
  }

  pub fn pixels(&self) -> &[[T; N]] {
    &self.data
  }

  pub fn pixels_mut(&mut self) -> &mut [[T; N]] {
    &mut self.data
  }

  pub fn pixel_rows(&self) -> std::slice::ChunksExact<'_, [T; N]> {
    self.data.chunks_exact(self.width)
  }

  pub fn pixel_rows_mut(&mut self) -> std::slice::ChunksExactMut<'_, [T; N]> {
    self.data.chunks_exact_mut(self.width)
  }

  #[inline(always)]
  pub fn at(&self, row: usize, col: usize) -> &[T; N] {
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
  pub fn at_mut(&mut self, row: usize, col: usize) -> &mut [T; N] {
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
    F: Fn([T; N]) -> [T; N] + Send + Sync,
  {
    self.data.par_iter_mut().for_each(|v| *v = op(*v));
  }

  #[inline(always)]
  pub fn for_each_row<F>(&mut self, op: F)
  where
    F: Fn(usize, &mut [[T; N]]) + Send + Sync,
  {
    self.data.par_chunks_exact_mut(self.width).enumerate().for_each(|(row, data)| op(row, data));
  }

  // TODO: use par_iterator
  #[inline(always)]
  pub fn for_each_index<F>(&mut self, op: F)
  where
    F: Fn([T; N], usize, usize) -> [T; N],
  {
    self
      .pixel_rows_mut()
      .enumerate()
      .for_each(|(row, rowbuf)| rowbuf.iter_mut().enumerate().for_each(|(col, v)| *v = op(*v, row, col)));
  }

  pub fn crop(&self, area: Rect) -> Self {
    let mut output = Vec::with_capacity(area.d.h * area.d.w);
    assert!(area.p.y + area.d.h <= self.height);
    assert!(area.p.x + area.d.w <= self.width);
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

  pub fn make_padded(&self, padding: usize) -> Self {
    let new_w = self.width + padding * 2;
    let new_h = self.height + padding * 2;
    let mut padded = Self::new(new_w, new_h);

    for y in 0..self.height {
      for x in 0..self.width {
        *padded.at_mut(y + padding, x + padding) = *self.at(y, x);
      }
    }
    padded
  }
}

impl<T, const N: usize> Default for Color2D<T, N>
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
pub struct Color2DPtr<T, const N: usize> {
  ptr: *const [T; N],
  pub width: usize,
  pub height: usize,
}
impl<T, const N: usize> Color2DPtr<T, N>
where
  T: Copy + Clone,
{
  fn new(orig: &Color2D<T, N>) -> Self {
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
  pub unsafe fn at(&self, row: usize, col: usize) -> &[T; N] {
    unsafe {
      debug_assert!(row * col < self.height * self.width);
      &*self.ptr.add(row * self.width + col)
    }
  }
}

unsafe impl<T, const N: usize> Sync for Color2DPtr<T, N> {}

/// Deinterleave a 2x2 interleaved buffer
pub fn deinterleave2x2(input: &PixU16) -> crate::Result<PixU16> {
  if input.width % 2 != 0 || input.height % 2 != 0 {
    return Err("deinterleave2x2: input dimensions must be even".into());
  }
  
  let new_width = input.width / 2;
  let new_height = input.height / 2;
  let mut output = PixU16::new(new_width * 2, new_height * 2);
  
  // Copy the data with deinterleaving
  for row in 0..new_height {
    for col in 0..new_width {
      let src_row = row * 2;
      let src_col = col * 2;
      
      // Top-left quadrant (R)
      output.data[row * new_width + col] = input.data[src_row * input.width + src_col];
      // Top-right quadrant (G1)
      output.data[row * new_width + col + new_width * new_height] = input.data[src_row * input.width + src_col + 1];
      // Bottom-left quadrant (G2)
      output.data[row * new_width + col + new_width * new_height * 2] = input.data[(src_row + 1) * input.width + src_col];
      // Bottom-right quadrant (B)
      output.data[row * new_width + col + new_width * new_height * 3] = input.data[(src_row + 1) * input.width + src_col + 1];
    }
  }
  
  Ok(output)
}

#[macro_export]
macro_rules! alloc_image_plain {
  ($width:expr, $height:expr, $dummy: expr) => {{
    if $width * $height > 500000000 || $width > 50000 || $height > 50000 {
      panic!("rawler: surely there's no such thing as a >500MP or >50000 px wide/tall image!");
    }
    if $dummy {
      $crate::pixarray::PixU16::new_uninit($width, $height)
    } else {
      $crate::pixarray::PixU16::new($width, $height)
    }
  }};
}

#[macro_export]
macro_rules! alloc_image {
  ($width:expr, $height:expr, $dummy: expr) => {{
    if $dummy {
      return $crate::pixarray::PixU16::new_uninit($width, $height);
    } else {
      $crate::alloc_image_plain!($width, $height, $dummy)
    }
  }};
}

#[macro_export]
macro_rules! alloc_image_ok {
  ($width:expr, $height:expr, $dummy: expr) => {{
    if $dummy {
      return Ok($crate::pixarray::PixU16::new_uninit($width, $height));
    } else {
      $crate::alloc_image_plain!($width, $height, $dummy)
    }
  }};
}

#[macro_export]
macro_rules! alloc_image_f32_plain {
  ($width:expr, $height:expr, $dummy: expr) => {{
    if $width * $height > 500000000 || $width > 50000 || $height > 50000 {
      panic!("rawler: surely there's no such thing as a >500MP or >50000 px wide/tall image!");
    }
    if $dummy {
      $crate::pixarray::PixF32::new_uninit($width, $height)
    } else {
      $crate::pixarray::PixF32::new($width, $height)
    }
  }};
}