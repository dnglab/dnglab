use std::ops::Range;

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

  /// Sample at `row` and `col`
  ///
  /// Each access is bounds checked, loops over many samples are faster with
  /// the rows ([`Self::pixel_rows`], [`ImageRegion::rows`]).
  ///
  /// # Panics
  /// Panics if the position is outside of the buffer. `col` is not checked
  /// against the width, a larger value addresses the following row.
  #[inline(always)]
  pub fn at(&self, row: usize, col: usize) -> &T {
    debug_assert!(self.initialized);
    &self.data[row * self.width + col]
  }

  /// Mutable sample at `row` and `col`, see [`Self::at`]
  #[inline(always)]
  pub fn at_mut(&mut self, row: usize, col: usize) -> &mut T {
    debug_assert!(self.initialized);
    &mut self.data[row * self.width + col]
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
    output
      .par_pixel_rows_mut()
      .zip(pixbuf.region(area).par_rows())
      .for_each(|(line, src)| line.copy_from_slice(src));
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
    self.data.as_flattened().to_vec()
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

  /// Pixel at `row` and `col`
  ///
  /// Each access is bounds checked, loops over many pixels are faster with
  /// the rows ([`Self::pixel_rows`], [`ImageRegion::rows`]).
  ///
  /// # Panics
  /// Panics if the position is outside of the buffer. `col` is not checked
  /// against the width, a larger value addresses the following row.
  #[inline(always)]
  pub fn at(&self, row: usize, col: usize) -> &[T; N] {
    &self.data[row * self.width + col]
  }

  /// Mutable pixel at `row` and `col`, see [`Self::at`]
  #[inline(always)]
  pub fn at_mut(&mut self, row: usize, col: usize) -> &mut [T; N] {
    &mut self.data[row * self.width + col]
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
    for row in self.region(area).rows() {
      output.extend_from_slice(row.as_chunks::<N>().0);
    }
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

impl<T, const N: usize> Color2D<T, N> {
  /// Samples of all pixels, without copying the data
  pub fn into_flatten(self) -> Vec<T> {
    self.data.into_flattened()
  }

  /// Borrowed view of the pixels with N channels
  pub fn view(&self) -> ImageView<'_, T> {
    self.into()
  }

  /// Borrowed mutable view of the pixels with N channels
  pub fn view_mut(&mut self) -> ImageViewMut<'_, T> {
    self.into()
  }

  /// Borrowed region `area` of the pixels with N channels
  ///
  /// # Panics
  /// Panics if `area` is not inside of the image.
  pub fn region(&self, area: Rect) -> ImageRegion<'_, T> {
    self.view().region(area)
  }

  /// Borrowed mutable region `area` of the pixels, see [`Self::region`]
  pub fn region_mut(&mut self, area: Rect) -> ImageRegionMut<'_, T> {
    self.view_mut().into_region_mut(area)
  }
}

impl<T: SubPixel> Pix2D<T> {
  /// Borrowed view of the pixels with one channel
  pub fn view(&self) -> ImageView<'_, T> {
    self.into()
  }

  /// Borrowed mutable view of the pixels with one channel
  pub fn view_mut(&mut self) -> ImageViewMut<'_, T> {
    self.into()
  }

  /// Borrowed region `area` of the pixels with one channel, `area` is in
  /// samples like [`Self::width`]
  ///
  /// # Panics
  /// Panics if `area` is not inside of the image.
  pub fn region(&self, area: Rect) -> ImageRegion<'_, T> {
    self.view().region(area)
  }

  /// Borrowed mutable region `area` of the pixels, see [`Self::region`]
  pub fn region_mut(&mut self, area: Rect) -> ImageRegionMut<'_, T> {
    self.view_mut().into_region_mut(area)
  }
}

impl<T: SubPixel> From<Color2D<T, 1>> for Pix2D<T> {
  fn from(pixels: Color2D<T, 1>) -> Self {
    Self::new_with(pixels.data.into_flattened(), pixels.width, pixels.height)
  }
}

/// Image data of `width * height` pixels with `channels` interleaved samples,
/// borrowed from a buffer
///
/// The samples are available as flat slice ([`Self::as_slice`]) or as slice of
/// pixel arrays ([`Self::as_array_slice`]), both refer to the same memory. The
/// number of channels is checked once when the array slice is created, so
/// loops over it are compiled for a fixed number of channels.
pub struct ImageView<'a, T> {
  data: &'a [T],
  width: usize,
  height: usize,
  channels: usize,
}

impl<'a, T> ImageView<'a, T> {
  /// View of `data`, which holds `width * height` pixels of `channels` samples
  ///
  /// # Panics
  /// Panics if the length of `data` doesn't match the dimensions.
  pub fn new(data: &'a [T], width: usize, height: usize, channels: usize) -> Self {
    assert_eq!(
      data.len(),
      width * height * channels,
      "Image data doesn't match {}x{} pixels with {} channels",
      width,
      height,
      channels
    );
    Self { data, width, height, channels }
  }

  /// View of `width * height` pixels with N channels
  pub fn from_array_slice<const N: usize>(data: &'a [[T; N]], width: usize, height: usize) -> Self {
    Self::new(data.as_flattened(), width, height, N)
  }

  /// Pixels as arrays of N samples, `None` if the image has not N channels
  pub fn as_array_slice<const N: usize>(&self) -> Option<&'a [[T; N]]> {
    (N == self.channels).then(|| self.data.as_chunks::<N>().0)
  }

  /// All samples
  pub fn as_slice(&self) -> &'a [T] {
    self.data
  }

  pub fn width(&self) -> usize {
    self.width
  }

  pub fn height(&self) -> usize {
    self.height
  }

  pub fn channels(&self) -> usize {
    self.channels
  }

  pub fn dim(&self) -> Dim2 {
    Dim2::new(self.width, self.height)
  }

  pub fn rect(&self) -> Rect {
    Rect::new(Point::default(), self.dim())
  }

  /// Rows of `width * channels` samples
  pub fn rows(&self) -> std::slice::ChunksExact<'a, T> {
    self.data.chunks_exact((self.width * self.channels).max(1))
  }

  /// Region `area` of the image
  ///
  /// # Panics
  /// Panics if `area` is not inside of the image.
  pub fn region(&self, area: Rect) -> ImageRegion<'a, T> {
    ImageRegion::from(*self).region(area)
  }
}

impl<T> Clone for ImageView<'_, T> {
  fn clone(&self) -> Self {
    *self
  }
}

impl<T> Copy for ImageView<'_, T> {}

impl<T> std::fmt::Debug for ImageView<'_, T> {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    write!(f, "ImageView({}x{}, {} channels)", self.width, self.height, self.channels)
  }
}

/// Mutable image data of `width * height` pixels with `channels` interleaved
/// samples, borrowed from a buffer, see [`ImageView`]
pub struct ImageViewMut<'a, T> {
  data: &'a mut [T],
  width: usize,
  height: usize,
  channels: usize,
}

impl<'a, T> ImageViewMut<'a, T> {
  /// View of `data`, which holds `width * height` pixels of `channels` samples
  ///
  /// # Panics
  /// Panics if the length of `data` doesn't match the dimensions.
  pub fn new(data: &'a mut [T], width: usize, height: usize, channels: usize) -> Self {
    assert_eq!(
      data.len(),
      width * height * channels,
      "Image data doesn't match {}x{} pixels with {} channels",
      width,
      height,
      channels
    );
    Self { data, width, height, channels }
  }

  /// View of `width * height` pixels with N channels
  pub fn from_array_slice<const N: usize>(data: &'a mut [[T; N]], width: usize, height: usize) -> Self {
    Self::new(data.as_flattened_mut(), width, height, N)
  }

  /// Pixels as arrays of N samples, `None` if the image has not N channels
  pub fn as_array_slice<const N: usize>(&self) -> Option<&[[T; N]]> {
    (N == self.channels).then(|| self.data.as_chunks::<N>().0)
  }

  /// Mutable pixels as arrays of N samples, `None` if the image has not N channels
  pub fn as_array_slice_mut<const N: usize>(&mut self) -> Option<&mut [[T; N]]> {
    (N == self.channels).then(|| self.data.as_chunks_mut::<N>().0)
  }

  /// All samples
  pub fn as_slice(&self) -> &[T] {
    self.data
  }

  /// All samples, mutable
  pub fn as_mut_slice(&mut self) -> &mut [T] {
    self.data
  }

  /// Immutable view of the same data
  pub fn as_view(&self) -> ImageView<'_, T> {
    ImageView::new(self.data, self.width, self.height, self.channels)
  }

  /// Shorter-lived mutable view, to pass the view on without giving it away
  pub fn reborrow(&mut self) -> ImageViewMut<'_, T> {
    ImageViewMut::new(self.data, self.width, self.height, self.channels)
  }

  pub fn width(&self) -> usize {
    self.width
  }

  pub fn height(&self) -> usize {
    self.height
  }

  pub fn channels(&self) -> usize {
    self.channels
  }

  pub fn dim(&self) -> Dim2 {
    Dim2::new(self.width, self.height)
  }

  pub fn rect(&self) -> Rect {
    Rect::new(Point::default(), self.dim())
  }

  /// Rows of `width * channels` samples
  pub fn rows(&self) -> std::slice::ChunksExact<'_, T> {
    self.data.chunks_exact((self.width * self.channels).max(1))
  }

  /// Mutable rows of `width * channels` samples
  pub fn rows_mut(&mut self) -> std::slice::ChunksExactMut<'_, T> {
    self.data.chunks_exact_mut((self.width * self.channels).max(1))
  }

  /// Crop to `area` without a new buffer: the rows of `area` are moved to the
  /// start of the data, the returned view covers them
  ///
  /// The owner of the data can truncate it to the length of the returned view.
  /// Fails if `area` is not inside of the image.
  pub fn crop_in_place(self, area: Rect) -> crate::Result<ImageViewMut<'a, T>>
  where
    T: Copy,
  {
    if area.p.x + area.d.w > self.width || area.p.y + area.d.h > self.height {
      return Err(format!("Crop {:?} is outside of {:?}", area, self).into());
    }
    let row_len = area.d.w * self.channels;
    let data = self.data;
    // Rows only move to lower positions, so each row is read before it is overwritten
    for row in 0..area.d.h {
      let start = ((area.p.y + row) * self.width + area.p.x) * self.channels;
      data.copy_within(start..start + row_len, row * row_len);
    }
    Ok(ImageViewMut::new(&mut data[..row_len * area.d.h], area.d.w, area.d.h, self.channels))
  }

  /// Region `area` of the image
  ///
  /// # Panics
  /// Panics if `area` is not inside of the image.
  pub fn region(&self, area: Rect) -> ImageRegion<'_, T> {
    self.as_view().region(area)
  }

  /// Mutable region `area` of the image, see [`Self::region`]
  pub fn region_mut(&mut self, area: Rect) -> ImageRegionMut<'_, T> {
    self.reborrow().into_region_mut(area)
  }

  /// Mutable region `area` of the image that keeps the borrow of the data,
  /// see [`Self::region`]
  pub fn into_region_mut(self, area: Rect) -> ImageRegionMut<'a, T> {
    ImageRegionMut::from(self).into_region_mut(area)
  }
}

impl<T> std::fmt::Debug for ImageViewMut<'_, T> {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    write!(f, "ImageViewMut({}x{}, {} channels)", self.width, self.height, self.channels)
  }
}

impl<'a, T> From<ImageViewMut<'a, T>> for ImageView<'a, T> {
  fn from(view: ImageViewMut<'a, T>) -> Self {
    Self::new(view.data, view.width, view.height, view.channels)
  }
}

impl<'a, T: SubPixel> From<&'a Pix2D<T>> for ImageView<'a, T> {
  fn from(pixels: &'a Pix2D<T>) -> Self {
    Self::new(&pixels.data, pixels.width, pixels.height, 1)
  }
}

impl<'a, T: SubPixel> From<&'a mut Pix2D<T>> for ImageViewMut<'a, T> {
  fn from(pixels: &'a mut Pix2D<T>) -> Self {
    Self::new(&mut pixels.data, pixels.width, pixels.height, 1)
  }
}

impl<'a, T, const N: usize> From<&'a Color2D<T, N>> for ImageView<'a, T> {
  fn from(pixels: &'a Color2D<T, N>) -> Self {
    Self::from_array_slice(&pixels.data, pixels.width, pixels.height)
  }
}

impl<'a, T, const N: usize> From<&'a mut Color2D<T, N>> for ImageViewMut<'a, T> {
  fn from(pixels: &'a mut Color2D<T, N>) -> Self {
    Self::from_array_slice(&mut pixels.data, pixels.width, pixels.height)
  }
}

/// Range of the samples of `area` in the data of a region of `dim` pixels with
/// `stride` samples per row: from the first sample of the first row of `area`
/// to the last sample of its last row. `area` is relative to the region.
///
/// # Panics
/// Panics if `area` is not inside of the region.
fn region_range(area: Rect, dim: Dim2, stride: usize, channels: usize) -> Range<usize> {
  assert!(
    area.p.x + area.d.w <= dim.w && area.p.y + area.d.h <= dim.h,
    "{:?} is outside of {}x{} pixels",
    area,
    dim.w,
    dim.h
  );
  if area.is_empty() {
    return 0..0;
  }
  let start = area.p.y * stride + area.p.x * channels;
  start..start + (area.d.h - 1) * stride + area.d.w * channels
}

/// Rectangular area of an image with `channels` interleaved samples per pixel,
/// borrowed from the image buffer
///
/// The rows of the region are [`Line`]s of `width * channels` samples, a row
/// starts one image row after the previous one. [`Self::area`] is the position
/// in the image buffer, also for a region of a region. If the samples are
/// contiguous, [`Self::image_view`] gives them as one slice. Like an empty
/// [`ImageView`], a region of an empty area has no rows.
pub struct ImageRegion<'a, T> {
  /// Part of the image buffer from the first sample of the top-left pixel of
  /// the region to the last sample of its bottom-right pixel. Between two
  /// rows it holds the samples outside of the region, rows are `stride`
  /// samples apart.
  data: &'a [T],
  /// Samples from the start of a row to the start of the next row
  stride: usize,
  /// Position in the image buffer
  area: Rect,
  channels: usize,
}

impl<'a, T> ImageRegion<'a, T> {
  /// Region `area` of `data`, which holds `width * height` pixels of `channels` samples
  ///
  /// # Panics
  /// Panics if the length of `data` doesn't match the dimensions or if `area`
  /// is not inside of the image.
  pub fn new(data: &'a [T], width: usize, height: usize, channels: usize, area: Rect) -> Self {
    ImageView::new(data, width, height, channels).region(area)
  }

  pub fn width(&self) -> usize {
    self.area.d.w
  }

  pub fn height(&self) -> usize {
    self.area.d.h
  }

  pub fn channels(&self) -> usize {
    self.channels
  }

  pub fn dim(&self) -> Dim2 {
    self.area.d
  }

  /// Position of the region in the image buffer
  pub fn area(&self) -> Rect {
    self.area
  }

  fn row_len(&self) -> usize {
    self.area.d.w * self.channels
  }

  /// The rows follow each other without gaps
  fn is_contiguous(&self) -> bool {
    self.area.is_empty() || self.area.d.h == 1 || self.row_len() == self.stride
  }

  /// Row `row` of `width * channels` samples
  ///
  /// # Panics
  /// Panics if `row` is not inside of the region.
  pub fn row(&self, row: usize) -> Line<'a, T> {
    match self.data.chunks(self.stride.max(1)).nth(row) {
      Some(line) => &line[..self.row_len()],
      None => panic!("Row {} is outside of {:?}", row, self),
    }
  }

  /// Rows of `width * channels` samples
  pub fn rows(&self) -> impl ExactSizeIterator<Item = Line<'a, T>> + use<'a, T> {
    let row_len = self.row_len();
    self.data.chunks(self.stride.max(1)).map(move |row| &row[..row_len])
  }

  /// Rows of `width * channels` samples, processed in parallel
  pub fn par_rows(&self) -> impl IndexedParallelIterator<Item = Line<'a, T>> + use<'a, T>
  where
    T: Sync,
  {
    let row_len = self.row_len();
    self.data.par_chunks(self.stride.max(1)).map(move |row| &row[..row_len])
  }

  /// Region `area` of this region, `area` is relative to it
  ///
  /// # Panics
  /// Panics if `area` is not inside of the region.
  pub fn region(&self, area: Rect) -> ImageRegion<'a, T> {
    let range = region_range(area, self.dim(), self.stride, self.channels);
    ImageRegion {
      data: &self.data[range],
      stride: self.stride,
      area: area.translate(self.area.p),
      channels: self.channels,
    }
  }

  /// Copy of the samples, row by row
  pub fn to_vec(&self) -> Vec<T>
  where
    T: Copy,
  {
    let mut samples = Vec::with_capacity(self.row_len() * self.height());
    self.rows().for_each(|row| samples.extend_from_slice(row));
    samples
  }

  /// View of the samples as one slice, `None` if they are not contiguous
  ///
  /// The samples are contiguous if the region has the width of the image, a
  /// single row or no pixels.
  pub fn image_view(&self) -> Option<ImageView<'a, T>> {
    self
      .is_contiguous()
      .then(|| ImageView::new(self.data, self.width(), self.height(), self.channels))
  }
}

impl<T> Clone for ImageRegion<'_, T> {
  fn clone(&self) -> Self {
    *self
  }
}

impl<T> Copy for ImageRegion<'_, T> {}

impl<T> std::fmt::Debug for ImageRegion<'_, T> {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    write!(
      f,
      "ImageRegion({}x{} at {}:{}, {} channels)",
      self.area.d.w, self.area.d.h, self.area.p.x, self.area.p.y, self.channels
    )
  }
}

/// Mutable rectangular area of an image with `channels` interleaved samples
/// per pixel, borrowed from the image buffer, see [`ImageRegion`]
pub struct ImageRegionMut<'a, T> {
  /// Part of the image buffer from the first sample of the top-left pixel of
  /// the region to the last sample of its bottom-right pixel. Between two
  /// rows it holds the samples outside of the region, rows are `stride`
  /// samples apart.
  data: &'a mut [T],
  /// Samples from the start of a row to the start of the next row
  stride: usize,
  /// Position in the image buffer
  area: Rect,
  channels: usize,
}

impl<'a, T> ImageRegionMut<'a, T> {
  /// Region `area` of `data`, which holds `width * height` pixels of `channels` samples
  ///
  /// # Panics
  /// Panics if the length of `data` doesn't match the dimensions or if `area`
  /// is not inside of the image.
  pub fn new(data: &'a mut [T], width: usize, height: usize, channels: usize, area: Rect) -> Self {
    ImageViewMut::new(data, width, height, channels).into_region_mut(area)
  }

  pub fn width(&self) -> usize {
    self.area.d.w
  }

  pub fn height(&self) -> usize {
    self.area.d.h
  }

  pub fn channels(&self) -> usize {
    self.channels
  }

  pub fn dim(&self) -> Dim2 {
    self.area.d
  }

  /// Position of the region in the image buffer
  pub fn area(&self) -> Rect {
    self.area
  }

  /// Immutable region of the same data
  pub fn as_region(&self) -> ImageRegion<'_, T> {
    ImageRegion {
      data: self.data,
      stride: self.stride,
      area: self.area,
      channels: self.channels,
    }
  }

  /// Shorter-lived mutable region, to pass the region on without giving it away
  pub fn reborrow(&mut self) -> ImageRegionMut<'_, T> {
    ImageRegionMut {
      data: self.data,
      stride: self.stride,
      area: self.area,
      channels: self.channels,
    }
  }

  /// Row `row` of `width * channels` samples
  ///
  /// # Panics
  /// Panics if `row` is not inside of the region.
  pub fn row(&self, row: usize) -> Line<'_, T> {
    self.as_region().row(row)
  }

  /// Mutable row `row` of `width * channels` samples
  ///
  /// # Panics
  /// Panics if `row` is not inside of the region.
  pub fn row_mut(&mut self, row: usize) -> LineMut<'_, T> {
    let (area, row_len) = (self.area, self.as_region().row_len());
    match self.data.chunks_mut(self.stride.max(1)).nth(row) {
      Some(line) => &mut line[..row_len],
      None => panic!("Row {} is outside of {:?}", row, area),
    }
  }

  /// Rows of `width * channels` samples
  pub fn rows(&self) -> impl ExactSizeIterator<Item = Line<'_, T>> {
    self.as_region().rows()
  }

  /// Mutable rows of `width * channels` samples
  pub fn rows_mut(&mut self) -> impl ExactSizeIterator<Item = LineMut<'_, T>> {
    let row_len = self.as_region().row_len();
    self.data.chunks_mut(self.stride.max(1)).map(move |row| &mut row[..row_len])
  }

  /// Mutable rows of `width * channels` samples, processed in parallel
  pub fn par_rows_mut(&mut self) -> impl IndexedParallelIterator<Item = LineMut<'_, T>>
  where
    T: Send,
  {
    let row_len = self.as_region().row_len();
    self.data.par_chunks_mut(self.stride.max(1)).map(move |row| &mut row[..row_len])
  }

  /// Mutable region `area` of this region, `area` is relative to it
  ///
  /// # Panics
  /// Panics if `area` is not inside of the region.
  pub fn region_mut(&mut self, area: Rect) -> ImageRegionMut<'_, T> {
    self.reborrow().into_region_mut(area)
  }

  /// Mutable region `area` of this region that keeps the borrow of the data,
  /// see [`Self::region_mut`]
  pub fn into_region_mut(self, area: Rect) -> ImageRegionMut<'a, T> {
    let range = region_range(area, self.dim(), self.stride, self.channels);
    let Self {
      data,
      stride,
      area: parent,
      channels,
    } = self;
    ImageRegionMut {
      data: &mut data[range],
      stride,
      area: area.translate(parent.p),
      channels,
    }
  }

  /// Write `samples`, the samples of the region row by row as returned by
  /// [`ImageRegion::to_vec`]
  ///
  /// # Panics
  /// Panics if the length of `samples` doesn't match the dimensions.
  pub fn copy_from_slice(&mut self, samples: &[T])
  where
    T: Copy,
  {
    let row_len = self.as_region().row_len();
    assert_eq!(samples.len(), row_len * self.height(), "Samples don't match {:?}", self);
    for (row, samples) in self.rows_mut().zip(samples.chunks_exact(row_len.max(1))) {
      row.copy_from_slice(samples);
    }
  }

  /// View of the samples as one slice, `None` if they are not contiguous,
  /// see [`ImageRegion::image_view`]
  pub fn image_view(&self) -> Option<ImageView<'_, T>> {
    self.as_region().image_view()
  }

  /// Mutable view of the samples as one slice, `None` if they are not
  /// contiguous, see [`ImageRegion::image_view`]
  pub fn image_view_mut(&mut self) -> Option<ImageViewMut<'_, T>> {
    self.reborrow().into_image_view_mut()
  }

  /// Mutable view of the samples that keeps the borrow of the data, `None` if
  /// they are not contiguous, see [`ImageRegion::image_view`]
  pub fn into_image_view_mut(self) -> Option<ImageViewMut<'a, T>> {
    if !self.as_region().is_contiguous() {
      return None;
    }
    Some(ImageViewMut::new(self.data, self.area.d.w, self.area.d.h, self.channels))
  }

  /// Split into mutable regions of `areas`, which are relative to this region
  ///
  /// The regions are returned in the order of `areas` and can be written at
  /// the same time, also by different threads. Like for [`ImageRegion`], the
  /// region of an empty area has no rows. Fails if an area is not inside of
  /// the region or overlaps another area.
  pub fn split_regions_mut(self, areas: &[Rect]) -> crate::Result<Vec<DisjointRegionMut<'a, T>>> {
    let dim = self.dim();
    if let Some(area) = areas.iter().find(|area| area.p.x + area.d.w > dim.w || area.p.y + area.d.h > dim.h) {
      return Err(format!("{:?} is outside of {}x{} pixels", area, dim.w, dim.h).into());
    }
    let row_len = self.as_region().row_len();
    let Self {
      data,
      stride,
      area: parent,
      channels,
    } = self;
    let mut regions: Vec<DisjointRegionMut<'a, T>> = areas
      .iter()
      .map(|area| DisjointRegionMut {
        rows: Vec::with_capacity(if area.is_empty() { 0 } else { area.d.h }),
        area: area.translate(parent.p),
        channels,
      })
      .collect();
    // Areas with pixels by their first row
    let mut by_y: Vec<usize> = (0..areas.len()).filter(|&i| !areas[i].is_empty()).collect();
    by_y.sort_by_key(|&i| areas[i].p.y);
    let mut by_y = by_y.into_iter().peekable();
    // Areas of the current row from left to right, their row segments are
    // split off in this order
    let mut row_areas: Vec<usize> = Vec::new();
    for (y, row) in data.chunks_mut(stride.max(1)).map(|row| &mut row[..row_len]).enumerate() {
      row_areas.retain(|&i| y < areas[i].p.y + areas[i].d.h);
      let count = row_areas.len();
      row_areas.extend(std::iter::from_fn(|| by_y.next_if(|&i| areas[i].p.y <= y)));
      if row_areas.len() > count {
        row_areas.sort_by_key(|&i| areas[i].p.x);
      }
      // Part of the row that is not split off yet, it starts at pixel x
      let (mut rest, mut x) = (row, 0);
      for &i in &row_areas {
        let area = areas[i];
        if area.p.x < x {
          return Err(format!("{:?} overlaps another area", area).into());
        }
        let (_, tail) = std::mem::take(&mut rest).split_at_mut((area.p.x - x) * channels);
        let (segment, tail) = tail.split_at_mut(area.d.w * channels);
        regions[i].rows.push(segment);
        (rest, x) = (tail, area.p.x + area.d.w);
      }
    }
    Ok(regions)
  }
}

impl<T> std::fmt::Debug for ImageRegionMut<'_, T> {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    write!(
      f,
      "ImageRegionMut({}x{} at {}:{}, {} channels)",
      self.area.d.w, self.area.d.h, self.area.p.x, self.area.p.y, self.channels
    )
  }
}

/// Mutable rectangular area of an image whose rows are borrowed one by one,
/// see [`ImageRegionMut::split_regions_mut`]
///
/// Unlike an [`ImageRegionMut`], it doesn't borrow the samples between its
/// rows, so regions side by side can be borrowed at the same time.
pub struct DisjointRegionMut<'a, T> {
  /// Rows of `width * channels` samples
  rows: Vec<LineMut<'a, T>>,
  /// Position in the image buffer
  area: Rect,
  channels: usize,
}

impl<T> DisjointRegionMut<'_, T> {
  pub fn width(&self) -> usize {
    self.area.d.w
  }

  pub fn height(&self) -> usize {
    self.area.d.h
  }

  pub fn channels(&self) -> usize {
    self.channels
  }

  pub fn dim(&self) -> Dim2 {
    self.area.d
  }

  /// Position of the region in the image buffer
  pub fn area(&self) -> Rect {
    self.area
  }

  /// Row `row` of `width * channels` samples
  ///
  /// # Panics
  /// Panics if `row` is not inside of the region.
  pub fn row(&self, row: usize) -> Line<'_, T> {
    match self.rows.get(row) {
      Some(line) => line,
      None => panic!("Row {} is outside of {:?}", row, self),
    }
  }

  /// Mutable row `row` of `width * channels` samples
  ///
  /// # Panics
  /// Panics if `row` is not inside of the region.
  pub fn row_mut(&mut self, row: usize) -> LineMut<'_, T> {
    let area = self.area;
    match self.rows.get_mut(row) {
      Some(line) => line,
      None => panic!("Row {} is outside of {:?}", row, area),
    }
  }

  /// Mutable rows of `width * channels` samples
  pub fn rows_mut(&mut self) -> impl ExactSizeIterator<Item = LineMut<'_, T>> {
    self.rows.iter_mut().map(|row| &mut **row)
  }
}

impl<T> std::fmt::Debug for DisjointRegionMut<'_, T> {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    write!(
      f,
      "DisjointRegionMut({}x{} at {}:{}, {} channels)",
      self.area.d.w, self.area.d.h, self.area.p.x, self.area.p.y, self.channels
    )
  }
}

impl<'a, T> From<ImageView<'a, T>> for ImageRegion<'a, T> {
  fn from(view: ImageView<'a, T>) -> Self {
    Self {
      data: view.data,
      stride: view.width * view.channels,
      area: view.rect(),
      channels: view.channels,
    }
  }
}

impl<'a, T> From<ImageViewMut<'a, T>> for ImageRegionMut<'a, T> {
  fn from(view: ImageViewMut<'a, T>) -> Self {
    let (stride, area) = (view.width * view.channels, view.rect());
    Self {
      data: view.data,
      stride,
      area,
      channels: view.channels,
    }
  }
}

impl<'a, T> From<ImageRegionMut<'a, T>> for ImageRegion<'a, T> {
  fn from(region: ImageRegionMut<'a, T>) -> Self {
    Self {
      data: region.data,
      stride: region.stride,
      area: region.area,
      channels: region.channels,
    }
  }
}

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

  #[test]
  fn image_view_array_slices() {
    let data: Vec<u16> = (0..12).collect();
    let view = ImageView::new(&data, 2, 2, 3);
    let pixels = view.as_array_slice::<3>().expect("three channels");
    assert_eq!(pixels, &[[0, 1, 2], [3, 4, 5], [6, 7, 8], [9, 10, 11]]);
    assert!(view.as_array_slice::<4>().is_none());
    // The pixel arrays are the same memory
    assert_eq!(pixels.as_ptr().cast::<u16>(), data.as_ptr());
    let back = ImageView::from_array_slice(pixels, 2, 2);
    assert_eq!((back.channels(), back.as_slice()), (3, data.as_slice()));
    assert_eq!(view.rows().count(), 2);

    let mut samples = vec![0.0_f32; 8];
    let mut view = ImageViewMut::new(&mut samples, 2, 1, 4);
    assert!(view.as_array_slice_mut::<3>().is_none());
    view.as_array_slice_mut::<4>().expect("four channels")[1] = [1.0, 2.0, 3.0, 4.0];
    assert_eq!(samples, [0.0, 0.0, 0.0, 0.0, 1.0, 2.0, 3.0, 4.0]);
  }

  #[test]
  #[should_panic(expected = "doesn't match")]
  fn image_view_checks_length() {
    let data = [0_u16; 5];
    let _ = ImageView::new(&data, 2, 1, 3);
  }

  /// Channels and samples of anything that converts into a view
  fn view_info<'a, T: 'a>(image: impl Into<ImageView<'a, T>>) -> (usize, usize) {
    let view = image.into();
    (view.channels(), view.as_slice().len())
  }

  #[test]
  fn image_view_from_containers() {
    let pixels = PixU16::new(4, 2);
    let mut rgb = RgbF32::new(4, 2);
    assert_eq!(view_info(&pixels), (1, 8));
    assert_eq!(view_info(&rgb), (3, 24));
    assert_eq!(view_info(rgb.view()), (3, 24));
    assert_eq!(view_info(rgb.view_mut()), (3, 24));
    assert_eq!(view_info(ImageView::new(&[0.5_f32; 6], 1, 2, 3)), (3, 6));

    let mut samples: Vec<u16> = (0..24).collect();
    let cropped = ImageViewMut::new(&mut samples, 4, 3, 2)
      .crop_in_place(Rect::new(Point::new(1, 1), Dim2::new(2, 2)))
      .expect("crop");
    assert_eq!(
      (cropped.width(), cropped.height(), cropped.as_slice()),
      (2, 2, &[10, 11, 12, 13, 18, 19, 20, 21][..])
    );
    assert!(
      ImageViewMut::new(&mut samples, 4, 3, 2)
        .crop_in_place(Rect::new(Point::new(3, 0), Dim2::new(2, 1)))
        .is_err()
    );

    let mono = Pix2D::from(Color2D::<u16, 1>::new_with(vec![[1], [2]], 2, 1));
    assert_eq!((mono.width, mono.data), (2, vec![1, 2]));
    let rgb = Color2D::<u16, 3>::new_with(vec![[1, 2, 3], [4, 5, 6]], 2, 1);
    assert_eq!(rgb.into_flatten(), [1, 2, 3, 4, 5, 6]);
  }

  #[test]
  fn image_region_rows() {
    // 4x3 pixels with 3 channels, the samples are their index
    let data: Vec<u16> = (0..36).collect();
    let view = ImageView::new(&data, 4, 3, 3);
    let region = view.region(Rect::new(Point::new(1, 1), Dim2::new(2, 2)));
    assert_eq!((region.dim(), region.channels()), (Dim2::new(2, 2), 3));
    assert_eq!(region.rows().collect::<Vec<_>>(), [&data[15..21], &data[27..33]]);
    assert_eq!(region.par_rows().collect::<Vec<_>>(), [&data[15..21], &data[27..33]]);
    assert_eq!(region.row(1), &data[27..33]);
    assert_eq!(region.to_vec(), [&data[15..21], &data[27..33]].concat());

    // A region of a region is positioned in the image buffer
    let inner = region.region(Rect::new(Point::new(1, 0), Dim2::new(1, 2)));
    assert_eq!(inner.area(), Rect::new(Point::new(2, 1), Dim2::new(1, 2)));
    assert_eq!(inner.to_vec(), [&data[18..21], &data[30..33]].concat());
    assert_eq!(ImageRegion::new(&data, 4, 3, 3, inner.area()).to_vec(), inner.to_vec());

    // The whole image, a region of a Pix2D is in samples
    let whole = ImageRegion::from(view);
    assert_eq!((whole.area(), whole.rows().len()), (view.rect(), 3));
    assert_eq!(whole.to_vec(), data);
    let pixels = PixU16::new_with(data.clone(), 12, 3);
    assert_eq!(pixels.region(Rect::new(Point::new(3, 1), Dim2::new(6, 2))).to_vec(), region.to_vec());

    // An empty area has no rows
    let empty = view.region(Rect::new(Point::new(4, 1), Dim2::new(0, 2)));
    assert_eq!((empty.height(), empty.rows().len(), empty.to_vec()), (2, 0, Vec::new()));
  }

  #[test]
  fn image_region_nested_levels() {
    // 8x6 pixels with 2 channels, the samples are their index
    let (width, channels) = (8, 2);
    let mut data: Vec<u16> = (0..96).collect();
    // Samples of `area` of the image, in buffer coordinates
    let expected = |area: Rect| -> Vec<u16> {
      (area.y()..area.y() + area.height())
        .flat_map(|row| (area.x() * channels..(area.x() + area.width()) * channels).map(move |col| (row * width * channels + col) as u16))
        .collect()
    };
    let outer_area = Rect::new(Point::new(3, 2), Dim2::new(3, 3));
    let inner_area = Rect::new(Point::new(4, 3), Dim2::new(2, 2));

    // Each level is positioned in the image buffer
    let view = ImageView::new(&data, width, 6, channels);
    let outer = view.region(Rect::new(Point::new(1, 1), Dim2::new(6, 4)));
    let middle = outer.region(Rect::new(Point::new(2, 1), Dim2::new(3, 3)));
    let inner = middle.region(Rect::new(Point::new(1, 1), Dim2::new(2, 2)));
    assert_eq!((middle.area(), inner.area()), (outer_area, inner_area));
    for region in [outer, middle, inner] {
      assert_eq!(region.to_vec(), expected(region.area()), "{:?}", region);
      assert_eq!(region.par_rows().flatten().copied().collect::<Vec<_>>(), expected(region.area()));
    }
    assert_eq!(inner.to_vec(), [56, 57, 58, 59, 72, 73, 74, 75]);

    // Mutable regions of a mutable view write only inside the innermost area
    let mut view = ImageViewMut::new(&mut data, width, 6, channels);
    let mut outer = view.region_mut(Rect::new(Point::new(1, 1), Dim2::new(6, 4)));
    let mut middle = outer.region_mut(Rect::new(Point::new(2, 1), Dim2::new(3, 3)));
    assert_eq!(middle.area(), outer_area);
    let mut inner = middle.region_mut(Rect::new(Point::new(1, 1), Dim2::new(2, 2)));
    assert_eq!(inner.area(), inner_area);
    inner.par_rows_mut().for_each(|row| row.fill(u16::MAX));
    let written = expected(inner_area);
    for (i, sample) in data.iter().enumerate() {
      let value = if written.contains(&(i as u16)) { u16::MAX } else { i as u16 };
      assert_eq!(*sample, value, "sample {}", i);
    }
  }

  #[test]
  #[should_panic(expected = "is outside of")]
  fn image_region_checks_area() {
    let data = [0_u16; 12];
    let _ = ImageView::new(&data, 4, 3, 1).region(Rect::new(Point::new(3, 0), Dim2::new(2, 1)));
  }

  #[test]
  fn image_region_mut_writes_inside_area() {
    // 4x3 pixels with 2 channels
    let mut samples = vec![0_u16; 24];
    let mut view = ImageViewMut::new(&mut samples, 4, 3, 2);
    let mut region = view.region_mut(Rect::new(Point::new(1, 0), Dim2::new(2, 3)));
    region.par_rows_mut().for_each(|row| row.fill(1));
    region.rows_mut().enumerate().for_each(|(i, row)| row[0] += i as u16);
    region.row_mut(2)[3] = 9;
    assert_eq!(samples, [0, 0, 1, 1, 1, 1, 0, 0, 0, 0, 2, 1, 1, 1, 0, 0, 0, 0, 3, 1, 1, 9, 0, 0]);

    // Nested regions, the samples are written row by row
    let mut region = ImageRegionMut::new(&mut samples, 4, 3, 2, Rect::new(Point::new(1, 1), Dim2::new(3, 2)));
    let mut inner = region.region_mut(Rect::new(Point::new(1, 0), Dim2::new(2, 2)));
    assert_eq!(inner.area(), Rect::new(Point::new(2, 1), Dim2::new(2, 2)));
    let values: Vec<u16> = (100..108).collect();
    inner.copy_from_slice(&values);
    assert_eq!(inner.as_region().to_vec(), values);
    assert_eq!(ImageRegion::from(region).to_vec(), [2, 1, 100, 101, 102, 103, 3, 1, 104, 105, 106, 107]);
    let whole = ImageRegionMut::from(ImageViewMut::new(&mut samples, 4, 3, 2));
    assert_eq!(whole.area(), Rect::new(Point::zero(), Dim2::new(4, 3)));

    let mut rgb = RgbF32::new(3, 2);
    rgb.region_mut(Rect::new(Point::new(1, 1), Dim2::new(2, 1))).row_mut(0).fill(0.5);
    assert_eq!(rgb.data, [[0.0; 3], [0.0; 3], [0.0; 3], [0.0; 3], [0.5; 3], [0.5; 3]]);
  }

  #[test]
  fn image_region_image_view() {
    let mut data: Vec<u16> = (0..36).collect();
    let view = ImageView::new(&data, 4, 3, 3);
    let contiguous = |area: Rect| view.region(area).image_view().map(|view| (view.dim(), view.channels(), view.as_slice()));
    // Whole image, full-width band, single row and empty area
    assert_eq!(contiguous(view.rect()), Some((Dim2::new(4, 3), 3, &data[..])));
    assert_eq!(
      contiguous(Rect::new(Point::new(0, 1), Dim2::new(4, 2))),
      Some((Dim2::new(4, 2), 3, &data[12..]))
    );
    assert_eq!(
      contiguous(Rect::new(Point::new(1, 2), Dim2::new(2, 1))),
      Some((Dim2::new(2, 1), 3, &data[27..33]))
    );
    assert_eq!(contiguous(Rect::new(Point::new(1, 1), Dim2::new(0, 2))), Some((Dim2::new(0, 2), 3, &data[..0])));
    // Rows of a part of the image width are not contiguous
    assert_eq!(contiguous(Rect::new(Point::new(1, 0), Dim2::new(2, 2))), None);

    let mut band = ImageRegionMut::new(&mut data, 4, 3, 3, Rect::new(Point::new(0, 1), Dim2::new(4, 2)));
    assert_eq!(band.image_view().map(|view| view.as_slice().len()), Some(24));
    band.image_view_mut().expect("contiguous").as_mut_slice().fill(0);
    assert!(
      ImageRegionMut::new(&mut data, 4, 3, 3, Rect::new(Point::new(1, 0), Dim2::new(2, 2)))
        .into_image_view_mut()
        .is_none()
    );
    assert_eq!(data[..12], (0..12).collect::<Vec<u16>>());
    assert!(data[12..].iter().all(|v| *v == 0));
  }

  fn area(x: usize, y: usize, w: usize, h: usize) -> Rect {
    Rect::new(Point::new(x, y), Dim2::new(w, h))
  }

  #[test]
  fn split_regions_mut_writes_disjoint_areas() {
    // 6x4 pixels, a grid of 2x2 tiles with 3x2 pixels, written in parallel
    let mut pixels = PixU16::new(6, 4);
    let areas = [area(0, 0, 3, 2), area(3, 0, 3, 2), area(0, 2, 3, 2), area(3, 2, 3, 2)];
    let regions = ImageRegionMut::from(pixels.view_mut()).split_regions_mut(&areas).expect("disjoint areas");
    assert_eq!(regions.iter().map(|region| region.area()).collect::<Vec<_>>(), areas);
    regions.into_par_iter().enumerate().for_each(|(i, mut region)| {
      region.rows_mut().enumerate().for_each(|(row, line)| line.fill(10 * i as u16 + row as u16));
    });
    #[rustfmt::skip]
    assert_eq!(pixels.data, [
      0, 0, 0, 10, 10, 10,
      1, 1, 1, 11, 11, 11,
      20, 20, 20, 30, 30, 30,
      21, 21, 21, 31, 31, 31,
    ]);

    // Column strips over the full height, from right to left with a gap between them
    let mut rgb = RgbF32::new(4, 2);
    let areas = [area(2, 0, 2, 2), area(0, 0, 1, 2)];
    let mut regions = ImageRegionMut::from(rgb.view_mut()).split_regions_mut(&areas).expect("disjoint areas");
    assert_eq!(regions[0].row(1).len(), 6);
    regions[0].row_mut(1)[3..].fill(0.5);
    regions[1].row_mut(0)[2] = 1.0;
    assert_eq!(regions[1].row(0), [0.0, 0.0, 1.0]);
    assert_eq!(
      rgb.data,
      [[0.0, 0.0, 1.0], [0.0; 3], [0.0; 3], [0.0; 3], [0.0; 3], [0.0; 3], [0.0; 3], [0.5; 3]]
    );
  }

  #[test]
  fn split_regions_mut_of_region() {
    // 4x3 pixels with 2 channels, split the region of 3x2 pixels at 1:1
    let mut samples = vec![0_u16; 24];
    let region = ImageRegionMut::new(&mut samples, 4, 3, 2, area(1, 1, 3, 2));
    let mut regions = region.split_regions_mut(&[area(0, 0, 1, 2), area(1, 1, 2, 1)]).expect("disjoint areas");
    assert_eq!(regions[0].area(), area(1, 1, 1, 2));
    assert_eq!(regions[1].area(), area(2, 2, 2, 1));
    assert_eq!(
      (regions[1].dim(), regions[1].width(), regions[1].height(), regions[1].channels()),
      (Dim2::new(2, 1), 2, 1, 2)
    );
    regions[0].rows_mut().for_each(|row| row.fill(1));
    regions[1].row_mut(0).copy_from_slice(&[2, 3, 4, 5]);
    #[rustfmt::skip]
    assert_eq!(samples, [
      0, 0, 0, 0, 0, 0, 0, 0,
      0, 0, 1, 1, 0, 0, 0, 0,
      0, 0, 1, 1, 2, 3, 4, 5,
    ]);
  }

  #[test]
  fn split_regions_mut_checks_areas() {
    let mut pixels = PixU16::new(4, 3);
    let mut split = |areas: &[Rect]| {
      ImageRegionMut::from(pixels.view_mut())
        .split_regions_mut(areas)
        .map(|mut regions| regions.iter_mut().map(|region| region.rows_mut().len()).collect::<Vec<_>>())
    };
    // Outside of the image
    assert!(split(&[area(3, 0, 2, 1)]).is_err());
    assert!(split(&[area(0, 2, 1, 2)]).is_err());
    assert!(split(&[area(5, 0, 0, 0)]).is_err());
    // Overlapping areas, in both orders
    assert!(split(&[area(0, 0, 2, 2), area(1, 1, 2, 2)]).is_err());
    assert!(split(&[area(1, 1, 2, 2), area(0, 0, 2, 2)]).is_err());
    assert!(split(&[area(0, 0, 4, 3), area(0, 0, 4, 3)]).is_err());
    // Adjacent areas, an empty area inside of another one has no rows
    assert_eq!(split(&[area(0, 0, 2, 3), area(2, 1, 2, 2), area(1, 1, 0, 1)]).ok(), Some(vec![3, 2, 0]));
    assert_eq!(split(&[]).ok(), Some(vec![]));
  }

  #[test]
  fn crop_pixels() {
    // 4x3 samples, the values are their index
    let pixels = PixU16::new_with((0..12).collect(), 4, 3);
    let area = Rect::new(Point::new(1, 1), Dim2::new(2, 2));
    let cropped = pixels.crop(area);
    assert_eq!((cropped.width, cropped.height, cropped.data), (2, 2, vec![5, 6, 9, 10]));
    assert_eq!(pixels.clone().into_crop(area).data, [5, 6, 9, 10]);
    // Cropping to the whole image is a no-op, an uninitialized buffer stays uninitialized
    assert_eq!(pixels.clone().into_crop(pixels.rect()).data, pixels.data);
    let uninit = PixU16::new_uninit(4, 3).into_crop(area);
    assert_eq!((uninit.dim(), uninit.is_initialized()), (Dim2::new(2, 2), false));

    let rgb = Color2D::<u16, 3>::new_with((0..12).map(|i| [i, i + 100, i + 200]).collect(), 4, 3);
    let cropped = rgb.crop(area);
    assert_eq!((cropped.width, cropped.height), (2, 2));
    assert_eq!(cropped.data, [5, 6, 9, 10].map(|i| [i, i + 100, i + 200]));
  }

  #[test]
  #[should_panic(expected = "index out of bounds")]
  fn pix2d_at_checks_the_buffer() {
    let _ = PixU16::new(4, 3).at(3, 4);
  }

  #[test]
  #[should_panic(expected = "index out of bounds")]
  fn color2d_at_mut_checks_the_buffer() {
    let _ = RgbF32::new(4, 3).at_mut(3, 0);
  }
}
