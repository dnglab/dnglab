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
    Point, Rect,
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

/// Pixels around the DefaultCrop that are demosaiced with it, so the
/// interpolation at the border of the crop uses the same neighbor pixels as
/// for the whole image
const DEMOSAIC_MARGIN: usize = 16;

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
  /// Area of the stage 3 image, which the gain table map refers to, once the
  /// image data is cropped from it
  stage3_area: Option<Rect>,
  /// Position of the image data in the coordinates of `stage3_area`
  origin: Point,
}

impl Executor {
  fn new(rawimage: RawImage) -> Self {
    Self {
      active_area: rawimage.active_area,
      crop_area: rawimage.crop_area,
      rawimage,
      image: None,
      stage3_area: None,
      origin: Point::default(),
    }
  }

  /// Area of the stage 3 image and the position of the image data in its
  /// coordinates. Before the image data is cropped, the stage 3 image is the
  /// ActiveArea or the whole image.
  fn stage3_area(&mut self) -> (Rect, Point) {
    match self.stage3_area {
      Some(area) => (area, self.origin),
      None => {
        let rect = self.image().view().rect();
        (self.active_area.unwrap_or(rect), Point::default())
      }
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
        crop_default,
      } => self.demosaic(*algorithm, cfa, *crop_to_active_area, *crop_default)?,
      Op::FujiRotate { width, extra_rotate } => {
        let image = self.image();
        *image = fuji_normalize_rotation(image.view(), *width, *extra_rotate)?.into();
        log::debug!("dimension after rotate {:?}", image.dim());
      }
      Op::Gain(gain) => apply_gain(self.image().view_mut(), *gain),
      Op::ToneCurve(curve) => curve.apply_image(self.image().view_mut()),
      Op::CameraToRgb(map, ops) => {
        // The pixel positions of the gain table map are relative to the stage 3 image
        let (area, origin) = self.stage3_area();
        let image = self.image();
        match map {
          CameraMap::Three(map) => camera_to_rgb_in_place(image.view_mut(), map, ops, area, origin)?,
          CameraMap::Four(map) => *image = camera_to_rgb(image.view(), map, ops, area, origin)?.into(),
        }
      }
      Op::CropDefault => {
        let crop = self.crop_area.or(self.active_area);
        log::debug!("Crop: {:?}", crop);
        let (area, mut origin) = self.stage3_area();
        // Only crop if the dimensions differ
        if let Some(crop) = crop
          && crop.d != self.image().dim()
        {
          self.image().crop(crop)?;
          origin = Point::new(origin.x + crop.p.x, origin.y + crop.p.y);
        }
        (self.stage3_area, self.origin) = (Some(area), origin);
        (self.active_area, self.crop_area) = (None, None);
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

  /// Demosaic the image data, see [`Op::Demosaic`]
  fn demosaic(&mut self, algorithm: DemosaicAlgorithm, cfa: &CFAConfig, crop_to_active_area: bool, crop_default: bool) -> crate::Result<()> {
    let (stage3_area, _) = self.stage3_area();
    let crop = self.crop_area.or(self.active_area).filter(|_| crop_default);
    let image = self.image();
    let bounds = if crop_to_active_area { stage3_area } else { image.view().rect() };
    let roi = crop
      .and_then(|crop| {
        let margin = Rect::new_with_points(
          Point::new(crop.p.x.saturating_sub(DEMOSAIC_MARGIN), crop.p.y.saturating_sub(DEMOSAIC_MARGIN)),
          Point::new(crop.p.x + crop.d.w + DEMOSAIC_MARGIN, crop.p.y + crop.d.h + DEMOSAIC_MARGIN),
        );
        margin.intersect(&bounds)
      })
      .unwrap_or(bounds);
    let pixels = image.view();
    let demosaiced = match algorithm {
      DemosaicAlgorithm::Ppg => PPGDemosaic::new().demosaic(pixels, &cfa.cfa, &cfa.colors, roi).into(),
      DemosaicAlgorithm::Bilinear4 => Bilinear4Channel::new().demosaic(pixels, &cfa.cfa, &cfa.colors, roi).into(),
      DemosaicAlgorithm::XTransBilinear => XTransBilinearDemosaic::new().demosaic(pixels, &cfa.cfa, &cfa.colors, roi).into(),
    };
    *image = demosaiced;
    log::debug!("Demosaiced {:?}", roi);

    if let Some(crop) = crop {
      // The pixels of the margin are cropped, the gain table map refers to the
      // stage 3 image in the coordinates of the mosaic data
      let crop = crop.rebase(&roi).unwrap_or(self.image().view().rect());
      if crop.d != self.image().dim() {
        self.image().crop(crop)?;
      }
      (self.stage3_area, self.origin) = (Some(stage3_area), Point::new(roi.p.x + crop.p.x, roi.p.y + crop.p.y));
      (self.active_area, self.crop_area) = (None, None);
    } else if crop_to_active_area && let Some(active_area) = self.active_area.take() {
      // The DefaultCrop is relative to the ActiveArea now
      self.crop_area = self.crop_area.and_then(|crop_area| crop_area.rebase(&active_area));
    }
    Ok(())
  }
}
