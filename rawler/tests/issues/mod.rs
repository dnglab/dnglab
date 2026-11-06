use std::io::Cursor;

use crate::common::check_md5_equal;
use rawler::RawImage;
use rawler::dng::DngCompression;
use rawler::dng::convert::ConvertParams;
use rawler::dng::convert::convert_raw_file;
use rawler::dng::original::extract_original;
use rawler::formats::jfif::Jfif;
use rawler::formats::tiff::GenericTiffReader;
use rawler::formats::tiff::SRational;
use rawler::formats::tiff::Value;
use rawler::formats::tiff::reader::TiffReader;
use rawler::imgop::chromatic_adaption::bradford_adaption_matrix;
use rawler::imgop::color::OutputSpace;
use rawler::imgop::develop::RawDevelop;
use rawler::imgop::icc::MatrixShaperProfile;
use rawler::imgop::matrix::{invert, multiply};
use rawler::imgop::xyz::Illuminant;
use rawler::rawdb;
use rawler::rawimage::ColorimetricReference;
use rawler::rawsource::RawSource;
use rawler::tags::{DngTag, ExifTag};
use rawler::{analyze::raw_pixels_digest, decoders::RawDecodeParams};

#[test]
fn dnglab_354_dng_mismatch_tile_dim_vs_ljpeg_sof_dim() -> std::result::Result<(), Box<dyn std::error::Error>> {
  let path = rawdb::sample("dnglab", "dnglab-issue-354", "testfiles/dnglab_354.dng")?;
  let digest = raw_pixels_digest(path, &RawDecodeParams::default())?;
  check_md5_equal(digest, "44e5e63b152719c0ff9eae9e25cdc275");
  Ok(())
}

#[test]
fn dnglab_366_monochrome_dng_support() -> std::result::Result<(), Box<dyn std::error::Error>> {
  let path = rawdb::sample("dnglab", "dnglab-issue-366", "testfiles/dnglab_366.dng")?;
  let digest = raw_pixels_digest(&path, &RawDecodeParams::default())?;
  check_md5_equal(digest, "f3549fafda97fca90b9993c1278bcd90");
  let mut dng = Cursor::new(Vec::new());
  convert_raw_file(&path, &mut dng, &ConvertParams::default())?;
  Ok(())
}

#[test]
fn dnglab_376_canon_crx_craw_qstep_shl_bug() -> std::result::Result<(), Box<dyn std::error::Error>> {
  {
    let path = rawdb::sample("dnglab", "dnglab-issue-376", "testfiles/Canon_EOS_R6M2_CRAW_ISO_25600.CR3")?;

    let digest = raw_pixels_digest(&path, &RawDecodeParams::default())?;
    check_md5_equal(digest, "66c9fcb6541c90bdfb06d876be5984ec");
    let mut dng = Cursor::new(Vec::new());
    convert_raw_file(&path, &mut dng, &ConvertParams::default())?;
  }
  {
    let path = rawdb::sample("dnglab", "dnglab-issue-376", "testfiles/_MGC9382.CR3")?;
    let digest = raw_pixels_digest(&path, &RawDecodeParams::default())?;
    check_md5_equal(digest, "aef96546a58e5265fb2f7b9e7498cbd0");
    let mut dng = Cursor::new(Vec::new());
    convert_raw_file(&path, &mut dng, &ConvertParams::default())?;
  }
  Ok(())
}

#[test]
fn dnglab_386_catch_jpeg_exif_tiff_ifd_error() -> std::result::Result<(), Box<dyn std::error::Error>> {
  let path = rawdb::sample("dnglab", "dnglab-issue-386", "testfiles/jpeg_ifd_error.jpg")?;
  let rawfile = RawSource::new(&path)?;
  let jfif = Jfif::new(&rawfile)?;
  assert!(jfif.exif_ifd().is_none());
  Ok(())
}

#[test]
fn dnglab_477_jpeg_quantization_table_with_zero_value() -> std::result::Result<(), Box<dyn std::error::Error>> {
  let path = rawdb::sample("dnglab", "dnglab-issue-477", "testfiles/dnglab_477.jpg")?;
  let image = image::open(&path)?;
  let _ = image.to_rgb8();
  Ok(())
}

#[test]
fn dnglab_619_silverfast_scan_missing_illuminant() -> std::result::Result<(), Box<dyn std::error::Error>> {
  let path = rawdb::sample("dnglab", "dnglab-issue-619", "testfiles/silverfast_scan.dng")?;
  let mut dng = Cursor::new(Vec::new());
  convert_raw_file(&path, &mut dng, &ConvertParams::default())?;
  Ok(())
}

#[test]
fn dnglab_807_extract_embedded_raw_invalid_digest() -> std::result::Result<(), Box<dyn std::error::Error>> {
  let path = rawdb::sample("dnglab", "dnglab-issue-807", "testfiles/dnglab-issue-807_embedded_original.dng")?;
  let mut original = Cursor::new(Vec::new());
  let dng = RawSource::new(&path)?;
  let verify_digest = true;
  extract_original(&dng, &mut original, verify_digest)?;
  let plain = original.into_inner();
  let plain_digest = md5::compute(&plain);
  assert_eq!(format!("{:x}", plain_digest), "a968cf0183a84c51ef492c631c6d16b7");
  Ok(())
}

#[test]
fn dnglab_807_digest_calculation() -> std::result::Result<(), Box<dyn std::error::Error>> {
  let path = rawdb::sample("Canon", "EOS 40D", "raw_modes/Canon EOS 40D_ISO_100_RAW.CR2")?;
  let mut dng = Cursor::new(Vec::new());
  convert_raw_file(&path, &mut dng, &ConvertParams::default().with_embedded(true))?;
  // Now verify
  let inner = dng.into_inner();
  let mut original = Cursor::new(Vec::new());
  let dng = RawSource::new_from_slice(&inner);
  let verify_digest = true;
  extract_original(&dng, &mut original, verify_digest)?;
  let plain = original.into_inner();
  let plain_digest = md5::compute(&plain);
  assert_eq!(format!("{:x}", plain_digest), "a968cf0183a84c51ef492c631c6d16b7");
  Ok(())
}

#[test]
fn cr3_highlight_tone_priority_sets_baseline_exposure() -> std::result::Result<(), Box<dyn std::error::Error>> {
  let baseline_exposure = |sample| -> std::result::Result<Option<SRational>, Box<dyn std::error::Error>> {
    let path = rawdb::sample("Canon", "EOS R5 Mark II", sample)?;
    let source = RawSource::new(&path)?;
    let decoder = rawler::get_decoder(&source)?;
    Ok(decoder.raw_image(&source, &RawDecodeParams::default(), true)?.baseline_exposure)
  };
  assert_eq!(baseline_exposure("raw_modes/Canon EOS R5m2_RAW_ISO_640_nocrop.CR3")?, Some(SRational::new(1, 1)));
  assert_eq!(baseline_exposure("raw_modes/Canon EOS R5m2_RAW_ISO_125_crop.CR3")?, None);
  Ok(())
}

/// Samples of the Nikon Scan software: model, file, bits per sample and description of its ICC profile
const COOLSCAN_5000: (&str, &str, usize, &str) = ("Nikon COOLSCAN 5000 ED", "raw_modes/nikon-super-coolscan-5000-ed.nef", 16, "Nikon sRGB 4.0.0.3001");
const COOLSCAN_IV: (&str, &str, usize, &str) = ("Nikon COOLSCAN IV ED", "raw_modes/NEF_from_Nikon_Scan_4.0.3_at_600dpi.nef", 8, "Nikon sRGB 4.0.0.3002");
const COOLSCAN_V: (&str, &str, usize, &str) = ("Nikon COOLSCAN V ED", "raw_modes/Coolscan_V_ED_600px.nef", 16, "Nikon AdobeWide RGB 4.0.0.3001");

fn decode_raw(source: &RawSource) -> std::result::Result<RawImage, Box<dyn std::error::Error>> {
  Ok(rawler::get_decoder(source)?.raw_image(source, &RawDecodeParams::default(), false)?)
}

/// ICC profile in IFD 0 of a TIFF based file
fn root_icc_profile(file: &[u8]) -> std::result::Result<Vec<u8>, Box<dyn std::error::Error>> {
  let tiff = GenericTiffReader::new_with_buffer(file, 0, 0, None)?;
  match &tiff.root_ifd().get_entry(ExifTag::IccProfile).ok_or("no ICC profile")?.value {
    Value::Undefined(data) => Ok(data.clone()),
    value => Err(format!("ICC profile has type {}", value.value_type_name()).into()),
  }
}

/// Mean and largest absolute difference
fn differences(values: impl Iterator<Item = (f32, f32)>) -> (f64, f32) {
  let (mut sum, mut largest, mut count) = (0.0_f64, 0.0_f32, 0_u64);
  for (a, b) in values {
    let difference = (a - b).abs();
    sum += f64::from(difference);
    largest = largest.max(difference);
    count += 1;
  }
  (sum / count.max(1) as f64, largest)
}

#[test]
fn coolscan_scans_are_output_referred() -> std::result::Result<(), Box<dyn std::error::Error>> {
  for (model, file, bits, description) in [COOLSCAN_5000, COOLSCAN_IV, COOLSCAN_V] {
    let source = RawSource::new(&rawdb::sample("Nikon", model, file)?)?;
    let raw = decode_raw(&source)?;
    assert_eq!(raw.colorimetric_reference, ColorimetricReference::OutputReferred, "{}", model);
    let table = raw.linearization_table.as_ref().ok_or("no LinearizationTable")?;
    assert_eq!(table.len(), 1 << bits, "{}", model);
    assert_eq!((table[0], table[table.len() - 1]), (0, u16::MAX), "{}", model);
    // The levels apply to the values of the table
    assert_eq!(raw.whitelevel.0, vec![u16::MAX as u32; 3], "{}", model);
    assert_eq!(raw.wb_coeffs[..3], [1.0; 3], "{}", model);
    assert_eq!(raw.camera_profiles.len(), 1, "{}", model);
    assert_eq!(raw.camera_profiles[0].name.as_deref(), Some(description), "{}", model);
    assert_eq!(raw.camera_profiles[0].calibration_illuminant[0], Some(Illuminant::D50), "{}", model);
    // The thumbnail is encoded in the working space of the scan
    let decoder = rawler::get_decoder(&source)?;
    assert!(decoder.preview_image(&source, &RawDecodeParams::default())?.is_none(), "{}", model);
  }
  Ok(())
}

/// The working space of these scans is sRGB, so the sRGB image has the stored values
#[test]
fn coolscan_srgb_scans_render_stored_values() -> std::result::Result<(), Box<dyn std::error::Error>> {
  for ((model, file, bits, _), mean_limit, largest_limit) in [(COOLSCAN_5000, 3.0, 32.0), (COOLSCAN_IV, 0.3, 1.0)] {
    let raw = decode_raw(&RawSource::new(&rawdb::sample("Nikon", model, file)?)?)?;
    let codes = raw.pixels_u16().to_vec();
    let rendered = RawDevelop::default().develop_image(raw)?;
    let max = ((1_u32 << bits) - 1) as f32;
    let (mean, largest) = differences(codes.iter().zip(&rendered.data).map(|(code, value)| (f32::from(*code), value * max)));
    eprintln!("{}: mean difference {} and largest difference {} of {} codes", model, mean, largest, max);
    assert!(mean <= mean_limit && largest <= largest_limit, "{}: mean {} largest {}", model, mean, largest);
  }
  Ok(())
}

/// The colors of a wide gamut scan are outside of sRGB, but not outside of ProPhoto RGB
#[test]
fn coolscan_wide_gamut_scan_keeps_colors_in_prophoto_rgb() -> std::result::Result<(), Box<dyn std::error::Error>> {
  let (model, file, _, _) = COOLSCAN_V;
  let path = rawdb::sample("Nikon", model, file)?;
  let raw = decode_raw(&RawSource::new(&path)?)?;
  let codes = raw.pixels_u16().to_vec();
  let table = raw.linearization_table.clone().ok_or("no LinearizationTable")?;

  let space = OutputSpace::ProPhotoRgb;
  let develop = RawDevelop { color_space: space, ..Default::default() };
  let rendered = develop.develop_image(raw)?;

  // Map the image back to the linear values of the working space of the scan
  let profile = MatrixShaperProfile::parse(&root_icc_profile(&std::fs::read(&path)?)?)?;
  let space_to_xyz = multiply(&bradford_adaption_matrix(&Illuminant::D65, &Illuminant::D50), &space.rgb_to_xyz());
  let to_scan = multiply(&invert(&profile.rgb_to_xyz).ok_or("colorants are not invertible")?, &space_to_xyz);
  let transfer = space.transfer_function();
  let linear: Vec<f32> = rendered
    .data
    .chunks_exact(3)
    .flat_map(|rgb| {
      let [r, g, b] = [transfer.decode(rgb[0]), transfer.decode(rgb[1]), transfer.decode(rgb[2])];
      to_scan.map(|row| row[0] * r + row[1] * g + row[2] * b)
    })
    .collect();
  let (mean, largest) = differences(codes.iter().zip(&linear).map(|(code, value)| (f32::from(table[*code as usize]) / f32::from(u16::MAX), *value)));
  eprintln!("{}: mean difference {} and largest difference {} of linear values", model, mean, largest);
  assert!(mean <= 1e-4 && largest <= 3e-3, "mean {} largest {}", mean, largest);
  Ok(())
}

/// The DNG has the stored values, and a reader of it renders the same image
#[test]
fn coolscan_dng_keeps_stored_values() -> std::result::Result<(), Box<dyn std::error::Error>> {
  for (model, file, _, _) in [COOLSCAN_5000, COOLSCAN_IV, COOLSCAN_V] {
    let path = rawdb::sample("Nikon", model, file)?;
    let raw = decode_raw(&RawSource::new(&path)?)?;
    let rendered = RawDevelop::default().develop_image(raw.clone())?;
    for compression in [DngCompression::Lossless, DngCompression::Uncompressed] {
      let mut dng = Cursor::new(Vec::new());
      convert_raw_file(&path, &mut dng, &ConvertParams::default().with_embedded(false).with_compression(compression))?;
      let data = dng.into_inner();

      let tiff = GenericTiffReader::new_with_buffer(&data, 0, 0, None)?;
      let root = tiff.root_ifd();
      assert_eq!(root.get_entry(DngTag::ColorimetricReference).map(|entry| entry.value.force_u16(0)), Some(1), "{}", model);
      assert!(root.get_entry(DngTag::AsShotNeutral).is_none(), "{}", model);
      assert!(root.get_entry(DngTag::AsShotWhiteXY).is_some(), "{}", model);

      let decoded = decode_raw(&RawSource::new_from_slice(&data))?;
      assert_eq!(decoded.colorimetric_reference, ColorimetricReference::OutputReferred, "{}", model);
      assert_eq!(decoded.linearization_table, raw.linearization_table, "{}", model);
      assert_eq!(decoded.whitelevel, raw.whitelevel, "{}", model);
      assert!(decoded.pixels_u16() == raw.pixels_u16(), "{}: the stored values are changed", model);

      let reread = RawDevelop::default().develop_image(decoded)?;
      let (mean, largest) = differences(rendered.data.iter().copied().zip(reread.data.iter().copied()));
      eprintln!("{} {:?}: mean difference {} and largest difference {} of rendered values", model, compression, mean, largest);
      assert!(mean <= 2e-5 && largest <= 2e-4, "{}: mean {} largest {}", model, mean, largest);
    }
  }
  Ok(())
}

/// A profile in a file doesn't mean that the raw data is encoded by it
#[test]
fn icc_profile_of_other_files_is_ignored() -> std::result::Result<(), Box<dyn std::error::Error>> {
  // The profile of a scan in IFD 0, next to its preview. It has lookup tables.
  let path = rawdb::sample("dnglab", "dnglab-issue-619", "testfiles/silverfast_scan.dng")?;
  let file = std::fs::read(&path)?;
  let profile = root_icc_profile(&file)?;
  assert_eq!(profile.len(), 217840);
  assert!(MatrixShaperProfile::parse(&profile).is_err());
  // The profile is no IFD
  let tiff = GenericTiffReader::new_with_buffer(&file, 0, 0, None)?;
  assert!(!tiff.root_ifd().sub_ifds().contains_key(&u16::from(ExifTag::IccProfile)));
  let raw = decode_raw(&RawSource::new(&path)?)?;
  assert_eq!(raw.colorimetric_reference, ColorimetricReference::SceneReferred);
  assert_eq!(raw.linearization_table, None);

  // A Adobe RGB profile with mosaic data
  let path = rawdb::sample("Phase One", "H 25", "raw_modes/H 25_IIQ S_ISO_100.IIQ")?;
  let raw = decode_raw(&RawSource::new(&path)?)?;
  assert_eq!(raw.colorimetric_reference, ColorimetricReference::SceneReferred);
  assert_eq!(raw.linearization_table, None);
  Ok(())
}
