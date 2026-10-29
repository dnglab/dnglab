// SPDX-License-Identifier: LGPL-2.1
// Copyright 2026 Daniel Vogelbacher <daniel@chaospixel.com>

//! Rendering of raw images
//!
//! [`plan`] decides which processing operations are applied to a raw image,
//! in which order and with which parameters, from the metadata of the image
//! and the [`RenderOptions`]. [`execute`] applies the operations of a [`Plan`]
//! to the image data. The operations are public processing steps that work on
//! borrowed image data, e.g. [`camera_to_rgb`](crate::imgop::color::camera_to_rgb).

mod exec;
mod plan;

pub use exec::execute;
pub use plan::{Plan, plan};

use image::{DynamicImage, ImageBuffer};

use crate::{
  RawImage,
  imgop::{Dim2, Rect, color::OutputSpace, convert_from_f32_scaled_u16},
  pixarray::{Color2D, ImageView, ImageViewMut},
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
  /// Crop CFA data to the ActiveArea while it is demosaiced
  CropActiveArea,
  /// Apply BaselineExposure and BaselineExposureOffset
  BaselineExposure,
  /// Apply the as shot white balance
  WhiteBalance,
  /// Map camera values to the output color space
  Calibrate,
  /// Apply the rendering tables of the camera profile (part of Calibrate)
  ProfileRender,
  /// Crop to the DefaultCrop, or the ActiveArea without DefaultCrop
  CropDefault,
  /// Encode the values by the transfer function of the output color space
  TransferFunction,
}

impl ProcessingStep {
  /// All steps
  pub const ALL: [Self; 10] = [
    Self::Rescale,
    Self::Demosaic,
    Self::FujiRotate,
    Self::CropActiveArea,
    Self::BaselineExposure,
    Self::WhiteBalance,
    Self::Calibrate,
    Self::ProfileRender,
    Self::CropDefault,
    Self::TransferFunction,
  ];
}

/// Set of processing steps
#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub struct StepSet(u16);

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

  /// The set without `step`
  pub fn without(mut self, step: ProcessingStep) -> Self {
    self.remove(step);
    self
  }

  fn bit(step: ProcessingStep) -> u16 {
    1 << step as u16
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

/// Options of the rendering
#[derive(Clone, Debug, PartialEq)]
pub struct RenderOptions {
  /// Processing steps to apply
  pub steps: StepSet,
  /// Color space of the output: the Calibrate step maps the camera values to
  /// it, the TransferFunction step encodes the values by its transfer function
  pub color_space: OutputSpace,
}

impl Default for RenderOptions {
  fn default() -> Self {
    Self {
      steps: StepSet::all(),
      color_space: OutputSpace::default(),
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
    let len = self.view_mut().crop_in_place(area)?.as_slice().len();
    self.data.truncate(len);
    (self.width, self.height) = (area.d.w, area.d.h);
    Ok(())
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

impl<const N: usize> From<Color2D<f32, N>> for RenderedImage {
  fn from(pixels: Color2D<f32, N>) -> Self {
    let (width, height) = (pixels.width, pixels.height);
    Self {
      data: pixels.into_flatten(),
      width,
      height,
      channels: N,
    }
  }
}

/// Render `rawimage` with `options`, see [`plan`] and [`execute`]
pub fn render(rawimage: RawImage, options: &RenderOptions) -> crate::Result<RenderedImage> {
  let plan = plan(&rawimage, options)?;
  execute(rawimage, &plan)
}

#[cfg(test)]
mod tests {
  use super::{plan::Op, *};
  use crate::{
    CFA, RawImageData,
    decoders::Camera,
    dng::opcodes::{OpcodeList, OpcodeStage},
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
        Op::Demosaic { .. } => "Demosaic",
        Op::FujiRotate { .. } => "FujiRotate",
        Op::Gain(_) => "Gain",
        Op::ToneCurve(_) => "ToneCurve",
        Op::CameraToRgb(..) => "CameraToRgb",
        Op::CropDefault => "CropDefault",
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
    rawimage.baseline_exposure = Some(0.5);
    let plan = plan(&rawimage, &RenderOptions::default())?;
    let ops = ["Rescale", "Demosaic", "List3", "Gain", "CameraToRgb", "CropDefault", "TransferFunction"];
    assert_eq!(op_names(&plan), ops);
    // OpcodeList3 keeps the data cropped to the ActiveArea, like the demosaic
    assert!(plan.ops.iter().any(|op| matches!(op, Op::Opcodes { crop_to_active_area: true, .. })));
    Ok(())
  }

  #[test]
  fn plan_for_negative_exposure() -> crate::Result<()> {
    // The color map renders the curve, without it a separate tone curve is applied
    let mut rawimage = rggb_rawimage();
    rawimage.baseline_exposure = Some(-1.0);
    assert_eq!(
      op_names(&plan(&rawimage, &RenderOptions::default())?),
      ["Rescale", "Demosaic", "CameraToRgb", "CropDefault", "TransferFunction"]
    );
    let options = RenderOptions {
      steps: StepSet::all().without(ProcessingStep::Calibrate),
      ..Default::default()
    };
    assert_eq!(
      op_names(&plan(&rawimage, &options)?),
      ["Rescale", "Demosaic", "ToneCurve", "CropDefault", "TransferFunction"]
    );
    // Monochrome images have no color map
    let mut mono = linear_raw_rawimage(4, 2, 1);
    mono.photometric = RawPhotometricInterpretation::BlackIsZero;
    mono.baseline_exposure = Some(-1.0);
    assert_eq!(
      op_names(&plan(&mono, &RenderOptions::default())?),
      ["Rescale", "ToneCurve", "CropDefault", "TransferFunction"]
    );
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
