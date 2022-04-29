use rayon::prelude::*;

use crate::imgop::Rect;

pub struct Pix2D<T> {
  pub width: usize,
  pub height: usize,
  pub data: Vec<T>,
}

pub type PixU16 = Pix2D<u16>;

impl<T> Pix2D<T>
where
  T: Copy + Default + Send,
{
  pub fn new_with(data: Vec<T>, width: usize, height: usize) -> Self {
    assert_eq!(data.len(), height * width);
    Self { data, width, height }
  }

  pub fn new(width: usize, height: usize) -> Self {
    let data = vec![T::default(); width * height];
    Self { data, width, height }
  }

  pub fn into_inner(self) -> Vec<T> {
    self.data
  }

  pub fn pixels(&self) -> &[T] {
    &self.data
  }

  pub fn pixels_mut(&mut self) -> &mut [T] {
    &mut self.data
  }

  pub fn pixel_rows(&self) -> std::slice::ChunksExact<T> {
    self.data.chunks_exact(self.width)
  }

  pub fn pixel_rows_mut(&mut self) -> std::slice::ChunksExactMut<T> {
    self.data.chunks_exact_mut(self.width)
  }

  #[inline(always)]
  pub fn at(&self, row: usize, col: usize) -> &T {
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
    self.data.par_iter_mut().for_each(|v| *v = op(*v));
  }

  #[inline(always)]
  pub fn for_each_index<F>(&mut self, op: F)
  where
    F: Fn(T, usize, usize) -> T,
  {
    self
      .pixel_rows_mut()
      .enumerate()
      .for_each(|(row, rowbuf)| rowbuf.iter_mut().enumerate().for_each(|(col, v)| *v = op(*v, row, col)));
  }
}

impl<T> Default for Pix2D<T>
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
