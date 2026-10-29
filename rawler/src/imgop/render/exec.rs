// SPDX-License-Identifier: LGPL-2.1
// Copyright 2026 Daniel Vogelbacher <daniel@chaospixel.com>

//! Execution of a [`Plan`]

use super::{
  RenderedImage,
  plan::{CameraMap, DemosaicAlgorithm, Op, Plan},
};
use crate::{
  RawImage, RawImageData,
  dng::opcodes::OpcodeStage,
  imgop::{
    Rect,
    color::{apply_gain, apply_transfer_function, camera_to_rgb, camera_to_rgb_in_place},
    fuji_rotate::fuji_normalize_rotation,
    sensor::{
      Demosaic,
      bayer::{bilinear::Bilinear4Channel, ppg::PPGDemosaic},
      xtrans::bilinear::XTransBilinearDemosaic,
    },
  },
  rawimage::CFAConfig,
};

/// Apply the operations of `plan` to the image data of `rawimage`
///
/// The plan must be made for this image by [`plan`](super::plan).
pub fn execute(rawimage: RawImage, plan: &Plan) -> crate::Result<RenderedImage> {
  let mut executor = Executor::new(rawimage);
  for op in &plan.ops {
    executor.apply(op)?;
  }
  Ok(std::mem::take(executor.image()))
}

/// Image data and its state while a plan is executed
struct Executor {
  /// Raw image with its metadata. The image data is moved into `image` for
  /// the first operation on floating point data.
  rawimage: RawImage,
  image: Option<RenderedImage>,
  /// ActiveArea, relative to the image data
  active_area: Option<Rect>,
  /// DefaultCrop, relative to the image data
  crop_area: Option<Rect>,
}

impl Executor {
  fn new(rawimage: RawImage) -> Self {
    Self {
      active_area: rawimage.active_area,
      crop_area: rawimage.crop_area,
      rawimage,
      image: None,
    }
  }

  /// Image data as floating point values
  fn image(&mut self) -> &mut RenderedImage {
    let rawimage = &mut self.rawimage;
    self.image.get_or_insert_with(|| {
      let data = match std::mem::replace(&mut rawimage.data, RawImageData::Float(Vec::new())) {
        RawImageData::Float(data) => data,
        data => data.as_f32().into_owned(),
      };
      RenderedImage {
        data,
        width: rawimage.width,
        height: rawimage.height,
        channels: rawimage.cpp,
      }
    })
  }

  /// Raw image for an operation on its image data as read from the file
  fn raw_data(&mut self) -> crate::Result<&mut RawImage> {
    match self.image {
      Some(_) => Err("Image data is already processed".into()),
      None => Ok(&mut self.rawimage),
    }
  }

  fn apply(&mut self, op: &Op) -> crate::Result<()> {
    match op {
      Op::Opcodes { stage, crop_to_active_area } => self.apply_opcodes(*stage, *crop_to_active_area)?,
      Op::Rescale => self.raw_data()?.apply_scaling()?,
      Op::Demosaic {
        algorithm,
        cfa,
        crop_to_active_area,
      } => self.demosaic(*algorithm, cfa, *crop_to_active_area),
      Op::FujiRotate { width, extra_rotate } => {
        let image = self.image();
        *image = fuji_normalize_rotation(image.view(), *width, *extra_rotate)?.into();
        log::debug!("dimension after rotate {:?}", image.dim());
      }
      Op::Gain(gain) => apply_gain(self.image().view_mut(), *gain),
      Op::ToneCurve(curve) => curve.apply_image(self.image().view_mut()),
      Op::CameraToRgb(map, ops) => {
        // Image area of the gain table map, the stage 3 image
        let rect = self.image().view().rect();
        let area = self.active_area.unwrap_or(rect);
        let image = self.image();
        match map {
          CameraMap::Three(map) => camera_to_rgb_in_place(image.view_mut(), map, ops, area)?,
          CameraMap::Four(map) => *image = camera_to_rgb(image.view(), map, ops, area)?.into(),
        }
      }
      Op::CropDefault => {
        let crop = self.crop_area.take().or(self.active_area.take());
        log::debug!("Crop: {:?}", crop);
        let image = self.image();
        // Only crop if the dimensions differ
        if let Some(crop) = crop
          && crop.d != image.dim()
        {
          image.crop(crop)?;
        }
      }
      Op::TransferFunction(function) => apply_transfer_function(self.image().view_mut(), *function),
    }
    Ok(())
  }

  /// Apply the opcode list of `stage`. While the opcodes are applied, the
  /// image data is the data of the raw image.
  fn apply_opcodes(&mut self, stage: OpcodeStage, crop_to_active_area: bool) -> crate::Result<()> {
    let processed = match self.image.take() {
      Some(image) => {
        self.rawimage.width = image.width;
        self.rawimage.height = image.height;
        self.rawimage.cpp = image.channels;
        self.rawimage.data = RawImageData::Float(image.data);
        true
      }
      None => false,
    };
    self.rawimage.active_area = self.active_area;
    self.rawimage.crop_area = self.crop_area;
    self.rawimage.apply_opcode_list(stage)?;
    self.active_area = self.rawimage.active_area;
    self.crop_area = self.rawimage.crop_area;
    if processed {
      self.image();
    }
    // A TrimBounds shrinks the ActiveArea, keep the image data cropped to it
    if crop_to_active_area && let Some(active_area) = self.active_area.take() {
      self.image().crop(active_area)?;
      self.crop_area = self.crop_area.and_then(|crop_area| crop_area.rebase(&active_area));
    }
    Ok(())
  }

  fn demosaic(&mut self, algorithm: DemosaicAlgorithm, cfa: &CFAConfig, crop_to_active_area: bool) {
    let active_area = if crop_to_active_area { self.active_area.take() } else { None };
    let image = self.image();
    let roi = active_area.unwrap_or(image.view().rect());
    let pixels = image.view();
    let demosaiced = match algorithm {
      DemosaicAlgorithm::Ppg => PPGDemosaic::new().demosaic(pixels, &cfa.cfa, &cfa.colors, roi).into(),
      DemosaicAlgorithm::Bilinear4 => Bilinear4Channel::new().demosaic(pixels, &cfa.cfa, &cfa.colors, roi).into(),
      DemosaicAlgorithm::XTransBilinear => XTransBilinearDemosaic::new().demosaic(pixels, &cfa.cfa, &cfa.colors, roi).into(),
    };
    *image = demosaiced;
    // The DefaultCrop is relative to the ActiveArea now
    if let Some(active_area) = active_area {
      self.crop_area = self.crop_area.and_then(|crop_area| crop_area.rebase(&active_area));
    }
  }
}
