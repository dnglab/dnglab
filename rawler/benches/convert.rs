use criterion::{BenchmarkGroup, Criterion, criterion_group, criterion_main, measurement::WallTime};
use rawler::{
  devtools::rawdb::{get_rawdb_cache, rawdb_ensure_file},
  dng::convert::{ConvertParams, convert_raw_file},
};
use std::{hint::black_box, io::Cursor, time::Duration};

fn bench_convert_raw(group: &mut BenchmarkGroup<'_, WallTime>, name: &str, make: &str, model: &str, sample: &str) {
  let mut inner = || -> anyhow::Result<()> {
    let sample = rawdb_ensure_file(&get_rawdb_cache(), make, model, sample)?;
    let mut dng = Cursor::new(Vec::with_capacity(100 * 1024 * 1024));
    let mut params = ConvertParams::default();
    params.embedded = false;
    group.bench_with_input(name, &sample, |b, sample| {
      b.iter(|| {
        convert_raw_file(black_box(&sample), &mut dng, black_box(&params)).expect("Convert failed");
      })
    });
    Ok(())
  };

  if let Err(err) = inner() {
    eprintln!("Warning: Bench failed: {:?}", err);
  }
}

fn convert(c: &mut Criterion) {
  let mut group = c.benchmark_group("convert-raw");
  // Configure Criterion.rs to detect smaller differences and increase sample size to improve
  // precision and counteract the resulting noise.
  group.significance_level(0.1).sample_size(20).measurement_time(Duration::from_secs(10));

  bench_convert_raw(
    &mut group,
    "convert_canon_eos_5dmk3_raw",
    "Canon",
    "EOS 5D Mark III",
    "raw_modes/Canon EOS 5D Mark III_RAW_ISO_200.CR2",
  );

  bench_convert_raw(
    &mut group,
    "convert_canon_eos_r6_raw",
    "Canon",
    "EOS R6",
    "raw_modes/Canon EOS R6_RAW_ISO_100_nocrop_nodual.CR3",
  );

  group.finish();
}

criterion_group!(benches, convert);
criterion_main!(benches);
