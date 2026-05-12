// Fuzz target: full decode pipeline
//
// Tests the complete decode path including format detection, TIFF/BMFF parsing,
// metadata extraction, and actual image decompression. This is the most
// comprehensive target — any crash here is a real bug.

#![no_main]
use libfuzzer_sys::fuzz_target;
use rawler::decoders::RawDecodeParams;
use rawler::rawsource::RawSource;

fuzz_target!(|data: &[u8]| {
    let source = RawSource::new_from_slice(data);
    let _ = rawler::decode(&source, &RawDecodeParams::default());
});
