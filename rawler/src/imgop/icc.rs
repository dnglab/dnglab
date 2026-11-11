// SPDX-License-Identifier: LGPL-2.1
// Copyright 2026 Daniel Vogelbacher <daniel@chaospixel.com>

//! ICC profiles (version 2) to tag rendered images with their color space, and
//! the reader for matrix/TRC profiles of input files
//!
//! The profiles are matrix/TRC profiles: the colorants are the primaries of
//! the color space, adapted to the D50 white of the profile connection space
//! by Bradford, the tone reproduction curves decode the transfer function.

use crate::RawlerError;
use moxcms::{ColorProfile, DataColorSpace, ProfileText, ToneReprCurve};

use crate::imgop::{
  chromatic_adaption::bradford_adaption_matrix,
  color::{OutputSpace, TransferFunction},
  matrix::multiply,
  xyz::{CIE_1931_TRISTIMULUS_D50, CIE_1931_TRISTIMULUS_D65, Illuminant},
};

/// Number of samples of a tone reproduction curve that isn't a gamma
const CURVE_SAMPLES: usize = 1024;

/// Largest difference of the sum of the colorants to the white of the PCS
const COLORANT_WHITE_TOLERANCE: f32 = 0.01;

/// RGB matrix/TRC profile, read from a file
#[derive(Clone, Debug, PartialEq)]
pub struct MatrixShaperProfile {
  /// Description of the profile
  pub description: Option<String>,
  /// Matrix from linear RGB values to XYZ (D50), its columns are the colorants
  pub rgb_to_xyz: [[f32; 3]; 3],
  /// Curve that decodes the RGB values of all channels
  curve: ToneReprCurve,
}

impl MatrixShaperProfile {
  /// Read a profile
  ///
  /// Fails for any other kind of profile: a profile with lookup tables needs
  /// a color management module, and one LinearizationTable can't decode
  /// different curves per channel.
  pub fn parse(data: &[u8]) -> crate::Result<Self> {
    let profile = ColorProfile::new_from_slice(data).map_err(|err| input_error!("Invalid ICC profile").context(err))?;
    if profile.color_space != DataColorSpace::Rgb || profile.pcs != DataColorSpace::Xyz {
      return Err(input_error!(
        "ICC profile with color space {:?} and PCS {:?} is not supported",
        profile.color_space,
        profile.pcs
      ));
    }
    if profile.lut_a_to_b_perceptual.is_some() || profile.lut_a_to_b_colorimetric.is_some() || profile.lut_a_to_b_saturation.is_some() {
      return Err(RawlerError::invalid_input("ICC profile with lookup tables is not supported"));
    }
    let (Some(red), Some(green), Some(blue)) = (&profile.red_trc, &profile.green_trc, &profile.blue_trc) else {
      return Err(RawlerError::invalid_input("ICC profile has no tone reproduction curves"));
    };
    if red != green || green != blue {
      return Err(RawlerError::invalid_input("ICC profile with different curves per channel is not supported"));
    }
    let [r, g, b] = [profile.red_colorant, profile.green_colorant, profile.blue_colorant];
    let rgb_to_xyz = [
      [r.x as f32, g.x as f32, b.x as f32],
      [r.y as f32, g.y as f32, b.y as f32],
      [r.z as f32, g.z as f32, b.z as f32],
    ];
    // Colorants of the PCS map white RGB values to its white
    if rgb_to_xyz
      .iter()
      .zip(CIE_1931_TRISTIMULUS_D50)
      .any(|(row, white)| (row.iter().sum::<f32>() - white).abs() > COLORANT_WHITE_TOLERANCE)
    {
      return Err(input_error!(
        "ICC profile with colorants {:?} doesn't map white to the white of the PCS",
        rgb_to_xyz
      ));
    }
    let description = profile.description.as_ref().and_then(description_text);
    Ok(Self {
      description,
      rgb_to_xyz,
      curve: red.clone(),
    })
  }

  /// LinearizationTable with `entries` evenly spaced values of the curve, scaled to 16 bits
  ///
  /// Fails for a curve that isn't monotonic, it can't be a LinearizationTable:
  /// readers interpolate between its neighbor entries.
  pub fn linearization_table(&self, entries: usize) -> crate::Result<Vec<u16>> {
    let curve = self
      .curve
      .make_linear_evaluator()
      .map_err(|err| input_error!("Invalid ICC curve").context(err))?;
    let last = entries.saturating_sub(1).max(1) as f32;
    let table: Vec<u16> = (0..entries)
      .map(|i| (curve.evaluate_value(i as f32 / last).clamp(0.0, 1.0) * f32::from(u16::MAX)).round() as u16)
      .collect();
    if table.windows(2).any(|pair| pair[0] > pair[1]) {
      return Err(RawlerError::invalid_input("ICC curve is not monotonic"));
    }
    Ok(table)
  }
}

/// Text of a description tag
fn description_text(text: &ProfileText) -> Option<String> {
  let text = match text {
    ProfileText::PlainString(text) => text.as_str(),
    ProfileText::Description(description) => description.ascii_string.as_str(),
    ProfileText::Localizable(strings) => strings.first()?.value.as_str(),
  };
  let text = text.trim_end_matches('\0').trim();
  (!text.is_empty()).then(|| text.to_string())
}

/// ICC profile of RGB values of `space`, encoded by `transfer`
pub fn rgb_profile(space: OutputSpace, transfer: TransferFunction) -> Vec<u8> {
  let white = match space {
    OutputSpace::Srgb | OutputSpace::AdobeRgb => CIE_1931_TRISTIMULUS_D65,
    OutputSpace::ProPhotoRgb => CIE_1931_TRISTIMULUS_D50,
  };
  // Colorants are the columns of the matrix to XYZ (D50)
  let rgb_to_xyz = multiply(&bradford_adaption_matrix(&Illuminant::D65, &Illuminant::D50), &space.rgb_to_xyz());
  let colorant = |c: usize| xyz_type([rgb_to_xyz[0][c], rgb_to_xyz[1][c], rgb_to_xyz[2][c]]);
  let curve = curve_type(transfer);
  let description = match transfer {
    TransferFunction::Linear => format!("{} linear", space.name()),
    _ => space.name().to_string(),
  };
  profile(
    b"RGB ",
    vec![
      (*b"desc", description_type(&description)),
      (*b"cprt", text_type("No copyright, use freely")),
      (*b"wtpt", xyz_type(white)),
      (*b"rXYZ", colorant(0)),
      (*b"gXYZ", colorant(1)),
      (*b"bXYZ", colorant(2)),
      (*b"rTRC", curve.clone()),
      (*b"gTRC", curve.clone()),
      (*b"bTRC", curve),
    ],
  )
}

/// ICC profile of gray values, encoded by `transfer`
pub fn gray_profile(transfer: TransferFunction) -> Vec<u8> {
  let description = match transfer {
    TransferFunction::Linear => "Gray linear",
    TransferFunction::Srgb => "Gray sRGB curve",
    TransferFunction::AdobeRgb => "Gray Adobe RGB curve",
    TransferFunction::ProPhotoRgb => "Gray ProPhoto RGB curve",
  };
  profile(
    b"GRAY",
    vec![
      (*b"desc", description_type(description)),
      (*b"cprt", text_type("No copyright, use freely")),
      (*b"wtpt", xyz_type(CIE_1931_TRISTIMULUS_D50)),
      (*b"kTRC", curve_type(transfer)),
    ],
  )
}

/// Display profile of `color_space` with `tags`, tags with the same data share it
fn profile(color_space: &[u8; 4], tags: Vec<([u8; 4], Vec<u8>)>) -> Vec<u8> {
  let mut header = Vec::with_capacity(128);
  header.extend_from_slice(&[0; 4]); // Size, set below
  header.extend_from_slice(&[0; 4]); // Preferred CMM
  header.extend_from_slice(&0x0210_0000_u32.to_be_bytes()); // Version 2.1
  header.extend_from_slice(b"mntr");
  header.extend_from_slice(color_space);
  header.extend_from_slice(b"XYZ ");
  for value in [2026_u16, 1, 1, 0, 0, 0] {
    header.extend_from_slice(&value.to_be_bytes()); // Fixed date, for reproducible files
  }
  header.extend_from_slice(b"acsp");
  header.extend_from_slice(&[0; 24]); // Platform, flags, manufacturer, model, attributes
  header.extend_from_slice(&0_u32.to_be_bytes()); // Perceptual rendering intent
  // D50 illuminant of the PCS, encoded as defined by ICC
  header.extend_from_slice(&[0x00, 0x00, 0xf6, 0xd6, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0xd3, 0x2d]);
  header.resize(128, 0);

  let table_size = 4 + 12 * tags.len();
  let mut data: Vec<u8> = Vec::new();
  let mut table = (tags.len() as u32).to_be_bytes().to_vec();
  let mut written: Vec<(usize, &[u8])> = Vec::new();
  for (signature, tag) in &tags {
    let offset = match written.iter().find(|(_, other)| *other == tag.as_slice()) {
      Some((offset, _)) => *offset,
      None => {
        let offset = 128 + table_size + data.len();
        data.extend_from_slice(tag);
        data.resize(data.len().next_multiple_of(4), 0);
        written.push((offset, tag));
        offset
      }
    };
    table.extend_from_slice(signature);
    table.extend_from_slice(&(offset as u32).to_be_bytes());
    table.extend_from_slice(&(tag.len() as u32).to_be_bytes());
  }

  let mut profile = header;
  profile.extend(table);
  profile.extend(data);
  let size = (profile.len() as u32).to_be_bytes();
  profile[..4].copy_from_slice(&size);
  profile
}

fn s15_fixed16(v: f32) -> [u8; 4] {
  ((f64::from(v) * 65536.0).round() as i32).to_be_bytes()
}

fn xyz_type(xyz: [f32; 3]) -> Vec<u8> {
  let mut tag = b"XYZ \0\0\0\0".to_vec();
  xyz.iter().for_each(|v| tag.extend_from_slice(&s15_fixed16(*v)));
  tag
}

fn text_type(text: &str) -> Vec<u8> {
  let mut tag = b"text\0\0\0\0".to_vec();
  tag.extend_from_slice(text.as_bytes());
  tag.push(0);
  tag
}

/// textDescriptionType of ICC version 2, with an ASCII description only
fn description_type(description: &str) -> Vec<u8> {
  let mut tag = b"desc\0\0\0\0".to_vec();
  tag.extend_from_slice(&(description.len() as u32 + 1).to_be_bytes());
  tag.extend_from_slice(description.as_bytes());
  tag.push(0);
  tag.extend_from_slice(&[0; 8]); // No Unicode description
  tag.extend_from_slice(&[0; 3 + 67]); // No ScriptCode description
  tag
}

/// Curve that decodes `transfer`: a gamma or sampled values
fn curve_type(transfer: TransferFunction) -> Vec<u8> {
  let mut tag = b"curv\0\0\0\0".to_vec();
  match transfer.gamma() {
    Some(gamma) => {
      tag.extend_from_slice(&1_u32.to_be_bytes());
      tag.extend_from_slice(&((gamma * 256.0).round() as u16).to_be_bytes());
    }
    None => {
      tag.extend_from_slice(&(CURVE_SAMPLES as u32).to_be_bytes());
      for i in 0..CURVE_SAMPLES {
        let linear = transfer.decode(i as f32 / (CURVE_SAMPLES - 1) as f32);
        tag.extend_from_slice(&((linear.clamp(0.0, 1.0) * 65535.0).round() as u16).to_be_bytes());
      }
    }
  }
  tag
}

#[cfg(test)]
mod tests {
  use super::*;

  fn u32_at(data: &[u8], offset: usize) -> u32 {
    u32::from_be_bytes(data[offset..offset + 4].try_into().expect("4 bytes"))
  }

  /// Data of the tag with `signature`
  fn tag<'a>(profile: &'a [u8], signature: &[u8; 4]) -> Option<&'a [u8]> {
    (0..u32_at(profile, 128) as usize).map(|i| 132 + 12 * i).find_map(|entry| {
      (&profile[entry..entry + 4] == signature).then(|| {
        let (offset, size) = (u32_at(profile, entry + 4) as usize, u32_at(profile, entry + 8) as usize);
        &profile[offset..offset + size]
      })
    })
  }

  fn xyz(tag: &[u8]) -> [f32; 3] {
    std::array::from_fn(|i| i32::from_be_bytes(tag[8 + 4 * i..12 + 4 * i].try_into().expect("4 bytes")) as f32 / 65536.0)
  }

  #[test]
  fn rgb_profiles() {
    for space in [OutputSpace::Srgb, OutputSpace::AdobeRgb, OutputSpace::ProPhotoRgb] {
      let profile = rgb_profile(space, space.transfer_function());
      assert_eq!(u32_at(&profile, 0) as usize, profile.len());
      assert_eq!((&profile[16..20], &profile[36..40]), (&b"RGB "[..], &b"acsp"[..]));
      // The colorants add up to the D50 white of the PCS
      let colorants = [b"rXYZ", b"gXYZ", b"bXYZ"].map(|signature| xyz(tag(&profile, signature).expect("colorant")));
      for (c, white) in CIE_1931_TRISTIMULUS_D50.iter().enumerate() {
        let sum: f32 = colorants.iter().map(|xyz| xyz[c]).sum();
        assert!((sum - white).abs() < 2e-4, "{:?}: {:?}", space, colorants);
      }
      // The curves of the channels share their data
      assert_eq!(tag(&profile, b"rTRC").map(<[u8]>::as_ptr), tag(&profile, b"bTRC").map(<[u8]>::as_ptr));
    }
    // Adobe RGB is a gamma, sRGB a sampled curve
    let adobe = rgb_profile(OutputSpace::AdobeRgb, TransferFunction::AdobeRgb);
    assert_eq!(tag(&adobe, b"gTRC"), Some(&b"curv\0\0\0\0\0\0\0\x01\x02\x33"[..]));
    let srgb = rgb_profile(OutputSpace::Srgb, TransferFunction::Srgb);
    let curve = tag(&srgb, b"gTRC").expect("curve");
    assert_eq!(u32_at(curve, 8) as usize, CURVE_SAMPLES);
    let middle = u16::from_be_bytes([curve[12 + 2 * 512], curve[13 + 2 * 512]]);
    assert_eq!(middle, (TransferFunction::Srgb.decode(512.0 / 1023.0) * 65535.0).round() as u16);
  }

  #[test]
  fn gray_profile_with_linear_curve() {
    let profile = gray_profile(TransferFunction::Linear);
    assert_eq!(u32_at(&profile, 0) as usize, profile.len());
    assert_eq!(&profile[16..20], b"GRAY");
    // A gamma of 1.0
    assert_eq!(tag(&profile, b"kTRC"), Some(&b"curv\0\0\0\0\0\0\0\x01\x01\x00"[..]));
    assert!(tag(&profile, b"rXYZ").is_none());
  }

  #[test]
  fn read_generated_profiles() -> crate::Result<()> {
    for (space, transfer) in [
      (OutputSpace::Srgb, TransferFunction::Srgb),
      (OutputSpace::Srgb, TransferFunction::Linear),
      (OutputSpace::AdobeRgb, TransferFunction::AdobeRgb),
      (OutputSpace::ProPhotoRgb, TransferFunction::ProPhotoRgb),
    ] {
      let profile = MatrixShaperProfile::parse(&rgb_profile(space, transfer))?;
      assert!(
        profile.description.as_deref().is_some_and(|text| text.starts_with(space.name())),
        "{:?}",
        profile.description
      );
      let expected = multiply(&bradford_adaption_matrix(&Illuminant::D65, &Illuminant::D50), &space.rgb_to_xyz());
      for (row, expected) in profile.rgb_to_xyz.as_flattened().iter().zip(expected.as_flattened()) {
        assert!((row - expected).abs() < 1e-4, "{:?} {:?}: {:?}", space, transfer, profile.rgb_to_xyz);
      }
      // The table has the values of the curve
      let table = profile.linearization_table(CURVE_SAMPLES)?;
      assert_eq!((table.len(), table[0], table[CURVE_SAMPLES - 1]), (CURVE_SAMPLES, 0, u16::MAX));
      for (i, value) in table.iter().enumerate() {
        let linear = transfer.decode(i as f32 / (CURVE_SAMPLES - 1) as f32) * f32::from(u16::MAX);
        assert!(
          (f32::from(*value) - linear).abs() <= 1.5,
          "{:?} {} is {}, expected {}",
          transfer,
          i,
          value,
          linear
        );
      }
    }
    Ok(())
  }

  /// Tags of an sRGB profile with `curves` for red, green and blue
  fn profile_with_curves(curves: [Vec<u8>; 3]) -> Vec<u8> {
    let rgb_to_xyz = multiply(&bradford_adaption_matrix(&Illuminant::D65, &Illuminant::D50), &OutputSpace::Srgb.rgb_to_xyz());
    let colorant = |c: usize| xyz_type([rgb_to_xyz[0][c], rgb_to_xyz[1][c], rgb_to_xyz[2][c]]);
    let [red, green, blue] = curves;
    profile(
      b"RGB ",
      vec![
        (*b"desc", description_type("test")),
        (*b"cprt", text_type("No copyright, use freely")),
        (*b"wtpt", xyz_type(CIE_1931_TRISTIMULUS_D50)),
        (*b"rXYZ", colorant(0)),
        (*b"gXYZ", colorant(1)),
        (*b"bXYZ", colorant(2)),
        (*b"rTRC", red),
        (*b"gTRC", green),
        (*b"bTRC", blue),
      ],
    )
  }

  /// `curv` tag with the 16 bit `samples`
  fn sampled_curve(samples: &[u16]) -> Vec<u8> {
    let mut tag = b"curv\0\0\0\0".to_vec();
    tag.extend_from_slice(&(samples.len() as u32).to_be_bytes());
    samples.iter().for_each(|sample| tag.extend_from_slice(&sample.to_be_bytes()));
    tag
  }

  #[test]
  fn reject_unsupported_profiles() {
    assert!(MatrixShaperProfile::parse(&[0; 16]).is_err());
    assert!(MatrixShaperProfile::parse(&gray_profile(TransferFunction::Srgb)).is_err());
    // One LinearizationTable can't decode different curves
    let gamma = |gamma: u8| b"curv\0\0\0\0\0\0\0\x01\x02".iter().copied().chain([gamma]).collect::<Vec<u8>>();
    assert!(MatrixShaperProfile::parse(&profile_with_curves([gamma(0x33), gamma(0x33), gamma(0x33)])).is_ok());
    assert!(MatrixShaperProfile::parse(&profile_with_curves([gamma(0x33), gamma(0x34), gamma(0x33)])).is_err());
    // Colorants that don't map white to the white of the PCS
    let tags = vec![
      (*b"desc", description_type("test")),
      (*b"cprt", text_type("No copyright, use freely")),
      (*b"wtpt", xyz_type(CIE_1931_TRISTIMULUS_D50)),
      (*b"rXYZ", xyz_type([0.5, 0.2, 0.0])),
      (*b"gXYZ", xyz_type([0.3, 0.7, 0.1])),
      (*b"bXYZ", xyz_type([0.2, 0.1, 0.9])),
      (*b"rTRC", gamma(0x33)),
      (*b"gTRC", gamma(0x33)),
      (*b"bTRC", gamma(0x33)),
    ];
    assert!(MatrixShaperProfile::parse(&profile(b"RGB ", tags)).is_err());
  }

  #[test]
  fn linearization_table_of_sampled_curve() -> crate::Result<()> {
    let rising = sampled_curve(&[0, 1000, 65535]);
    let profile = MatrixShaperProfile::parse(&profile_with_curves([rising.clone(), rising.clone(), rising]))?;
    // Samples between the entries of the curve are interpolated
    assert_eq!(profile.linearization_table(3)?, vec![0, 1000, 65535]);
    assert_eq!(profile.linearization_table(5)?, vec![0, 500, 1000, 33268, 65535]);
    // The table of a curve that isn't monotonic is not usable
    let falling = sampled_curve(&[0, 40000, 20000, 65535]);
    let profile = MatrixShaperProfile::parse(&profile_with_curves([falling.clone(), falling.clone(), falling]))?;
    assert!(profile.linearization_table(256).is_err());
    Ok(())
  }
}
