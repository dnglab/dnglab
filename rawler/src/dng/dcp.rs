// SPDX-License-Identifier: LGPL-2.1
// Copyright 2026 Daniel Vogelbacher <daniel@chaospixel.com>

//! DNG camera profile files (.dcp)
//!
//! A DCP file has the format of an extra camera profile of a DNG file: the
//! byte order mark, the magic number 0x4352 and the offset of an IFD with the
//! camera profile tags. The IFD also has the UniqueCameraModel of the camera
//! the profile is made for.

use std::fs::File;
use std::io::{BufReader, Read, Seek, Write};
use std::path::Path;

use crate::camera_profile::CameraProfile;
use crate::dng::camera_profile_tags::{camera_profile_data, read_camera_profile, read_camera_profile_ifd, read_string};
use crate::fileutils;
use crate::rawimage::RawImage;
use crate::tags::DngTag;
use crate::{RawlerError, Result};

/// DNG camera profile file (.dcp): a camera profile with the UniqueCameraModel it is made for
#[derive(Debug, Clone, PartialEq)]
pub struct CameraProfileFile {
  /// UniqueCameraModel of the camera the profile is made for
  pub unique_camera_model: Option<String>,
  pub profile: CameraProfile,
}

impl CameraProfileFile {
  /// Read a DCP file
  pub fn read<R: Read + Seek>(reader: &mut R) -> Result<Self> {
    let invalid = |err: String| RawlerError::DecoderFailed(format!("Invalid camera profile file: {}", err));
    let ifd = read_camera_profile_ifd(reader, 0).map_err(invalid)?;
    let profile = read_camera_profile(&ifd).ok_or_else(|| invalid("no valid ColorMatrix1".into()))?;
    Ok(Self {
      unique_camera_model: read_string(&ifd, DngTag::UniqueCameraModel),
      profile,
    })
  }

  /// Read the DCP file `path`
  pub fn open(path: impl AsRef<Path>) -> Result<Self> {
    let path = path.as_ref();
    let file = File::open(path).map_err(|err| RawlerError::with_io_error("open camera profile", path, err))?;
    Self::read(&mut BufReader::new(file))
  }

  /// DCP file data, in the byte order of the host
  pub fn to_bytes(&self) -> Result<Vec<u8>> {
    Ok(camera_profile_data(&self.profile, self.unique_camera_model.as_deref())?)
  }

  /// Write the DCP file data to `writer`
  pub fn write<W: Write>(&self, writer: &mut W) -> Result<()> {
    writer
      .write_all(&self.to_bytes()?)
      .map_err(|err| RawlerError::DecoderFailed(format!("Can't write camera profile file: {}", err)))
  }

  /// DCP file of camera profile `index` of `rawimage`
  ///
  /// The ProfileEmbedPolicy is not checked, see [`Self::check_copying`].
  /// The camera calibration and analog balance belong to the image, they are
  /// not part of the profile. A profile without name gets the name
  /// "<make> <model> standard profile".
  pub fn from_raw_image(rawimage: &RawImage, index: usize) -> Result<Self> {
    let mut profile = rawimage
      .camera_profiles
      .get(index)
      .ok_or_else(|| RawlerError::DecoderFailed(format!("The image has no camera profile {}", index)))?
      .clone();
    // ReductionMatrix is required for more than three color planes, and not allowed otherwise
    profile.reduction_matrix = if profile.color_planes().is_some_and(|planes| planes > 3) {
      profile.reduction_matrices()
    } else {
      Default::default()
    };
    if profile.name.is_none() {
      profile.name = Some(format!("{} {} standard profile", rawimage.clean_make, rawimage.clean_model));
    }
    Ok(Self {
      unique_camera_model: Some(rawimage.unique_camera_model()),
      profile,
    })
  }

  /// Name of the profile
  pub fn name(&self) -> &str {
    self.profile.name.as_deref().unwrap_or_default()
  }

  /// File name "<UniqueCameraModel> <ProfileName>.dcp"
  ///
  /// Characters other than ASCII letters, digits and a few punctuation
  /// characters are replaced by '_'.
  pub fn file_name(&self) -> String {
    let stem = [self.unique_camera_model.as_deref(), self.profile.name.as_deref()]
      .into_iter()
      .flatten()
      .filter(|part| !part.is_empty())
      .collect::<Vec<_>>()
      .join(" ");
    let stem = fileutils::sanitize(&stem);
    format!("{}.dcp", if stem.is_empty() { "Camera profile" } else { stem.as_str() })
  }

  /// The ProfileEmbedPolicy must allow to embed the profile in a DNG file
  pub fn check_embedding(&self) -> Result<()> {
    if self.profile.allows_embedding() {
      Ok(())
    } else {
      Err(RawlerError::DecoderFailed(format!(
        "Camera profile '{}' can't be embedded, its ProfileEmbedPolicy is '{}'",
        self.name(),
        self.profile.embed_policy()
      )))
    }
  }

  /// The ProfileEmbedPolicy must allow to copy the profile from the file it
  /// is embedded in, to process other images
  pub fn check_copying(&self) -> Result<()> {
    if self.profile.allows_copying() {
      Ok(())
    } else {
      Err(RawlerError::DecoderFailed(format!(
        "Camera profile '{}' can't be copied, its ProfileEmbedPolicy is '{}'",
        self.name(),
        self.profile.embed_policy()
      )))
    }
  }

  /// Warn if the profile is made for another camera than the one of `rawimage`
  ///
  /// It is not an error: the camera names don't always match, and a profile
  /// of a similar camera can be useful.
  pub fn check_camera(&self, rawimage: &RawImage) {
    let camera = rawimage.unique_camera_model();
    if let Some(model) = self.unique_camera_model.as_deref().filter(|model| !model.eq_ignore_ascii_case(&camera)) {
      log::warn!(
        "Camera profile '{}' is made for the camera '{}', the image is from '{}'",
        self.name(),
        model,
        camera
      );
    }
  }
}

#[cfg(test)]
mod tests {
  use std::io::Cursor;

  use super::*;
  use crate::CFA;
  use crate::camera_profile::ProfileEmbedPolicy;
  use crate::decoders::Camera;
  use crate::imgop::xyz::Illuminant;
  use crate::pixarray::PixU16;
  use crate::rawimage::{BlackLevel, CFAConfig, RawPhotometricInterpretation, WhiteLevel};

  /// Profile with two calibration illuminants, tables, tone curve and metadata,
  /// the values are exact in DNG rationals
  fn profile() -> CameraProfile {
    let matrix = |f: f32| (0..9).map(|i| (i as f32 - 4.0) * 0.25 * f).collect::<Vec<f32>>();
    let mut profile = CameraProfile {
      name: Some("Camera Standard".into()),
      calibration_signature: Some("com.dnglab.test".into()),
      copyright: Some("Copyright © DNGLab".into()),
      embed_policy: Some(ProfileEmbedPolicy::AllowCopying),
      hue_sat_map_dims: Some([2, 2, 1]),
      look_table_dims: Some([2, 1, 2]),
      look_table_data: Some((0..12).map(|i| i as f32 / 8.0).collect()),
      look_table_encoding: Some(1),
      tone_curve: Some(vec![0.0, 0.0, 0.5, 0.625, 1.0, 1.0]),
      baseline_exposure_offset: Some(-0.25),
      default_black_render: Some(1),
      ..Default::default()
    };
    for (slot, illuminant) in [Illuminant::A, Illuminant::D65].into_iter().enumerate() {
      profile.calibration_illuminant[slot] = Some(illuminant);
      profile.color_matrix[slot] = Some(matrix(slot as f32 + 1.0));
      profile.forward_matrix[slot] = Some(matrix(0.5));
      profile.hue_sat_map_data[slot] = Some((0..12).map(|i| i as f32 * 0.5).collect());
    }
    profile
  }

  fn profile_file() -> CameraProfileFile {
    CameraProfileFile {
      unique_camera_model: Some("Canon EOS 100D".into()),
      profile: profile(),
    }
  }

  /// RGGB image of `clean_make` and `clean_model`
  fn rawimage(clean_make: &str, clean_model: &str) -> RawImage {
    let mut camera = Camera::new();
    camera.clean_make = clean_make.into();
    camera.clean_model = clean_model.into();
    camera.cfa = CFA::new("RGGB");
    let photometric = RawPhotometricInterpretation::Cfa(CFAConfig::new_from_camera(&camera));
    RawImage::new(
      camera,
      PixU16::new_with(vec![0; 4 * 4], 4, 4),
      1,
      [2.0, 1.0, 1.5, f32::NAN],
      photometric,
      Some(BlackLevel::new(&[0_u32; 4], 2, 2, 1)),
      Some(WhiteLevel::new_bits(12, 1)),
      false,
    )
  }

  #[test]
  fn roundtrip() -> Result<()> {
    let file = profile_file();
    let data = file.to_bytes()?;
    assert_eq!(CameraProfileFile::read(&mut Cursor::new(&data))?, file);
    let mut written = Vec::new();
    file.write(&mut written)?;
    assert_eq!(written, data);
    Ok(())
  }

  /// The header is "IIRC" on little-endian hosts
  #[test]
  #[cfg(target_endian = "little")]
  fn header_is_iirc() -> Result<()> {
    assert_eq!(&profile_file().to_bytes()?[0..4], b"IIRC");
    Ok(())
  }

  #[test]
  fn invalid_files() -> Result<()> {
    let mut data = profile_file().to_bytes()?;
    // A TIFF file is not a camera profile
    let mut tiff = data.clone();
    tiff[2..4].copy_from_slice(&42_u16.to_le_bytes());
    assert!(CameraProfileFile::read(&mut Cursor::new(&tiff)).is_err());
    // A camera profile requires ColorMatrix1
    let mut file = profile_file();
    file.profile.color_matrix[0] = None;
    file.profile.calibration_illuminant[0] = None;
    assert!(CameraProfileFile::read(&mut Cursor::new(&file.to_bytes()?)).is_err());
    data.truncate(6);
    assert!(CameraProfileFile::read(&mut Cursor::new(&data)).is_err());
    Ok(())
  }

  #[test]
  fn file_name() {
    let mut file = profile_file();
    assert_eq!(file.file_name(), "Canon EOS 100D Camera Standard.dcp");
    file.profile.name = Some("A/B: C?".into());
    assert_eq!(file.file_name(), "Canon EOS 100D A_B_ C_.dcp");
    file.unique_camera_model = None;
    file.profile.name = None;
    assert_eq!(file.file_name(), "Camera profile.dcp");
  }

  #[test]
  fn embed_policy() {
    let mut file = profile_file();
    for (policy, embedding, copying) in [
      (None, true, true),
      (Some(ProfileEmbedPolicy::AllowCopying), true, true),
      (Some(ProfileEmbedPolicy::EmbedIfUsed), true, false),
      (Some(ProfileEmbedPolicy::EmbedNever), false, false),
      (Some(ProfileEmbedPolicy::NoRestrictions), true, true),
      (Some(ProfileEmbedPolicy::Unknown(4)), false, false),
    ] {
      file.profile.embed_policy = policy;
      assert_eq!(file.check_embedding().is_ok(), embedding, "embedding with {:?}", policy);
      assert_eq!(file.check_copying().is_ok(), copying, "copying with {:?}", policy);
      // Reading a profile doesn't check the policy
      let mut rawimage = rawimage("Canon", "EOS 100D");
      rawimage.camera_profiles = vec![file.profile.clone()];
      assert!(CameraProfileFile::from_raw_image(&rawimage, 0).is_ok(), "reading with {:?}", policy);
    }
  }

  #[test]
  fn from_raw_image() -> Result<()> {
    let mut rawimage = rawimage("Canon", "EOS 100D");
    let mut unnamed = profile();
    unnamed.name = None;
    rawimage.camera_profiles = vec![unnamed, profile()];
    let file = CameraProfileFile::from_raw_image(&rawimage, 0)?;
    assert_eq!(file.unique_camera_model.as_deref(), Some("Canon EOS 100D"));
    assert_eq!(file.name(), "Canon EOS 100D standard profile");
    assert_eq!(CameraProfileFile::from_raw_image(&rawimage, 1)?, profile_file());
    assert!(CameraProfileFile::from_raw_image(&rawimage, 2).is_err());
    Ok(())
  }
}
