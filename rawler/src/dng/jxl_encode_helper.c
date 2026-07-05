// SPDX-License-Identifier: LGPL-2.1
// Copyright 2025 Daniel Vogelbacher <daniel@chaospixel.com>
//
// Thin C shim around libjxl encoding.  Keeping the JXL struct layout on the C
// side avoids the need to replicate every field of JxlBasicInfo / JxlPixelFormat
// in Rust FFI declarations, and lets us compile against any 0.11.x release of
// libjxl regardless of minor-patch differences that trip up version-pinned
// Cargo crates.

#include <jxl/encode.h>
#include <jxl/color_encoding.h>
#include <stdint.h>
#include <stdlib.h>
#include <string.h>

/// Encode a planar u16 raster into a JXL bare bitstream.
///
/// @param pixels       Interleaved u16 pixel data, native endian.
/// @param width        Image width in pixels.
/// @param height       Image height in pixels.
/// @param num_channels 1 (grayscale) or 3 (RGB).
/// @param bits_per_sample  Actual bit depth stored in each u16 (8–16).
/// @param distance     JXL distance: 0.0 = lossless, 1.0 ≈ visually lossless.
/// @param effort       Encoder effort 1 (fast) … 10 (best quality), default 7.
/// @param output       Receives a malloc-allocated byte buffer on success.
/// @param output_size  Receives the number of valid bytes in *output.
///
/// @return 0 on success, non-zero on error.  The caller must free *output with
///         rawler_jxl_free() after a successful call.
int rawler_jxl_encode(
    const uint16_t* pixels,
    uint32_t width,
    uint32_t height,
    uint32_t num_channels,
    uint32_t bits_per_sample,
    float distance,
    uint32_t effort,
    uint8_t** output,
    size_t* output_size)
{
  *output = NULL;
  *output_size = 0;

  JxlEncoder* enc = JxlEncoderCreate(NULL);
  if (!enc) return 1;

  /* Basic image info */
  JxlBasicInfo info;
  JxlEncoderInitBasicInfo(&info);
  info.xsize = width;
  info.ysize = height;
  info.num_color_channels = (num_channels >= 3) ? 3 : 1;
  info.num_extra_channels = 0;
  info.bits_per_sample = bits_per_sample;
  info.exponent_bits_per_sample = 0; /* integer, not float */
  info.alpha_bits = 0;
  info.alpha_exponent_bits = 0;
  info.alpha_premultiplied = 0;

  if (JxlEncoderSetBasicInfo(enc, &info) != JXL_ENC_SUCCESS) {
    JxlEncoderDestroy(enc);
    return 2;
  }

  /* Color encoding – use linear light (no gamma) so raw values are preserved */
  JxlColorEncoding color_enc;
  memset(&color_enc, 0, sizeof(color_enc));
  if (num_channels >= 3) {
    JxlColorEncodingSetToLinearSRGB(&color_enc, JXL_FALSE);
  } else {
    JxlColorEncodingSetToLinearSRGB(&color_enc, JXL_TRUE);
  }
  if (JxlEncoderSetColorEncoding(enc, &color_enc) != JXL_ENC_SUCCESS) {
    JxlEncoderDestroy(enc);
    return 3;
  }

  /* Frame settings */
  JxlEncoderFrameSettings* fs = JxlEncoderFrameSettingsCreate(enc, NULL);
  if (!fs) {
    JxlEncoderDestroy(enc);
    return 4;
  }

  int lossless = (distance <= 0.0f);
  if (JxlEncoderSetFrameLossless(fs, lossless ? JXL_TRUE : JXL_FALSE) != JXL_ENC_SUCCESS) {
    JxlEncoderDestroy(enc);
    return 5;
  }
  if (!lossless) {
    if (JxlEncoderSetFrameDistance(fs, distance) != JXL_ENC_SUCCESS) {
      JxlEncoderDestroy(enc);
      return 6;
    }
  }
  if (JxlEncoderFrameSettingsSetOption(fs, JXL_ENC_FRAME_SETTING_EFFORT, (int64_t)effort) != JXL_ENC_SUCCESS) {
    JxlEncoderDestroy(enc);
    return 7;
  }

  /* Add the image frame */
  JxlPixelFormat fmt;
  fmt.num_channels = (num_channels >= 3) ? 3 : 1;
  fmt.data_type = JXL_TYPE_UINT16;
  fmt.endianness = JXL_NATIVE_ENDIAN;
  fmt.align = 0;

  size_t pixels_bytes = (size_t)width * height * fmt.num_channels * sizeof(uint16_t);
  if (JxlEncoderAddImageFrame(fs, &fmt, pixels, pixels_bytes) != JXL_ENC_SUCCESS) {
    JxlEncoderDestroy(enc);
    return 8;
  }
  JxlEncoderCloseFrames(enc);

  /* Collect compressed output */
  size_t cap = 65536;
  uint8_t* buf = (uint8_t*)malloc(cap);
  if (!buf) {
    JxlEncoderDestroy(enc);
    return 9;
  }
  size_t used = 0;

  for (;;) {
    size_t avail = cap - used;
    uint8_t* next = buf + used;
    JxlEncoderStatus st = JxlEncoderProcessOutput(enc, &next, &avail);
    used = cap - avail;

    if (st == JXL_ENC_SUCCESS) break;
    if (st == JXL_ENC_NEED_MORE_OUTPUT) {
      cap *= 2;
      uint8_t* nb = (uint8_t*)realloc(buf, cap);
      if (!nb) {
        free(buf);
        JxlEncoderDestroy(enc);
        return 10;
      }
      buf = nb;
      continue;
    }
    /* Error */
    free(buf);
    JxlEncoderDestroy(enc);
    return 11;
  }

  JxlEncoderDestroy(enc);
  *output = buf;
  *output_size = used;
  return 0;
}

/// Free a buffer returned by rawler_jxl_encode.
void rawler_jxl_free(uint8_t* ptr)
{
  free(ptr);
}
