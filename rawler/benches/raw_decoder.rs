use criterion::{BenchmarkGroup, Criterion, criterion_group, criterion_main, measurement::WallTime};
use rawler::{decoders::RawDecodeParams, rawsource::RawSource};
use std::{hint::black_box, path::PathBuf, time::Duration};

fn rawdb_sample(sample: &str) -> PathBuf {
  let mut path = PathBuf::from(std::env::var("RAWLER_RAWDB").expect("RAWLER_RAWDB variable must be set in order to run RAW test!"));
  path.push(sample);
  if !path.exists() {
    eprintln!("Sample \"{}\" not found", path.display());
  }
  path
}

fn bench_raw_image(group: &mut BenchmarkGroup<'_, WallTime>, name: &str, sample: &str) {
  let mut inner = || -> anyhow::Result<()> {
    let sample = rawdb_sample(sample);
    let rawfile = RawSource::new(&sample)?;
    let decoder = rawler::get_decoder(&rawfile)?;
    group.bench_with_input(name, &rawfile, |b, rawfile| {
      b.iter(|| {
        decoder
          .raw_image(black_box(rawfile), black_box(&RawDecodeParams::default()), black_box(false))
          .expect("Decoder failed")
      })
    });
    Ok(())
  };

  if let Err(err) = inner() {
    eprintln!("Warning: Bench failed: {:?}", err);
  }
}

fn decoding_raw_frame(c: &mut Criterion) {
  let mut group = c.benchmark_group("decoding-raw-frame");
  // Configure Criterion.rs to detect smaller differences and increase sample size to improve
  // precision and counteract the resulting noise.
  group.significance_level(0.1).sample_size(20).measurement_time(Duration::from_secs(10));

  bench_raw_image(
    &mut group,
    "decode_canon_eos_5dmk3_raw",
    "cameras/Canon/EOS 5D Mark III/raw_modes/Canon EOS 5D Mark III_RAW_ISO_200.CR2",
  );

  bench_raw_image(
    &mut group,
    "decode_canon_eos_5dmk3_sraw1",
    "cameras/Canon/EOS 5D Mark III/raw_modes/Canon EOS 5D Mark III_sRAW1_ISO_200.CR2",
  );

  bench_raw_image(
    &mut group,
    "decode_canon_eos_5dmk3_sraw2",
    "cameras/Canon/EOS 5D Mark III/raw_modes/Canon EOS 5D Mark III_sRAW2_ISO_200.CR2",
  );

  bench_raw_image(
    &mut group,
    "decode_canon_eos_r6_raw",
    "cameras/Canon/EOS R6/raw_modes/Canon EOS R6_RAW_ISO_100_nocrop_nodual.CR3",
  );

  bench_raw_image(
    &mut group,
    "decode_canon_eos_r6_craw",
    "cameras/Canon/EOS R6/raw_modes/Canon EOS R6_CRAW_ISO_100_nocrop_nodual.CR3",
  );

  bench_raw_image(&mut group, "decode_dng_ljpeg_tiles", "dng/compression-sets/ljpeg_tiles.dng");

  bench_raw_image(&mut group, "decode_dng_10bit", "dng/compression-sets/10bit.dng");

  bench_raw_image(&mut group, "decode_dng_12bit", "dng/compression-sets/12bit.dng");

  bench_raw_image(&mut group, "decode_dng_16bit_be", "dng/compression-sets/16bit_bigend.dng");

  bench_raw_image(&mut group, "decode_dng_8bit_lintable", "dng/compression-sets/8bit_lintable.dng");

  bench_raw_image(&mut group, "decode_dng_ljpeg_singlestrip", "dng/compression-sets/ljpeg_singlestrip.dng");

  bench_raw_image(&mut group, "decode_dng_lossy_jpeg_tiles", "dng/compression-sets/lossy_tiles.dng");

  bench_raw_image(
    &mut group,
    "decode_dng_uncomp_multistrip_16rows",
    "dng/compression-sets/uncompressed_multistrip_16row.dng",
  );

  bench_raw_image(
    &mut group,
    "decode_dng_uncomp_multistrip_1row",
    "dng/compression-sets/uncompressed_multistrip_1row.dng",
  );

  group.finish();
}

criterion_group!(benches, decoding_raw_frame);
criterion_main!(benches);
