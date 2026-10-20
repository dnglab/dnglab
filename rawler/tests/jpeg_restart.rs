use rawler::decompressors::ljpeg::LjpegDecompressor;
use rawler::pumps::{BitPump, BitPumpJPEG};

fn segment(jpeg: &mut Vec<u8>, marker: u8, payload: &[u8]) {
  jpeg.extend_from_slice(&[0xff, marker]);
  jpeg.extend_from_slice(&u16::try_from(payload.len() + 2).unwrap().to_be_bytes());
  jpeg.extend_from_slice(payload);
}

fn jpeg_with_sampling(width: u16, height: u16, sampling: &[u8], predictor: u8, dri: Option<u16>, entropy: &[u8]) -> Vec<u8> {
  let mut jpeg = vec![0xff, 0xd8];
  let mut sof = vec![12];
  sof.extend_from_slice(&height.to_be_bytes());
  sof.extend_from_slice(&width.to_be_bytes());
  sof.push(sampling.len() as u8);
  for (i, sampling) in sampling.iter().enumerate() {
    sof.extend_from_slice(&[(i + 1) as u8, *sampling, 0]);
  }
  segment(&mut jpeg, 0xc3, &sof);

  // Difference category 0 is code 0, category 1 is code 10. Leave the
  // all-ones code unused, as JPEG requires, and write exactly 16 counts.
  let mut dht = vec![0];
  let mut counts = [0; 16];
  counts[0] = 1;
  counts[1] = 1;
  dht.extend_from_slice(&counts);
  dht.extend_from_slice(&[0, 1]);
  segment(&mut jpeg, 0xc4, &dht);
  if let Some(interval) = dri {
    segment(&mut jpeg, 0xdd, &interval.to_be_bytes());
  }

  let mut sos = vec![sampling.len() as u8];
  for i in 0..sampling.len() {
    sos.extend_from_slice(&[(i + 1) as u8, 0]);
  }
  sos.extend_from_slice(&[predictor, 0, 0]);
  segment(&mut jpeg, 0xda, &sos);
  jpeg.extend_from_slice(entropy);
  jpeg.extend_from_slice(&[0xff, 0xd9]);
  jpeg
}

fn jpeg(width: u16, height: u16, predictor: u8, dri: Option<u16>, entropy: &[u8]) -> Vec<u8> {
  jpeg_with_sampling(width, height, &[0x11], predictor, dri, entropy)
}

// Encode only explicit differences, not predictions. Expected pixel values
// below are independent, fixed examples rather than a second decoder.
fn small_differences(differences: &[i8]) -> Vec<u8> {
  let bits: String = differences
    .iter()
    .map(|difference| match difference {
      -1 => "100",
      0 => "0",
      1 => "101",
      _ => panic!("test fixture supports only -1, 0 and +1"),
    })
    .collect();
  let mut bytes = Vec::new();
  for chunk in bits.as_bytes().chunks(8) {
    let mut byte = 0;
    for i in 0..8 {
      byte = (byte << 1) | u8::from(chunk.get(i).copied().unwrap_or(b'1') == b'1');
    }
    bytes.push(byte);
    if byte == 0xff {
      bytes.push(0); // JPEG byte stuffing
    }
  }
  bytes
}

fn restart_entropy(intervals: &[&[i8]]) -> Vec<u8> {
  let mut entropy = Vec::new();
  for (i, differences) in intervals.iter().enumerate() {
    if i != 0 {
      entropy.extend_from_slice(&[0xff, 0xd0 + ((i - 1) % 8) as u8]);
    }
    entropy.extend_from_slice(&small_differences(differences));
  }
  entropy
}

fn decode(jpeg: &[u8]) -> Result<Vec<u16>, String> {
  let _ = env_logger::builder().is_test(true).try_init();
  let decompressor = LjpegDecompressor::new(jpeg)?;
  let width = decompressor.width();
  let height = decompressor.height();
  let mut output = vec![0; width * height];
  decompressor.decode(&mut output, 0, width, width, height, false)?;
  Ok(output)
}

#[test]
fn restart_predictor_two_matches_review_example() {
  let entropy = restart_entropy(&[&[1, 0], &[0, 1]]);
  assert_eq!(decode(&jpeg(2, 2, 2, Some(2), &entropy)).unwrap(), [2049, 2049, 2048, 2049]);
}

#[test]
fn restart_first_row_uses_left_neighbour_for_every_predictor() {
  for predictor in 1..=7 {
    // Together these examples expose the stale preceding-row dependency for
    // every predictor 2-7, including the rounding in predictors 5-7.
    for (differences, first_row) in [([1, 1], [2049, 2050]), ([1, -1], [2049, 2048])] {
      let entropy = restart_entropy(&[&differences, &[0, 1]]);
      assert_eq!(decode(&jpeg(2, 2, predictor, Some(2), &entropy)).unwrap(), [first_row[0], first_row[1], 2048, 2049], "predictor {predictor}");
    }
  }
}

// Two 3x2 intervals with fixed differences. The first row of each interval
// must reset, but the second must still use the requested predictor.
fn multirow_case(predictor: u8) -> (Vec<u8>, Vec<u16>) {
  let first_second_rows = [[2050, 2050, 2051], [2050, 2050, 2052], [2050, 2049, 2051], [2050, 2051, 2053], [2050, 2050, 2051], [2050, 2050, 2052], [2050, 2050, 2051]];
  let last_rows = [[2047, 2048, 2047], [2047, 2047, 2045], [2047, 2048, 2045], [2047, 2047, 2046], [2047, 2047, 2046], [2047, 2047, 2045], [2047, 2047, 2045]];
  let entropy = restart_entropy(&[&[1, 1, 1, 1, 0, 1], &[-1, -1, 0, 0, 1, -1]]);
  let expected = [[2049, 2050, 2051], first_second_rows[(predictor - 1) as usize], [2047, 2046, 2046], last_rows[(predictor - 1) as usize]].concat();
  (jpeg(3, 4, predictor, Some(6), &entropy), expected)
}

#[test]
fn restores_selected_predictor_after_each_restart_first_row() {
  for predictor in 1..=7 {
    let (jpeg, expected) = multirow_case(predictor);
    assert_eq!(decode(&jpeg).unwrap(), expected, "predictor {predictor}");
  }
}

#[test]
fn cropped_rows_preserve_restart_boundaries_and_output_padding() {
  for predictor in 1..=7 {
    let (jpeg, expected) = multirow_case(predictor);
    let decompressor = LjpegDecompressor::new(&jpeg).unwrap();
    for width in [1, 2] {
      let mut output = vec![u16::MAX; 6 * 4];
      decompressor.decode(&mut output, 2, 6, width, 4, false).unwrap();
      for row in 0..4 {
        assert_eq!(&output[row * 6 + 2..row * 6 + 2 + width], &expected[row * 3..row * 3 + width], "predictor {predictor}, width {width}");
        assert!(output[row * 6..row * 6 + 2].iter().all(|pixel| *pixel == u16::MAX));
        assert!(output[row * 6 + 2 + width..(row + 1) * 6].iter().all(|pixel| *pixel == u16::MAX));
      }
    }
  }
}

#[test]
fn restart_counts_pixels_not_components_and_handles_one_pixel_rows() {
  let entropy = restart_entropy(&[&[1, 0, -1], &[0, 1, 0]]);
  for predictor in 1..=7 {
    let jpeg = jpeg_with_sampling(1, 2, &[0x11; 3], predictor, Some(1), &entropy);
    assert_eq!(decode(&jpeg).unwrap(), [2049, 2048, 2047, 2048, 2049, 2048]);
  }
}

#[test]
fn restart_sequence_wraps_from_rst7_to_rst0() {
  let intervals: Vec<&[i8]> = vec![&[0, 0]; 10];
  let entropy = restart_entropy(&intervals);
  assert_eq!(decode(&jpeg(2, 10, 7, Some(2), &entropy)).unwrap(), vec![2048; 20]);
}

#[test]
fn partial_height_does_not_require_the_next_restart_marker() {
  let entropy = small_differences(&[1, 1, 1, 1, 0, 1]);
  let jpeg = jpeg(3, 4, 2, Some(6), &entropy);
  let decompressor = LjpegDecompressor::new(&jpeg).unwrap();
  let mut output = [0; 6];
  decompressor.decode(&mut output, 0, 3, 3, 2, false).unwrap();
  assert_eq!(output, [2049, 2050, 2051, 2050, 2050, 2052]);
}

#[test]
fn rejects_restart_intervals_that_end_mid_row() {
  for interval in [1, 3] {
    let error = decode(&jpeg(2, 2, 1, Some(interval), &[])).unwrap_err();
    assert!(error.contains("must be a multiple of image width 2"));
  }
}

#[test]
fn rejects_zero_width_before_restart_arithmetic() {
  assert!(decode(&jpeg(0, 2, 1, Some(2), &[])).is_err());
}

#[test]
fn rejects_restart_markers_for_unimplemented_sampling_modes() {
  for sampling in [[0x21, 0x11, 0x11], [0x22, 0x11, 0x11], [0x11, 0x21, 0x11]] {
    let jpeg = jpeg_with_sampling(2, 2, &sampling, 1, Some(2), &[]);
    let decompressor = LjpegDecompressor::new(&jpeg).unwrap();
    let mut output = [0; 12];
    for sony in [false, true] {
      let result = if sony { decompressor.decode_sony(&mut output, 0, 6, 6, 2, false) } else { decompressor.decode(&mut output, 0, 6, 6, 2, false) };
      assert!(result.unwrap_err().contains("restart markers are not supported for component"));
    }
  }
}

#[test]
fn rejects_restart_markers_with_predictor_eight() {
  assert!(decode(&jpeg(2, 2, 8, Some(2), &[])).unwrap_err().contains("predictor 8"));
}

#[test]
fn rejects_truncated_restart_interval_segment_without_panicking() {
  let error = LjpegDecompressor::new(&[0xff, 0xd8, 0xff, 0xdd, 0, 4, 0]).unwrap_err();
  assert!(error.contains("truncated DRI segment"));
}

#[test]
fn rejects_invalid_restart_interval_segment_length() {
  for length in [2, 5] {
    let error = LjpegDecompressor::new(&[0xff, 0xd8, 0xff, 0xdd, 0, length, 0, 2]).unwrap_err();
    assert!(error.contains("invalid DRI length"));
  }
}

#[test]
fn rejects_truncated_entropy_before_restart_marker() {
  let error = decode(&jpeg(2, 2, 1, Some(2), &[0xff, 0xd0, 0x5f])).unwrap_err();
  assert!(error.contains("Truncated JPEG entropy data before RST0"));
}

#[test]
fn rejects_wrong_restart_marker_in_decoded_image() {
  let error = decode(&jpeg(2, 2, 1, Some(2), &[0xaf, 0xff, 0xd1, 0x5f])).unwrap_err();
  assert!(error.contains("expected RST0"));
}

#[test]
fn rejects_missing_restart_marker_between_rows() {
  let error = decode(&jpeg(2, 2, 1, Some(2), &[0xaf, 0x5f])).unwrap_err();
  assert!(error.contains("RST0"));
}

#[test]
fn accepts_one_zero_and_mixed_padding_before_restart() {
  for entropy in [0xaf, 0xa0, 0xa5] {
    let jpeg = jpeg(2, 2, 2, Some(2), &[entropy, 0xff, 0xd0, 0x5f]);
    assert_eq!(decode(&jpeg).unwrap(), [2049, 2049, 2048, 2049]);
  }
}

#[test]
fn discards_buffered_trailing_bits_before_restart() {
  let jpeg = jpeg(2, 2, 2, Some(2), &[0xaf, 0xaa, 0xff, 0xd0, 0x5f]);
  assert_eq!(decode(&jpeg).unwrap(), [2049, 2049, 2048, 2049]);
}

#[test]
fn final_padding_never_discards_decoded_pixels() {
  for entropy in [&[0xaf][..], &[0xa0], &[0xa5], &[0xa5, 0xaa]] {
    assert_eq!(decode(&jpeg(2, 1, 1, None, entropy)).unwrap(), [2049, 2049]);
  }
  for entropy in [&[0x5f][..], &[0x50], &[0x55], &[0x55, 0xaa]] {
    let mut restarted = vec![0xaf, 0xff, 0xd0];
    restarted.extend_from_slice(entropy);
    assert_eq!(decode(&jpeg(2, 2, 2, Some(2), &restarted)).unwrap(), [2049, 2049, 2048, 2049]);
  }
}

#[test]
fn preserves_best_effort_decoding_of_truncated_final_entropy() {
  assert_eq!(decode(&jpeg(2, 2, 1, None, &[])).unwrap(), [2048; 4]);
  assert_eq!(decode(&jpeg(2, 2, 2, Some(2), &[0xaf, 0xff, 0xd0])).unwrap(), [2049, 2049, 2048, 2048]);
}

#[test]
fn undeclared_restart_markers_do_not_discard_recovered_pixels() {
  for dri in [None, Some(0)] {
    assert_eq!(decode(&jpeg(2, 2, 1, dri, &[0xaf, 0xff, 0xd0, 0x5f])).unwrap(), [2049; 4]);
  }
}

#[test]
fn trailing_ff_does_not_panic_or_discard_pixels() {
  let mut jpeg = jpeg(2, 1, 1, None, &[0xaf]);
  jpeg.pop(); // Keep a lone 0xff instead of EOI.
  assert_eq!(decode(&jpeg).unwrap(), [2049, 2049]);
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
  let data = [0xff, 0, 0xaa, 0xff, 0xd0, 0xbb];
  let mut pump = BitPumpJPEG::new(&data);
  assert_eq!(pump.get_bits(16), 0xffaa);
  pump.consume_restart_marker(0).unwrap();
  assert_eq!(pump.get_bits(8), 0xbb);
}

#[test]
fn jpeg_bit_pump_allows_lookahead_without_consuming_synthetic_bits() {
  let data = [0xa5, 0xff, 0xd0, 0x34];
  let mut pump = BitPumpJPEG::new(&data);
  assert_eq!(pump.get_bits(4), 0xa);
  pump.peek_bits(16); // Huffman lookup may peek past the marker.
  pump.consume_restart_marker(0).unwrap();
  assert_eq!(pump.get_bits(8), 0x34);
}

#[test]
fn jpeg_bit_pump_rejects_consumed_synthetic_bits_before_restart() {
  let data = [0xa5, 0xff, 0xd0, 0x34];
  let mut pump = BitPumpJPEG::new(&data);
  pump.get_bits(16);
  assert!(pump.consume_restart_marker(0).unwrap_err().contains("Truncated JPEG entropy data before RST0"));
}

#[test]
fn jpeg_bit_pump_rejects_invalid_or_truncated_restart_marker() {
  let data = [0x12, 0xff];
  let mut pump = BitPumpJPEG::new(&data);
  pump.get_bits(8);
  assert!(pump.consume_restart_marker(8).unwrap_err().contains("Invalid JPEG restart marker index"));
  assert!(pump.consume_restart_marker(0).unwrap_err().contains("Truncated JPEG restart marker"));
}

#[test]
fn end_of_scan_diagnostics_accept_standard_and_legacy_padding() {
  for data in [[0xaf, 0xff, 0xd9], [0xa0, 0xff, 0xd9]] {
    let mut pump = BitPumpJPEG::new(&data);
    pump.get_bits(4);
    pump.validate_end_of_scan().unwrap();
  }
}

#[test]
fn end_of_scan_diagnostics_report_both_short_and_long_trailing_data() {
  for (data, message) in [
    (&[0xa5, 0xff, 0xd9][..], "Invalid JPEG entropy padding"),
    (&[0xa5, 0xaa, 0xff, 0xd9][..], "Unexpected trailing JPEG entropy data"),
    (&[0xa5, 0xaa][..], "Unexpected trailing JPEG entropy data"),
  ] {
    let mut pump = BitPumpJPEG::new(data);
    pump.get_bits(4);
    assert!(pump.validate_end_of_scan().unwrap_err().contains(message));
  }
}

#[test]
fn end_of_scan_diagnostics_report_truncation() {
  let mut pump = BitPumpJPEG::new(&[0xff, 0xd9]);
  pump.get_bits(1);
  assert!(pump.validate_end_of_scan().unwrap_err().contains("Truncated JPEG entropy data"));
}

#[test]
fn subsampled_decoders_keep_pixels_with_irregular_scan_endings() {
  for (sampling, height, valid_entropy) in [(0x22, 2, 0x03), (0x21, 1, 0x0f)] {
    for entropy in [&[valid_entropy][..], &[], &[valid_entropy, 0xff, 0xd0]] {
      let jpeg = jpeg_with_sampling(2, height, &[sampling, 0x11, 0x11], 1, None, entropy);
      assert_eq!(decode(&jpeg).unwrap(), vec![2048; 6 * height as usize]);
      let decompressor = LjpegDecompressor::new(&jpeg).unwrap();
      let mut output = vec![0; 6 * height as usize];
      decompressor.decode_sony(&mut output, 0, 6, 6, height as usize, false).unwrap();
      assert_eq!(output, vec![2048; 6 * height as usize]);
    }
  }
}
