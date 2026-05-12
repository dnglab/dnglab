// Fuzz target: CIFF (Canon Image File Format) parsing
//
// Tests the CIFF container parser used for older Canon CRW files.
// Exercises recursive IFD parsing, heap traversal, and entry extraction.

#![no_main]
use libfuzzer_sys::fuzz_target;
use rawler::formats::ciff::CiffIFD;

fuzz_target!(|data: &[u8]| {
    let _ = CiffIFD::new(data, 0, data.len(), 0);
});
