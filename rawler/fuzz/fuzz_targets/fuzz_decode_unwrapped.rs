// Fuzz target: raw decoders (bypasses format parsing)
//
// Tests the actual decompression engines directly — LJPEG, packed bits, CRX,
// RADC, etc. Format detection uses simplified logic so most of the fuzzer
// effort goes into the decompressor code paths.

#![no_main]
use libfuzzer_sys::fuzz_target;
use rawler::rawsource::RawSource;

fuzz_target!(|data: &[u8]| {
    let source = RawSource::new_from_slice(data);
    let _ = rawler::decode_unwrapped(&source);
});
