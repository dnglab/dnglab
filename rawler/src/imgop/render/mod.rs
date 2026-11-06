// SPDX-License-Identifier: LGPL-2.1
// Copyright 2026 Daniel Vogelbacher <daniel@chaospixel.com>

//! Rendering of raw images
//!
//! [`plan`] decides which processing operations are applied to a raw image,
//! in which order and with which parameters, from the metadata of the image
//! and the [`RenderOptions`]. Only the estimation of a missing white balance
//! reads the image data. [`execute`] applies the operations of a [`Plan`]
//! to the image data. The operations are public processing steps that work on
//! borrowed image data, e.g. [`camera_to_rgb`](crate::imgop::color::camera_to_rgb).

mod exec;
mod plan;

pub use exec::{execute, execute_raw};
pub use plan::{Plan, plan};

use image::{DynamicImage, ImageBuffer};

use crate::{
  RawImage,
  imgop::{
    Dim2, Rect,
    color::{OutputSpace, TransferFunction},
    convert_from_f32_scaled_u16,
    denoise::DenoiseStrength,
    icc,
  },
  pixarray::{Color2D, ImageRegion, ImageView, ImageViewMut, Pix2D},
};

/// Processing step of the rendering, see [`StepSet`]
#[derive(PartialEq, Eq, Debug, Clone, Copy)]
pub enum ProcessingStep {
  /// Map raw values to linear reference values, apply OpcodeList2
  Rescale,
  /// Demosaic CFA data, apply OpcodeList3
  Demosaic,
  /// Rotate images of sensors with a 45° layout (Fujifilm)
  FujiRotate,
  /// Crop to the ActiveArea: CFA data while it is demosaiced, other data
  /// before the color operations
  CropActiveArea,
  /// Apply BaselineExposure and BaselineExposureOffset
  BaselineExposure,
  /// Apply the as shot white balance
  WhiteBalance,
  /// Estimate the white balance from the image data if the as shot white
  /// balance is not known, instead of using the one for D65 light
  AutoWhiteBalance,
  /// Map camera values to the output color space
  Calibrate,
  /// Apply the rendering tables of the camera profile (part of Calibrate)
  ProfileRender,
  /// Apply the default tone curve. Without effect for output-referred data,
  /// which is already rendered, and for a camera profile with a tone curve,
  /// which replaces it with the ProfileRender step.
  ToneCurve,
  /// Crop to the DefaultCrop, or the ActiveArea without DefaultCrop
  CropDefault,
  /// Reduce the noise of demosaiced data by [`RenderOptions::denoise`]. Without
  /// effect for output-referred data, which is already rendered.
  Denoise,
  /// Scale the image to square pixels by DefaultScale and to the size of the
  /// best quality by BestQualityScale
  DefaultScale,
  /// Encode the values by the transfer function of the output color space
  TransferFunction,
}

impl ProcessingStep {
  /// All steps
  pub const ALL: [Self; 14] = [
    Self::Rescale,
    Self::Demosaic,
    Self::FujiRotate,
    Self::CropActiveArea,
    Self::BaselineExposure,
    Self::WhiteBalance,
    Self::AutoWhiteBalance,
    Self::Calibrate,
    Self::ProfileRender,
    Self::ToneCurve,
    Self::CropDefault,
    Self::Denoise,
    Self::DefaultScale,
    Self::TransferFunction,
  ];
}

/// Set of processing steps
#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub struct StepSet(u64);

impl StepSet {
  /// All processing steps
  pub fn all() -> Self {
    Self::from_iter(ProcessingStep::ALL)
  }

  pub fn contains(self, step: ProcessingStep) -> bool {
    self.0 & Self::bit(step) != 0
  }

  pub fn insert(&mut self, step: ProcessingStep) {
    self.0 |= Self::bit(step);
  }

  pub fn remove(&mut self, step: ProcessingStep) {
    self.0 &= !Self::bit(step);
  }

  /// The set with `step`
  pub fn with(mut self, step: ProcessingStep) -> Self {
    self.insert(step);
    self
  }

  /// The set without `step`
  pub fn without(mut self, step: ProcessingStep) -> Self {
    self.remove(step);
    self
  }

  fn bit(step: ProcessingStep) -> u64 {
    1 << step as u64
  }
}

impl FromIterator<ProcessingStep> for StepSet {
  fn from_iter<I: IntoIterator<Item = ProcessingStep>>(steps: I) -> Self {
    let mut set = Self::default();
    steps.into_iter().for_each(|step| set.insert(step));
    set
  }
}

impl std::fmt::Debug for StepSet {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    f.debug_set()
      .entries(ProcessingStep::ALL.into_iter().filter(|step| self.contains(*step)))
      .finish()
  }
}

/// Position of the white balance in the processing
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum WhiteBalancePlacement {
  /// CFA data is white balanced before it is demosaiced. Demosaic algorithms
  /// that interpolate by the differences of the color planes work better on
  /// white balanced data, and clipped highlights stay neutral.
  #[default]
  BeforeDemosaic,
  /// The white balance is applied after demosaicing, with the map of the
  /// camera values to the output color space, as defined by DNG
  AfterDemosaic,
}

/// Result of the rendering
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RenderTarget {
  /// Image of the processing steps
  #[default]
  Image,
  /// LinearRaw data of the stage 3 image, like in a DNG file: CFA data is
  /// demosaiced (cropped to the ActiveArea with the CropActiveArea step), the
  /// values are neither white balanced nor clipped. OpcodeList3, the color
  /// operations and the DefaultCrop are not applied.
  LinearRaw,
}

/// Quality of the demosaic
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DemosaicQuality {
  /// PPG for Bayer, Markesteijn (3 passes) for X-Trans data
  #[default]
  Quality,
  /// PPG for Bayer, bilinear interpolation for X-Trans data
  Fast,
  /// Half size for previews: each 2x2 block of Bayer data is one pixel
  /// (superpixel), bilinear interpolation for X-Trans data. Images of rotated
  /// sensors (Fujifilm) are demosaiced like Fast.
  Preview,
}

/// Options of the rendering
#[derive(Clone, Debug, PartialEq)]
pub struct RenderOptions {
  /// Result of the rendering
  pub target: RenderTarget,
  /// Processing steps to apply
  pub steps: StepSet,
  /// Position of the WhiteBalance step
  pub white_balance: WhiteBalancePlacement,
  /// Quality of the Demosaic step
  pub demosaic: DemosaicQuality,
  /// Color space of the output: the Calibrate step maps the camera values to
  /// it, the TransferFunction step encodes the values by its transfer function
  pub color_space: OutputSpace,
  /// Strength of the Denoise step
  pub denoise: DenoiseStrength,
}

impl Default for RenderOptions {
  fn default() -> Self {
    Self {
      target: RenderTarget::default(),
      steps: StepSet::all(),
      white_balance: WhiteBalancePlacement::default(),
      demosaic: DemosaicQuality::default(),
      color_space: OutputSpace::default(),
      denoise: DenoiseStrength::default(),
    }
  }
}

/// Rendered image data with `channels` interleaved samples per pixel
#[derive(Clone, Default)]
pub struct RenderedImage {
  pub data: Vec<f32>,
  pub width: usize,
  pub height: usize,
  pub channels: usize,
  /// Color space of RGB data, `None` for camera values
  pub color_space: Option<OutputSpace>,
  /// Transfer function that encodes the values
  pub transfer_function: TransferFunction,
  /// Opacity of the pixels from the transparency mask, 0.0 (transparent) to
  /// 1.0 (opaque). The values in `data` are not matted.
  pub alpha: Option<Vec<f32>>,
}

impl RenderedImage {
  pub fn view(&self) -> ImageView<'_, f32> {
    ImageView::new(&self.data, self.width, self.height, self.channels)
  }

  pub fn view_mut(&mut self) -> ImageViewMut<'_, f32> {
    ImageViewMut::new(&mut self.data, self.width, self.height, self.channels)
  }

  pub fn dim(&self) -> Dim2 {
    Dim2::new(self.width, self.height)
  }

  /// Crop to `area`, in place
  pub fn crop(&mut self, area: Rect) -> crate::Result<()> {
    let dim = self.dim();
    let len = self.view_mut().crop_in_place(area)?.as_slice().len();
    self.data.truncate(len);
    if let Some(alpha) = &mut self.alpha {
      *alpha = ImageRegion::new(alpha, dim.w, dim.h, 1, area).to_vec();
    }
    (self.width, self.height) = (area.d.w, area.d.h);
    Ok(())
  }

  /// ICC profile of the image data: of its color space, or of gray values for
  /// an image with one channel. `None` for camera values.
  pub fn icc_profile(&self) -> Option<Vec<u8>> {
    match (self.color_space, self.channels) {
      (Some(space), 3) => Some(icc::rgb_profile(space, self.transfer_function)),
      (None, 1) => Some(icc::gray_profile(self.transfer_function)),
      _ => None,
    }
  }

  /// Image of 16 bit integers, values of 0.0 to 1.0 are scaled to the full range
  ///
  /// Images with one, three (RGB) or four (RGBA) channels are supported.
  pub fn to_dynamic_image(&self) -> Option<DynamicImage> {
    let data = convert_from_f32_scaled_u16(&self.data, 0, u16::MAX);
    let (width, height) = (self.width as u32, self.height as u32);
    Some(match self.channels {
      1 => DynamicImage::ImageLuma16(ImageBuffer::from_raw(width, height, data)?),
      3 => DynamicImage::ImageRgb16(ImageBuffer::from_raw(width, height, data)?),
      4 => DynamicImage::ImageRgba16(ImageBuffer::from_raw(width, height, data)?),
      _ => return None,
    })
  }
}

impl std::fmt::Debug for RenderedImage {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    write!(f, "RenderedImage({}x{}, {} channels)", self.width, self.height, self.channels)
  }
}

impl From<Pix2D<f32>> for RenderedImage {
  fn from(pixels: Pix2D<f32>) -> Self {
    let (width, height) = (pixels.width, pixels.height);
    Self {
      data: pixels.into_inner(),
      width,
      height,
      channels: 1,
      ..Default::default()
    }
  }
}

impl<const N: usize> From<Color2D<f32, N>> for RenderedImage {
  fn from(pixels: Color2D<f32, N>) -> Self {
    let (width, height) = (pixels.width, pixels.height);
    Self {
      data: pixels.into_flatten(),
      width,
      height,
      channels: N,
      ..Default::default()
    }
  }
}

/// Render `rawimage` with `options`, see [`plan`] and [`execute`]
pub fn render(rawimage: RawImage, options: &RenderOptions) -> crate::Result<RenderedImage> {
  let plan = plan(&rawimage, options)?;
  execute(rawimage, &plan)
}

/// LinearRaw data of `rawimage` at stage 3, with its metadata, see
/// [`RenderTarget::LinearRaw`] and [`execute_raw`]
///
/// Without the Rescale step, the image data must already be mapped to linear
/// reference values. OpcodeList3 is kept in the raw image.
pub fn render_linear_raw(rawimage: RawImage, options: &RenderOptions) -> crate::Result<RawImage> {
  let options = RenderOptions {
    target: RenderTarget::LinearRaw,
    ..options.clone()
  };
  let plan = plan(&rawimage, &options)?;
  execute_raw(rawimage, &plan)
}

#[cfg(test)]
mod tests {
  use super::{
    plan::{CameraMap, DemosaicAlgorithm, Op},
    *,
  };
  use crate::imgop::color::{PixelOp, TransferFunction};
  use crate::{
    CFA, RawImageData,
    decoders::Camera,
    dng::opcodes::{OpcodeList, OpcodeStage},
    formats::tiff::SRational,
    imgop::Point,
    pixarray::PixU16,
    rawimage::{BlackLevel, CFAConfig, RawPhotometricInterpretation, WhiteLevel},
  };

  fn op_names(plan: &Plan) -> Vec<&'static str> {
    plan
      .ops
      .iter()
      .map(|op| match op {
        Op::Opcodes { stage: OpcodeStage::List1, .. } => "List1",
        Op::Opcodes { stage: OpcodeStage::List2, .. } => "List2",
        Op::Opcodes { stage: OpcodeStage::List3, .. } => "List3",
        Op::Rescale => "Rescale",
        Op::RescaleMosaic { .. } => "RescaleMosaic",
        Op::WhiteBalanceMosaic { .. } => "WhiteBalanceMosaic",
        Op::WhiteBalance(_) => "WhiteBalance",
        Op::Demosaic { .. } => "Demosaic",
        Op::FujiRotate { .. } => "FujiRotate",
        Op::Denoise { .. } => "Denoise",
        Op::Gain(_) => "Gain",
        Op::ToneCurve(_) => "ToneCurve",
        Op::CameraToRgb { .. } => "CameraToRgb",
        Op::CropDefault => "CropDefault",
        Op::CropActiveArea => "CropActiveArea",
        Op::Scale { .. } => "Scale",
        Op::TransferFunction(_) => "TransferFunction",
      })
      .collect()
  }

  fn rggb_rawimage() -> RawImage {
    let mut camera = Camera::new();
    camera.cfa = CFA::new("RGGB");
    let photometric = RawPhotometricInterpretation::Cfa(CFAConfig::new_from_camera(&camera));
    RawImage::new(
      camera,
      PixU16::new_with(vec![1000; 8 * 6], 8, 6),
      1,
      [2.0, 1.0, 1.5, f32::NAN],
      photometric,
      Some(BlackLevel::zero(2, 2, 1)),
      Some(WhiteLevel::new_bits(12, 1)),
      false,
    )
  }

  /// LinearRaw image whose samples are their index
  fn linear_raw_rawimage(width: usize, height: usize, cpp: usize) -> RawImage {
    RawImage::new_with_data(
      Camera::default(),
      RawImageData::Float((0..width * height * cpp).map(|i| i as f32).collect()),
      width * cpp,
      height,
      cpp,
      [f32::NAN; 4],
      RawPhotometricInterpretation::LinearRaw,
      Some(BlackLevel::zero(1, 1, cpp)),
      Some(WhiteLevel::new(vec![1; cpp])),
      false,
    )
  }

  #[test]
  fn plan_for_cfa_image() -> crate::Result<()> {
    let mut rawimage = rggb_rawimage();
    rawimage.dng_opcodes_3 = Some(OpcodeList::new(Vec::new()));
    rawimage.baseline_exposure = Some(SRational::new(1, 2));
    let plan = plan(&rawimage, &RenderOptions::default())?;
    // The crop follows OpcodeList3, the gain and the transfer function are part of the color map
    assert_eq!(op_names(&plan), ["RescaleMosaic", "Demosaic", "List3", "CropDefault", "Denoise", "CameraToRgb"]);
    // OpcodeList3 keeps the data cropped to the ActiveArea, like the demosaic
    assert!(plan.ops.iter().any(|op| matches!(op, Op::Opcodes { crop_to_active_area: true, .. })));
    let Some(Op::CameraToRgb { ops: pixel_ops, .. }) = plan.ops.last() else {
      panic!("color map expected");
    };
    assert_eq!(pixel_ops.last(), Some(&PixelOp::TransferFunction(TransferFunction::Srgb)));
    Ok(())
  }

  #[test]
  fn plan_clips_encoded_values_only() -> crate::Result<()> {
    let pixel_ops = |options: &RenderOptions| -> crate::Result<Vec<PixelOp>> {
      match plan(&rggb_rawimage(), options)?.ops.pop() {
        Some(Op::CameraToRgb { ops, .. }) => Ok(ops),
        _ => panic!("color map expected"),
      }
    };
    let steps = StepSet::all().without(ProcessingStep::ToneCurve);
    // Values are clipped before they are encoded
    let encoded = pixel_ops(&RenderOptions { steps, ..Default::default() })?;
    assert_eq!(encoded, [PixelOp::Clip, PixelOp::TransferFunction(TransferFunction::Srgb)]);
    // Linear values keep highlights above white and colors outside of the output space
    let linear = RenderOptions {
      steps: steps.without(ProcessingStep::TransferFunction),
      ..Default::default()
    };
    assert!(pixel_ops(&linear)?.is_empty());
    Ok(())
  }

  #[test]
  fn plan_crops_in_demosaic() -> crate::Result<()> {
    // Without OpcodeList3, only the DefaultCrop is demosaiced
    let cropped = plan(&rggb_rawimage(), &RenderOptions::default())?;
    assert_eq!(op_names(&cropped), ["RescaleMosaic", "Demosaic", "Denoise", "CameraToRgb"]);
    assert!(matches!(cropped.ops[1], Op::Demosaic { crop_default: true, .. }));
    let options = RenderOptions {
      steps: StepSet::all().without(ProcessingStep::CropDefault),
      ..Default::default()
    };
    assert!(matches!(plan(&rggb_rawimage(), &options)?.ops[1], Op::Demosaic { crop_default: false, .. }));
    Ok(())
  }

  #[test]
  fn plan_for_negative_exposure() -> crate::Result<()> {
    // The color map renders the curve, without it a separate tone curve is applied
    let mut rawimage = rggb_rawimage();
    rawimage.baseline_exposure = Some(SRational::new(-1, 1));
    assert_eq!(
      op_names(&plan(&rawimage, &RenderOptions::default())?),
      ["RescaleMosaic", "Demosaic", "Denoise", "CameraToRgb"]
    );
    let options = RenderOptions {
      steps: StepSet::all().without(ProcessingStep::Calibrate),
      ..Default::default()
    };
    assert_eq!(
      op_names(&plan(&rawimage, &options)?),
      ["RescaleMosaic", "Demosaic", "Denoise", "ToneCurve", "TransferFunction"]
    );
    // Monochrome images have no color map
    let mut mono = linear_raw_rawimage(4, 2, 1);
    mono.photometric = RawPhotometricInterpretation::BlackIsZero;
    mono.baseline_exposure = Some(SRational::new(-1, 1));
    assert_eq!(
      op_names(&plan(&mono, &RenderOptions::default())?),
      ["Rescale", "CropDefault", "Denoise", "ToneCurve", "TransferFunction"]
    );
    Ok(())
  }

  #[test]
  fn plan_applies_default_tone_curve() -> crate::Result<()> {
    use crate::{camera_profile::CameraProfile, dng::profile_render::ToneCurve, imgop::xyz::Illuminant, rawimage::ColorimetricReference};

    /// Tone curve of the rendering of the color map
    fn map_curve(rawimage: &RawImage, options: &RenderOptions) -> crate::Result<Option<ToneCurve>> {
      let mut plan = plan(rawimage, options)?;
      let Some(Op::CameraToRgb { ops, .. }) = plan.ops.pop() else {
        panic!("color map expected");
      };
      Ok(ops.into_iter().find_map(|op| match op {
        PixelOp::Profile(render) => render.tone_curve,
        _ => None,
      }))
    }

    let all = RenderOptions::default();
    let without = |step: ProcessingStep| RenderOptions {
      steps: StepSet::all().without(step),
      ..Default::default()
    };
    let default_curve = ToneCurve::default_render();
    assert!(all.steps.contains(ProcessingStep::ToneCurve));

    // On by default, and the step removes it
    assert_eq!(map_curve(&rggb_rawimage(), &all)?, Some(default_curve.clone()));
    assert_eq!(map_curve(&rggb_rawimage(), &without(ProcessingStep::ToneCurve))?, None);

    // Output-referred data is already rendered
    for reference in [ColorimetricReference::OutputReferred, ColorimetricReference::OutputReferredHdr] {
      let mut scan = rggb_rawimage();
      scan.colorimetric_reference = reference;
      assert_eq!(map_curve(&scan, &all)?, None, "{:?}", reference);
    }

    // The curve of a camera profile replaces it, unless the profile is not rendered
    let points = vec![0.0, 0.0, 0.5, 0.6, 1.0, 1.0];
    let mut profiled = rggb_rawimage();
    profiled.camera_profiles = vec![CameraProfile {
      calibration_illuminant: [Some(Illuminant::D65), None, None],
      color_matrix: [Some(vec![1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0]), None, None],
      tone_curve: Some(points.clone()),
      ..Default::default()
    }];
    assert_eq!(map_curve(&profiled, &all)?, ToneCurve::new(&points));
    assert_eq!(map_curve(&profiled, &without(ProcessingStep::ToneCurve))?, ToneCurve::new(&points));
    assert_eq!(map_curve(&profiled, &without(ProcessingStep::ProfileRender))?, Some(default_curve.clone()));

    // A negative BaselineExposure is applied before it
    let mut darkened = rggb_rawimage();
    darkened.baseline_exposure = Some(SRational::new(-1, 1));
    let exposure_curve = ToneCurve::negative_exposure(-1.0).expect("curve");
    assert_eq!(map_curve(&darkened, &all)?, Some(default_curve.after(&exposure_curve)));

    // Without a color map it is a separate operation before the transfer function
    let mut mono = linear_raw_rawimage(4, 2, 1);
    mono.photometric = RawPhotometricInterpretation::BlackIsZero;
    let plan_of_mono = plan(&mono, &all)?;
    assert_eq!(op_names(&plan_of_mono), ["Rescale", "CropDefault", "Denoise", "ToneCurve", "TransferFunction"]);
    assert!(matches!(&plan_of_mono.ops[3], Op::ToneCurve(curve) if *curve == default_curve));
    assert_eq!(
      op_names(&plan(&mono, &without(ProcessingStep::ToneCurve))?),
      ["Rescale", "CropDefault", "Denoise", "TransferFunction"]
    );
    mono.baseline_exposure = Some(SRational::new(-1, 1));
    let plan_of_mono = plan(&mono, &all)?;
    assert_eq!(op_names(&plan_of_mono), ["Rescale", "CropDefault", "Denoise", "ToneCurve", "TransferFunction"]);
    assert!(matches!(&plan_of_mono.ops[3], Op::ToneCurve(curve) if *curve == default_curve.after(&exposure_curve)));

    // LinearRaw data is not rendered
    let linear = RenderOptions {
      target: RenderTarget::LinearRaw,
      ..Default::default()
    };
    assert!(!op_names(&plan(&rggb_rawimage(), &linear)?).contains(&"ToneCurve"));
    assert!(!op_names(&plan(&rggb_rawimage(), &linear)?).contains(&"CameraToRgb"));
    Ok(())
  }

  #[test]
  fn plan_applies_denoise() -> crate::Result<()> {
    use crate::{imgop::denoise::DenoiseStrength, rawimage::ColorimetricReference};

    fn denoise_op(rawimage: &RawImage, options: &RenderOptions) -> crate::Result<Option<(DenoiseStrength, [f32; 4])>> {
      Ok(plan(rawimage, options)?.ops.iter().find_map(|op| match op {
        Op::Denoise { strength, wb } => Some((*strength, *wb)),
        _ => None,
      }))
    }

    let all = RenderOptions::default();
    let default = DenoiseStrength::default();
    assert!(all.steps.contains(ProcessingStep::Denoise) && default.is_active());

    // On by default, and off without the step or without a strength
    assert_eq!(denoise_op(&rggb_rawimage(), &all)?.map(|op| op.0), Some(default));
    let without_step = RenderOptions {
      steps: StepSet::all().without(ProcessingStep::Denoise),
      ..Default::default()
    };
    assert!(denoise_op(&rggb_rawimage(), &without_step)?.is_none());
    let no_strength = RenderOptions {
      denoise: DenoiseStrength::none(),
      ..Default::default()
    };
    assert!(denoise_op(&rggb_rawimage(), &no_strength)?.is_none());
    let strength = DenoiseStrength { luma: 1.5, chroma: 0.0 };
    let with_strength = RenderOptions {
      denoise: strength,
      ..Default::default()
    };
    assert_eq!(denoise_op(&rggb_rawimage(), &with_strength)?.map(|op| op.0), Some(strength));

    // Output-referred data is already rendered
    for reference in [ColorimetricReference::OutputReferred, ColorimetricReference::OutputReferredHdr] {
      let mut scan = linear_raw_rawimage(8, 6, 3);
      scan.colorimetric_reference = reference;
      assert!(denoise_op(&scan, &all)?.is_none(), "{:?}", reference);
    }

    // CFA data that is not demosaiced has no luma and chroma planes
    let mosaic = RenderOptions {
      steps: StepSet::all().without(ProcessingStep::Demosaic),
      ..Default::default()
    };
    assert!(denoise_op(&rggb_rawimage(), &mosaic)?.is_none());

    // LinearRaw data is not rendered
    let linear = RenderOptions {
      target: RenderTarget::LinearRaw,
      ..Default::default()
    };
    assert!(denoise_op(&rggb_rawimage(), &linear)?.is_none());

    // Monochrome data has one plane
    let mut mono = linear_raw_rawimage(8, 6, 1);
    mono.photometric = RawPhotometricInterpretation::BlackIsZero;
    assert_eq!(denoise_op(&mono, &all)?.map(|op| op.0), Some(default));

    // After the crop and before the color map, while the values are linear
    let names = op_names(&plan(&linear_raw_rawimage(8, 6, 3), &all)?);
    assert_eq!(names, ["Rescale", "CropDefault", "Denoise", "CameraToRgb"]);

    // The white balance is passed on if it is not applied yet
    assert_eq!(denoise_op(&rggb_rawimage(), &all)?.map(|op| op.1), Some([1.0; 4]));
    let after_demosaic = RenderOptions {
      white_balance: WhiteBalancePlacement::AfterDemosaic,
      ..Default::default()
    };
    let (_, wb) = denoise_op(&rggb_rawimage(), &after_demosaic)?.expect("denoise");
    assert_eq!(wb[..3], [2.0, 1.0, 1.5]);
    let mut linear_raw = linear_raw_rawimage(8, 6, 3);
    linear_raw.wb_coeffs = [2.0, 1.0, 1.5, f32::NAN];
    let (_, wb) = denoise_op(&linear_raw, &all)?.expect("denoise");
    assert_eq!(wb[..3], [2.0, 1.0, 1.5]);
    Ok(())
  }

  #[test]
  fn step_set_holds_all_steps() {
    // The steps have different bits, and there is room for more of them
    let set = StepSet::all();
    assert!(ProcessingStep::ALL.into_iter().all(|step| set.contains(step)));
    for step in ProcessingStep::ALL {
      let without = set.without(step);
      assert!(!without.contains(step));
      assert!(
        ProcessingStep::ALL
          .into_iter()
          .filter(|other| *other != step)
          .all(|other| without.contains(other))
      );
    }
    assert_eq!(StepSet::bit(ProcessingStep::ALL[ProcessingStep::ALL.len() - 1]).count_ones(), 1);
    assert_eq!(std::mem::size_of::<StepSet>(), std::mem::size_of::<u64>());
  }

  #[test]
  fn plan_white_balance_placement() -> crate::Result<()> {
    // Integer CFA data is rescaled and white balanced in one pass, limited to the camera white
    let rawimage = rggb_rawimage();
    let before = plan(&rawimage, &RenderOptions::default())?;
    let Op::RescaleMosaic { white_balance: Some(wb), .. } = &before.ops[0] else {
      panic!("white balanced rescale expected");
    };
    assert_eq!((&wb.coeffs[..3], wb.clip), (&[2.0, 1.0, 1.5][..], Some(1.0)));
    // The camera map applies no white balance then
    let Some(Op::CameraToRgb {
      map: CameraMap::Three(map), ..
    }) = before.ops.last()
    else {
      panic!("color map expected");
    };
    assert_eq!((map.wb, map.white), ([1.0; 3], 1.0));

    // With OpcodeList2 between, the white balance is a separate operation
    let mut with_list2 = rggb_rawimage();
    with_list2.dng_opcodes_2 = Some(OpcodeList::new(Vec::new()));
    let ops = ["Rescale", "List2", "WhiteBalanceMosaic", "Demosaic", "Denoise", "CameraToRgb"];
    assert_eq!(op_names(&plan(&with_list2, &RenderOptions::default())?), ops);

    // After demosaicing, the camera map applies it
    let options = RenderOptions {
      white_balance: WhiteBalancePlacement::AfterDemosaic,
      ..Default::default()
    };
    let after = plan(&rawimage, &options)?;
    assert!(matches!(after.ops[0], Op::RescaleMosaic { white_balance: None, .. }));
    let Some(Op::CameraToRgb {
      map: CameraMap::Three(map), ..
    }) = after.ops.last()
    else {
      panic!("color map expected");
    };
    assert_eq!((map.wb, map.white), ([2.0, 1.0, 1.5], 1.0));

    // Without the map to a color space, color planes are white balanced separately
    let mut linear_raw = linear_raw_rawimage(4, 2, 3);
    linear_raw.wb_coeffs = [2.0, 1.0, 1.5, f32::NAN];
    let options = RenderOptions {
      steps: StepSet::all().without(ProcessingStep::Calibrate),
      ..Default::default()
    };
    assert_eq!(
      op_names(&plan(&linear_raw, &options)?),
      ["Rescale", "CropDefault", "Denoise", "WhiteBalance", "ToneCurve", "TransferFunction"]
    );
    Ok(())
  }

  #[test]
  fn plan_for_linear_raw() -> crate::Result<()> {
    // LinearRaw data of scaled CFA data: white balanced for the demosaic only,
    // without limit, and OpcodeList3 is kept
    let mut rawimage = rggb_rawimage();
    rawimage.dng_opcodes_3 = Some(OpcodeList::new(Vec::new()));
    let options = RenderOptions {
      target: RenderTarget::LinearRaw,
      steps: [ProcessingStep::Demosaic, ProcessingStep::CropActiveArea, ProcessingStep::WhiteBalance]
        .into_iter()
        .collect(),
      ..Default::default()
    };
    let linear = plan(&rawimage, &options)?;
    assert_eq!(op_names(&linear), ["WhiteBalanceMosaic", "Demosaic", "WhiteBalance"]);
    let (Op::WhiteBalanceMosaic { white_balance: wb, .. }, Op::WhiteBalance(inverse)) = (&linear.ops[0], &linear.ops[2]) else {
      panic!("white balance expected");
    };
    assert_eq!((&wb.coeffs[..3], wb.clip), (&[2.0, 1.0, 1.5][..], None));
    assert_eq!((&inverse.coeffs[..3], inverse.clip), (&[0.5, 1.0, 1.0 / 1.5][..], None));
    // With the Rescale step, the white balance is part of the rescale
    let options = RenderOptions {
      steps: options.steps.with(ProcessingStep::Rescale),
      ..options
    };
    assert_eq!(op_names(&plan(&rawimage, &options)?), ["RescaleMosaic", "Demosaic", "WhiteBalance"]);
    Ok(())
  }

  #[test]
  fn plan_demosaic_quality() -> crate::Result<()> {
    let mut camera = Camera::new();
    camera.cfa = CFA::new("GGRGGBGGBGGRBRGRBGGGBGGRGGRGGBRBGBRG");
    let photometric = RawPhotometricInterpretation::Cfa(CFAConfig::new_from_camera(&camera));
    let rawimage = RawImage::new(
      camera,
      PixU16::new_with(vec![1000; 12 * 12], 12, 12),
      1,
      [2.0, 1.0, 1.5, f32::NAN],
      photometric,
      Some(BlackLevel::zero(2, 2, 1)),
      Some(WhiteLevel::new_bits(12, 1)),
      false,
    );
    let algorithm_of = |rawimage: &RawImage, demosaic| -> crate::Result<Option<DemosaicAlgorithm>> {
      let options = RenderOptions {
        demosaic,
        ..Default::default()
      };
      Ok(plan(rawimage, &options)?.ops.iter().find_map(|op| match op {
        Op::Demosaic { algorithm, .. } => Some(*algorithm),
        _ => None,
      }))
    };
    for (demosaic, algorithm) in [
      (DemosaicQuality::Quality, DemosaicAlgorithm::Markesteijn),
      (DemosaicQuality::Fast, DemosaicAlgorithm::XTransBilinear),
      (DemosaicQuality::Preview, DemosaicAlgorithm::XTransBilinear),
    ] {
      assert_eq!(algorithm_of(&rawimage, demosaic)?, Some(algorithm), "{:?}", demosaic);
    }
    // Bayer data of half size for previews, but rotated sensors are rotated at full size
    let mut bayer = rggb_rawimage();
    assert_eq!(algorithm_of(&bayer, DemosaicQuality::Preview)?, Some(DemosaicAlgorithm::Superpixel3));
    bayer.fuji_rotation_width = Some(4);
    assert_eq!(algorithm_of(&bayer, DemosaicQuality::Preview)?, Some(DemosaicAlgorithm::Ppg));
    Ok(())
  }

  #[test]
  fn plan_crops_other_data_to_active_area() -> crate::Result<()> {
    // Data that is not demosaiced is cropped to the ActiveArea without DefaultCrop
    let rawimage = linear_raw_rawimage(4, 2, 3);
    let options = RenderOptions {
      steps: StepSet::all().without(ProcessingStep::CropDefault),
      ..Default::default()
    };
    assert_eq!(op_names(&plan(&rawimage, &options)?), ["Rescale", "CropActiveArea", "Denoise", "CameraToRgb"]);
    // LinearRaw data is not cropped
    let options = RenderOptions {
      target: RenderTarget::LinearRaw,
      ..options
    };
    assert_eq!(op_names(&plan(&rawimage, &options)?), ["Rescale"]);
    Ok(())
  }

  #[test]
  fn plan_rejects_unsupported_images() {
    assert!(plan(&linear_raw_rawimage(4, 2, 2), &RenderOptions::default()).is_err());
  }

  #[test]
  fn default_crop_of_linear_raw_with_active_area() -> crate::Result<()> {
    // LinearRaw data is not cropped to the ActiveArea, the DefaultCrop stays relative to the image
    let mut rawimage = linear_raw_rawimage(8, 6, 1);
    rawimage.active_area = Some(Rect::new(Point::new(1, 1), Dim2::new(6, 4)));
    rawimage.crop_area = Some(Rect::new(Point::new(2, 2), Dim2::new(4, 2)));
    let options = RenderOptions {
      steps: [ProcessingStep::Demosaic, ProcessingStep::CropActiveArea, ProcessingStep::CropDefault]
        .into_iter()
        .collect(),
      ..Default::default()
    };
    let image = render(rawimage, &options)?;
    assert_eq!(image.dim(), Dim2::new(4, 2));
    assert_eq!(image.data, [18.0, 19.0, 20.0, 21.0, 26.0, 27.0, 28.0, 29.0]);
    Ok(())
  }

  #[test]
  fn step_set() {
    let steps = StepSet::all().without(ProcessingStep::TransferFunction);
    assert!(steps.contains(ProcessingStep::Rescale) && !steps.contains(ProcessingStep::TransferFunction));
    assert_eq!(
      format!("{:?}", StepSet::from_iter([ProcessingStep::Demosaic, ProcessingStep::Rescale])),
      "{Rescale, Demosaic}"
    );
  }
}
