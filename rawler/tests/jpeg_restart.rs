use rawler::decompressors::ljpeg::LjpegDecompressor;
use rawler::pumps::{BitPump, BitPumpJPEG};

fn restart_marked_jpeg(dri: Option<u16>) -> Vec<u8> {
  let mut jpeg = vec![
    0xff, 0xd8, // SOI
    0xff, 0xc3, 0x00, 0x0b, 0x0c, 0x00, 0x01, 0x00, 0x04, 0x01, 0x01, 0x11, 0x00, // SOF3
    0xff, 0xc4, 0x00, 0x15, 0x00, 0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, // DHT: category 0 = code 0, category 1 = code 1
  ];
  if let Some(interval) = dri {
    jpeg.extend_from_slice(&[0xff, 0xdd, 0x00, 0x04]);
    jpeg.extend_from_slice(&interval.to_be_bytes());
  }
  jpeg.extend_from_slice(&[
    0xff, 0xda, 0x00, 0x08, 0x01, 0x01, 0x00, 0x01, 0x00, 0x00, // SOS
    0xdf, // +1, then 0, followed by entropy padding
    0xff, 0xd0, // RST0
    0x3f, // 0, then 0, followed by entropy padding
    0xff, 0xd9, // EOI
  ]);
  jpeg
}

fn subsampled_jpeg(vertical_sampling: u8, entropy: &[u8]) -> Vec<u8> {
  let height = if vertical_sampling == 2 { 2 } else { 1 };
  let mut jpeg = vec![
    0xff,
    0xd8, // SOI
    0xff,
    0xc3,
    0x00,
    0x11,
    0x0c,
    0x00,
    height,
    0x00,
    0x02,
    0x03, // SOF3: 2 pixels, 3 components
    0x01,
    0x20 | vertical_sampling,
    0x00, // Y: 2x1 or 2x2 sampling
    0x02,
    0x11,
    0x00, // Cb: 1x1 sampling
    0x03,
    0x11,
    0x00, // Cr: 1x1 sampling
    0xff,
    0xc4,
    0x00,
    0x14,
    0x00,
    0x01,
    0x00,
    0x00,
    0x00,
    0x00,
    0x00,
    0x00,
    0x00,
    0x00,
    0x00,
    0x00,
    0x00,
    0x00,
    0x00,
    0x00,
    0x00,
    0x00, // DHT: category 0 = code 0
    0xff,
    0xda,
    0x00,
    0x0c,
    0x03,
    0x01,
    0x00,
    0x02,
    0x00,
    0x03,
    0x00,
    0x01,
    0x00,
    0x00, // SOS: predictor 1
  ];
  jpeg.extend_from_slice(entropy);
  jpeg.extend_from_slice(&[0xff, 0xd9]); // EOI
  jpeg
}

fn decode_subsampled(jpeg: &[u8], vertical_sampling: u8, sony: bool) -> Result<Vec<u16>, String> {
  let width = 6;
  let height = if vertical_sampling == 2 { 2 } else { 1 };
  let decompressor = LjpegDecompressor::new(jpeg)?;
  let mut output = vec![0_u16; width * height];
  if sony {
    decompressor.decode_sony(&mut output, 0, width, width, height, false)?;
  } else {
    decompressor.decode(&mut output, 0, width, width, height, false)?;
  }
  Ok(output)
}

#[test]
fn lossless_jpeg_resets_prediction_after_restart() {
  // Four 12-bit samples, one component, predictor 1, restart interval 2.
  // The first interval decodes to [2049, 2049]. After RST0, a zero
  // difference must use the initial predictor again and produce 2048.
  let jpeg = [
    0xff, 0xd8, // SOI
    0xff, 0xc3, 0x00, 0x0b, 0x0c, 0x00, 0x01, 0x00, 0x04, 0x01, 0x01, 0x11, 0x00, // SOF3
    0xff, 0xc4, 0x00, 0x15, 0x00, 0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, // DHT: category 0 = code 0, category 1 = code 1
    0xff, 0xdd, 0x00, 0x04, 0x00, 0x02, // DRI: two MCUs
    0xff, 0xda, 0x00, 0x08, 0x01, 0x01, 0x00, 0x01, 0x00, 0x00, // SOS
    0xdf, // +1, then 0, followed by entropy padding
    0xff, 0xd0, // RST0
    0x3f, // 0, then 0, followed by entropy padding
    0xff, 0xd9, // EOI
  ];

  let decompressor = LjpegDecompressor::new(&jpeg).unwrap();
  let mut output = [0_u16; 4];
  decompressor.decode(&mut output, 0, 4, 4, 1, false).unwrap();

  assert_eq!(output, [2049, 2049, 2048, 2048]);
}

#[test]
fn rejects_restart_markers_for_unimplemented_sampling_modes() {
  let jpeg = [
    0xff, 0xd8, // SOI
    0xff, 0xc3, 0x00, 0x0b, 0x0c, 0x00, 0x01, 0x00, 0x04, 0x01, 0x01, 0x21, 0x00, // SOF3 with 2x1 sampling
    0xff, 0xc4, 0x00, 0x15, 0x00, 0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, // DHT
    0xff, 0xdd, 0x00, 0x04, 0x00, 0x02, // DRI
    0xff, 0xda, 0x00, 0x08, 0x01, 0x01, 0x00, 0x01, 0x00, 0x00, // SOS
    0xdf, 0xff, 0xd0, 0x3f, 0xff, 0xd9,
  ];

  let decompressor = LjpegDecompressor::new(&jpeg).unwrap();
  let mut output = [0_u16; 8];
  let error = decompressor.decode(&mut output, 0, 8, 8, 1, false).unwrap_err();

  assert!(error.contains("restart markers are not supported for component 1 sampling 2x1"));
}

#[test]
fn rejects_restart_markers_when_a_later_component_is_subsampled() {
  let jpeg = [
    0xff, 0xd8, // SOI
    0xff, 0xc3, 0x00, 0x0e, 0x0c, 0x00, 0x01, 0x00, 0x02, 0x02, // SOF3 header
    0x01, 0x11, 0x00, // component 1: 1x1 sampling
    0x02, 0x21, 0x00, // component 2: 2x1 sampling
    0xff, 0xc4, 0x00, 0x15, 0x00, 0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, // DHT
    0xff, 0xdd, 0x00, 0x04, 0x00, 0x01, // DRI: one MCU
    0xff, 0xda, 0x00, 0x0a, 0x02, 0x01, 0x00, 0x02, 0x00, 0x01, 0x00, 0x00, // SOS
    0xff, 0xd9, // EOI
  ];

  let decompressor = LjpegDecompressor::new(&jpeg).unwrap();
  let mut output = [0_u16; 4];
  let error = decompressor.decode(&mut output, 0, 4, 4, 1, false).unwrap_err();

  assert!(error.contains("restart markers are not supported for component 2 sampling 2x1"));
}

#[test]
fn jpeg_bit_pump_resumes_after_restart_markers() {
  let data = [0x12, 0xff, 0xd0, 0x34, 0xff, 0xff, 0xd1, 0x56];
  let mut pump = BitPumpJPEG::new(&data);

  assert_eq!(pump.get_bits(8), 0x12);
  pump.consume_restart_marker(0).unwrap();
  assert_eq!(pump.get_bits(8), 0x34);
  pump.consume_restart_marker(1).unwrap();
  assert_eq!(pump.get_bits(8), 0x56);
}

#[test]
fn jpeg_bit_pump_preserves_stuffed_ff_before_restart() {
  let data = [0xff, 0x00, 0xaa, 0xff, 0xd0, 0xbb];
  let mut pump = BitPumpJPEG::new(&data);

  assert_eq!(pump.get_bits(16), 0xffaa);
  pump.consume_restart_marker(0).unwrap();
  assert_eq!(pump.get_bits(8), 0xbb);
}

#[test]
fn jpeg_bit_pump_rejects_wrong_restart_sequence() {
  let data = [0x12, 0xff, 0xd1, 0x34];
  let mut pump = BitPumpJPEG::new(&data);

  assert_eq!(pump.get_bits(8), 0x12);
  let error = pump.consume_restart_marker(0).unwrap_err();
  assert!(error.contains("expected RST0"));
}

#[test]
fn jpeg_bit_pump_accepts_legacy_zero_padding_at_end_of_scan() {
  let data = [0b1000_0000];
  let mut pump = BitPumpJPEG::new(&data);

  assert_eq!(pump.get_bits(1), 1);
  pump.validate_end_of_scan().unwrap();
}

#[test]
fn jpeg_bit_pump_accepts_legacy_trailing_entropy_before_eoi() {
  let data = [0b1010_0000, 0xaa, 0xff, 0xd9];
  let mut pump = BitPumpJPEG::new(&data);

  assert_eq!(pump.get_bits(1), 1);
  pump.validate_end_of_scan().unwrap();
}

#[test]
fn jpeg_bit_pump_still_validates_final_padding_without_trailing_bytes() {
  let data = [0b1010_0000, 0xff, 0xd9];
  let mut pump = BitPumpJPEG::new(&data);

  assert_eq!(pump.get_bits(1), 1);
  let error = pump.validate_end_of_scan().unwrap_err();
  assert!(error.contains("Invalid JPEG entropy padding at end of scan"));
}

#[test]
fn jpeg_bit_pump_rejects_trailing_entropy_without_eoi() {
  let data = [0b1010_0000, 0xaa];
  let mut pump = BitPumpJPEG::new(&data);

  assert_eq!(pump.get_bits(1), 1);
  let error = pump.validate_end_of_scan().unwrap_err();
  assert!(error.contains("Unexpected trailing JPEG entropy data at end of scan"));
}

#[test]
fn rejects_truncated_restart_interval_segment_without_panicking() {
  let jpeg = [
    0xff, 0xd8, // SOI
    0xff, 0xdd, 0x00, 0x04, 0x00, // DRI with one interval byte missing
  ];

  let error = LjpegDecompressor::new(&jpeg).unwrap_err();
  assert!(error.contains("truncated DRI segment"));
}

#[test]
fn rejects_truncated_entropy_before_restart_marker() {
  // The first entropy byte is missing entirely. The decoder must not use
  // synthetic zeroes to produce two plausible samples before RST0.
  let jpeg = [
    0xff, 0xd8, // SOI
    0xff, 0xc3, 0x00, 0x0b, 0x0c, 0x00, 0x01, 0x00, 0x04, 0x01, 0x01, 0x11, 0x00, // SOF3
    0xff, 0xc4, 0x00, 0x15, 0x00, 0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, // DHT
    0xff, 0xdd, 0x00, 0x04, 0x00, 0x02, // DRI: two MCUs
    0xff, 0xda, 0x00, 0x08, 0x01, 0x01, 0x00, 0x01, 0x00, 0x00, // SOS
    0xff, 0xd0, // RST0 without preceding entropy data
    0x3f, // second interval
    0xff, 0xd9, // EOI
  ];

  let decompressor = LjpegDecompressor::new(&jpeg).unwrap();
  let mut output = [0_u16; 4];
  let error = decompressor.decode(&mut output, 0, 4, 4, 1, false).unwrap_err();

  assert!(error.contains("Truncated JPEG entropy data before RST0"));
}

#[test]
fn rejects_invalid_entropy_padding_before_restart_marker() {
  let jpeg = [
    0xff, 0xd8, // SOI
    0xff, 0xc3, 0x00, 0x0b, 0x0c, 0x00, 0x01, 0x00, 0x04, 0x01, 0x01, 0x11, 0x00, // SOF3
    0xff, 0xc4, 0x00, 0x15, 0x00, 0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, // DHT
    0xff, 0xdd, 0x00, 0x04, 0x00, 0x02, // DRI: two MCUs
    0xff, 0xda, 0x00, 0x08, 0x01, 0x01, 0x00, 0x01, 0x00, 0x00, // SOS
    0xd7, // +1, then 0, followed by a zero among the padding bits
    0xff, 0xd0, // RST0
    0x3f, // second interval
    0xff, 0xd9, // EOI
  ];

  let decompressor = LjpegDecompressor::new(&jpeg).unwrap();
  let mut output = [0_u16; 4];
  let error = decompressor.decode(&mut output, 0, 4, 4, 1, false).unwrap_err();

  assert!(error.contains("Invalid JPEG entropy padding before RST0"));
}

#[test]
fn rejects_unexpected_trailing_entropy_before_restart_marker() {
  let jpeg = [
    0xff, 0xd8, // SOI
    0xff, 0xc3, 0x00, 0x0b, 0x0c, 0x00, 0x01, 0x00, 0x04, 0x01, 0x01, 0x11, 0x00, // SOF3
    0xff, 0xc4, 0x00, 0x15, 0x00, 0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, // DHT
    0xff, 0xdd, 0x00, 0x04, 0x00, 0x02, // DRI: two MCUs
    0xff, 0xda, 0x00, 0x08, 0x01, 0x01, 0x00, 0x01, 0x00, 0x00, // SOS
    0xdf, 0xaa, // valid first interval followed by an extra entropy byte
    0xff, 0xd0, // RST0
    0x3f, // second interval
    0xff, 0xd9, // EOI
  ];

  let decompressor = LjpegDecompressor::new(&jpeg).unwrap();
  let mut output = [0_u16; 4];
  let error = decompressor.decode(&mut output, 0, 4, 4, 1, false).unwrap_err();

  assert!(error.contains("Unexpected trailing JPEG entropy data before RST0"));
}

#[test]
fn rejects_truncated_final_entropy_segment() {
  let jpeg = [
    0xff, 0xd8, // SOI
    0xff, 0xc3, 0x00, 0x0b, 0x0c, 0x00, 0x01, 0x00, 0x04, 0x01, 0x01, 0x11, 0x00, // SOF3
    0xff, 0xc4, 0x00, 0x15, 0x00, 0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, // DHT
    0xff, 0xdd, 0x00, 0x04, 0x00, 0x02, // DRI: two MCUs
    0xff, 0xda, 0x00, 0x08, 0x01, 0x01, 0x00, 0x01, 0x00, 0x00, // SOS
    0xdf, // first interval
    0xff, 0xd0, // RST0
    0xff, 0xd9, // EOI without second-interval entropy data
  ];

  let decompressor = LjpegDecompressor::new(&jpeg).unwrap();
  let mut output = [0_u16; 4];
  let error = decompressor.decode(&mut output, 0, 4, 4, 1, false).unwrap_err();

  assert!(error.contains("Truncated JPEG entropy data at end of scan"));
}

#[test]
fn rejects_restart_marker_without_dri() {
  let jpeg = restart_marked_jpeg(None);
  let decompressor = LjpegDecompressor::new(&jpeg).unwrap();
  let mut output = [0_u16; 4];

  let error = decompressor.decode(&mut output, 0, 4, 4, 1, false).unwrap_err();

  assert!(error.contains("Unexpected JPEG marker 0xd0 at end of scan"));
}

#[test]
fn rejects_restart_marker_with_zero_dri() {
  let jpeg = restart_marked_jpeg(Some(0));
  let decompressor = LjpegDecompressor::new(&jpeg).unwrap();
  let mut output = [0_u16; 4];

  let error = decompressor.decode(&mut output, 0, 4, 4, 1, false).unwrap_err();

  assert!(error.contains("Unexpected JPEG marker 0xd0 at end of scan"));
}

#[test]
fn subsampled_420_rejects_truncated_final_entropy() {
  let valid = subsampled_jpeg(2, &[0x03]); // six zero differences plus one-padding
  assert_eq!(decode_subsampled(&valid, 2, false).unwrap(), vec![2048; 12]);
  assert_eq!(decode_subsampled(&valid, 2, true).unwrap(), vec![2048; 12]);

  let truncated = subsampled_jpeg(2, &[]);
  for sony in [false, true] {
    let error = decode_subsampled(&truncated, 2, sony).unwrap_err();
    assert!(error.contains("Truncated JPEG entropy data at end of scan"));
  }
}

#[test]
fn subsampled_422_rejects_truncated_final_entropy() {
  let valid = subsampled_jpeg(1, &[0x0f]); // four zero differences plus one-padding
  assert_eq!(decode_subsampled(&valid, 1, false).unwrap(), vec![2048; 6]);

  let truncated = subsampled_jpeg(1, &[]);
  let error = decode_subsampled(&truncated, 1, false).unwrap_err();
  assert!(error.contains("Truncated JPEG entropy data at end of scan"));
}

#[test]
fn subsampled_decoders_reject_restart_markers_without_dri() {
  for (vertical_sampling, entropy) in [(2, 0x03), (1, 0x0f)] {
    let jpeg = subsampled_jpeg(vertical_sampling, &[entropy, 0xff, 0xd0]);
    let error = decode_subsampled(&jpeg, vertical_sampling, false).unwrap_err();
    assert!(error.contains("Unexpected JPEG marker 0xd0 at end of scan"));
  }
}
