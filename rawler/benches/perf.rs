use criterion::{Criterion, criterion_group, criterion_main};
use std::hint::black_box;
use rawler::ljpeg92::LjpegCompressor;

fn generate_data(w: usize, h: usize, ncomp: usize) -> Vec<u16> {
  let mut img = vec![0; w * h * ncomp];

  for (i, pix) in img.iter_mut().enumerate() {
    *pix = i as u16 % 4u16;
  }
  img
}

fn encode_ljpeg(img: &[u16], w: usize, h: usize, ncomp: usize) {
  let pred = 1;
  let bps = 16;

  let state = LjpegCompressor::new(img, w, h, ncomp, bps, pred, 0, 0).unwrap();
  let _result = state.encode().unwrap();
}

fn criterion_benchmark(c: &mut Criterion) {
  let mut group = c.benchmark_group("ljpeg-encoder");
  // Configure Criterion.rs to detect smaller differences and increase sample size to improve
  // precision and counteract the resulting noise.
  group.significance_level(0.1).sample_size(20); //.measurement_time(Duration::from_secs(10));

  let x = generate_data(3000, 2000, 1);

  group.bench_with_input("encode_3000x2000", &x, |b, data| {
    b.iter(|| encode_ljpeg(black_box(data), black_box(3000), black_box(2000), black_box(1)))
  });

  group.finish();
}

criterion_group!(benches, criterion_benchmark);
criterion_main!(benches);
