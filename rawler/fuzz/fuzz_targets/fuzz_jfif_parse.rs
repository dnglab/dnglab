// Fuzz target: JFIF/EXIF segment parsing
//
// Tests JPEG segment marker parsing, EXIF IFD extraction, and segment
// boundary validation. Exercises the 0xFFxx marker state machine.

#![no_main]
use libfuzzer_sys::fuzz_target;
use rawler::formats::jfif::Jfif;
use std::io::Cursor;

fuzz_target!(|data: &[u8]| {
    let mut cursor = Cursor::new(data);
    let _ = Jfif::parse(&mut cursor);
});
