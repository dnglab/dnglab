use std::cell::UnsafeCell;

use itertools::Itertools;
use multiversion::multiversion;
use rayon::prelude::*;

use crate::{
  decompressors::decompress_lines_fn,
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
pub(crate) fn deinterleave2x2<T>(pixbuf: &Pix2D<T>) -> std::result::Result<Pix2D<T>, String>
where
  T: SubPixel,
{
  if pixbuf.initialized {
    let line_width = pixbuf.width;

    let half_width = line_width / 2;
    let line_distance = line_width;
    let ch0 = &pixbuf[..];
    let ch1 = &pixbuf[half_width..];
    let ch2 = &pixbuf[pixbuf.len() / 2..];
    let ch3 = &pixbuf[pixbuf.len() / 2 + half_width..];

    decompress_lines_fn(pixbuf.width, pixbuf.height, false, &|line, row| {
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
    })
  } else {
    Ok(pixbuf.clone())
  }
}

/// Reorder tiles stored in sub-tile blocks (DNG SubTileBlockSize) into row-scan order.
///
/// Each tile of `tile_w` x `tile_h` pixels (clipped to the buffer) contains blocks of
/// `block_rows` x `block_cols` pixels, stored in row-scan order. The pixels within each
/// block are stored in row-scan order, too. Strips are handled as tiles with full image width.
pub(crate) fn reorder_sub_tile_blocks<T>(
  pixbuf: &mut Pix2D<T>,
  cpp: usize,
  tile_w: usize,
  tile_h: usize,
  block_rows: usize,
  block_cols: usize,
) -> std::result::Result<(), String>
where
  T: SubPixel,
{
  if !pixbuf.initialized {
    return Ok(());
  }
  if cpp == 0 || tile_w == 0 || tile_h == 0 || block_rows == 0 || block_cols == 0 {
    return Err(format!("Invalid SubTileBlockSize {block_rows}x{block_cols} or tile size {tile_w}x{tile_h}"));
  }
  let row_samples = pixbuf.width;
  let block_samples = block_cols * cpp;
  let mut stored = Vec::with_capacity(tile_w * cpp * tile_h);
  for tile_y in (0..pixbuf.height).step_by(tile_h) {
    for tile_x in (0..row_samples).step_by(tile_w * cpp) {
      let w = (tile_w * cpp).min(row_samples - tile_x);
      let h = tile_h.min(pixbuf.height - tile_y);
      if !w.is_multiple_of(block_samples) || !h.is_multiple_of(block_rows) {
        return Err(format!("SubTileBlockSize {block_rows}x{block_cols} does not divide tile area {}x{h}", w / cpp));
      }
      // Tile data in stored order
      stored.clear();
      for row in tile_y..tile_y + h {
        stored.extend_from_slice(&pixbuf.data[row * row_samples + tile_x..row * row_samples + tile_x + w]);
      }
      let mut src = stored.chunks_exact(block_samples);
      for block_y in (tile_y..tile_y + h).step_by(block_rows) {
        for block_x in (tile_x..tile_x + w).step_by(block_samples) {
          for row in block_y..block_y + block_rows {
            let start = row * row_samples + block_x;
            pixbuf.data[start..start + block_samples].copy_from_slice(src.next().expect("Block data available"));
          }
        }
      }
    }
  }
  Ok(())
}

/// Start index of each field in the stored order for `count` elements interleaved
/// into `factor` fields (field `f` contains the elements `f`, `f + factor`, ...).
fn interleave_field_starts(count: usize, factor: usize) -> Vec<usize> {
  let mut start = 0;
  (0..factor)
    .map(|field| {
      let field_start = start;
      start += (count.saturating_sub(field)).div_ceil(factor);
      field_start
    })
    .collect()
}

/// Restore row order for rows stored in `factor` interleaved fields (DNG RowInterleaveFactor).
pub(crate) fn deinterleave_rows<T>(pixbuf: &Pix2D<T>, factor: usize) -> Pix2D<T>
where
  T: SubPixel,
{
  if !pixbuf.initialized || factor <= 1 {
    return pixbuf.clone();
  }
  let width = pixbuf.width;
  let field_starts = interleave_field_starts(pixbuf.height, factor);
  let mut data = Vec::with_capacity(pixbuf.data.len());
  for row in 0..pixbuf.height {
    let stored = field_starts[row % factor] + row / factor;
    data.extend_from_slice(&pixbuf.data[stored * width..(stored + 1) * width]);
  }
  Pix2D::new_with(data, width, pixbuf.height)
}

/// Restore column order for pixels stored in `factor` interleaved fields per row
/// (DNG ColumnInterleaveFactor). Each pixel consists of `cpp` samples.
pub(crate) fn deinterleave_cols<T>(pixbuf: &Pix2D<T>, cpp: usize, factor: usize) -> Pix2D<T>
where
  T: SubPixel,
{
  if !pixbuf.initialized || factor <= 1 || cpp == 0 {
    return pixbuf.clone();
  }
  let pixels = pixbuf.width / cpp;
  let field_starts = interleave_field_starts(pixels, factor);
  let mut data = Vec::with_capacity(pixbuf.data.len());
  for line in pixbuf.data.chunks_exact(pixbuf.width) {
    for col in 0..pixels {
      let stored = field_starts[col % factor] + col / factor;
      data.extend_from_slice(&line[stored * cpp..(stored + 1) * cpp]);
    }
  }
  Pix2D::new_with(data, pixbuf.width, pixbuf.height)
}

/// Store rows in `factor` interleaved fields (DNG RowInterleaveFactor),
/// the inverse of [`deinterleave_rows`].
pub(crate) fn interleave_rows<T>(pixbuf: &Pix2D<T>, factor: usize) -> Pix2D<T>
where
  T: SubPixel,
{
  if !pixbuf.initialized || factor <= 1 {
    return pixbuf.clone();
  }
  let width = pixbuf.width;
  let mut data = Vec::with_capacity(pixbuf.data.len());
  for field in 0..factor {
    for row in (field..pixbuf.height).step_by(factor) {
      data.extend_from_slice(&pixbuf.data[row * width..(row + 1) * width]);
    }
  }
  Pix2D::new_with(data, width, pixbuf.height)
}

/// Store the pixels of each row in `factor` interleaved fields (DNG ColumnInterleaveFactor),
/// the inverse of [`deinterleave_cols`]. Each pixel consists of `cpp` samples.
pub(crate) fn interleave_cols<T>(pixbuf: &Pix2D<T>, cpp: usize, factor: usize) -> Pix2D<T>
where
  T: SubPixel,
{
  if !pixbuf.initialized || factor <= 1 || cpp == 0 {
    return pixbuf.clone();
  }
  let pixels = pixbuf.width / cpp;
  let mut data = Vec::with_capacity(pixbuf.data.len());
  for line in pixbuf.data.chunks_exact(pixbuf.width) {
    for field in 0..factor {
      for col in (field..pixels).step_by(factor) {
        data.extend_from_slice(&line[col * cpp..(col + 1) * cpp]);
      }
    }
  }
  Pix2D::new_with(data, pixbuf.width, pixbuf.height)
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

  pub fn new_with_default(width: usize, height: usize, default: T) -> Self {
    let data = vec![[default; N]; width * height];
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

  pub fn into_flatten(self) -> Vec<T> {
    self.data.into_iter().flatten().collect::<Vec<T>>()
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

  pub fn rotate_90cw(&self) -> Self {
    let mut out = Self::new(self.height, self.width);
    let out_width = out.width;
    out.pixels_mut().chunks_exact_mut(out_width).enumerate().for_each(|(row, line)| {
      line.iter_mut().enumerate().for_each(|(col, pixel)| {
        *pixel = *self.at(self.height - 1 - col, row);
      });
    });
    out
  }

  pub fn rotate_90ccw(&self) -> Self {
    let mut out = Self::new(self.height, self.width);
    let out_width = out.width;
    out.pixels_mut().chunks_exact_mut(out_width).enumerate().for_each(|(row, line)| {
      line.iter_mut().enumerate().for_each(|(col, pixel)| {
        *pixel = *self.at(col, self.width - 1 - row);
      });
    });
    out
  }

  pub fn rotate_180(&self) -> Self {
    let data = self.pixels().iter().rev().copied().collect_vec();
    Self::new_with(data, self.width, self.height)
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

/// An ugly hack to get multiple mutable references to Pix2D
pub struct SharedColor2D<T: SubPixel, const N: usize> {
  pub inner: UnsafeCell<Color2D<T, N>>,
}

impl<T, const N: usize> SharedColor2D<T, N>
where
  T: SubPixel,
{
  pub fn new(inner: Color2D<T, N>) -> Self {
    Self { inner: inner.into() }
  }

  /// Get inner Pix2D<> reference
  ///
  /// # Safety
  /// Only use this inside Rayon parallel iterators.
  #[allow(clippy::mut_from_ref)]
  pub unsafe fn inner_mut(&self) -> &mut Color2D<T, N> {
    unsafe { &mut *self.inner.get() }
  }

  pub fn into_inner(self) -> Color2D<T, N> {
    self.inner.into_inner()
  }
}

unsafe impl<T, const N: usize> Sync for SharedColor2D<T, N> where T: SubPixel {}

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
      debug_assert!(row < self.height && col < self.width);
      &*self.ptr.add(row * self.width + col)
    }
  }
}

unsafe impl<T, const N: usize> Sync for Color2DPtr<T, N> {}

#[macro_export]
macro_rules! alloc_image_f32_plain {
  ($width:expr, $height:expr, $dummy: expr) => {{
    // if $width * $height > 500000000 || $width > 50000 || $height > 50000 {
    //   panic!("rawler: surely there's no such thing as a >500MP or >50000 px wide/tall image!");
    // }
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
    // if $width * $height > 500000000 || $width > 50000 || $height > 50000 {
    //   panic!("rawler: surely there's no such thing as a >500MP or >50000 px wide/tall image!");
    // }
    if $dummy {
      $crate::pixarray::PixU16::new_uninit($width, $height)
    } else {
      $crate::pixarray::PixU16::new($width, $height)
    }
  }};
}

#[macro_export]
macro_rules! alloc_image_plain_typed {
  ($T:tt, $width:expr, $height:expr, $dummy: expr) => {{
    // if $width * $height > 500000000 || $width > 50000 || $height > 50000 {
    //   panic!("rawler: surely there's no such thing as a >500MP or >50000 px wide/tall image!");
    // }
    if $dummy {
      $crate::pixarray::Pix2D::<T>::new_uninit($width, $height)
    } else {
      $crate::pixarray::Pix2D::<T>::new($width, $height)
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
macro_rules! alloc_image_typed_ok {
  ($T:tt, $width:expr, $height:expr, $dummy: expr) => {{
    if $dummy {
      return Ok($crate::pixarray::Pix2D::<T>::new_uninit($width, $height));
    } else {
      $crate::alloc_image_plain_typed!($T, $width, $height, $dummy)
    }
  }};
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn deinterleave_rows_uneven_fields() {
    // 7 rows, factor 3: fields are rows [0, 3, 6], [1, 4], [2, 5]
    let stored: Vec<u16> = [0, 3, 6, 1, 4, 2, 5].into_iter().flat_map(|r| [r, r + 100]).collect();
    let out = deinterleave_rows(&Pix2D::new_with(stored, 2, 7), 3);
    let expected: Vec<u16> = (0..7).flat_map(|r| [r, r + 100]).collect();
    assert_eq!(out.data, expected);
  }

  #[test]
  fn deinterleave_cols_with_cpp() {
    // 5 pixels with 3 samples, factor 2: fields are pixels [0, 2, 4], [1, 3]
    let stored: Vec<u16> = [0, 2, 4, 1, 3].into_iter().flat_map(|p| [p, p + 10, p + 20]).collect();
    let out = deinterleave_cols(&Pix2D::new_with(stored, 15, 1), 3, 2);
    let expected: Vec<u16> = (0..5).flat_map(|p| [p, p + 10, p + 20]).collect();
    assert_eq!(out.data, expected);
  }

  #[test]
  fn interleave_is_inverse_of_deinterleave() {
    for (width, height, cpp, rows, cols) in [(7, 5, 1, 2, 2), (13, 11, 1, 6, 6), (5, 4, 3, 2, 3), (6, 6, 1, 6, 6)] {
      let image: Vec<u16> = (0..(width * cpp * height) as u16).collect();
      let pixbuf = Pix2D::new_with(image.clone(), width * cpp, height);
      let stored = interleave_cols(&interleave_rows(&pixbuf, rows), cpp, cols);
      let restored = deinterleave_rows(&deinterleave_cols(&stored, cpp, cols), rows);
      assert_eq!(restored.data, image, "{}x{} cpp {}", width, height, cpp);
    }
  }

  /// Store an image (cpp = 1) in sub-tile block order per tile
  fn to_block_order(image: &[u16], width: usize, height: usize, tile: (usize, usize), block: (usize, usize)) -> Vec<u16> {
    let mut out = image.to_vec();
    for tile_y in (0..height).step_by(tile.1) {
      for tile_x in (0..width).step_by(tile.0) {
        let mut stored = Vec::new();
        for block_y in (tile_y..tile_y + tile.1).step_by(block.0) {
          for block_x in (tile_x..tile_x + tile.0).step_by(block.1) {
            for row in block_y..block_y + block.0 {
              stored.extend_from_slice(&image[row * width + block_x..row * width + block_x + block.1]);
            }
          }
        }
        // Write stored data back into the tile area in row-scan order
        for (i, row) in (tile_y..tile_y + tile.1).enumerate() {
          out[row * width + tile_x..row * width + tile_x + tile.0].copy_from_slice(&stored[i * tile.0..(i + 1) * tile.0]);
        }
      }
    }
    out
  }

  #[test]
  fn reorder_sub_tile_blocks_roundtrip() -> std::result::Result<(), String> {
    let (width, height) = (16, 8);
    let image: Vec<u16> = (0..(width * height) as u16).collect();
    for block in [(2, 4), (4, 2), (2, 1)] {
      let stored = to_block_order(&image, width, height, (8, 4), block);
      let mut pixbuf = Pix2D::new_with(stored, width, height);
      reorder_sub_tile_blocks(&mut pixbuf, 1, 8, 4, block.0, block.1)?;
      assert_eq!(pixbuf.data, image, "block {block:?}");
    }
    Ok(())
  }

  #[test]
  fn reorder_sub_tile_blocks_rejects_non_multiple() {
    let mut pixbuf = Pix2D::new_with(vec![0_u16; 16 * 8], 16, 8);
    assert!(reorder_sub_tile_blocks(&mut pixbuf, 1, 8, 4, 3, 2).is_err());
  }
}
