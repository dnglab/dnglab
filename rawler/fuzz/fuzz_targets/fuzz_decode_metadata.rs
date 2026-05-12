// Fuzz target: format parsing and metadata extraction (dummy decoders)
//
// Tests TIFF/BMFF/CIFF format parsing, IFD chain traversal, makernote parsing,
// and metadata extraction — but uses dummy decoders so actual decompression
// is skipped. Good for catching bugs in format parsers.

#![no_main]
use libfuzzer_sys::fuzz_target;
use rawler::rawsource::RawSource;

fuzz_target!(|data: &[u8]| {
    let source = RawSource::new_from_slice(data);
    let _ = rawler::decode_dummy(&source);
});
