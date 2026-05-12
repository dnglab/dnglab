// Fuzz target: TIFF IFD chain parsing
//
// Tests the TIFF structure parser — IFD chain traversal, entry extraction,
// offset validation, and endian handling. Covers both big-endian and
// little-endian TIFF variants.

#![no_main]
use libfuzzer_sys::fuzz_target;
use rawler::formats::tiff::reader::GenericTiffReader;

fuzz_target!(|data: &[u8]| {
    let _ = GenericTiffReader::new_with_buffer(data, 0, 0, Some(10));
});
