use std::cell::UnsafeCell;

use multiversion::multiversion;
use rayon::prelude::*;

use crate::{
  decoders::decode_threaded_prealloc,
  formats::tiff::Rational,
  imgop::{Dim2, Point, Rect},
};

pub trait SubPixel: Default + std::fmt::Debug + Clone + Copy + Send + Sync + Into<Rational> {
  fn as_f32(self) -> f32;
  fn as_u16(self) -> u16;
}

impl SubPixel for u16 {
  fn as_f32(self) -> f32 {
    self as f32
  }

  fn as_u16(self) -> u16 {
    self as u16
  }
}
impl SubPixel for f32 {
  fn as_f32(self) -> f32 {
    self as f32
  }

  fn as_u16(self) -> u16 {
    self as u16
  }
}

pub type LineMut<'a, T> = &'a mut [T];

pub type Line<'a, T> = &'a [T];

#[derive(Clone)]
pub struct Pix2D<T: SubPixel> {
  pub width: usize,
  pub height: usize,
  pub data: Vec<T>,
  pub initialized: bool,
}

pub type PixU16 = Pix2D<u16>;
pub type PixF32 = Pix2D<f32>;

impl<T> Pix2D<T>
where
  T: SubPixel,
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

  pub fn len(&self) -> usize {
    self.data.len()
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
  pub fn row(&self, row: usize) -> &[T] {
    debug_assert!(self.initialized);
    let start = row * self.width;
    &self.data[start..start + self.width]
  }

  #[inline(always)]
  pub fn row_mut(&mut self, row: usize) -> &mut [T] {
    debug_assert!(self.initialized);
    let start = row * self.width;
    &mut self.data[start..start + self.width]
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
    assert!(self.initialized);
    self.data.par_iter_mut().for_each(|v| *v = op(*v));
  }

  #[inline(always)]
  pub fn for_each_index<F>(&mut self, op: F)
  where
    F: Fn(T, usize, usize) -> T,
  {
    assert!(self.initialized);
    self
      .pixel_rows_mut()
      .enumerate()
      .for_each(|(row, rowbuf)| rowbuf.iter_mut().enumerate().for_each(|(col, v)| *v = op(*v, row, col)));
  }

  pub fn crop(&self, area: Rect) -> Self {
    assert!(self.initialized);
    crop(&self, area)
  }

  pub fn into_crop(self, area: Rect) -> Self {
    if self.dim() == area.d && area.p == Point::zero() {
      self // No-Op
    } else {
      crop(&self, area)
    }
  }
}

#[multiversion(targets("x86_64+avx+avx2", "x86+sse", "aarch64+neon"))]
fn crop<T>(pixbuf: &Pix2D<T>, area: Rect) -> Pix2D<T>
where
  T: SubPixel,
{
  let mut output;
  if pixbuf.initialized {
    output = Pix2D::<T>::new(area.width(), area.height());

    output.par_pixel_rows_mut().enumerate().for_each(|(row, line)| {
      let src_row = pixbuf.row(area.y() + row);
      line.copy_from_slice(&src_row[area.x()..area.x() + line.len()]);
    });
  } else {
    output = Pix2D::<T>::new_uninit(area.width(), area.height());
  }
  output
}

#[multiversion(targets("x86_64+avx+avx2", "x86+sse", "aarch64+neon"))]
pub(crate) fn deinterleave2x2<T>(pixbuf: &Pix2D<T>) -> crate::Result<Pix2D<T>>
where
  T: SubPixel,
{
  if pixbuf.initialized {
    let mut output = Pix2D::<T>::new(pixbuf.width, pixbuf.height);

    let line_width = pixbuf.width;

    let half_width = line_width / 2;
    let line_distance = line_width;
    let ch0 = &pixbuf[..];
    let ch1 = &pixbuf[half_width..];
    let ch2 = &pixbuf[pixbuf.len() / 2..];
    let ch3 = &pixbuf[pixbuf.len() / 2 + half_width..];

    decode_threaded_prealloc(&mut output, &|line, row| {
      let src_row = row / 2;
      let offset = src_row * line_distance;
      let ch_a;
      let ch_b;
      if row & 1 == 0 {
        // For even rows, we take top-left and top-right channel data.
        ch_a = &ch0[offset..offset + half_width];
        ch_b = &ch1[offset..offset + half_width];
      } else {
        // For odd rows, we take bottom-left and bottom-right channel data.
        ch_a = &ch2[offset..offset + half_width];
        ch_b = &ch3[offset..offset + half_width];
      }

      debug_assert_eq!(ch_a.len(), ch_b.len());
      debug_assert_eq!(ch_a.len() + ch_b.len(), line.len());
      debug_assert_eq!(line_width, line.len());
      line.chunks_exact_mut(2).zip(ch_a.iter().zip(ch_b.iter())).for_each(|(dst, (a, b))| {
        dst[0] = *a;
        dst[1] = *b;
      });
      Ok(())
    })?;
    Ok(output)
  } else {
    Ok(pixbuf.clone())
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
  T: SubPixel,
{
  type Output = I::Output;

  fn index(&self, index: I) -> &Self::Output {
    &self.data[index]
  }
}

impl<I, T> std::ops::IndexMut<I> for Pix2D<T>
where
  I: std::slice::SliceIndex<[T]>,
  T: SubPixel,
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
pub struct SharedPix2D<T: SubPixel> {
  pub inner: UnsafeCell<Pix2D<T>>,
}

impl<T> SharedPix2D<T>
where
  T: SubPixel,
{
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

unsafe impl<T> Sync for SharedPix2D<T> where T: SubPixel {}

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
