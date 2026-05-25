use criterion::{BenchmarkGroup, Criterion, criterion_group, criterion_main, measurement::WallTime};
use rawler::{
  decoders::RawDecodeParams,
  devtools::rawdb::{get_rawdb_cache, rawdb_ensure_file},
  rawsource::RawSource,
};
use std::{hint::black_box, time::Duration};

fn bench_raw_image(group: &mut BenchmarkGroup<'_, WallTime>, name: &str, make: &str, model: &str, sample: &str) {
  let mut inner = || -> anyhow::Result<()> {
    let sample = rawdb_ensure_file(&get_rawdb_cache(), make, model, sample)?;
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
    "Canon",
    "EOS 5D Mark III",
    "raw_modes/Canon EOS 5D Mark III_RAW_ISO_200.CR2",
  );

  bench_raw_image(
    &mut group,
    "decode_canon_eos_5dmk3_sraw1",
    "Canon",
    "EOS 5D Mark III",
    "raw_modes/Canon EOS 5D Mark III_sRAW1_ISO_200.CR2",
  );

  bench_raw_image(
    &mut group,
    "decode_canon_eos_5dmk3_sraw2",
    "Canon",
    "EOS 5D Mark III",
    "raw_modes/Canon EOS 5D Mark III_sRAW2_ISO_200.CR2",
  );

  bench_raw_image(
    &mut group,
    "decode_canon_eos_r6_raw",
    "Canon",
    "EOS R6",
    "raw_modes/Canon EOS R6_RAW_ISO_100_nocrop_nodual.CR3",
  );

  bench_raw_image(
    &mut group,
    "decode_canon_eos_r6_craw",
    "Canon",
    "EOS R6",
    "raw_modes/Canon EOS R6_CRAW_ISO_100_nocrop_nodual.CR3",
  );

  bench_raw_image(
    &mut group,
    "decode_dng_ljpeg_tiles",
    "dnglab",
    "dng-compression-variants",
    "variants/ljpeg_tiles.dng",
  );

  bench_raw_image(&mut group, "decode_dng_10bit", "dnglab", "dng-compression-variants", "variants/10bit.dng");

  bench_raw_image(&mut group, "decode_dng_12bit", "dnglab", "dng-compression-variants", "variants/12bit.dng");

  bench_raw_image(
    &mut group,
    "decode_dng_16bit_be",
    "dnglab",
    "dng-compression-variants",
    "variants/16bit_bigend.dng",
  );

  bench_raw_image(
    &mut group,
    "decode_dng_8bit_lintable",
    "dnglab",
    "dng-compression-variants",
    "variants/8bit_lintable.dng",
  );

  bench_raw_image(
    &mut group,
    "decode_dng_ljpeg_singlestrip",
    "dnglab",
    "dng-compression-variants",
    "variants/ljpeg_singlestrip.dng",
  );

  bench_raw_image(
    &mut group,
    "decode_dng_lossy_jpeg_tiles",
    "dnglab",
    "dng-compression-variants",
    "variants/lossy_tiles.dng",
  );

  bench_raw_image(
    &mut group,
    "decode_dng_uncomp_multistrip_16rows",
    "dnglab",
    "dng-compression-variants",
    "variants/uncompressed_multistrip_16row.dng",
  );

  bench_raw_image(
    &mut group,
    "decode_dng_uncomp_multistrip_1row",
    "dnglab",
    "dng-compression-variants",
    "variants/uncompressed_multistrip_1row.dng",
  );

  group.finish();
}

criterion_group!(benches, decoding_raw_frame);
criterion_main!(benches);
