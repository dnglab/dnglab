use criterion::{BenchmarkGroup, Criterion, criterion_group, criterion_main, measurement::WallTime};
use rawler::{
  decoders::RawDecodeParams,
  imgop::develop::{ProcessingStep, RawDevelop},
  rawsource::RawSource,
};
use std::{hint::black_box, path::PathBuf, time::Duration};

fn rawdb_sample(sample: &str) -> PathBuf {
  let mut path = PathBuf::from(std::env::var("RAWLER_RAWDB").expect("RAWLER_RAWDB variable must be set in order to run RAW test!"));
  path.push(sample);
  if !path.exists() {
    eprintln!("Sample \"{}\" not found", path.display());
  }
  path
}

fn bench_full_image(group: &mut BenchmarkGroup<'_, WallTime>, name: &str, sample: &str) {
  let mut inner = || -> anyhow::Result<()> {
    let sample = rawdb_sample(sample);
    let rawfile = RawSource::new(&sample)?;
    let decoder = rawler::get_decoder(&rawfile)?;
    let raw_params = RawDecodeParams::default();
    let rawimage = decoder.raw_image(&rawfile, &raw_params, false)?;
    let develop = RawDevelop::new_with(&[ProcessingStep::Demosaic]);

    group.bench_with_input(name, &develop, |b, develop| {
      b.iter(|| {
        let _ = develop.develop_intermediate(black_box(&rawimage)).expect("Development failed");
      })
    });
    Ok(())
  };

  if let Err(err) = inner() {
    eprintln!("Warning: Bench failed: {:?}", err);
  }
}

fn decoding_full_frame(c: &mut Criterion) {
  let mut group = c.benchmark_group("decoding-full-frame");
  // Configure Criterion.rs to detect smaller differences and increase sample size to improve
  // precision and counteract the resulting noise.
  group.significance_level(0.1).sample_size(20).measurement_time(Duration::from_secs(60));

  bench_full_image(
    &mut group,
    "full_image_fuji_xt5",
    "cameras/Fujifilm/X-T5/raw_modes/X-T5_ISO_125_Bitdepth_14_lossless.RAF",
  );

  bench_full_image(
    &mut group,
    "full_image_canon_r5",
    "cameras/Canon/EOS R5/raw_modes/Canon EOS R5_CRAW_ISO_100_nocrop_nodual.CR3",
  );

  group.finish();
}

criterion_group!(benches, decoding_full_frame);
criterion_main!(benches);
