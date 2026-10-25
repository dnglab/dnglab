// SPDX-License-Identifier: LGPL-2.1
// Copyright 2026 Daniel Vogelbacher <daniel@chaospixel.com>

//! Minimal C ABI of the libjxl encoder, mirrored from the libjxl 0.11 headers
//! (`jxl/types.h`, `jxl/codestream_header.h`, `jxl/color_encoding.h`, `jxl/encode.h`).
//!
//! Only the parts used by rawler are declared. libjxl is loaded at runtime, so the
//! functions are declared as function pointer types. C enums are `int` sized.

#![allow(non_camel_case_types)]

use std::ffi::{c_int, c_void};

pub type JXL_BOOL = c_int;
pub const JXL_TRUE: JXL_BOOL = 1;
pub const JXL_FALSE: JXL_BOOL = 0;

// JxlDataType
pub const JXL_TYPE_UINT16: c_int = 3;
pub const JXL_TYPE_FLOAT16: c_int = 5;

// JxlEndianness
pub const JXL_NATIVE_ENDIAN: c_int = 0;

// JxlColorSpace
pub const JXL_COLOR_SPACE_RGB: c_int = 0;
pub const JXL_COLOR_SPACE_GRAY: c_int = 1;

// JxlWhitePoint
pub const JXL_WHITE_POINT_D65: c_int = 1;

// JxlPrimaries
pub const JXL_PRIMARIES_2100: c_int = 9;

// JxlTransferFunction
pub const JXL_TRANSFER_FUNCTION_LINEAR: c_int = 8;
pub const JXL_TRANSFER_FUNCTION_SRGB: c_int = 13;

// JxlRenderingIntent
pub const JXL_RENDERING_INTENT_RELATIVE: c_int = 1;

// JxlEncoderStatus
pub type JxlEncoderStatus = c_int;
pub const JXL_ENC_SUCCESS: JxlEncoderStatus = 0;
pub const JXL_ENC_NEED_MORE_OUTPUT: JxlEncoderStatus = 2;

// JxlEncoderError
pub type JxlEncoderError = c_int;
pub const JXL_ENC_ERR_OK: JxlEncoderError = 0;
pub const JXL_ENC_ERR_GENERIC: JxlEncoderError = 1;
pub const JXL_ENC_ERR_OOM: JxlEncoderError = 2;
pub const JXL_ENC_ERR_JBRD: JxlEncoderError = 3;
pub const JXL_ENC_ERR_BAD_INPUT: JxlEncoderError = 4;
pub const JXL_ENC_ERR_NOT_SUPPORTED: JxlEncoderError = 0x80;
pub const JXL_ENC_ERR_API_USAGE: JxlEncoderError = 0x81;

// JxlEncoderFrameSettingId
pub type JxlEncoderFrameSettingId = c_int;
pub const JXL_ENC_FRAME_SETTING_EFFORT: JxlEncoderFrameSettingId = 0;
pub const JXL_ENC_FRAME_SETTING_DECODING_SPEED: JxlEncoderFrameSettingId = 1;
pub const JXL_ENC_FRAME_SETTING_PATCHES: JxlEncoderFrameSettingId = 8;
pub const JXL_ENC_FRAME_SETTING_EPF: JxlEncoderFrameSettingId = 9;
pub const JXL_ENC_FRAME_SETTING_GABORISH: JxlEncoderFrameSettingId = 10;

/// Opaque encoder instance
#[repr(C)]
pub struct JxlEncoder {
  _private: [u8; 0],
}

/// Opaque frame settings, owned by the encoder
#[repr(C)]
pub struct JxlEncoderFrameSettings {
  _private: [u8; 0],
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct JxlPreviewHeader {
  pub xsize: u32,
  pub ysize: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct JxlAnimationHeader {
  pub tps_numerator: u32,
  pub tps_denominator: u32,
  pub num_loops: u32,
  pub have_timecodes: JXL_BOOL,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct JxlBasicInfo {
  pub have_container: JXL_BOOL,
  pub xsize: u32,
  pub ysize: u32,
  pub bits_per_sample: u32,
  pub exponent_bits_per_sample: u32,
  pub intensity_target: f32,
  pub min_nits: f32,
  pub relative_to_max_display: JXL_BOOL,
  pub linear_below: f32,
  pub uses_original_profile: JXL_BOOL,
  pub have_preview: JXL_BOOL,
  pub have_animation: JXL_BOOL,
  pub orientation: c_int,
  pub num_color_channels: u32,
  pub num_extra_channels: u32,
  pub alpha_bits: u32,
  pub alpha_exponent_bits: u32,
  pub alpha_premultiplied: JXL_BOOL,
  pub preview: JxlPreviewHeader,
  pub animation: JxlAnimationHeader,
  pub intrinsic_xsize: u32,
  pub intrinsic_ysize: u32,
  pub padding: [u8; 100],
}

impl Default for JxlBasicInfo {
  fn default() -> Self {
    Self {
      have_container: JXL_FALSE,
      xsize: 0,
      ysize: 0,
      bits_per_sample: 0,
      exponent_bits_per_sample: 0,
      intensity_target: 0.0,
      min_nits: 0.0,
      relative_to_max_display: JXL_FALSE,
      linear_below: 0.0,
      uses_original_profile: JXL_FALSE,
      have_preview: JXL_FALSE,
      have_animation: JXL_FALSE,
      orientation: 0,
      num_color_channels: 0,
      num_extra_channels: 0,
      alpha_bits: 0,
      alpha_exponent_bits: 0,
      alpha_premultiplied: JXL_FALSE,
      preview: JxlPreviewHeader::default(),
      animation: JxlAnimationHeader::default(),
      intrinsic_xsize: 0,
      intrinsic_ysize: 0,
      padding: [0; 100],
    }
  }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct JxlColorEncoding {
  pub color_space: c_int,
  pub white_point: c_int,
  pub white_point_xy: [f64; 2],
  pub primaries: c_int,
  pub primaries_red_xy: [f64; 2],
  pub primaries_green_xy: [f64; 2],
  pub primaries_blue_xy: [f64; 2],
  pub transfer_function: c_int,
  pub gamma: f64,
  pub rendering_intent: c_int,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct JxlPixelFormat {
  pub num_channels: u32,
  pub data_type: c_int,
  pub endianness: c_int,
  pub align: usize,
}

pub type JxlEncoderVersionFn = unsafe extern "C" fn() -> u32;
pub type JxlEncoderCreateFn = unsafe extern "C" fn(memory_manager: *const c_void) -> *mut JxlEncoder;
pub type JxlEncoderDestroyFn = unsafe extern "C" fn(enc: *mut JxlEncoder);
pub type JxlEncoderGetErrorFn = unsafe extern "C" fn(enc: *mut JxlEncoder) -> JxlEncoderError;
pub type JxlEncoderUseContainerFn = unsafe extern "C" fn(enc: *mut JxlEncoder, use_container: JXL_BOOL) -> JxlEncoderStatus;
pub type JxlEncoderInitBasicInfoFn = unsafe extern "C" fn(info: *mut JxlBasicInfo);
pub type JxlEncoderSetBasicInfoFn = unsafe extern "C" fn(enc: *mut JxlEncoder, info: *const JxlBasicInfo) -> JxlEncoderStatus;
pub type JxlColorEncodingSetToSrgbFn = unsafe extern "C" fn(color_encoding: *mut JxlColorEncoding, is_gray: JXL_BOOL);
pub type JxlEncoderSetColorEncodingFn = unsafe extern "C" fn(enc: *mut JxlEncoder, color: *const JxlColorEncoding) -> JxlEncoderStatus;
pub type JxlEncoderFrameSettingsCreateFn = unsafe extern "C" fn(enc: *mut JxlEncoder, source: *const JxlEncoderFrameSettings) -> *mut JxlEncoderFrameSettings;
pub type JxlEncoderSetFrameLosslessFn = unsafe extern "C" fn(frame_settings: *mut JxlEncoderFrameSettings, lossless: JXL_BOOL) -> JxlEncoderStatus;
pub type JxlEncoderSetFrameDistanceFn = unsafe extern "C" fn(frame_settings: *mut JxlEncoderFrameSettings, distance: f32) -> JxlEncoderStatus;
pub type JxlEncoderFrameSettingsSetOptionFn =
  unsafe extern "C" fn(frame_settings: *mut JxlEncoderFrameSettings, option: JxlEncoderFrameSettingId, value: i64) -> JxlEncoderStatus;
pub type JxlEncoderAddImageFrameFn = unsafe extern "C" fn(
  frame_settings: *const JxlEncoderFrameSettings,
  pixel_format: *const JxlPixelFormat,
  buffer: *const c_void,
  size: usize,
) -> JxlEncoderStatus;
pub type JxlEncoderCloseInputFn = unsafe extern "C" fn(enc: *mut JxlEncoder);
pub type JxlEncoderProcessOutputFn = unsafe extern "C" fn(enc: *mut JxlEncoder, next_out: *mut *mut u8, avail_out: *mut usize) -> JxlEncoderStatus;

/// Name of a libjxl encoder error code
pub fn error_name(error: JxlEncoderError) -> &'static str {
  match error {
    JXL_ENC_ERR_OK => "no error",
    JXL_ENC_ERR_GENERIC => "generic error",
    JXL_ENC_ERR_OOM => "out of memory",
    JXL_ENC_ERR_JBRD => "JPEG bitstream reconstruction error",
    JXL_ENC_ERR_BAD_INPUT => "bad input",
    JXL_ENC_ERR_NOT_SUPPORTED => "not supported",
    JXL_ENC_ERR_API_USAGE => "API usage error",
    _ => "unknown error",
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  /// The structs must match the C layout of libjxl
  #[cfg(target_pointer_width = "64")]
  #[test]
  fn struct_sizes_match_libjxl() {
    assert_eq!(std::mem::size_of::<JxlBasicInfo>(), 204);
    assert_eq!(std::mem::size_of::<JxlColorEncoding>(), 104);
    assert_eq!(std::mem::size_of::<JxlPixelFormat>(), 24);
  }
}
