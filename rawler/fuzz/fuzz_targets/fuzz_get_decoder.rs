// Fuzz target: decoder detection and format identification
//
// Tests just the format sniffing and decoder lookup logic. This exercises
// the initial file header parsing, magic byte matching, and TIFF/BMFF
// structure validation that happens before any decoding begins.

#![no_main]
use libfuzzer_sys::fuzz_target;
use rawler::rawsource::RawSource;

fuzz_target!(|data: &[u8]| {
    let source = RawSource::new_from_slice(data);
    let _ = rawler::get_decoder(&source);
});
