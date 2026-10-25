//! Raw image

use std::borrow::Cow;
use std::collections::HashMap;

use itertools::Itertools;
use log::debug;
use serde::{Deserialize, Serialize};

use crate::Result;
use crate::cfa::PlaneColor;
use crate::dng::opcodes::{OpcodeList, OpcodeSelector, OpcodeStage};
use crate::imgop::raw::{apply_linearization_table, correct_blacklevel, correct_blacklevel_cfa};
use crate::imgop::sensor::SensorType;
use crate::imgop::{convert_from_f32_scaled_u16, convert_to_f32_unscaled};
use crate::pixarray::SubPixel;
use crate::{
  CFA,
  decoders::*,
  formats::tiff::{Rational, Value},
  imgop::{
    Dim2, Point, Rect,
    chromatic_adaption::adapt_xyz2cam_bradford,
    matrix::{determinant, multiply, normalize, project_rows_to_simplex, pseudo_inverse, transform_1d, transform_2d},
    raw::{ColorMatrix, DevelopParams},
    xyz::{FlatColorMatrix, Illuminant, XYZ_TO_PROFOTORGB_D50},
  },
  pixarray::PixU16,
  tags::TiffTag,
};

/// Illuminants in order of preference for color calculations based on D65.
/// Color matrices of these illuminants can be adapted to D65.
const D65_PREFERENCE: [Illuminant; 9] = [
  Illuminant::D65,
  Illuminant::A,
  Illuminant::B,
  Illuminant::C,
  Illuminant::D50,
  Illuminant::D55,
  Illuminant::D75,
  Illuminant::Daylight,
  Illuminant::Flash,
];

/// Sort key for illuminants in order of [`D65_PREFERENCE`], other illuminants follow by their EXIF value
pub(crate) fn d65_preference(illuminant: &Illuminant) -> (usize, u16) {
  let position = D65_PREFERENCE.iter().position(|x| x == illuminant).unwrap_or(D65_PREFERENCE.len());
  (position, u16::from(*illuminant))
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WhiteLevel(pub Vec<u32>);

impl Default for WhiteLevel {
  fn default() -> Self {
    Self(vec![u16::MAX as u32; 1])
  }
}

impl WhiteLevel {
  pub fn new(level: impl Into<Vec<u32>>) -> Self {
    Self(level.into())
  }

  pub fn new_bits(bits: u32, cpp: usize) -> Self {
    if bits > 32 {
      panic!("Whitelevel can only be calculated for max. 32 bits, but {} bits given", bits);
    }
    let level: u32 = ((1_u64 << bits) - 1) as u32;
    Self(vec![level; cpp])
  }

  pub fn as_vec(&self) -> Vec<f32> {
    self.0.iter().cloned().map(|x| x as f32).collect_vec()
  }

  pub fn as_bayer_array(&self) -> [f32; 4] {
    if self.0.len() == 4 {
      [self.0[0] as f32, self.0[1] as f32, self.0[2] as f32, self.0[3] as f32]
    } else {
      [self.0[0] as f32, self.0[0] as f32, self.0[0] as f32, self.0[0] as f32]
    }
  }
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct BlackLevel {
  pub levels: Vec<Rational>,
  pub cpp: usize,
  pub width: usize,
  pub height: usize,
}

impl Default for BlackLevel {
  fn default() -> Self {
    Self {
      levels: [Rational::from(0_u32)].into(),
      width: 1,
      height: 1,
      cpp: 1,
    }
  }
}

impl std::fmt::Debug for BlackLevel {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    let levels: Vec<f32> = self.levels.iter().map(|x| x.as_f32()).collect();
    f.write_fmt(format_args!("RepeatDim: {}:{}, cpp: {}, {:?}", self.height, self.width, self.cpp, levels))
  }
}

impl BlackLevel {
  pub fn new<T>(levels: &[T], width: usize, height: usize, cpp: usize) -> Self
  where
    T: Copy + Into<Rational>,
  {
    assert_eq!(levels.len(), width * height * cpp);
    Self {
      levels: levels.iter().map(|x| (*x).into()).collect(),
      width,
      height,
      cpp,
    }
  }

  pub fn zero(width: usize, height: usize, cpp: usize) -> Self {
    if width == 0 || height == 0 || cpp == 0 {
      panic!(
        "Blacklevel::zero() must not be called with zero value arguments: width: {}, height: {}, cpp: {}",
        width, height, cpp
      );
    }
    Self {
      levels: vec![0_u16; cpp * width * height].iter().map(|x| Rational::from(*x)).collect(),
      width,
      height,
      cpp,
    }
  }

  pub fn as_vec(&self) -> Vec<f32> {
    self.levels.iter().map(Rational::as_f32).collect_vec()
  }

  pub fn as_bayer_array(&self) -> [f32; 4] {
    if self.levels.len() == 4 {
      [
        self.levels[0].as_f32(),
        self.levels[1].as_f32(),
        self.levels[2].as_f32(),
        self.levels[3].as_f32(),
      ]
    } else {
      [
        self.levels[0].as_f32(),
        self.levels[0].as_f32(),
        self.levels[0].as_f32(),
        self.levels[0].as_f32(),
      ]
    }
  }

  pub fn sample_count(&self) -> usize {
    self.cpp * self.width * self.height
  }

  // TODO: write test
  pub fn shift(&self, x: usize, y: usize) -> Self {
    if self.sample_count() == 1 {
      self.clone()
    } else {
      let mut trans = self.clone();
      let (w, h, cpp) = (trans.width, trans.height, trans.cpp);
      for yn in 0..h {
        for xn in 0..w {
          let ys = (yn + y) % h;
          let xs = (xn + x) % w;
          trans.levels[yn * w * cpp + xn * cpp..yn * w * cpp + xn * cpp + cpp]
            .copy_from_slice(&self.levels[ys * w * cpp + xs * cpp..ys * w * cpp + xs * cpp + cpp]);
        }
      }
      trans
    }
  }
}

#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq)]
pub enum RawPhotometricInterpretation {
  BlackIsZero,
  // Defined by DNG
  Cfa(CFAConfig),
  LinearRaw,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CFAConfig {
  pub cfa: CFA,
  pub colors: PlaneColor,
  pub sensor: SensorType,
}

impl CFAConfig {
  pub fn new(cfa: &CFA, colors: &PlaneColor) -> Self {
    Self {
      cfa: cfa.clone(),
      colors: colors.clone(),
      sensor: SensorType::from_cfa(cfa),
    }
  }

  pub fn new_from_camera(cam: &Camera) -> Self {
    Self {
      cfa: cam.cfa.clone(),
      colors: cam.plane_color.clone(),
      sensor: SensorType::from_cfa(&cam.cfa),
    }
  }
}

/// All the data needed to process this raw image, including the image data itself as well
/// as all the needed metadata
#[derive(Debug, Clone)]
pub struct RawImage {
  /// Camera definition
  pub camera: Camera,
  /// camera make as encoded in the file
  pub make: String,
  /// camera model as encoded in the file
  pub model: String,
  /// make cleaned up to be consistent and short
  pub clean_make: String,
  /// model cleaned up to be consistent and short
  pub clean_model: String,
  /// width of the full image
  pub width: usize,
  /// height of the full image
  pub height: usize,
  /// number of components per pixel (1 for bayer, 3 for RGB images)
  pub cpp: usize,
  /// Bits per pixel
  pub bps: usize,
  /// whitebalance coefficients encoded in the file in RGBE order
  pub wb_coeffs: [f32; 4],
  /// image whitelevels in RGBE order
  pub whitelevel: WhiteLevel,
  /// image blacklevels in RGBE order
  pub blacklevel: BlackLevel,
  /// matrix to convert XYZ to camera RGBE
  pub xyz_to_cam: [[f32; 3]; 4], // TODO: deprecated, use color_matrix
  /// Photometric interpretation
  pub photometric: RawPhotometricInterpretation,
  /// how much to crop the image to get all the usable (non-black) area
  pub active_area: Option<Rect>,
  /// how much to crop the image to get all the recommended area
  pub crop_area: Option<Rect>,

  /// Areas of the sensor that is masked to prevent it from receiving light. Used to calculate
  /// black levels and noise.
  pub blackareas: Vec<Rect>,

  /// orientation of the image as indicated by the image metadata
  pub orientation: Orientation,
  /// image data itself, has `width`\*`height`\*`cpp` elements
  pub data: RawImageData,
  /// The image data was scaled from 16 bit integer samples to floats by `apply_scaling()`
  pub scaled_from_integer: bool,

  pub color_matrix: HashMap<Illuminant, FlatColorMatrix>,

  /// Reduction matrices (3 x color planes) for images with more than three color planes
  pub reduction_matrix: HashMap<Illuminant, FlatColorMatrix>,

  pub dng_tags: HashMap<u16, Value>,

  /// For Fuji rotated sensors: the split point T used to compute the
  /// inscribed rectangle after 45° rotation (equivalent to dcraw's fuji_width).
  pub fuji_rotation_width: Option<usize>,

  pub dng_opcodes_1: Option<OpcodeList>,
  pub dng_opcodes_2: Option<OpcodeList>,
  pub dng_opcodes_3: Option<OpcodeList>,

  /// DNG LinearizationTable, not yet applied to the integer image data.
  /// It is applied by `apply_scaling()`, after OpcodeList1.
  pub linearization_table: Option<Vec<u16>>,

  /// DNG BaselineExposure in EV, applied by the develop pipeline.
  pub baseline_exposure: Option<f32>,
  /// DNG BaselineExposureOffset in EV, added to BaselineExposure by the develop pipeline.
  pub baseline_exposure_offset: Option<f32>,
}

/// The actual image data, after decoding
#[derive(Debug, Clone)]
pub enum RawImageData {
  /// The most usual u16 output of almost all formats
  Integer(Vec<u16>),
  /// Some formats are directly encoded as f32, most notably some DNGs
  Float(Vec<f32>),
}

impl RawImageData {
  pub fn as_f32<'a>(&'a self) -> Cow<'a, Vec<f32>> {
    match self {
      Self::Integer(data) => Cow::Owned(convert_to_f32_unscaled(data)),
      Self::Float(data) => Cow::Borrowed(data),
    }
  }

  pub fn force_integer(&mut self) {
    if let Self::Float(data) = self {
      *self = Self::Integer(convert_from_f32_scaled_u16(data, 0, u16::MAX));
    }
  }
}

impl RawImage {
  pub fn calc_black_levels<T>(cfa: &CFA, blackareas: &[Rect], width: usize, _height: usize, image: &[T]) -> Option<BlackLevel>
  where
    T: SubPixel,
  {
    let x = cfa.width * cfa.height;
    if x == 0 {
      return None;
    }
    assert!(!image.is_empty());

    #[derive(Clone, Copy)]
    struct Sample {
      avg: f32,
      count: usize,
    }

    if !blackareas.is_empty() {
      let mut samples = vec![Sample { avg: 0.0, count: 0 }; x];

      for area in blackareas {
        for row in area.p.y..area.p.y + area.d.h {
          for col in area.p.x..area.p.x + area.d.w {
            //let color = cfa.color_at(row, col);
            let color = (row % cfa.height) * cfa.width + (col % cfa.width);
            samples[color].avg += image[row * width + col].as_f32();
            samples[color].count += 1;
          }
        }
      }

      let blacklevels: Vec<f32> = samples.into_iter().map(|s| s.avg / s.count as f32).collect();

      debug!("Calculated blacklevels: {:?}", blacklevels);
      // TODO: support other then RGGB levels
      assert_eq!(cfa.width * cfa.height, 4);
      Some(BlackLevel::new(&[blacklevels[0], blacklevels[1], blacklevels[2], blacklevels[3]], 2, 2, 1))
    } else {
      None
    }
  }

  #[doc(hidden)]
  pub fn new(
    cam: Camera,
    image: PixU16,
    cpp: usize,
    wb_coeffs: [f32; 4],
    photometric: RawPhotometricInterpretation,
    blacklevel: Option<BlackLevel>,
    whitelevel: Option<WhiteLevel>,
    dummy: bool,
  ) -> RawImage {
    assert_eq!(image.width % cpp, 0);
    assert_eq!(dummy, !image.is_initialized());
    let sample_width = image.width;
    let pixel_width = image.width / cpp;

    let mut blackareas: Vec<Rect> = Vec::new();

    let active_area = cam.active_area.map(|area| Rect::new_with_borders(Dim2::new(pixel_width, image.height), &area));

    let blackarea_base = active_area.unwrap_or_else(|| Rect::new(Point::zero(), Dim2::new(sample_width, image.height)));

    // For now, we only use masked areas when cpp is 1. For color images (RGB)
    // like Canon SRAW, we ignore it (it isn't provided anyway).
    if cpp == 1 {
      // Build black areas
      // First value (.0) is start and (.1) is length!
      if let Some(ah) = cam.blackareah {
        blackareas.push(Rect::new_with_points(
          Point::new(blackarea_base.p.x, ah.0),
          Point::new(blackarea_base.p.x + blackarea_base.d.w, ah.0 + ah.1),
        ));
      }
      if let Some(av) = cam.blackareav {
        blackareas.push(Rect::new_with_points(
          Point::new(av.0, blackarea_base.p.y),
          Point::new(av.0 + av.1, blackarea_base.p.y + blackarea_base.d.h),
        ));
      }
    }

    let blacklevel = cam
      .make_blacklevel(cpp)
      .or_else(|| if cam.find_hint("invalid_blacklevel") { None } else { blacklevel })
      .or_else(|| {
        if dummy {
          Some(BlackLevel::default())
        } else {
          Self::calc_black_levels::<u16>(&cam.cfa, &blackareas, image.width, image.height, image.pixels())
        }
      })
      .unwrap_or_else(|| BlackLevel::zero(1, 1, cpp));

    let whitelevel = cam
      .make_whitelevel(cpp)
      .or(whitelevel)
      .unwrap_or_else(|| panic!("Need whitelevel in config: {}", cam.clean_model));

    let crop_area = cam.crop_area.map(|area| Rect::new_with_borders(Dim2::new(pixel_width, image.height), &area));

    RawImage {
      camera: cam.clone(),
      make: cam.make.clone(),
      model: cam.model.clone(),
      clean_make: cam.clean_make.clone(),
      clean_model: cam.clean_model.clone(),
      width: image.width / cpp,
      height: image.height,
      cpp,
      bps: cam.real_bps,
      wb_coeffs,
      data: RawImageData::Integer(image.into_inner()),
      blacklevel,
      whitelevel,
      xyz_to_cam: cam.xyz_to_cam,
      photometric,
      active_area,
      crop_area,
      blackareas,
      orientation: Orientation::Normal, //cam.orientation, // TODO fixme
      color_matrix: cam.color_matrix,
      reduction_matrix: cam.reduction_matrix,
      dng_tags: HashMap::new(),
      fuji_rotation_width: None,
      scaled_from_integer: false,
      dng_opcodes_1: None,
      dng_opcodes_2: None,
      dng_opcodes_3: None,
      linearization_table: None,
      baseline_exposure: None,
      baseline_exposure_offset: None,
    }
  }

  #[doc(hidden)]
  pub fn new_with_data(
    cam: Camera,
    image: RawImageData,
    sample_width: usize,
    height: usize,
    cpp: usize,
    wb_coeffs: [f32; 4],
    photometric: RawPhotometricInterpretation,
    blacklevel: Option<BlackLevel>,
    whitelevel: Option<WhiteLevel>,
    dummy: bool,
  ) -> RawImage {
    //assert_eq!(image.width % cpp, 0);
    //assert_eq!(dummy, !image.is_initialized());
    let pixel_width = sample_width / cpp;

    let mut blackareas: Vec<Rect> = Vec::new();

    let active_area = cam.active_area.map(|area| Rect::new_with_borders(Dim2::new(pixel_width, height), &area));

    let blackarea_base = active_area.unwrap_or_else(|| Rect::new(Point::zero(), Dim2::new(sample_width, height)));

    // For now, we only use masked areas when cpp is 1. For color images (RGB)
    // like Canon SRAW, we ignore it (it isn't provided anyway).
    if cpp == 1 {
      // Build black areas
      // First value (.0) is start and (.1) is length!
      if let Some(ah) = cam.blackareah {
        blackareas.push(Rect::new_with_points(
          Point::new(blackarea_base.p.x, ah.0),
          Point::new(blackarea_base.p.x + blackarea_base.d.w, ah.0 + ah.1),
        ));
      }
      if let Some(av) = cam.blackareav {
        blackareas.push(Rect::new_with_points(
          Point::new(av.0, blackarea_base.p.y),
          Point::new(av.0 + av.1, blackarea_base.p.y + blackarea_base.d.h),
        ));
      }
    }

    let blacklevel = cam
      .make_blacklevel(cpp)
      .or_else(|| if cam.find_hint("invalid_blacklevel") { None } else { blacklevel })
      .or_else(|| {
        if dummy {
          Some(BlackLevel::default())
        } else {
          match &image {
            RawImageData::Integer(pix) => Self::calc_black_levels(&cam.cfa, &blackareas, sample_width, height, pix),
            RawImageData::Float(pix) => Self::calc_black_levels(&cam.cfa, &blackareas, sample_width, height, pix),
          }
        }
      })
      .unwrap_or_else(|| BlackLevel::zero(1, 1, cpp));

    let whitelevel = cam
      .make_whitelevel(cpp)
      .or(whitelevel)
      .unwrap_or_else(|| panic!("Need whitelevel in config: {}", cam.clean_model));

    let crop_area = cam.crop_area.map(|area| Rect::new_with_borders(Dim2::new(pixel_width, height), &area));

    RawImage {
      camera: cam.clone(),
      make: cam.make.clone(),
      model: cam.model.clone(),
      clean_make: cam.clean_make.clone(),
      clean_model: cam.clean_model.clone(),
      width: pixel_width,
      height: height,
      cpp,
      bps: cam.real_bps,
      wb_coeffs,
      data: image,
      blacklevel,
      whitelevel,
      xyz_to_cam: cam.xyz_to_cam,
      photometric,
      active_area,
      crop_area,
      blackareas,
      orientation: Orientation::Normal, //cam.orientation, // TODO fixme
      color_matrix: cam.color_matrix,
      reduction_matrix: cam.reduction_matrix,
      dng_tags: HashMap::new(),
      fuji_rotation_width: None,
      scaled_from_integer: false,
      dng_opcodes_1: None,
      dng_opcodes_2: None,
      dng_opcodes_3: None,
      linearization_table: None,
      baseline_exposure: None,
      baseline_exposure_offset: None,
    }
  }

  pub fn dim(&self) -> Dim2 {
    Dim2::new(self.width, self.height)
  }

  pub fn pixels_u16(&self) -> &[u16] {
    if let RawImageData::Integer(data) = &self.data {
      data
    } else {
      panic!("Data is not u16");
    }
  }

  pub fn pixels_u16_mut(&mut self) -> &mut [u16] {
    if let RawImageData::Integer(data) = &mut self.data {
      data
    } else {
      panic!("Data is not u16");
    }
  }

  /// Apply LinearizationTable, blacklevel and whitelevel scaling, replacing raw
  /// image data with floating point values in range 0.0 .. 1.0.
  /// Internal blacklevel and whitelevel is reset to 0.0 and 1.0 to match image data.
  pub fn apply_scaling(&mut self) -> crate::Result<()> {
    self.scaled_from_integer = matches!(self.data, RawImageData::Integer(_));
    // The LinearizationTable is applied before black and white levels
    if let Some(table) = self.linearization_table.take()
      && let RawImageData::Integer(samples) = &mut self.data
    {
      apply_linearization_table(samples, self.width * self.cpp, &table);
    }
    let mut pixels = self.data.as_f32();
    match &self.photometric {
      // Black levels of a single plane repeat spatially like on CFA data
      RawPhotometricInterpretation::BlackIsZero | RawPhotometricInterpretation::Cfa(_) => {
        correct_blacklevel_cfa(
          pixels.to_mut(),
          self.width,
          self.height,
          &self.blacklevel.as_bayer_array(),
          &self.whitelevel.as_bayer_array(),
        );
        self.data = RawImageData::Float(pixels.into_owned());
      }
      RawPhotometricInterpretation::LinearRaw => {
        correct_blacklevel(pixels.to_mut(), &self.blacklevel.as_vec(), &self.whitelevel.as_vec());
        self.data = RawImageData::Float(pixels.into_owned());
      }
    }
    self.blacklevel.levels.iter_mut().for_each(|x| *x = Rational::new(0, 1));
    self.whitelevel.0.iter_mut().for_each(|x| *x = 1);
    Ok(())
  }

  /// DNG opcode list for the given processing stage
  pub fn opcode_list(&self, stage: OpcodeStage) -> Option<&OpcodeList> {
    match stage {
      OpcodeStage::List1 => self.dng_opcodes_1.as_ref(),
      OpcodeStage::List2 => self.dng_opcodes_2.as_ref(),
      OpcodeStage::List3 => self.dng_opcodes_3.as_ref(),
    }
  }

  /// Apply the DNG opcode list of the given processing stage to the image data.
  /// The image data must be in the state the stage requires, e.g. mapped to
  /// linear reference values for OpcodeList2. The list is consumed, so it is
  /// not written again into a DNG file.
  /// Remove the DNG opcodes selected by `selectors`, so they are neither applied
  /// nor written into a DNG file. Opcode lists without opcodes left are removed.
  pub fn remove_opcodes(&mut self, selectors: &[OpcodeSelector]) {
    if selectors.is_empty() {
      return;
    }
    for (stage, list) in [
      (OpcodeStage::List1, &mut self.dng_opcodes_1),
      (OpcodeStage::List2, &mut self.dng_opcodes_2),
      (OpcodeStage::List3, &mut self.dng_opcodes_3),
    ] {
      let Some(opcodes) = list.as_mut() else {
        continue;
      };
      for opcode in opcodes.remove(stage, selectors) {
        log::debug!("Removed DNG opcode {} from {}", opcode.name(), stage);
      }
      if opcodes.is_empty() {
        *list = None;
      }
    }
  }

  pub fn apply_opcode_list(&mut self, stage: OpcodeStage) -> Result<()> {
    let list = match stage {
      OpcodeStage::List1 => self.dng_opcodes_1.take(),
      OpcodeStage::List2 => self.dng_opcodes_2.take(),
      OpcodeStage::List3 => self.dng_opcodes_3.take(),
    };
    match list {
      Some(list) => list.apply(self, stage),
      None => Ok(()),
    }
  }

  pub fn develop_params(&self) -> Result<DevelopParams> {
    let mut xyz2cam: [[f32; 3]; 4] = [[0.0; 3]; 4];
    //let color_matrix = self.color_matrix.get(&Illuminant::D65).unwrap(); // TODO fixme
    let color_matrix = self
      .color_matrix
      .values()
      .next()
      .cloned()
      .unwrap_or(vec![1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0]); // TODO: invalid
    assert_eq!(color_matrix.len() % 3, 0); // this is not so nice...
    let components = color_matrix.len() / 3;
    for i in 0..components {
      for j in 0..3 {
        xyz2cam[i][j] = color_matrix[i * 3 + j];
      }
    }

    /*
    let active_area = Rect::new(
      Point::new(self.crops[3], self.crops[0]),
      Dim2::new(self.width - self.crops[3] - self.crops[1], self.height - self.crops[0] - self.crops[2]),
    );
    */
    debug!("RAW developing active area: {:?}", self.active_area);

    let wb_coeff = if self.wb_coeffs[0].is_nan() { [1.0, 1.0, 1.0, 1.0] } else { self.wb_coeffs };

    let params = DevelopParams {
      width: self.width,
      height: self.height,
      color_matrices: vec![ColorMatrix {
        illuminant: Illuminant::D65, // TODO: need CAT
        matrix: xyz2cam,
      }],
      whitelevel: self.whitelevel.clone(),
      blacklevel: self.blacklevel.clone(),
      //pattern,
      photometric: self.photometric.clone(),
      wb_coeff,
      cpp: self.cpp,
      active_area: self.active_area,
      crop_area: self.crop_area,
      //gamma: 2.4,
    };

    Ok(params)
  }

  /// Add a DNG tag override
  pub fn add_dng_tag<T: TiffTag, V: Into<Value>>(&mut self, tag: T, value: V) {
    let tag: u16 = tag.into();
    self.dng_tags.insert(tag, value.into());
  }

  pub fn linearize(&self) -> Result<Self> {
    todo!()
  }

  /// Outputs the inverted matrix that converts pixels in the camera colorspace into
  /// XYZ components.
  pub fn cam_to_xyz(&self) -> [[f32; 4]; 3] {
    self.pseudoinverse(self.xyz_to_cam)
  }

  /// Outputs the inverted matrix that converts pixels in the camera colorspace into
  /// XYZ components normalized to be easily used to convert to Lab or a RGB output space
  pub fn cam_to_xyz_normalized(&self) -> [[f32; 4]; 3] {
    let mut xyz_to_cam = self.xyz_to_cam;
    // Normalize xyz_to_cam so that xyz_to_cam * (1,1,1) is (1,1,1,1)
    for i in 0..4 {
      let mut num = 0.0;
      for j in 0..3 {
        num += xyz_to_cam[i][j];
      }
      for j in 0..3 {
        xyz_to_cam[i][j] = if num == 0.0 { 0.0 } else { xyz_to_cam[i][j] / num };
      }
    }

    self.pseudoinverse(xyz_to_cam)
  }

  /// Not all cameras encode a whitebalance so in those cases just using a 6500K neutral one
  /// is a good compromise
  pub fn neutralwb(&self) -> [f32; 4] {
    let rgb_to_xyz = [
      // sRGB D65
      [0.412453, 0.357580, 0.180423],
      [0.212671, 0.715160, 0.072169],
      [0.019334, 0.119193, 0.950227],
    ];

    // Multiply RGB matrix
    let mut rgb_to_cam = [[0.0; 3]; 4];
    for i in 0..4 {
      for j in 0..3 {
        rgb_to_cam[i][j] = 0.0;
        for k in 0..3 {
          rgb_to_cam[i][j] += self.xyz_to_cam[i][k] * rgb_to_xyz[k][j];
        }
      }
    }

    let mut neutralwb = [0 as f32; 4];
    for i in 0..4 {
      let mut num = 0.0;
      for j in 0..3 {
        num += rgb_to_cam[i][j];
      }
      neutralwb[i] = 1.0 / num;
    }

    [
      neutralwb[0] / neutralwb[1],
      neutralwb[1] / neutralwb[1],
      neutralwb[2] / neutralwb[1],
      neutralwb[3] / neutralwb[1],
    ]
  }

  fn pseudoinverse(&self, inm: [[f32; 3]; 4]) -> [[f32; 4]; 3] {
    let mut temp: [[f32; 6]; 3] = [[0.0; 6]; 3];

    for i in 0..3 {
      for j in 0..6 {
        temp[i][j] = if j == i + 3 { 1.0 } else { 0.0 };
      }
      for j in 0..3 {
        for k in 0..4 {
          temp[i][j] += inm[k][i] * inm[k][j];
        }
      }
    }

    for i in 0..3 {
      let mut num = temp[i][i];
      for j in 0..6 {
        temp[i][j] /= num;
      }
      for k in 0..3 {
        if k == i {
          continue;
        }
        num = temp[k][i];
        for j in 0..6 {
          temp[k][j] -= temp[i][j] * num;
        }
      }
    }

    let mut out: [[f32; 4]; 3] = [[0.0; 4]; 3];

    for i in 0..4 {
      for j in 0..3 {
        out[j][i] = 0.0;
        for k in 0..3 {
          out[j][i] += temp[j][k + 3] * inm[i][k];
        }
      }
    }

    out
  }

  /// Returns the CFA pattern after the crop has been applied (and thus the pattern
  /// potentially shifted)
  pub fn cropped_cfa(&self) -> CFA {
    //self.cfa.shift(self.crops[3], self.crops[0])
    todo!()
    // Need to specify which crop, active or DefaultCrop
  }

  /// Checks if the image is monochrome
  pub fn is_monochrome(&self) -> bool {
    self.photometric == RawPhotometricInterpretation::BlackIsZero
    //self.cpp == 1 && !self.cfa.is_valid()
  }

  /// Find the first matching color matrix by illumninat
  ///
  /// First item in iterator has highest prio
  pub fn color_matrix_find_first(&self, illuminants: impl IntoIterator<Item = Illuminant>) -> Option<(Illuminant, FlatColorMatrix)> {
    for illu in illuminants.into_iter() {
      if let Some(matrix) = self.color_matrix.get(&illu) {
        return Some((illu, matrix.clone()));
      }
    }
    None
  }

  /// Reduction matrix for each color matrix with four color planes
  ///
  /// Matrices of the camera definition are used as they are, missing ones are
  /// replaced by [`Self::calc_reduction_matrix`].
  pub fn reduction_matrices(&self) -> HashMap<Illuminant, FlatColorMatrix> {
    let calculated = self.calc_reduction_matrix().map(|matrix| transform_2d(&matrix));
    self
      .color_matrix
      .iter()
      .filter(|(_, matrix)| matrix.len() == 12)
      .filter_map(|(illuminant, _)| {
        let matrix = self
          .reduction_matrix
          .get(illuminant)
          .filter(|matrix| matrix.len() == 12)
          .or(calculated.as_ref())?;
        Some((*illuminant, matrix.clone()))
      })
      .collect()
  }

  /// Calculate a reduction matrix (3x4) for four color planes from the color matrix
  /// for D65, or from another color matrix adapted to D65 by Bradford.
  ///
  /// The matrix has non-negative entries and each row sums to 1.0, so the camera
  /// white (1, 1, 1, 1) is mapped to (1, 1, 1), values never leave the range
  /// 0.0 .. 1.0 and clipped channels stay mostly clipped. A matrix with negative
  /// entries turns partially clipped highlights into noisy, wrong colors when the
  /// image data is reduced. The matrix is the projection of the least squares
  /// mapping into a wide gamut RGB space onto such matrices.
  pub fn calc_reduction_matrix(&self) -> Option<[[f32; 4]; 3]> {
    let (illuminant, matrix) = self
      .color_matrix
      .iter()
      .filter_map(|(illuminant, matrix)| transform_1d::<4, 3>(matrix).map(|matrix| (*illuminant, matrix)))
      .min_by_key(|(illuminant, _)| d65_preference(illuminant))?;
    let xyz2cam = if illuminant == Illuminant::D65 {
      matrix
    } else if D65_PREFERENCE.contains(&illuminant) {
      adapt_xyz2cam_bradford(&illuminant, &Illuminant::D65, &matrix)
    } else {
      debug!("Color matrix for {:?} can't be adapted to D65, it is used as it is", illuminant);
      matrix
    };
    let prophoto2xyz = pseudo_inverse(XYZ_TO_PROFOTORGB_D50);
    let wide_gamut = pseudo_inverse(normalize(multiply(&xyz2cam, &prophoto2xyz)));
    let projected = project_rows_to_simplex(wide_gamut);
    // The reduced color matrix must stay invertible, compare its determinant
    // to the volume of its rows (Hadamard's inequality).
    let reduced = multiply(&projected, &xyz2cam);
    let volume: f32 = reduced.iter().map(|row| row.iter().map(|v| v * v).sum::<f32>().sqrt()).product();
    if determinant(&reduced).abs() > 0.01 * volume {
      debug!("Reduction matrix {:?} based on color matrix for {:?}", projected, illuminant);
      Some(projected)
    } else {
      log::warn!("Non-negative reduction matrix is degenerate, highlights may clip");
      Some(wide_gamut)
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::imgop::xyz::XYZ_TO_SRGB_D65;

  /// XYZ to camera matrix of a CYGM sensor: cyan = G + B, yellow = R + G, green = G, magenta = R + B
  fn cygm_color_matrix() -> FlatColorMatrix {
    let rgb2cam = [[0.0, 1.0, 1.0], [1.0, 1.0, 0.0], [0.0, 1.0, 0.0], [1.0, 0.0, 1.0]];
    transform_2d(&multiply(&rgb2cam, &XYZ_TO_SRGB_D65))
  }

  /// Four-color image with the given color and reduction matrices
  fn four_color_image(color_matrix: &[(Illuminant, FlatColorMatrix)], reduction_matrix: &[(Illuminant, FlatColorMatrix)]) -> RawImage {
    let mut camera = Camera::new();
    camera.color_matrix = color_matrix.iter().cloned().collect();
    camera.reduction_matrix = reduction_matrix.iter().cloned().collect();
    RawImage::new(
      camera,
      PixU16::new_with(vec![0; 4], 4, 1),
      4,
      [1.0, 1.0, 1.0, 1.0],
      RawPhotometricInterpretation::LinearRaw,
      Some(BlackLevel::zero(1, 1, 4)),
      Some(WhiteLevel::new(vec![1; 4])),
      false,
    )
  }

  #[test]
  fn reduction_matrices_use_camera_definition() {
    let provided = vec![1.0, 0.0, 0.0, 0.0, 0.0, 0.5, 0.0, 0.5, 0.0, 0.0, 1.0, 0.0];
    let image = four_color_image(
      &[(Illuminant::A, cygm_color_matrix()), (Illuminant::D65, cygm_color_matrix())],
      &[(Illuminant::A, provided.clone())],
    );
    let matrices = image.reduction_matrices();
    assert_eq!(matrices.len(), 2);
    assert_eq!(matrices.get(&Illuminant::A), Some(&provided));
    // Missing matrices are calculated, rows are non-negative and sum to 1.0
    let calculated = image.calc_reduction_matrix().expect("reduction matrix");
    assert_eq!(matrices.get(&Illuminant::D65), Some(&transform_2d(&calculated)));
    for row in calculated {
      assert!(row.iter().all(|v| *v >= 0.0), "{:?}", calculated);
      assert!((row.iter().sum::<f32>() - 1.0).abs() < 1e-5, "{:?}", calculated);
    }
  }

  #[test]
  fn reduction_matrix_from_bradford_adapted_matrix() {
    let matrix_a = cygm_color_matrix();
    let adapted = adapt_xyz2cam_bradford(&Illuminant::A, &Illuminant::D65, &transform_1d::<4, 3>(&matrix_a).expect("4x3 matrix"));
    let image_a = four_color_image(&[(Illuminant::A, matrix_a)], &[]);
    let image_d65 = four_color_image(&[(Illuminant::D65, transform_2d(&adapted))], &[]);
    assert_eq!(image_a.calc_reduction_matrix(), image_d65.calc_reduction_matrix());
    assert!(image_a.calc_reduction_matrix().is_some());
  }

  #[test]
  fn no_reduction_matrix_for_three_color_planes() {
    let mut image = four_color_image(&[], &[]);
    image.color_matrix.insert(Illuminant::D65, transform_2d(&XYZ_TO_SRGB_D65));
    assert!(image.calc_reduction_matrix().is_none());
    assert!(image.reduction_matrices().is_empty());
  }

  #[test]
  fn blacklevel_shift() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let black = BlackLevel::new(&[1_u16, 2, 3, 4], 2, 2, 1).shift(1, 1);
    assert_eq!(black.levels, vec![4_u16, 3, 2, 1].into_iter().map(Rational::from).collect::<Vec<Rational>>());

    let black = BlackLevel::new(&[1_u16, 1, 1, 2, 2, 2, 3, 3, 3, 4, 4, 4], 2, 2, 3).shift(3, 3);
    assert_eq!(
      black.levels,
      vec![4_u16, 4, 4, 3, 3, 3, 2, 2, 2, 1, 1, 1]
        .into_iter()
        .map(Rational::from)
        .collect::<Vec<Rational>>()
    );
    Ok(())
  }
}
