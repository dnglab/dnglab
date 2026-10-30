// Benchmark of the mapping of raw values of a synthetic image of 24 MP to
// linear values, by the black and white levels

use criterion::{BatchSize, Criterion, criterion_group, criterion_main};
use rawler::{
  CFA, RawImage,
  decoders::Camera,
  formats::tiff::SRational,
  imgop::render::{ProcessingStep, RenderOptions, render},
  pixarray::PixU16,
  rawimage::{BlackLevel, CFAConfig, RawPhotometricInterpretation, WhiteLevel},
};
use std::{hint::black_box, time::Duration};

const WIDTH: usize = 6000;
const HEIGHT: usize = 4000;

const XTRANS: &str = "GGRGGBGGBGGRBRGRBGGGBGGRGGRGGBRBGBRG";

/// Mosaic image of 14 bits with `pattern`, a gradient with noise
fn mosaic_rawimage(pattern: &str, blacklevel: BlackLevel) -> RawImage {
  let mut state = 0x2545_f491_u32;
  let data = (0..WIDTH * HEIGHT)
    .map(|i| {
      state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
      let (x, y) = (i % WIDTH, i / WIDTH);
      (512 + (x + y) * 15000 / (WIDTH + HEIGHT) + (state >> 24) as usize) as u16
    })
    .collect();
  let mut camera = Camera::new();
  camera.cfa = CFA::new(pattern);
  let photometric = RawPhotometricInterpretation::Cfa(CFAConfig::new_from_camera(&camera));
  RawImage::new(
    camera,
    PixU16::new_with(data, WIDTH, HEIGHT),
    1,
    [2.0, 1.0, 1.5, f32::NAN],
    photometric,
    Some(blacklevel),
    Some(WhiteLevel::new_bits(14, 1)),
    false,
  )
}

fn bayer() -> RawImage {
  mosaic_rawimage("RGGB", BlackLevel::new(&[510_u32, 512, 514, 511], 2, 2, 1))
}

fn xtrans() -> RawImage {
  let levels: Vec<u32> = (0..36).map(|i| 1020 + i % 5).collect();
  mosaic_rawimage(XTRANS, BlackLevel::new(&levels, 6, 6, 1))
}

fn uniform() -> RawImage {
  mosaic_rawimage("RGGB", BlackLevel::new(&[512_u32], 1, 1, 1))
}

/// Bayer image with differences of the black level for each column and row
fn deltas() -> RawImage {
  let delta = |i: usize| SRational::new(i as i32 % 9 - 4, 4);
  let blacklevel = BlackLevel::new(&[510_u32, 512, 514, 511], 2, 2, 1).with_deltas((0..WIDTH).map(delta).collect(), (0..HEIGHT).map(delta).collect());
  mosaic_rawimage("RGGB", blacklevel)
}

fn rescale(c: &mut Criterion) {
  let mut group = c.benchmark_group("rescale");
  group.sample_size(10).measurement_time(Duration::from_secs(10));
  for (name, rawimage) in [("bayer", bayer()), ("xtrans", xtrans()), ("uniform", uniform()), ("deltas", deltas())] {
    // Mapping to linear values without white balance
    group.bench_function(format!("apply_scaling_{}", name), |b| {
      b.iter_batched(
        || rawimage.clone(),
        |mut rawimage| black_box(&mut rawimage).apply_scaling().expect("Scaling failed"),
        BatchSize::LargeInput,
      )
    });
    // Mapping to linear values and white balance of the mosaic data in the render pipeline
    let options = RenderOptions {
      steps: [ProcessingStep::Rescale, ProcessingStep::WhiteBalance].into_iter().collect(),
      ..Default::default()
    };
    group.bench_function(format!("render_wb_{}", name), |b| {
      b.iter_batched(
        || rawimage.clone(),
        |rawimage| render(black_box(rawimage), &options).expect("Rendering failed"),
        BatchSize::LargeInput,
      )
    });
  }
  group.finish();
}

criterion_group!(benches, rescale);
criterion_main!(benches);
