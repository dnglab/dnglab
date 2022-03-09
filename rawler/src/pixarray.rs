pub struct Pix2D<T> {
  pub width: usize,
  pub height: usize,
  pub data: Vec<T>,
}

pub type PixU16 = Pix2D<u16>;

impl<T> Pix2D<T>
where
  T: Copy,
{
  pub fn new(data: Vec<T>, width: usize, height: usize) -> Self {
    assert_eq!(data.len(), height * width);
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

  pub fn for_each<F>(&mut self, op: F)
  where
    F: Fn(T) -> T,
  {
    self.data.iter_mut().for_each(|v| *v = op(*v));
  }

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

impl<T> Default for Pix2D<T> where T: Default {
    fn default() -> Self {
        Self { width: 0, height: 0, data: Default::default() }
    }
}
