use criterion::{BenchmarkGroup, Criterion, criterion_group, criterion_main, measurement::WallTime};
use rawler::{decoders::RawDecodeParams, rawsource::RawSource};
use std::{hint::black_box, time::Duration};

fn bench_raw_image(group: &mut BenchmarkGroup<'_, WallTime>, name: &str, make: &str, model: &str, sample: &str) {
  let mut inner = || -> anyhow::Result<()> {
    let sample = rawler::rawdb::sample(make, model, sample)?;
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

  bench_raw_image(
    &mut group,
    "decode_nikon_d850_lossless_14bit",
    "Nikon",
    "D850",
    "raw_modes/NIKON D850_Large_ISO_64_14bits_Lossless.NEF",
  );

  bench_raw_image(
    &mut group,
    "decode_nikon_d850_lossy_12bit",
    "Nikon",
    "D850",
    "raw_modes/NIKON D850_Large_ISO_64_12bits_Lossy (type 2).NEF",
  );

  bench_raw_image(
    &mut group,
    "decode_sony_ilce1_compressed",
    "Sony",
    "ILCE-1",
    "raw_modes/ILCE-1_ISO_500_14bits_Compressed RAW.ARW",
  );

  bench_raw_image(
    &mut group,
    "decode_sony_ilce1_lossless",
    "Sony",
    "ILCE-1",
    "raw_modes/ILCE-1_ISO_500_14bits_Lossless Compressed RAW.ARW",
  );

  bench_raw_image(
    &mut group,
    "decode_fuji_xh2_lossless",
    "Fujifilm",
    "X-H2",
    "raw_modes/Fujifilm_X-H2_14bit_lossless.raf",
  );

  bench_raw_image(
    &mut group,
    "decode_fuji_xh2_uncompressed",
    "Fujifilm",
    "X-H2",
    "raw_modes/Fujifilm_X-H2_14bit_uncompressed.raf",
  );

  bench_raw_image(&mut group, "decode_fuji_xa7_msb32", "Fujifilm", "X-A7", "raw_modes/X-A7_ISO_200.RAF");

  bench_raw_image(
    &mut group,
    "decode_panasonic_gh5m2_v6",
    "Panasonic",
    "DC-GH5M2",
    "raw_modes/DC-GH5M2_ISO_250_6fmt_4x3.RW2",
  );

  bench_raw_image(&mut group, "decode_panasonic_s9_v8", "Panasonic", "DC-S9", "raw_modes/DC-S9_ISO100.RW2");

  bench_raw_image(&mut group, "decode_om_1_orf", "OM", "OM-1", "raw_modes/OM-1_ISO_200_16bits_normal.ORF");

  bench_raw_image(
    &mut group,
    "decode_pentax_k1m2_pef",
    "Pentax",
    "K-1 Mark II",
    "raw_modes/PENTAX K-1 Mark II_RAW_ISO_125.PEF",
  );

  bench_raw_image(
    &mut group,
    "decode_samsung_nx1000_srw",
    "Samsung",
    "NX1000",
    "raw_modes/NX1000_ISO_400_Samsung SRW Compressed.SRW",
  );

  bench_raw_image(
    &mut group,
    "decode_phaseone_iq180_iiq",
    "Phase One",
    "IQ180",
    "raw_modes/IQ180_IIQ L_ISO_100.IIQ",
  );

  bench_raw_image(
    &mut group,
    "decode_phaseone_p65_iiq",
    "Phase One",
    "P65",
    "raw_modes/P65 Plus_IIQ L_ISO_200.IIQ",
  );

  bench_raw_image(&mut group, "decode_arri_alexa65", "Arri", "Alexa 65", "raw_modes/Alexa_65.ari");

  group.finish();
}

criterion_group!(benches, decoding_raw_frame);
criterion_main!(benches);
