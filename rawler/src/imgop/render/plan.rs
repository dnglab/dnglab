// SPDX-License-Identifier: LGPL-2.1
// Copyright 2026 Daniel Vogelbacher <daniel@chaospixel.com>

//! Planning of the processing operations for an image

use super::{BayerDemosaic, DemosaicQuality, ProcessingStep, RenderOptions, RenderTarget, WhiteBalancePlacement, XTransDemosaic};
use crate::RawlerError;
use crate::{
  RawImage,
  dng::{
    opcodes::OpcodeStage,
    profile_render::{ProfileRender, RgbTablesRender, ToneCurve, prophoto_to_rgb, rgb_to_prophoto},
  },
  imgop::{
    balance::auto_white_balance,
    color::{CameraToRgb, OutputSpace, PixelOp, TransferFunction, camera_to_output, d65_white_balance},
    denoise::DenoiseStrength,
    highlights::{HighlightMode, HighlightShoulder},
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
  /// The result has linear values of a High Dynamic Range render
  pub(super) extended_range: bool,
  /// The result has the values of the raw data, without the Rescale step
  pub(super) raw_values: bool,
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
  /// Reduce the noise of linear values. `wb` are the white balance coefficients
  /// to reach neutral colors, all ones if they are already white balanced.
  Denoise { strength: DenoiseStrength, wb: [f32; 4] },
  /// Change the exposure of linear values by `manual` EV and, if `meter_scale` is
  /// there, by an estimate from the image data. `meter_scale` maps them to neutral
  /// colors in the brightness of the image, `scale` to neutral colors with a white
  /// of 1.0: the white balance that is not applied yet and the exposure gain.
  Exposure {
    manual: f32,
    meter_scale: Option<[f32; 4]>,
    scale: [f32; 4],
  },
  /// Sharpen linear values. `scale` maps them to neutral colors with a white of
  /// 1.0: the white balance that is not applied yet and the exposure gain.
  Sharpen { strength: f32, scale: [f32; 4] },
  /// Multiply all samples
  Gain(f32),
  /// Apply a tone curve to the image
  ToneCurve(ToneCurve),
  /// Map camera values to RGB of `space`, then apply the pixel operations
  CameraToRgb { map: CameraMap, ops: Vec<PixelOp>, space: OutputSpace },
  /// Crop to the DefaultCrop, or the ActiveArea without DefaultCrop
  CropDefault,
  /// Crop to the ActiveArea
  CropActiveArea,
  /// Scale the image data by these horizontal and vertical factors
  Scale { horizontal: f64, vertical: f64 },
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
  Ahd,
  Vng,
  Rcd,
  Bilinear,
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
  // Opcode lists that are applied: the image has them and their step is set
  let has_opcodes = |stage| rawimage.opcode_list(stage).is_some() && steps.contains(opcode_list_step(stage));
  if !matches!(rawimage.cpp, 1 | 3 | 4) {
    return Err(input_error!("Images with {} color planes are not supported", rawimage.cpp));
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
  let demosaic = mosaic
    .map(|config| demosaic_algorithm(config, quality, options.bayer_demosaic, options.xtrans_demosaic))
    .transpose()?;
  let channels = demosaic.map_or(rawimage.cpp, |(_, planes)| planes);

  // As shot white balance, images without it use an estimated one or the one
  // for D65 light. Like the camera neutral of DNG readers, the coefficients are
  // normalized to a smallest coefficient of 1.0. White balanced values are
  // limited to 1.0, the camera white, so clipped highlights are white.
  let planes = channels.min(4);
  let auto_wb = steps.contains(Step::AutoWhiteBalance);
  let wb = steps
    .contains(Step::WhiteBalance)
    .then(|| white_balance(rawimage, planes, auto_wb))
    .flatten()
    .map(|coeffs| {
      let smallest = coeffs[..planes].iter().copied().fold(f32::INFINITY, f32::min);
      coeffs.map(|c| c / smallest)
    });
  // CFA data is white balanced before it is demosaiced, see WhiteBalancePlacement.
  // LinearRaw data is not limited, its white balance is divided out again.
  let mut mosaic_wb = wb
    .filter(|_| mosaic.is_some() && options.white_balance == WhiteBalancePlacement::BeforeDemosaic)
    .filter(|_| linear_raw || steps.contains(Step::Rescale))
    .map(|coeffs| WhiteBalance {
      coeffs,
      clip: (!linear_raw).then_some(1.0),
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
    // CFA data is mapped and white balanced in one pass, unless a
    // LinearizationTable or OpcodeList2 is applied between
    let fused = rawimage.linearization_table.is_none() && !has_opcodes(OpcodeStage::List1) && !has_opcodes(OpcodeStage::List2);
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
        return Err(RawlerError::invalid_input("ActiveArea is not possible when rotation is not normalized"));
      }
      // Data of rotated sensors is rotated at full size
      let fuji_rotation_width = rawimage
        .fuji_rotation_width
        .filter(|_| !algorithm.half_size() && steps.contains(Step::FujiRotate));
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
    return Ok(Plan {
      ops,
      extended_range: false,
      raw_values: false,
    });
  }
  // Without the Rescale step, the values stay those of the raw data, unless
  // they are mapped to linear reference values already (a white level of 1)
  let raw_values = !steps.contains(Step::Rescale) && rawimage.whitelevel.0.iter().any(|white| *white != 1);
  // Cropping before the color operations saves their work for the cropped pixels
  if crop_default && !cropped {
    ops.push(Op::CropDefault);
  } else if !crop_default && demosaic.is_none() && steps.contains(Step::CropActiveArea) {
    ops.push(Op::CropActiveArea);
  }
  let cfa_data = matches!(rawimage.photometric, RawPhotometricInterpretation::Cfa(_)) && demosaic.is_none();
  // White balance that is applied after the operations on linear values
  let pending_wb = wb.filter(|_| !wb_before_demosaic).unwrap_or([1.0; 4]);
  // The noise is reduced before scaling changes it, CFA data has no luma and
  // chroma planes. Output-referred data is already rendered.
  if steps.contains(Step::Denoise) && options.denoise.is_active() && !cfa_data {
    if rawimage.colorimetric_reference.is_output_referred() {
      log::debug!("Output-referred data is already rendered, the noise is not reduced");
    } else {
      ops.push(Op::Denoise {
        strength: options.denoise,
        wb: pending_wb,
      });
    }
  }
  // The image is scaled while its values are linear, CFA data can't be scaled
  let (horizontal, vertical) = rawimage.camera.best_quality_scale_factors();
  if steps.contains(Step::DefaultScale) && !cfa_data && (horizontal, vertical) != (1.0, 1.0) {
    ops.push(Op::Scale { horizontal, vertical });
  }

  // A positive BaselineExposure is applied as linear gain to the scene-linear
  // data, the values above white are compressed (RenderOptions::highlights)
  // or clipped before the transfer function, or clipped by the final
  // conversion to integers. A negative BaselineExposure is applied as tone
  // curve that keeps white at 1.0, by the rendering of the camera map or as
  // separate operation without it.
  let exposure = if steps.contains(Step::BaselineExposure) {
    rawimage.baseline_exposure.map_or(0.0, |exposure| exposure.as_f32()) + rawimage.baseline_exposure_offset().unwrap_or(0.0)
  } else {
    0.0
  };
  let gain = exposure.max(0.0).exp2();
  log::debug!("BaselineExposure {} EV, gain: {}", exposure, gain);
  let transfer_function = steps.contains(Step::TransferFunction).then(|| options.color_space.transfer_function());
  // Output-referred data is already rendered
  let default_tone_curve = (steps.contains(Step::ToneCurve) && !rawimage.colorimetric_reference.is_output_referred()).then(ToneCurve::default_render);
  let linear_scale = pending_wb.map(|c| c * gain);
  // The exposure is changed on the pixels of the output while the values are
  // scene-referred. CFA data is no image yet. Output-referred data has its intended
  // brightness, only an explicit exposure changes it.
  if !cfa_data {
    let manual = if steps.contains(Step::Exposure) { options.exposure } else { 0.0 };
    let meter_scale = (steps.contains(Step::AutoExposure) && options.auto_exposure && !rawimage.colorimetric_reference.is_output_referred())
      .then(|| pending_wb.map(|c| c * exposure.exp2()));
    if manual != 0.0 || meter_scale.is_some() {
      ops.push(Op::Exposure {
        manual,
        meter_scale,
        scale: linear_scale,
      });
    }
  }
  // The image is sharpened on the pixels of the output while its values are
  // scene-referred, before the color operations render them. CFA data is no image
  // yet and output-referred data is already rendered.
  if steps.contains(Step::Sharpen) && options.sharpen > 0.0 && !cfa_data {
    if rawimage.colorimetric_reference.is_output_referred() {
      log::debug!("Output-referred data is already rendered, it is not sharpened");
    } else {
      ops.push(Op::Sharpen {
        strength: options.sharpen,
        scale: linear_scale,
      });
    }
  }
  let mut extended_range = false;
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
    // RGBTables apply to the regions of the semantic masks. The masks are not
    // rotated, a rotated image keeps only the background table.
    let rotated = ops.iter().any(|op| matches!(op, Op::FujiRotate { .. }));
    let rgb_tables = rawimage
      .as_shot_camera_profile_index()
      .and_then(|index| rawimage.effective_rgb_tables(index))
      .filter(|_| tables)
      .and_then(|rgb_tables| {
        if rotated && !rawimage.semantic_masks.is_empty() {
          log::warn!("Semantic masks of RGBTables are not used: the image data is rotated");
        }
        RgbTablesRender::new(rgb_tables, if rotated { &[] } else { &rawimage.semantic_masks })
      });
    let render = ProfileRender::new(
      slot.map(|slot| (slot.profile, slot.slot)),
      gain_table_map,
      exposure,
      default_tone_curve,
      options.highlights,
      rgb_tables,
    );
    // Without rendering tables, the values above white are compressed before
    // they are clipped for the transfer function
    let shoulder = (render.is_none() && options.highlights == HighlightMode::Compress)
      .then(|| HighlightShoulder::new(gain))
      .flatten();
    // A High Dynamic Range profile is rendered to linear values without
    // clipping, if the output can hold them
    extended_range = options.extended_range && slot.is_some_and(|slot| slot.profile.is_hdr());
    let transfer_function = transfer_function.filter(|_| !extended_range);
    // Otherwise the output has no values above white
    if !extended_range && let Some(slot) = slot.filter(|slot| slot.profile.is_hdr()) {
      log::warn!(
        "Camera profile '{}' is a High Dynamic Range profile, values above white are clipped in the output",
        slot.profile.name.as_deref().unwrap_or_default()
      );
    }
    let (map, mut pixel_ops) = match channels {
      3 => {
        let (map, pixel_ops) = camera_map::<3>(rawimage, neutral, wb_before_demosaic, gain, render, options.color_space);
        (CameraMap::Three(map), pixel_ops)
      }
      _ => {
        let (map, pixel_ops) = camera_map::<4>(rawimage, neutral, wb_before_demosaic, gain, render, options.color_space);
        (CameraMap::Four(map), pixel_ops)
      }
    };
    // Encoded values are clipped per channel to the output range. Linear
    // values keep highlights above white and colors outside of the output
    // space, the conversion to integers clips them, unless they are written
    // as floating point values.
    if let Some(function) = transfer_function {
      pixel_ops.extend(shoulder.map(PixelOp::Highlights));
      pixel_ops.extend([PixelOp::Clip, PixelOp::TransferFunction(function)]);
    }
    ops.push(Op::CameraToRgb {
      map,
      ops: pixel_ops,
      space: options.color_space,
    });
  } else {
    if let Some(coeffs) = wb.filter(|_| !wb_before_demosaic && channels > 1) {
      ops.push(Op::WhiteBalance(WhiteBalance { coeffs, clip: Some(1.0) }));
    }
    if exposure > 0.0 {
      ops.push(Op::Gain(gain));
    }
    if let Some(curve) = ToneCurve::with_exposure(exposure, default_tone_curve) {
      ops.push(Op::ToneCurve(curve));
    }
    if let Some(function) = transfer_function {
      ops.push(Op::TransferFunction(function));
    }
  }
  Ok(Plan {
    ops,
    extended_range,
    raw_values,
  })
}

/// Processing step that applies the opcode list of `stage`
fn opcode_list_step(stage: OpcodeStage) -> ProcessingStep {
  match stage {
    OpcodeStage::List1 => ProcessingStep::OpcodeList1,
    OpcodeStage::List2 => ProcessingStep::OpcodeList2,
    OpcodeStage::List3 => ProcessingStep::OpcodeList3,
  }
}

/// Demosaic algorithm for a CFA and the number of color planes of its result,
/// `bayer` and `xtrans` are the algorithms for Bayer RGB and X-Trans data at
/// [`DemosaicQuality::Quality`]
fn demosaic_algorithm(config: &CFAConfig, quality: DemosaicQuality, bayer: BayerDemosaic, xtrans: XTransDemosaic) -> crate::Result<(DemosaicAlgorithm, usize)> {
  match config.sensor {
    SensorType::Bayer if config.cfa.is_rgb() => match (quality, bayer) {
      (DemosaicQuality::Preview, _) => Ok((DemosaicAlgorithm::Superpixel3, 3)),
      (DemosaicQuality::Quality, BayerDemosaic::Ahd) => Ok((DemosaicAlgorithm::Ahd, 3)),
      (DemosaicQuality::Quality, BayerDemosaic::Vng) => Ok((DemosaicAlgorithm::Vng, 3)),
      (DemosaicQuality::Quality, BayerDemosaic::Rcd) => Ok((DemosaicAlgorithm::Rcd, 3)),
      (DemosaicQuality::Quality, BayerDemosaic::Bilinear) => Ok((DemosaicAlgorithm::Bilinear, 3)),
      _ => Ok((DemosaicAlgorithm::Ppg, 3)),
    },
    SensorType::Bayer if config.cfa.unique_colors() == 4 => match quality {
      DemosaicQuality::Preview => Ok((DemosaicAlgorithm::Superpixel4, 4)),
      DemosaicQuality::Quality => Ok((DemosaicAlgorithm::Vng, 4)),
      DemosaicQuality::Fast => Ok((DemosaicAlgorithm::Bilinear, 4)),
    },
    SensorType::Xtrans if config.cfa.is_rgb() => match (quality, xtrans) {
      (DemosaicQuality::Quality, XTransDemosaic::Markesteijn) => Ok((DemosaicAlgorithm::Markesteijn, 3)),
      (DemosaicQuality::Quality, XTransDemosaic::Vng) => Ok((DemosaicAlgorithm::Vng, 3)),
      (DemosaicQuality::Fast | DemosaicQuality::Preview, _) => Ok((DemosaicAlgorithm::XTransBilinear, 3)),
    },
    _ => Err(input_error!("Demosaic of CFA pattern '{}' is not supported", config.cfa)),
  }
}

/// White balance coefficients for `planes` color planes: the as shot values
/// or, if the image doesn't provide them, the values estimated from the image
/// data with `auto_wb` or the values for D65 light
fn white_balance(rawimage: &RawImage, planes: usize, auto_wb: bool) -> Option<[f32; 4]> {
  if rawimage.wb_coeffs[..planes].iter().all(|c| c.is_finite() && *c > 0.0) {
    return Some(rawimage.wb_coeffs);
  }
  if let Some(coeffs) = auto_wb.then(|| auto_white_balance(rawimage, planes)).flatten() {
    return Some(coeffs);
  }
  let slot = rawimage.best_calibration_slot()?;
  match planes {
    3 => d65_white_balance::<3>(&slot).map(|[r, g, b]| [r, g, b, f32::NAN]),
    4 => d65_white_balance::<4>(&slot),
    _ => None,
  }
}

/// Map of camera values to `output` and the pixel operations that follow it
///
/// `wb` are the white balance coefficients, the reciprocal of the camera
/// neutral, with a smallest coefficient of 1.0. The map applies them, unless
/// the values are `balanced` already, and the exposure `gain`. Like a DNG
/// reader, white balanced camera values are clipped at the camera white, 1.0
/// scaled by the gain, so clipped highlights stay neutral. The RGB values are
/// not clipped, they can be above 1.0 or negative for colors outside of the
/// output space. With rendering tables, the camera values are mapped to linear
/// ProPhoto RGB for the tables.
fn camera_map<const N: usize>(
  rawimage: &RawImage,
  wb: [f32; 4],
  balanced: bool,
  gain: f32,
  render: Option<ProfileRender>,
  output: OutputSpace,
) -> (CameraToRgb<N>, Vec<PixelOp>) {
  let neutral: [f32; N] = std::array::from_fn(|i| wb[i]);
  let cam2rgb = camera_to_output(rawimage.best_calibration_slot().as_ref(), rawimage.analog_balance.as_deref(), &neutral, output);
  log::debug!("wb: {:?}, camera to {:?}: {:?}", neutral, output, cam2rgb);
  let wb = if balanced { [gain; N] } else { neutral.map(|c| c * gain) };
  let (matrix, pixel_ops) = match render {
    Some(render) => (
      multiply(&rgb_to_prophoto(output), &cam2rgb),
      vec![PixelOp::Profile(render), PixelOp::Matrix(prophoto_to_rgb(output))],
    ),
    None => (cam2rgb, Vec::new()),
  };
  (CameraToRgb { wb, white: gain, matrix }, pixel_ops)
}
