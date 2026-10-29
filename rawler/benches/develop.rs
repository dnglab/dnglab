// Benchmark of the development of a synthetic Bayer image of 24 MP. Besides
// the time, the peak of the memory allocated by one development is printed.

use criterion::{BatchSize, Criterion, criterion_group, criterion_main};
use rawler::{
  CFA, RawImage,
  camera_profile::CameraProfile,
  decoders::Camera,
  imgop::{Dim2, Point, Rect, develop::RawDevelop, matrix::transform_2d, xyz::Illuminant},
  pixarray::PixU16,
  rawimage::{BlackLevel, CFAConfig, RawPhotometricInterpretation, WhiteLevel},
};
use std::{
  alloc::{GlobalAlloc, Layout, System},
  hint::black_box,
  sync::atomic::{AtomicUsize, Ordering},
  time::Duration,
};

/// Allocator that tracks the allocated memory and its peak
struct PeakAllocator;

static ALLOCATED: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);

impl PeakAllocator {
  fn add(size: usize) {
    let allocated = ALLOCATED.fetch_add(size, Ordering::Relaxed) + size;
    PEAK.fetch_max(allocated, Ordering::Relaxed);
  }

  fn sub(size: usize) {
    ALLOCATED.fetch_sub(size, Ordering::Relaxed);
  }
}

unsafe impl GlobalAlloc for PeakAllocator {
  unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
    let ptr = unsafe { System.alloc(layout) };
    if !ptr.is_null() {
      Self::add(layout.size());
    }
    ptr
  }

  unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
    let ptr = unsafe { System.alloc_zeroed(layout) };
    if !ptr.is_null() {
      Self::add(layout.size());
    }
    ptr
  }

  unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
    unsafe { System.dealloc(ptr, layout) };
    Self::sub(layout.size());
  }

  unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
    let new_ptr = unsafe { System.realloc(ptr, layout, new_size) };
    if !new_ptr.is_null() {
      Self::sub(layout.size());
      Self::add(new_size);
    }
    new_ptr
  }
}

#[global_allocator]
static ALLOCATOR: PeakAllocator = PeakAllocator;

const WIDTH: usize = 6000;
const HEIGHT: usize = 4000;

const COLOR_MATRIX: [[f32; 3]; 3] = [[0.6722, -0.0635, -0.0963], [-0.4287, 1.2460, 0.2028], [-0.0908, 0.2162, 0.5668]];

/// RGGB image of 14 bits, a gradient with noise, with ActiveArea and DefaultCrop
fn bayer_rawimage() -> RawImage {
  let mut state = 0x2545_f491_u32;
  let data = (0..WIDTH * HEIGHT)
    .map(|i| {
      state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
      let (x, y) = (i % WIDTH, i / WIDTH);
      (512 + (x + y) * 15000 / (WIDTH + HEIGHT) + (state >> 24) as usize) as u16
    })
    .collect();
  let mut camera = Camera::new();
  camera.cfa = CFA::new("RGGB");
  let photometric = RawPhotometricInterpretation::Cfa(CFAConfig::new_from_camera(&camera));
  let mut rawimage = RawImage::new(
    camera,
    PixU16::new_with(data, WIDTH, HEIGHT),
    1,
    [2.0, 1.0, 1.5, f32::NAN],
    photometric,
    Some(BlackLevel::new(&[512_u32; 4], 2, 2, 1)),
    Some(WhiteLevel::new_bits(14, 1)),
    false,
  );
  rawimage.active_area = Some(Rect::new(Point::new(8, 8), Dim2::new(WIDTH - 16, HEIGHT - 16)));
  rawimage.crop_area = Some(Rect::new(Point::new(16, 16), Dim2::new(WIDTH - 32, HEIGHT - 32)));
  rawimage.camera_profiles = vec![CameraProfile {
    calibration_illuminant: [Some(Illuminant::D65), None, None],
    color_matrix: [Some(transform_2d(&COLOR_MATRIX)), None, None],
    ..Default::default()
  }];
  rawimage
}

fn develop(c: &mut Criterion) {
  let rawimage = bayer_rawimage();
  let develop = RawDevelop::default();

  // Peak memory of one development, including the copy of the raw image it consumes
  let allocated = ALLOCATED.load(Ordering::Relaxed);
  PEAK.store(allocated, Ordering::Relaxed);
  let image = develop.develop_image(rawimage.clone()).expect("Development failed");
  let peak = PEAK.load(Ordering::Relaxed) - allocated;
  drop(image);
  println!(
    "develop_bayer_24mp: peak memory {:.0} MiB, {:.1} bytes per pixel",
    peak as f64 / f64::from(1 << 20),
    peak as f64 / (WIDTH * HEIGHT) as f64
  );

  let mut group = c.benchmark_group("develop");
  group.sample_size(10).measurement_time(Duration::from_secs(30));
  group.bench_function("develop_bayer_24mp", |b| {
    b.iter_batched(
      || rawimage.clone(),
      |rawimage| develop.develop_image(black_box(rawimage)).expect("Development failed"),
      BatchSize::LargeInput,
    )
  });
  group.finish();
}

criterion_group!(benches, develop);
criterion_main!(benches);
