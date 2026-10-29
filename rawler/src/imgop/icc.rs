// SPDX-License-Identifier: LGPL-2.1
// Copyright 2026 Daniel Vogelbacher <daniel@chaospixel.com>

//! ICC profiles (version 2) to tag rendered images with their color space
//!
//! The profiles are matrix/TRC profiles: the colorants are the primaries of
//! the color space, adapted to the D50 white of the profile connection space
//! by Bradford, the tone reproduction curves decode the transfer function.

use crate::imgop::{
  chromatic_adaption::bradford_adaption_matrix,
  color::{OutputSpace, TransferFunction},
  matrix::multiply,
  xyz::{CIE_1931_TRISTIMULUS_D50, CIE_1931_TRISTIMULUS_D65, Illuminant},
};

/// Number of samples of a tone reproduction curve that isn't a gamma
const CURVE_SAMPLES: usize = 1024;

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
}
