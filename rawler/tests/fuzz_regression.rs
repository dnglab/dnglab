//! Stable regression tests for fuzzer-found panics.
//!
//! Each input under `tests/fuzz_corpus/` is a minimized reproducer for a panic
//! that was fixed (out-of-range slice, integer under/overflow, non-advancing
//! container loop, over-full Huffman table, bit-pump exhaustion, …). Every
//! reproducer is replayed through the public entry point of the matching
//! libFuzzer target and must NOT panic — a decoder has to fail gracefully on
//! malformed input, not abort.
//!
//! Unlike the `rawdb` suite these need no nightly, no `cargo fuzz`, and no
//! network: they run under a plain `cargo test`, so they gate every PR/CI run.
//! (libFuzzer corpora live in `fuzz/corpus/`, which is gitignored and only
//! exercised by `cargo fuzz`; this harness is the version-controlled, stable
//! counterpart.)

use std::path::PathBuf;

use rawler::decoders::RawDecodeParams;
use rawler::formats::ciff::CiffIFD;
use rawler::formats::jfif::Jfif;
use rawler::formats::tiff::reader::GenericTiffReader;
use rawler::rawsource::RawSource;

/// Load every committed reproducer (filename + bytes).
fn corpus() -> Vec<(String, Vec<u8>)> {
  let mut dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
  dir.push("tests/fuzz_corpus");
  let mut out = Vec::new();
  for entry in std::fs::read_dir(&dir).unwrap_or_else(|e| panic!("read {dir:?}: {e}")) {
    let path = entry.expect("dir entry").path();
    if path.is_file() {
      let name = path.file_name().unwrap().to_string_lossy().into_owned();
      out.push((name, std::fs::read(&path).expect("read seed")));
    }
  }
  assert!(!out.is_empty(), "no reproducers found in {dir:?}");
  out
}

/// Run `f` over every reproducer, printing the name first so a panicking seed
/// is identifiable in the test output.
fn for_each_seed(f: impl Fn(&[u8])) {
  for (name, data) in corpus() {
    eprintln!("fuzz_regression seed: {name}");
    f(&data);
  }
}

#[test]
fn fuzz_decode_full_no_panic() {
  for_each_seed(|data| {
    let source = RawSource::new_from_slice(data);
    let _ = rawler::decode(&source, &RawDecodeParams::default());
  });
}

#[test]
fn fuzz_decode_metadata_no_panic() {
  for_each_seed(|data| {
    let source = RawSource::new_from_slice(data);
    let _ = rawler::decode_dummy(&source);
  });
}

#[test]
fn fuzz_decode_unwrapped_no_panic() {
  for_each_seed(|data| {
    let source = RawSource::new_from_slice(data);
    let _ = rawler::decode_unwrapped(&source);
  });
}

#[test]
fn fuzz_get_decoder_no_panic() {
  for_each_seed(|data| {
    let source = RawSource::new_from_slice(data);
    let _ = rawler::get_decoder(&source);
  });
}

#[test]
fn fuzz_bmff_parse_no_panic() {
  for_each_seed(|data| {
    let _ = rawler::formats::bmff::parse_buffer(data);
  });
}

#[test]
fn fuzz_ciff_parse_no_panic() {
  for_each_seed(|data| {
    let _ = CiffIFD::new(data, 0, data.len(), 0);
  });
}

#[test]
fn fuzz_jfif_parse_no_panic() {
  for_each_seed(|data| {
    let mut cursor = std::io::Cursor::new(data);
    let _ = Jfif::parse(&mut cursor);
  });
}

#[test]
fn fuzz_tiff_parse_no_panic() {
  for_each_seed(|data| {
    let _ = GenericTiffReader::new_with_buffer(data, 0, 0, Some(10));
  });
}

/// Full pixel decode via the public `Decoder::raw_image` (the path `analyze` and
/// the rawdb suite use). Unlike `rawler::decode`/`decode_unwrapped`, this does
/// NOT wrap the decoder in `catch_unwind`, so a panic inside a pixel-decode path
/// (LJPEG Huffman, ORF/NEF/SRW/PEF predictors, the bit pumps, lookup tables, …)
/// propagates here and fails the test — which is what most of these reproducers
/// guard against. (`decode`'s catch_unwind would otherwise swallow them under a
/// unwinding test build.)
#[test]
fn fuzz_raw_image_no_panic() {
  for_each_seed(|data| {
    let source = RawSource::new_from_slice(data);
    if let Ok(decoder) = rawler::get_decoder(&source) {
      let _ = decoder.raw_image(&source, &RawDecodeParams::default(), false);
    }
  });
}
