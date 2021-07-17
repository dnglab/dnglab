// SPDX-License-Identifier: LGPL-2.1
// Copyright 2021 Daniel Vogelbacher <daniel@chaospixel.com>

#[cfg(feature = "samplecheck")]
mod camera_samples {

  use rawler::analyze::{analyze_file, extract_raw_pixels, raw_as_pgm, AnalyzerResult};
  use std::{cmp::Ordering, io::Cursor, path::PathBuf};

  macro_rules! camera_file_check {
    ($make:expr, $model:expr, $test:ident, $file:expr) => {
      #[test]
      fn $test() -> std::result::Result<(), Box<dyn std::error::Error>> {
        check_camera_raw_file_conversion($make, $model, $file)
      }
    };
  }

  camera_file_check!(
    "Canon",
    "EOS R5",
    canon_eos_r5_raw_nocrop_nodual,
    "Canon EOS R5_RAW_ISO_100_nocrop_nodual.CR3.raw"
  );
  camera_file_check!("Canon", "EOS R5", canon_eos_r5_raw_nocrop_dual, "Canon EOS R5_RAW_ISO_100_nocrop_dual.CR3.raw");
  camera_file_check!("Canon", "EOS R5", canon_eos_r5_raw_crop_dual, "Canon EOS R5_RAW_ISO_100_crop_dual.CR3.raw");
  camera_file_check!("Canon", "EOS R5", canon_eos_r5_raw_crop_nodual, "Canon EOS R5_RAW_ISO_100_crop_nodual.CR3.raw");

  camera_file_check!("Canon", "EOS R5", canon_eos_r5_raw_biasframe, "Canon EOS R5_RAW_biasframe.CR3.raw");
  camera_file_check!("Canon", "EOS R5", canon_eos_r5_raw_whiteframe, "Canon EOS R5_RAW_whiteframe.CR3.raw");

  camera_file_check!(
    "Canon",
    "EOS R6",
    canon_eos_r6_raw_nocrop_nodual,
    "Canon EOS R6_RAW_ISO_100_nocrop_nodual.CR3.raw"
  );
  camera_file_check!("Canon", "EOS R6", canon_eos_r6_raw_crop_nodual, "Canon EOS R6_RAW_ISO_100_crop_nodual.CR3.raw");

  camera_file_check!(
    "Canon",
    "EOS RP",
    canon_eos_rp_raw_nocrop_nodual,
    "Canon EOS RP_RAW_ISO_100_nocrop_nodual.CR3.raw"
  );
  camera_file_check!("Canon", "EOS RP", canon_eos_rp_raw_crop_nodual, "Canon EOS RP_RAW_ISO_100_crop_nodual.CR3.raw");

  camera_file_check!("Canon", "EOS R", canon_eos_r_raw_nocrop_nodual, "Canon EOS R_RAW_ISO_100_nocrop_nodual.CR3.raw");
  camera_file_check!("Canon", "EOS R", canon_eos_r_raw_nocrop_dual, "Canon EOS R_RAW_ISO_100_nocrop_dual.CR3.raw");
  camera_file_check!("Canon", "EOS R", canon_eos_r_raw_crop_dual, "Canon EOS R_RAW_ISO_100_crop_dual.CR3.raw");
  camera_file_check!("Canon", "EOS R", canon_eos_r_raw_crop_nodual, "Canon EOS R_RAW_ISO_100_crop_nodual.CR3.raw");

  camera_file_check!(
    "Canon",
    "EOS-1D X Mark III",
    canon_eos_1dx_mark3_raw_nocrop_nodual,
    "Canon EOS-1D X Mark III_RAW_ISO_100_nocrop_nodual.CR3.raw"
  );

  camera_file_check!(
    "Canon",
    "EOS 250D",
    canon_eos_250d_raw_nocrop_nodual,
    "Canon EOS 250D_RAW_ISO_100_nocrop_nodual.CR3.raw"
  );

  camera_file_check!(
    "Canon",
    "EOS 850D",
    canon_eos_850d_raw_nocrop_nodual,
    "Canon EOS 850D_RAW_ISO_100_nocrop_nodual.CR3.raw"
  );

  camera_file_check!(
    "Canon",
    "EOS 90D",
    canon_eos_90d_raw_nocrop_nodual,
    "Canon EOS 90D_RAW_ISO_100_nocrop_nodual.CR3.raw"
  );

  camera_file_check!(
    "Canon",
    "EOS M50",
    canon_eos_m50_raw_nocrop_nodual,
    "Canon EOS M50_RAW_ISO_100_nocrop_nodual.CR3.raw"
  );

  camera_file_check!(
    "Canon",
    "EOS M50 Mark II",
    canon_eos_m50_mark2_raw_nocrop_nodual,
    "Canon EOS M50m2_RAW_ISO_100_nocrop_nodual.CR3.raw"
  );

  camera_file_check!(
    "Canon",
    "EOS M6 Mark II",
    canon_eos_m6_mark2_raw_nocrop_nodual,
    "Canon EOS M6 Mark II_RAW_ISO_100_nocrop_nodual.CR3.raw"
  );

  camera_file_check!(
    "Canon",
    "PowerShot G5 X Mark II",
    canon_powershot_g5x_mark2_raw_nocrop_nodual,
    "Canon PowerShot G5 X Mark II_RAW_ISO_200_nocrop_nodual.CR3.raw"
  );

  camera_file_check!(
    "Canon",
    "PowerShot G7 X Mark III",
    canon_powershot_g7x_mark3_raw_nocrop_nodual,
    "Canon PowerShot G7 X Mark III_RAW_ISO_200_nocrop_nodual.CR3.raw"
  );

  /// Generic function to check camera raw files against
  /// pre-generated stats and pixel files.
  fn check_camera_raw_file_conversion(make: &str, model: &str, sample: &str) -> std::result::Result<(), Box<dyn std::error::Error>> {
    let base_path = PathBuf::from("../testdata/cameras").join(make).join(model);

    let raw_file = base_path.join(&sample);
    let stats_file = base_path.join(&sample).with_extension("analyze");
    let pixel_file = base_path.join(&sample).with_extension("pixel");

    assert_eq!(raw_file.exists(), true);

    // Validate stats file
    let new_stats = analyze_file(&PathBuf::from(&raw_file)).unwrap();
    let old_stats = std::fs::read_to_string(&stats_file)?;

    let old_stats: AnalyzerResult = serde_yaml::from_str(&old_stats)?;

    assert_eq!(old_stats, new_stats);

    // Validate pixel data
    let (width, height, buf) = extract_raw_pixels(&raw_file).unwrap();
    let mut new_pgm = Vec::new();
    raw_as_pgm(width, height, &buf, &mut Cursor::new(&mut new_pgm))?;
    let old_pgm = std::fs::read(&pixel_file)?;
    assert!(old_pgm.partial_cmp(&new_pgm) == Some(Ordering::Equal));
    Ok(())
  }
}
