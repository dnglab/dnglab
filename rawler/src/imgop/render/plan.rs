// SPDX-License-Identifier: LGPL-2.1
// Copyright 2026 Daniel Vogelbacher <daniel@chaospixel.com>

//! Planning of the processing operations for an image

use super::{DemosaicQuality, ProcessingStep, RenderOptions, RenderTarget, WhiteBalancePlacement};
use crate::{
  RawImage, RawImageData,
  dng::{
    opcodes::OpcodeStage,
    profile_render::{ProfileRender, ToneCurve, prophoto_to_srgb, srgb_to_prophoto},
  },
  imgop::{
    color::{CameraToRgb, OutputSpace, PixelOp, TransferFunction, camera_to_output},
    matrix::multiply,
    raw::WhiteBalance,
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
  /// Map integer CFA data to linear reference values and white balance it
  RescaleMosaic { cfa: Box<CFAConfig>, white_balance: Option<WhiteBalance> },
  /// White balance CFA data
  WhiteBalanceMosaic { cfa: Box<CFAConfig>, white_balance: WhiteBalance },
  /// White balance color planes
  WhiteBalance(WhiteBalance),
  /// Demosaic CFA data, cropped to the ActiveArea with `crop_to_active_area`.
  /// With `crop_default`, only the DefaultCrop and a margin around it for the
  /// interpolation are demosaiced, then the image is cropped to it.
  Demosaic {
    algorithm: DemosaicAlgorithm,
    cfa: Box<CFAConfig>,
    crop_to_active_area: bool,
    crop_default: bool,
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
  /// Crop to the ActiveArea
  CropActiveArea,
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
  Markesteijn,
  Superpixel3,
  Superpixel4,
}

impl DemosaicAlgorithm {
  /// The result has half the width and height of the mosaic data
  pub(super) fn half_size(self) -> bool {
    matches!(self, Self::Superpixel3 | Self::Superpixel4)
  }
}

/// Plan the processing of `rawimage` with `options`
///
/// Fails for image data that can't be processed, e.g. an unsupported CFA.
pub fn plan(rawimage: &RawImage, options: &RenderOptions) -> crate::Result<Plan> {
  use ProcessingStep as Step;

  let steps = options.steps;
  let linear_raw = options.target == RenderTarget::LinearRaw;
  let mut ops = Vec::new();
  let has_opcodes = |stage| rawimage.opcode_list(stage).is_some();
  if !matches!(rawimage.cpp, 1 | 3 | 4) {
    return Err(format!("Images with {} color planes are not supported", rawimage.cpp).into());
  }

  // CFA data that is demosaiced, its demosaic algorithm and the number of
  // color planes after the demosaic
  let mosaic = match &rawimage.photometric {
    RawPhotometricInterpretation::Cfa(config) if steps.contains(Step::Demosaic) && rawimage.cpp == 1 => Some(config),
    _ => None,
  };
  // Images of rotated sensors are rotated at full size
  let quality = match options.demosaic {
    DemosaicQuality::Preview if rawimage.fuji_rotation_width.is_some() && steps.contains(Step::FujiRotate) => DemosaicQuality::Fast,
    quality => quality,
  };
  let demosaic = mosaic.map(|config| demosaic_algorithm(config, quality)).transpose()?;
  let channels = demosaic.map_or(rawimage.cpp, |(_, planes)| planes);

  // As shot white balance, some old images may not provide WB coeffs. White
  // balanced values are limited to the camera white, the smallest coefficient.
  let planes = channels.min(4);
  let valid = rawimage.wb_coeffs[..planes].iter().all(|c| c.is_finite() && *c > 0.0);
  let wb = (steps.contains(Step::WhiteBalance) && valid).then_some(rawimage.wb_coeffs);
  let white = wb.map_or(1.0, |wb| wb[..planes].iter().copied().fold(f32::INFINITY, f32::min));
  // CFA data is white balanced before it is demosaiced, see WhiteBalancePlacement.
  // LinearRaw data is not limited, its white balance is divided out again.
  let mut mosaic_wb = wb
    .filter(|_| mosaic.is_some() && options.white_balance == WhiteBalancePlacement::BeforeDemosaic)
    .filter(|_| linear_raw || steps.contains(Step::Rescale))
    .map(|coeffs| WhiteBalance {
      coeffs,
      clip: (!linear_raw).then_some(white),
    });
  let wb_before_demosaic = mosaic_wb.is_some();

  // OpcodeList1 is applied to the raw image as read from the file
  if has_opcodes(OpcodeStage::List1) {
    ops.push(Op::Opcodes {
      stage: OpcodeStage::List1,
      crop_to_active_area: false,
    });
  }
  if steps.contains(Step::Rescale) {
    // Integer CFA data is mapped and white balanced in one pass, unless a
    // LinearizationTable or OpcodeList2 is applied between
    let fused = matches!(rawimage.data, RawImageData::Integer(_))
      && rawimage.linearization_table.is_none()
      && !has_opcodes(OpcodeStage::List1)
      && !has_opcodes(OpcodeStage::List2);
    match mosaic.filter(|_| fused) {
      Some(config) => ops.push(Op::RescaleMosaic {
        cfa: Box::new(config.clone()),
        white_balance: mosaic_wb.take(),
      }),
      None => {
        ops.push(Op::Rescale);
        // OpcodeList2 is applied just after mapping to linear reference values
        if has_opcodes(OpcodeStage::List2) {
          ops.push(Op::Opcodes {
            stage: OpcodeStage::List2,
            crop_to_active_area: false,
          });
        }
      }
    }
  }
  // White balance that is not applied with the rescale, for LinearRaw data
  // also without the Rescale step
  if let (Some(config), Some(white_balance)) = (mosaic, mosaic_wb) {
    ops.push(Op::WhiteBalanceMosaic {
      cfa: Box::new(config.clone()),
      white_balance,
    });
  }

  let crop_default = steps.contains(Step::CropDefault) && !linear_raw;
  let mut cropped = false;
  if steps.contains(Step::Demosaic) {
    let crop_to_active_area = matches!(rawimage.photometric, RawPhotometricInterpretation::Cfa(_)) && steps.contains(Step::CropActiveArea);
    // OpcodeList3 of LinearRaw data is kept for the DNG reader
    let list3 = has_opcodes(OpcodeStage::List3) && !linear_raw;
    if let (Some(config), Some((algorithm, _))) = (mosaic, demosaic) {
      if crop_to_active_area && rawimage.active_area.is_some() && rawimage.fuji_rotation_width.is_some() {
        return Err("ActiveArea is not possible when rotation is not normalized".into());
      }
      let fuji_rotation_width = rawimage
        .fuji_rotation_width
        .filter(|_| algorithm == DemosaicAlgorithm::Ppg && steps.contains(Step::FujiRotate));
      // Without operations on the whole stage 3 image, only the DefaultCrop is demosaiced
      cropped = crop_default && fuji_rotation_width.is_none() && !list3;
      ops.push(Op::Demosaic {
        algorithm,
        cfa: Box::new(config.clone()),
        crop_to_active_area,
        crop_default: cropped,
      });
      if let Some(width) = fuji_rotation_width {
        let extra_rotate = rawimage.camera.find_hint("fuji_rotate_90cw");
        ops.push(Op::FujiRotate { width, extra_rotate });
      }
    }
    // OpcodeList3 is applied just after demosaicing
    if list3 {
      ops.push(Op::Opcodes {
        stage: OpcodeStage::List3,
        crop_to_active_area,
      });
    }
  }
  if linear_raw {
    // The white balance for the demosaic is divided out, DNG readers apply it
    if let Some(coeffs) = wb.filter(|_| wb_before_demosaic) {
      ops.push(Op::WhiteBalance(WhiteBalance {
        coeffs: coeffs.map(|c| 1.0 / c),
        clip: None,
      }));
    }
    return Ok(Plan { ops });
  }
  // Cropping before the color operations saves their work for the cropped pixels
  if crop_default && !cropped {
    ops.push(Op::CropDefault);
  } else if !crop_default && demosaic.is_none() && steps.contains(Step::CropActiveArea) {
    ops.push(Op::CropActiveArea);
  }

  // A positive BaselineExposure is applied as linear gain to the scene-linear
  // data, clipping is done by the color space conversion or the final
  // conversion to integers. A negative BaselineExposure is applied as tone
  // curve that keeps white at 1.0, by the rendering of the camera map or as
  // separate operation without it.
  let exposure = if steps.contains(Step::BaselineExposure) {
    rawimage.baseline_exposure.unwrap_or(0.0) + rawimage.baseline_exposure_offset().unwrap_or(0.0)
  } else {
    0.0
  };
  let gain = exposure.max(0.0).exp2();
  log::debug!("BaselineExposure {} EV, gain: {}", exposure, gain);
  let transfer_function = steps.contains(Step::TransferFunction).then(|| options.color_space.transfer_function());
  if steps.contains(Step::Calibrate) && channels > 1 {
    // All operations on the color image are applied in one pass, the gain
    // with the white balance and the transfer function after the rendering
    let neutral = wb.unwrap_or([1.0; 4]);
    // Rendering tables of the camera profile, for the same slot as the matrices,
    // and a negative BaselineExposure
    let tables = steps.contains(Step::ProfileRender);
    let slot = rawimage.best_calibration_slot().filter(|_| tables);
    let gain_table_map = rawimage
      .as_shot_camera_profile_index()
      .and_then(|index| rawimage.effective_gain_table_map(index))
      .filter(|_| tables);
    let render = ProfileRender::new(slot.map(|slot| (slot.profile, slot.slot)), gain_table_map, exposure);
    let (map, mut pixel_ops) = match channels {
      3 => {
        let (map, pixel_ops) = camera_map::<3>(rawimage, neutral, wb_before_demosaic, white, gain, render, options.color_space);
        (CameraMap::Three(map), pixel_ops)
      }
      _ => {
        let (map, pixel_ops) = camera_map::<4>(rawimage, neutral, wb_before_demosaic, white, gain, render, options.color_space);
        (CameraMap::Four(map), pixel_ops)
      }
    };
    pixel_ops.extend(transfer_function.map(PixelOp::TransferFunction));
    ops.push(Op::CameraToRgb(map, pixel_ops));
  } else {
    if let Some(coeffs) = wb.filter(|_| !wb_before_demosaic && channels > 1) {
      ops.push(Op::WhiteBalance(WhiteBalance { coeffs, clip: Some(white) }));
    }
    if exposure > 0.0 {
      ops.push(Op::Gain(gain));
    } else if let Some(curve) = ToneCurve::negative_exposure(exposure) {
      ops.push(Op::ToneCurve(curve));
    }
    if let Some(function) = transfer_function {
      ops.push(Op::TransferFunction(function));
    }
  }
  Ok(Plan { ops })
}

/// Demosaic algorithm for a CFA and the number of color planes of its result
fn demosaic_algorithm(config: &CFAConfig, quality: DemosaicQuality) -> crate::Result<(DemosaicAlgorithm, usize)> {
  match config.sensor {
    SensorType::Bayer if config.cfa.is_rgb() => match quality {
      DemosaicQuality::Preview => Ok((DemosaicAlgorithm::Superpixel3, 3)),
      _ => Ok((DemosaicAlgorithm::Ppg, 3)),
    },
    SensorType::Bayer if config.cfa.unique_colors() == 4 => match quality {
      DemosaicQuality::Preview => Ok((DemosaicAlgorithm::Superpixel4, 4)),
      _ => Ok((DemosaicAlgorithm::Bilinear4, 4)),
    },
    SensorType::Xtrans if config.cfa.is_rgb() => match quality {
      DemosaicQuality::Quality => Ok((DemosaicAlgorithm::Markesteijn, 3)),
      DemosaicQuality::Fast | DemosaicQuality::Preview => Ok((DemosaicAlgorithm::XTransBilinear, 3)),
    },
    _ => Err(format!("Demosaic of CFA pattern '{}' is not supported", config.cfa).into()),
  }
}

/// Map of camera values to `output` and the pixel operations that follow it
///
/// `wb` are the white balance coefficients, the reciprocal of the camera
/// neutral. The map applies them, unless the values are `balanced` already,
/// and the exposure `gain`. Like a DNG reader, white balanced camera values
/// are clipped at the camera `white` (scaled by the gain), so clipped
/// highlights stay neutral, and colors outside of the output space are clipped
/// per channel. With rendering tables, the camera values are mapped to linear
/// ProPhoto RGB for the tables.
fn camera_map<const N: usize>(
  rawimage: &RawImage,
  wb: [f32; 4],
  balanced: bool,
  white: f32,
  gain: f32,
  render: Option<ProfileRender>,
  output: OutputSpace,
) -> (CameraToRgb<N>, Vec<PixelOp>) {
  let neutral: [f32; N] = std::array::from_fn(|i| wb[i]);
  let cam2rgb = camera_to_output(rawimage.best_calibration_slot().as_ref(), rawimage.analog_balance.as_deref(), &neutral, output);
  log::debug!("wb: {:?}, camera to {:?}: {:?}", neutral, output, cam2rgb);
  let wb = if balanced { [gain; N] } else { neutral.map(|c| c * gain) };
  let white = gain * white;
  match render {
    Some(render) => {
      let (to_prophoto, from_prophoto) = match output {
        OutputSpace::Srgb => (srgb_to_prophoto(), prophoto_to_srgb()),
      };
      let map = CameraToRgb {
        wb,
        white,
        matrix: multiply(&to_prophoto, &cam2rgb),
      };
      (map, vec![PixelOp::Profile(render), PixelOp::Matrix(from_prophoto), PixelOp::Clip])
    }
    None => (CameraToRgb { wb, white, matrix: cam2rgb }, vec![PixelOp::Clip]),
  }
}
