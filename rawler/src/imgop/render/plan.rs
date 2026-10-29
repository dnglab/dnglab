// SPDX-License-Identifier: LGPL-2.1
// Copyright 2026 Daniel Vogelbacher <daniel@chaospixel.com>

//! Planning of the processing operations for an image

use super::{ProcessingStep, RenderOptions};
use crate::{
  RawImage,
  dng::{
    opcodes::OpcodeStage,
    profile_render::{ProfileRender, ToneCurve, prophoto_to_srgb, srgb_to_prophoto},
  },
  imgop::{
    color::{CameraToRgb, OutputSpace, PixelOp, TransferFunction, camera_to_output},
    matrix::multiply,
    sensor::SensorType,
  },
  rawimage::{CFAConfig, RawPhotometricInterpretation},
};

/// Processing operations for a raw image, in the order of execution
#[derive(Clone, Debug)]
pub struct Plan {
  pub(super) ops: Vec<Op>,
}

/// Processing operation with its parameters
#[derive(Clone, Debug)]
pub(super) enum Op {
  /// Apply a DNG opcode list to the image data. With `crop_to_active_area`,
  /// the image data stays cropped to an ActiveArea that the list changes.
  Opcodes { stage: OpcodeStage, crop_to_active_area: bool },
  /// Map raw values to linear reference values
  Rescale,
  /// Demosaic CFA data, cropped to the ActiveArea with `crop_to_active_area`
  Demosaic {
    algorithm: DemosaicAlgorithm,
    cfa: Box<CFAConfig>,
    crop_to_active_area: bool,
  },
  /// Rotate an RGB image of a sensor with a 45° layout
  FujiRotate { width: usize, extra_rotate: bool },
  /// Multiply all samples
  Gain(f32),
  /// Apply a tone curve to the image
  ToneCurve(ToneCurve),
  /// Map camera values to RGB, then apply the pixel operations
  CameraToRgb(CameraMap, Vec<PixelOp>),
  /// Crop to the DefaultCrop, or the ActiveArea without DefaultCrop
  CropDefault,
  /// Encode all samples by a transfer function
  TransferFunction(TransferFunction),
}

/// Map of camera values to RGB for three or four color planes
#[derive(Clone, Debug)]
pub(super) enum CameraMap {
  Three(CameraToRgb<3>),
  Four(CameraToRgb<4>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum DemosaicAlgorithm {
  Ppg,
  Bilinear4,
  XTransBilinear,
}

/// Plan the processing of `rawimage` with `options`
///
/// Fails for image data that can't be processed, e.g. an unsupported CFA.
pub fn plan(rawimage: &RawImage, options: &RenderOptions) -> crate::Result<Plan> {
  use ProcessingStep as Step;

  let steps = options.steps;
  let mut ops = Vec::new();
  let has_opcodes = |stage| rawimage.opcode_list(stage).is_some();
  if !matches!(rawimage.cpp, 1 | 3 | 4) {
    return Err(format!("Images with {} color planes are not supported", rawimage.cpp).into());
  }

  // OpcodeList1 is applied to the raw image as read from the file
  if has_opcodes(OpcodeStage::List1) {
    ops.push(Op::Opcodes {
      stage: OpcodeStage::List1,
      crop_to_active_area: false,
    });
  }
  if steps.contains(Step::Rescale) {
    ops.push(Op::Rescale);
    // OpcodeList2 is applied just after mapping to linear reference values
    if has_opcodes(OpcodeStage::List2) {
      ops.push(Op::Opcodes {
        stage: OpcodeStage::List2,
        crop_to_active_area: false,
      });
    }
  }

  let mut channels = rawimage.cpp;
  if steps.contains(Step::Demosaic) {
    let cfa = match &rawimage.photometric {
      RawPhotometricInterpretation::Cfa(config) => Some(config),
      _ => None,
    };
    let crop_to_active_area = cfa.is_some() && steps.contains(Step::CropActiveArea);
    if let Some(config) = cfa
      && rawimage.cpp == 1
    {
      if crop_to_active_area && rawimage.active_area.is_some() && rawimage.fuji_rotation_width.is_some() {
        return Err("ActiveArea is not possible when rotation is not normalized".into());
      }
      let (algorithm, planes) = demosaic_algorithm(config)?;
      ops.push(Op::Demosaic {
        algorithm,
        cfa: Box::new(config.clone()),
        crop_to_active_area,
      });
      channels = planes;
      if algorithm == DemosaicAlgorithm::Ppg
        && steps.contains(Step::FujiRotate)
        && let Some(width) = rawimage.fuji_rotation_width
      {
        let extra_rotate = rawimage.camera.find_hint("fuji_rotate_90cw");
        ops.push(Op::FujiRotate { width, extra_rotate });
      }
    }
    // OpcodeList3 is applied just after demosaicing
    if has_opcodes(OpcodeStage::List3) {
      ops.push(Op::Opcodes {
        stage: OpcodeStage::List3,
        crop_to_active_area,
      });
    }
  }

  // A positive BaselineExposure is applied as linear gain to the scene-linear
  // data, clipping is done by the color space conversion or the final
  // conversion to integers. A negative BaselineExposure is applied as tone
  // curve that keeps white at 1.0, by the rendering of the camera map or here
  // without it.
  let exposure = if steps.contains(Step::BaselineExposure) {
    rawimage.baseline_exposure.unwrap_or(0.0) + rawimage.baseline_exposure_offset().unwrap_or(0.0)
  } else {
    0.0
  };
  let calibrate = steps.contains(Step::Calibrate) && channels > 1;
  if exposure > 0.0 {
    log::debug!("BaselineExposure {} EV, gain: {}", exposure, exposure.exp2());
    ops.push(Op::Gain(exposure.exp2()));
  } else if !calibrate && let Some(curve) = ToneCurve::negative_exposure(exposure) {
    log::debug!("BaselineExposure {} EV as tone curve", exposure);
    ops.push(Op::ToneCurve(curve));
  }

  if calibrate {
    // Some old images may not provide WB coeffs. Assume 1.0 in this case.
    let wb = if steps.contains(Step::WhiteBalance) && !rawimage.wb_coeffs[0].is_nan() {
      rawimage.wb_coeffs
    } else {
      [1.0; 4]
    };
    // Rendering tables of the camera profile, for the same slot as the matrices,
    // and a negative BaselineExposure
    let tables = steps.contains(Step::ProfileRender);
    let slot = rawimage.best_calibration_slot().filter(|_| tables);
    let gain_table_map = rawimage
      .as_shot_camera_profile_index()
      .and_then(|index| rawimage.effective_gain_table_map(index))
      .filter(|_| tables);
    let render = ProfileRender::new(slot.map(|slot| (slot.profile, slot.slot)), gain_table_map, exposure);
    let (map, pixel_ops) = match channels {
      3 => {
        let (map, pixel_ops) = camera_map(rawimage, [wb[0], wb[1], wb[2]], render, options.color_space);
        (CameraMap::Three(map), pixel_ops)
      }
      _ => {
        let (map, pixel_ops) = camera_map(rawimage, wb, render, options.color_space);
        (CameraMap::Four(map), pixel_ops)
      }
    };
    ops.push(Op::CameraToRgb(map, pixel_ops));
  }

  if steps.contains(Step::CropDefault) {
    ops.push(Op::CropDefault);
  }
  if steps.contains(Step::TransferFunction) {
    ops.push(Op::TransferFunction(options.color_space.transfer_function()));
  }
  Ok(Plan { ops })
}

/// Demosaic algorithm for a CFA and the number of color planes of its result
fn demosaic_algorithm(config: &CFAConfig) -> crate::Result<(DemosaicAlgorithm, usize)> {
  match config.sensor {
    SensorType::Bayer if config.cfa.is_rgb() => Ok((DemosaicAlgorithm::Ppg, 3)),
    SensorType::Bayer if config.cfa.unique_colors() == 4 => Ok((DemosaicAlgorithm::Bilinear4, 4)),
    SensorType::Xtrans if config.cfa.is_rgb() => Ok((DemosaicAlgorithm::XTransBilinear, 3)),
    _ => Err(format!("Demosaic of CFA pattern '{}' is not supported", config.cfa).into()),
  }
}

/// Map of white balanced camera values to `output` and the pixel operations
/// that follow it
///
/// With rendering tables, the camera values are mapped to linear ProPhoto RGB
/// for the tables. Like a DNG reader, white balanced camera values are clipped
/// at the camera white (scaled by the gain of a positive BaselineExposure), so
/// clipped highlights stay neutral, and colors outside of the output space are
/// clipped per channel.
fn camera_map<const N: usize>(rawimage: &RawImage, wb: [f32; N], render: Option<ProfileRender>, output: OutputSpace) -> (CameraToRgb<N>, Vec<PixelOp>) {
  let cam2rgb = camera_to_output(rawimage.best_calibration_slot().as_ref(), rawimage.analog_balance.as_deref(), &wb, output);
  log::debug!("wb: {:?}, camera to {:?}: {:?}", wb, output, cam2rgb);
  match render {
    Some(render) => {
      let (to_prophoto, from_prophoto) = match output {
        OutputSpace::Srgb => (srgb_to_prophoto(), prophoto_to_srgb()),
      };
      let white = render.exposure_gain() * wb.iter().copied().fold(f32::INFINITY, f32::min);
      let map = CameraToRgb {
        wb,
        white,
        matrix: multiply(&to_prophoto, &cam2rgb),
      };
      (map, vec![PixelOp::Profile(render), PixelOp::Matrix(from_prophoto), PixelOp::Clip])
    }
    None => {
      let map = CameraToRgb {
        wb,
        white: f32::INFINITY,
        matrix: cam2rgb,
      };
      (map, vec![PixelOp::ClipNormAvg])
    }
  }
}
