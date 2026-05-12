// Fuzz target: BMFF/CR3 container parsing
//
// Tests the ISO BMFF (Base Media File Format) box parser used for CR3 and
// similar container formats. Exercises box header parsing, size validation,
// and nested box traversal.

#![no_main]
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = rawler::formats::bmff::parse_buffer(data);
});
