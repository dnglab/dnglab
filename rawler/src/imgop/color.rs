// SPDX-License-Identifier: LGPL-2.1
// Copyright 2026 Daniel Vogelbacher <daniel@chaospixel.com>

//! Color processing steps on borrowed image data: the map of camera values
//! to RGB, operations on RGB pixels and the transfer function of the output

use rayon::prelude::*;

use crate::{
  camera_profile::{CalibrationSlot, D65_PREFERENCE},
  dng::profile_render::ProfileRender,
  imgop::{
    Point, Rect,
    chromatic_adaption::{adapt_xyz2cam_bradford, bradford_adaption_matrix},
    matrix::{diagonal, identity, invert, multiply, multiply_vector, normalize, pseudo_inverse, transform_1d},
    raw::clip_euclidean_norm_avg,
    srgb::{srgb_apply_gamma, srgb_invert_gamma},
    xyz::{ADOBERGB_TO_XYZ_D65, Illuminant, PROPHOTORGB_TO_XYZ_D50, SRGB_TO_XYZ_D65, XYZ_TO_ADOBERGB_D65, XYZ_TO_PROFOTORGB_D50, XYZ_TO_SRGB_D65},
  },
  pixarray::{ImageView, ImageViewMut, RgbF32},
};

/// Color space of rendered images
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum OutputSpace {
  /// sRGB, D65 white
  #[default]
  Srgb,
  /// Adobe RGB (1998), D65 white
  AdobeRgb,
  /// ProPhoto RGB (ROMM RGB), D50 white
  ProPhotoRgb,
}

impl OutputSpace {
  /// Matrix from linear RGB of the color space to XYZ (D65). A white other
  /// than D65 is adapted by Bradford.
  pub fn rgb_to_xyz(self) -> [[f32; 3]; 3] {
    match self {
      Self::Srgb => SRGB_TO_XYZ_D65,
      Self::AdobeRgb => ADOBERGB_TO_XYZ_D65,
      Self::ProPhotoRgb => multiply(&bradford_adaption_matrix(&Illuminant::D50, &Illuminant::D65), &PROPHOTORGB_TO_XYZ_D50),
    }
  }

  /// Matrix from XYZ (D65) to linear RGB of the color space, see [`Self::rgb_to_xyz`]
  pub fn xyz_to_rgb(self) -> [[f32; 3]; 3] {
    match self {
      Self::Srgb => XYZ_TO_SRGB_D65,
      Self::AdobeRgb => XYZ_TO_ADOBERGB_D65,
      Self::ProPhotoRgb => multiply(&XYZ_TO_PROFOTORGB_D50, &bradford_adaption_matrix(&Illuminant::D65, &Illuminant::D50)),
    }
  }

  /// Transfer function that encodes linear RGB of the color space
  pub fn transfer_function(self) -> TransferFunction {
    match self {
      Self::Srgb => TransferFunction::Srgb,
      Self::AdobeRgb => TransferFunction::AdobeRgb,
      Self::ProPhotoRgb => TransferFunction::ProPhotoRgb,
    }
  }

  /// Name of the color space
  pub fn name(self) -> &'static str {
    match self {
      Self::Srgb => "sRGB",
      Self::AdobeRgb => "Adobe RGB (1998) compatible",
      Self::ProPhotoRgb => "ProPhoto RGB",
    }
  }
}

#[cfg(feature = "clap")]
impl clap::ValueEnum for OutputSpace {
  fn value_variants<'a>() -> &'a [Self] {
    &[Self::Srgb, Self::AdobeRgb, Self::ProPhotoRgb]
  }

  fn to_possible_value(&self) -> Option<clap::builder::PossibleValue> {
    Some(match self {
      Self::Srgb => clap::builder::PossibleValue::new("srgb"),
      Self::AdobeRgb => clap::builder::PossibleValue::new("adobergb"),
      Self::ProPhotoRgb => clap::builder::PossibleValue::new("prophoto"),
    })
  }
}

/// Gamma of Adobe RGB (1998), 2 + 51/256
const ADOBE_RGB_GAMMA: f32 = 563.0 / 256.0;

/// Gamma of ProPhoto RGB (ROMM RGB)
const PROPHOTO_RGB_GAMMA: f32 = 1.8;

/// End of the linear segment of ProPhoto RGB, 16^(1.8 / (1 - 1.8))
const PROPHOTO_RGB_LINEAR_END: f32 = 1.0 / 512.0;

/// Slope of the linear segment of ProPhoto RGB
const PROPHOTO_RGB_LINEAR_SLOPE: f32 = 16.0;

/// Transfer function that encodes the linear values of rendered images
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TransferFunction {
  /// Linear values
  #[default]
  Linear,
  /// sRGB: power function with gamma 2.4 and a linear segment near black
  Srgb,
  /// Adobe RGB (1998): power function with gamma 2.19921875
  AdobeRgb,
  /// ProPhoto RGB (ROMM RGB): power function with gamma 1.8 and a linear
  /// segment below 1/512
  ProPhotoRgb,
}

impl TransferFunction {
  /// Encode a linear value
  ///
  /// Negative values are encoded by the linear segment, or mirrored for a
  /// function without one.
  pub fn encode(self, v: f32) -> f32 {
    match self {
      Self::Linear => v,
      Self::Srgb => srgb_apply_gamma(v),
      Self::AdobeRgb => v.abs().powf(1.0 / ADOBE_RGB_GAMMA).copysign(v),
      Self::ProPhotoRgb => {
        if v < PROPHOTO_RGB_LINEAR_END {
          v * PROPHOTO_RGB_LINEAR_SLOPE
        } else {
          v.powf(1.0 / PROPHOTO_RGB_GAMMA)
        }
      }
    }
  }

  /// Linear value of an encoded value, the inverse of [`Self::encode`]
  pub fn decode(self, v: f32) -> f32 {
    match self {
      Self::Linear => v,
      Self::Srgb => srgb_invert_gamma(v),
      Self::AdobeRgb => v.abs().powf(ADOBE_RGB_GAMMA).copysign(v),
      Self::ProPhotoRgb => {
        if v < PROPHOTO_RGB_LINEAR_END * PROPHOTO_RGB_LINEAR_SLOPE {
          v / PROPHOTO_RGB_LINEAR_SLOPE
        } else {
          v.powf(PROPHOTO_RGB_GAMMA)
        }
      }
    }
  }

  /// Gamma of a pure power function, `None` for other functions
  pub fn gamma(self) -> Option<f32> {
    match self {
      Self::Linear => Some(1.0),
      Self::AdobeRgb => Some(ADOBE_RGB_GAMMA),
      Self::Srgb | Self::ProPhotoRgb => None,
    }
  }
}

/// Encode all samples of `image` by `function`
pub fn apply_transfer_function<'a>(image: impl Into<ImageViewMut<'a, f32>>, function: TransferFunction) {
  if function != TransferFunction::Linear {
    image.into().as_mut_slice().par_iter_mut().for_each(|v| *v = function.encode(*v));
  }
}

/// Multiply all samples of `image` by `gain`
pub fn apply_gain<'a>(image: impl Into<ImageViewMut<'a, f32>>, gain: f32) {
  image.into().as_mut_slice().par_iter_mut().for_each(|v| *v *= gain);
}

/// Matrix from white balanced camera values to linear RGB of `output`
///
/// Camera values are white balanced by `wb`, the reciprocal of the camera
/// neutral. AnalogBalance and CameraCalibration (AB * CC) of the calibration
/// slot are applied. With a ForwardMatrix, the camera values are mapped to XYZ
/// D50 as defined by DNG and adapted to D65 by Bradford. Otherwise the
/// ColorMatrix, adapted to D65, is normalized to map the camera neutral to the
/// white of `output`. Without a slot, the identity matrix is used.
pub fn camera_to_output<const N: usize>(
  slot: Option<&CalibrationSlot<'_>>,
  analog_balance: Option<&[f32]>,
  wb: &[f32; N],
  output: OutputSpace,
) -> [[f32; N]; 3] {
  let analog_balance = analog_balance.and_then(|balance| {
    let balance = <[f32; N]>::try_from(balance).ok();
    if balance.is_none() {
      log::warn!("AnalogBalance doesn't match {} color planes, ignoring it", N);
    }
    balance
  });
  let calibration = slot.and_then(|slot| slot.camera_calibration).and_then(|matrix| {
    let matrix = transform_1d::<N, N>(matrix);
    if matrix.is_none() {
      log::warn!("CameraCalibration doesn't match {} color planes, ignoring it", N);
    }
    matrix
  });
  let abcc = match (analog_balance, calibration) {
    (None, None) => None,
    (balance, calibration) => Some(multiply(&diagonal(&balance.unwrap_or([1.0; N])), &calibration.unwrap_or(identity()))),
  };

  if let Some(forward_matrix) = slot.and_then(|slot| slot.forward_matrix) {
    match (transform_1d::<3, N>(forward_matrix), invert(&abcc.unwrap_or(identity()))) {
      (Some(forward_matrix), Some(abcc_inv)) => {
        // CameraToXYZ_D50 = FM * D * Inverse(AB * CC), D white balances the reference neutral
        let neutral = wb.map(|coeff| 1.0 / coeff);
        let reference_neutral = multiply_vector(&abcc_inv, &neutral);
        let camera_to_xyz = multiply(&forward_matrix, &multiply(&diagonal(&reference_neutral.map(|v| 1.0 / v)), &abcc_inv));
        // The camera values are already white balanced by `wb`
        let camera_to_xyz = multiply(&camera_to_xyz, &diagonal(&neutral));
        let xyz_to_rgb = multiply(&output.xyz_to_rgb(), &bradford_adaption_matrix(&Illuminant::D50, &Illuminant::D65));
        let cam2rgb = multiply(&xyz_to_rgb, &camera_to_xyz);
        if cam2rgb.as_flattened().iter().all(|v| v.is_finite()) {
          return cam2rgb;
        }
        log::warn!("Invalid ForwardMatrix result for white balance {:?}, using ColorMatrix", wb);
      }
      (None, _) => log::warn!("ForwardMatrix doesn't match {} color planes, using ColorMatrix", N),
      (_, None) => log::warn!("AnalogBalance * CameraCalibration is not invertible, using ColorMatrix"),
    }
  }

  let xyz2cam = match slot.and_then(|slot| Some((slot.illuminant, transform_1d::<N, 3>(slot.color_matrix)?))) {
    Some((Illuminant::D65, matrix)) => matrix,
    Some((illuminant, matrix)) if !D65_PREFERENCE.contains(&illuminant) => {
      log::warn!("Color matrix for {:?} can't be adapted to D65, it is used as it is", illuminant);
      matrix
    }
    Some((illuminant, matrix)) => {
      log::warn!("Image has no D65 illuminant matrix, alternative for Bradford adaption: {:?}", illuminant);
      adapt_xyz2cam_bradford(&illuminant, &Illuminant::D65, &matrix)
    }
    None => {
      log::warn!("No color matrix for {} color planes found, using identity matrix", N);
      std::array::from_fn(|i| std::array::from_fn(|j| if i == j { 1.0 } else { 0.0 }))
    }
  };
  let xyz2cam = match abcc {
    Some(abcc) => multiply(&abcc, &xyz2cam),
    None => xyz2cam,
  };
  pseudo_inverse(normalize(multiply(&xyz2cam, &output.rgb_to_xyz())))
}

/// Map of camera values with N color planes to RGB: the values are white
/// balanced by `wb`, limited to `white` and transformed by `matrix`
#[derive(Clone, Debug, PartialEq)]
pub struct CameraToRgb<const N: usize> {
  pub wb: [f32; N],
  pub white: f32,
  pub matrix: [[f32; N]; 3],
}

impl<const N: usize> CameraToRgb<N> {
  /// RGB values of one pixel
  #[inline]
  pub fn apply(&self, pixel: &[f32; N]) -> [f32; 3] {
    let balanced: [f32; N] = std::array::from_fn(|i| (pixel[i] * self.wb[i]).min(self.white));
    self.matrix.map(|row| row.iter().zip(&balanced).map(|(m, v)| m * v).sum())
  }
}

/// Operation on the values of one RGB pixel
#[derive(Clone, Debug, PartialEq)]
pub enum PixelOp {
  /// Linear transformation of the RGB values
  Matrix([[f32; 3]; 3]),
  /// Rendering tables of a camera profile, see [`ProfileRender::apply_pixel`]
  Profile(ProfileRender),
  /// Clip each value to 0.0..=1.0
  Clip,
  /// Clip values above 1.0 and keep some of the color, see [`clip_euclidean_norm_avg`]
  ClipNormAvg,
  /// Encode each value by a transfer function
  TransferFunction(TransferFunction),
}

impl PixelOp {
  /// Apply the operation to `rgb`, a pixel at the relative position `u`
  /// (horizontal) and `v` (vertical) in the image area
  #[inline]
  pub fn apply(&self, rgb: [f32; 3], u: f32, v: f32) -> [f32; 3] {
    match self {
      Self::Matrix(matrix) => matrix.map(|row| row.iter().zip(&rgb).map(|(m, c)| m * c).sum()),
      Self::Profile(render) => render.apply_pixel(rgb, u, v),
      Self::Clip => rgb.map(|c| c.clamp(0.0, 1.0)),
      Self::ClipNormAvg => clip_euclidean_norm_avg(&rgb),
      Self::TransferFunction(function) => rgb.map(|c| function.encode(c)),
    }
  }
}

fn channel_error(expected: usize, channels: usize) -> crate::RawlerError {
  format!("Expected an image with {} channels, image has {}", expected, channels).into()
}

/// Map the camera values of `src`, which has N channels, to RGB by `map` and
/// apply `ops` to each pixel
///
/// `area` is the image area that the pixel positions of the operations refer
/// to (see [`Rect::relative_position`]), `origin` is the position of `src` in
/// the coordinates of `area`. Unless `src` is cropped from the image that
/// `area` refers to, `origin` is (0, 0). Fails if `src` has not N channels.
pub fn camera_to_rgb<'a, const N: usize>(
  src: impl Into<ImageView<'a, f32>>,
  map: &CameraToRgb<N>,
  ops: &[PixelOp],
  area: Rect,
  origin: Point,
) -> crate::Result<RgbF32> {
  camera_to_rgb_view(src.into(), map, ops, area, origin)
}

fn camera_to_rgb_view<const N: usize>(src: ImageView<'_, f32>, map: &CameraToRgb<N>, ops: &[PixelOp], area: Rect, origin: Point) -> crate::Result<RgbF32> {
  let pixels = src.as_array_slice::<N>().ok_or_else(|| channel_error(N, src.channels()))?;
  let width = src.width().max(1);
  let mut out = RgbF32::new(src.width(), src.height());
  out
    .data
    .par_chunks_mut(width)
    .zip(pixels.par_chunks(width))
    .enumerate()
    .for_each(|(row, (out, pixels))| {
      for (col, (out, pixel)) in out.iter_mut().zip(pixels).enumerate() {
        let (u, v) = area.relative_position(origin.x + col, origin.y + row);
        *out = ops.iter().fold(map.apply(pixel), |rgb, op| op.apply(rgb, u, v));
      }
    });
  Ok(out)
}

/// Map the camera values of `image`, which has three channels, to RGB by
/// `map` and apply `ops` to each pixel, see [`camera_to_rgb`]
pub fn camera_to_rgb_in_place<'a>(
  image: impl Into<ImageViewMut<'a, f32>>,
  map: &CameraToRgb<3>,
  ops: &[PixelOp],
  area: Rect,
  origin: Point,
) -> crate::Result<()> {
  camera_to_rgb_in_place_view(image.into(), map, ops, area, origin)
}

fn camera_to_rgb_in_place_view(mut image: ImageViewMut<'_, f32>, map: &CameraToRgb<3>, ops: &[PixelOp], area: Rect, origin: Point) -> crate::Result<()> {
  let (width, channels) = (image.width().max(1), image.channels());
  let pixels = image.as_array_slice_mut::<3>().ok_or_else(|| channel_error(3, channels))?;
  pixels.par_chunks_mut(width).enumerate().for_each(|(row, pixels)| {
    for (col, pixel) in pixels.iter_mut().enumerate() {
      let (u, v) = area.relative_position(origin.x + col, origin.y + row);
      *pixel = ops.iter().fold(map.apply(pixel), |rgb, op| op.apply(rgb, u, v));
    }
  });
  Ok(())
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::{
    camera_profile::{CameraCalibration, CameraProfile},
    imgop::{
      Dim2,
      matrix::{FlatMatrix, transform_2d},
    },
  };

  /// ForwardMatrix of the Adobe Standard profile for the Phase One IQ3 100MP
  const FORWARD_MATRIX: [[f32; 3]; 3] = [[0.8296, 0.0538, 0.0809], [0.3137, 0.8399, -0.1536], [0.0133, -0.4061, 1.2179]];
  const COLOR_MATRIX: [[f32; 3]; 3] = [[0.6722, -0.0635, -0.0963], [-0.4287, 1.2460, 0.2028], [-0.0908, 0.2162, 0.5668]];
  const CALIBRATION: [[f32; 3]; 3] = [[1.0, 0.1, 0.0], [0.0, 0.9, 0.05], [0.0, 0.0, 1.1]];

  /// Profile with a D65 ColorMatrix and `forward_matrix`
  fn d65_profile(color_matrix: FlatMatrix, forward_matrix: Option<FlatMatrix>) -> CameraProfile {
    CameraProfile {
      calibration_illuminant: [Some(Illuminant::D65), None, None],
      color_matrix: [Some(color_matrix), None, None],
      forward_matrix: [forward_matrix, None, None],
      ..Default::default()
    }
  }

  /// Camera to sRGB for the D65 slot of `profile` with `calibration`
  fn camera_to_srgb<const N: usize>(profile: &CameraProfile, calibration: Option<FlatMatrix>, analog_balance: Option<&[f32]>, wb: &[f32; N]) -> [[f32; N]; 3] {
    let calibration = CameraCalibration {
      matrices: [calibration, None, None],
      signature: None,
    };
    camera_to_output(profile.calibration_slot(0, Some(&calibration)).as_ref(), analog_balance, wb, OutputSpace::Srgb)
  }

  fn assert_matrix_eq<const N: usize>(a: &[[f32; N]; 3], b: &[[f32; N]; 3]) {
    for (x, y) in a.as_flattened().iter().zip(b.as_flattened()) {
      assert!((x - y).abs() < 1e-4, "{:?} != {:?}", a, b);
    }
  }

  /// The white balanced camera neutral is the sRGB white
  fn assert_neutral_is_white<const N: usize>(cam2rgb: &[[f32; N]; 3]) {
    for v in multiply_vector(cam2rgb, &[1.0; N]) {
      assert!((v - 1.0).abs() < 1e-3, "{:?}", cam2rgb);
    }
  }

  #[test]
  fn forward_matrix_maps_to_xyz_d50() {
    let profile = d65_profile(transform_2d(&COLOR_MATRIX), Some(transform_2d(&FORWARD_MATRIX)));
    let cam2rgb = camera_to_srgb(&profile, None, None, &[2.0, 1.0, 1.5]);
    let expected = multiply(
      &multiply(&XYZ_TO_SRGB_D65, &bradford_adaption_matrix(&Illuminant::D50, &Illuminant::D65)),
      &FORWARD_MATRIX,
    );
    assert_matrix_eq(&cam2rgb, &expected);
    assert_neutral_is_white(&cam2rgb);
  }

  #[test]
  fn forward_matrix_with_calibration() {
    let wb = [2.0, 1.0, 1.5];
    let profile = d65_profile(transform_2d(&COLOR_MATRIX), Some(transform_2d(&FORWARD_MATRIX)));
    let plain = camera_to_srgb(&profile, None, None, &wb);
    // A diagonal AB * CC is removed by the white balance
    let diagonal_calibration = camera_to_srgb(&profile, Some(transform_2d(&diagonal(&[1.1, 1.0, 0.9]))), Some(&[1.2, 1.0, 0.8]), &wb);
    assert_matrix_eq(&diagonal_calibration, &plain);
    // A full CameraCalibration changes the colors, the neutral stays white
    let calibrated = camera_to_srgb(&profile, Some(transform_2d(&CALIBRATION)), Some(&[1.2, 1.0, 0.8]), &wb);
    assert_neutral_is_white(&calibrated);
    assert!(calibrated.as_flattened().iter().zip(plain.as_flattened()).any(|(a, b)| (a - b).abs() > 0.01));
  }

  #[test]
  fn color_matrix_with_calibration() {
    let wb = [2.0, 1.0, 1.5];
    let profile = d65_profile(transform_2d(&COLOR_MATRIX), None);
    // Without calibration, the matrix is calculated as before
    let plain = camera_to_srgb(&profile, None, None, &wb);
    assert_eq!(plain, pseudo_inverse(normalize(multiply(&COLOR_MATRIX, &SRGB_TO_XYZ_D65))));
    let diagonal_calibration = camera_to_srgb(&profile, Some(transform_2d(&diagonal(&[1.1, 1.0, 0.9]))), Some(&[1.2, 1.0, 0.8]), &wb);
    assert_matrix_eq(&diagonal_calibration, &plain);
    // XYZtoCamera = AB * CC * CM
    let calibrated = camera_to_srgb(&profile, Some(transform_2d(&CALIBRATION)), Some(&[1.2, 1.0, 0.8]), &wb);
    let xyz2cam = multiply(&diagonal(&[1.2, 1.0, 0.8]), &multiply(&CALIBRATION, &COLOR_MATRIX));
    assert_matrix_eq(&calibrated, &pseudo_inverse(normalize(multiply(&xyz2cam, &SRGB_TO_XYZ_D65))));
    assert_neutral_is_white(&calibrated);
  }

  #[test]
  fn forward_matrix_with_four_color_planes() {
    let forward_matrix = [[0.4, 0.2, 0.3, 0.0644], [0.2, 0.5, 0.1, 0.2], [0.1, 0.0, 0.6, 0.1252]];
    let profile = d65_profile(vec![1.0; 12], Some(transform_2d(&forward_matrix)));
    let cam2rgb = camera_to_srgb(&profile, None, Some(&[1.0, 1.0, 1.0, 2.0]), &[2.0, 1.0, 1.5, 1.2]);
    let expected = multiply(
      &multiply(&XYZ_TO_SRGB_D65, &bradford_adaption_matrix(&Illuminant::D50, &Illuminant::D65)),
      &forward_matrix,
    );
    assert_matrix_eq(&cam2rgb, &expected);
  }

  #[test]
  fn camera_to_rgb_on_plain_buffers() -> crate::Result<()> {
    // Two pixels of four planes: the planes are white balanced and limited to
    // 0.9, the last row of the matrix averages them
    let samples = [0.1, 0.2, 0.3, 0.4, 0.5, 0.5, 0.5, 0.5_f32];
    let map = CameraToRgb {
      wb: [2.0, 1.0, 1.0, 1.0],
      white: 0.9,
      matrix: [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 1.0, 0.0], [0.25; 4]],
    };
    let rgb = camera_to_rgb(
      ImageView::new(&samples, 2, 1, 4),
      &map,
      &[PixelOp::Clip],
      Rect::new(Point::default(), Dim2::new(2, 1)),
      Point::default(),
    )?;
    assert_eq!(rgb.data, [[0.2, 0.5, 0.275], [0.9, 1.0, 0.6]]);
    // The image must have four channels
    assert!(camera_to_rgb(ImageView::new(&samples[..6], 2, 1, 3), &map, &[], Rect::default(), Point::default()).is_err());

    // In place for three channels, the same map applies
    let mut samples = [0.1, 0.2, 0.3, 0.5, 0.5, 0.5_f32];
    let map = CameraToRgb {
      wb: [2.0, 1.0, 1.0],
      white: f32::INFINITY,
      matrix: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
    };
    camera_to_rgb_in_place(ImageViewMut::new(&mut samples, 2, 1, 3), &map, &[], Rect::default(), Point::default())?;
    assert_eq!(samples, [0.2, 0.2, 0.3, 1.0, 0.5, 0.5]);
    Ok(())
  }

  #[test]
  fn output_spaces() {
    for space in [OutputSpace::Srgb, OutputSpace::AdobeRgb, OutputSpace::ProPhotoRgb] {
      // The matrices are inverse, RGB white is D65
      let identity = multiply(&space.xyz_to_rgb(), &space.rgb_to_xyz());
      for (i, row) in identity.iter().enumerate() {
        assert!(
          row.iter().enumerate().all(|(j, v)| (v - if i == j { 1.0 } else { 0.0 }).abs() < 1e-4),
          "{:?}: {:?}",
          space,
          identity
        );
      }
      let white = multiply_vector(&space.rgb_to_xyz(), &[1.0; 3]);
      assert!(
        white.iter().zip(crate::imgop::xyz::CIE_1931_TRISTIMULUS_D65).all(|(a, b)| (a - b).abs() < 1e-3),
        "{:?}: {:?}",
        space,
        white
      );
      // Decoding inverts the encoding
      let function = space.transfer_function();
      for v in [0.0, 0.001, 0.01, 0.2, 0.5, 1.0] {
        assert!((function.decode(function.encode(v)) - v).abs() < 1e-5, "{:?} {}", function, v);
      }
    }
  }

  #[test]
  fn transfer_functions() {
    let close = |a: f32, b: f32| (a - b).abs() < 1e-6;
    for function in [
      TransferFunction::Linear,
      TransferFunction::Srgb,
      TransferFunction::AdobeRgb,
      TransferFunction::ProPhotoRgb,
    ] {
      assert_eq!(function.encode(0.0), 0.0, "{:?}", function);
      assert!(close(function.encode(1.0), 1.0), "{:?}", function);
    }
    // Adobe RGB is a power function, negative values are mirrored
    assert!(close(TransferFunction::AdobeRgb.encode(0.5), 0.5_f32.powf(256.0 / 563.0)));
    assert!(close(TransferFunction::AdobeRgb.encode(-0.5), -TransferFunction::AdobeRgb.encode(0.5)));
    // Both segments of ProPhoto RGB meet at 1/512
    let end = PROPHOTO_RGB_LINEAR_END;
    assert!(close(TransferFunction::ProPhotoRgb.encode(end), 1.0 / 32.0));
    assert!(close(TransferFunction::ProPhotoRgb.encode(end * 0.999_99), 1.0 / 32.0));
    assert!(close(TransferFunction::ProPhotoRgb.encode(0.001), 0.016));
    assert!(close(TransferFunction::ProPhotoRgb.encode(0.5), 0.5_f32.powf(1.0 / 1.8)));
  }

  #[test]
  fn gain_and_transfer_function_on_plain_buffers() {
    let mut samples = vec![0.0, 0.25, 0.5_f32];
    apply_gain(ImageViewMut::new(&mut samples, 3, 1, 1), 2.0);
    assert_eq!(samples, [0.0, 0.5, 1.0]);
    apply_transfer_function(ImageViewMut::new(&mut samples, 3, 1, 1), TransferFunction::Srgb);
    assert_eq!(samples, [0.0, srgb_apply_gamma(0.5), srgb_apply_gamma(1.0)]);
  }
}
