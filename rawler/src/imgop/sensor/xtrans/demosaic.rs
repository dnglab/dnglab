use crate::{
  cfa::{PlaneColor, CFA, CFA_COLOR_B, CFA_COLOR_G, CFA_COLOR_R},
  imgop::{
    sensor::bayer::Demosaic,
    Dim2, Point, // Import Point
    Rect,
  },
  pixarray::{Color2D, PixF32},
};
use rayon::prelude::*;

#[derive(Default)]
pub struct XTransDemosaic {}

impl XTransDemosaic {
  pub fn new() -> Self {
    Self {}
  }
}

impl Demosaic<f32, 3> for XTransDemosaic {
  /// Demosaics an X-Trans sensor using a sophisticated 4-pass interpolation method.
  /// This version is corrected to handle the non-uniform X-Trans CFA pattern,
  /// avoiding artifacts by correctly identifying and averaging available neighbor pixels.
  fn demosaic(&self, pixels: &PixF32, cfa: &CFA, _colors: &PlaneColor, roi: Rect) -> Color2D<f32, 3> {
    let mut out = Color2D::<f32, 3>::new(roi.width(), roi.height());
    let cfa = cfa.shift(roi.p.x, roi.p.y);

    // Pass 1: Copy known sensor values into the correct channels of the output buffer.
    for y in 0..roi.height() {
      for x in 0..roi.width() {
        let color_idx = cfa.color_at(y, x);
        if color_idx < 3 {
          // Ensure we only handle R, G, B
          out.at_mut(y, x)[color_idx] = *pixels.at(roi.p.y + y, roi.p.x + x);
        }
      }
    }

    // Create a padded copy for easier border handling during interpolation
    let mut padded = out.make_padded(2);

    // Pass 2: Interpolate Green channel at R and B locations using gradient detection.
    for y in 2..padded.height - 2 {
      for x in 2..padded.width - 2 {
        let roi_y = y - 2;
        let roi_x = x - 2;
        let color_idx = cfa.color_at(roi_y, roi_x);

        if color_idx == CFA_COLOR_R || color_idx == CFA_COLOR_B {
          // Gradients of the current color (R or B). In X-Trans, same-colored
          // neighbors are typically 2 pixels away in cardinal directions.
          let h_grad = (padded.at(y, x - 2)[color_idx] - padded.at(y, x + 2)[color_idx]).abs();
          let v_grad = (padded.at(y - 2, x)[color_idx] - padded.at(y + 2, x)[color_idx]).abs();

          let mut g_h_sum = 0.0;
          let mut g_h_count = 0;
          let mut g_v_sum = 0.0;
          let mut g_v_count = 0;

          // Find and sum horizontal Green neighbors
          if roi_x > 0 && cfa.color_at(roi_y, roi_x - 1) == CFA_COLOR_G {
            g_h_sum += padded.at(y, x - 1)[CFA_COLOR_G];
            g_h_count += 1;
          }
          if cfa.color_at(roi_y, roi_x + 1) == CFA_COLOR_G {
            g_h_sum += padded.at(y, x + 1)[CFA_COLOR_G];
            g_h_count += 1;
          }

          // Find and sum vertical Green neighbors
          if roi_y > 0 && cfa.color_at(roi_y - 1, roi_x) == CFA_COLOR_G {
            g_v_sum += padded.at(y - 1, x)[CFA_COLOR_G];
            g_v_count += 1;
          }
          if cfa.color_at(roi_y + 1, roi_x) == CFA_COLOR_G {
            g_v_sum += padded.at(y + 1, x)[CFA_COLOR_G];
            g_v_count += 1;
          }

          let g_h = if g_h_count > 0 { g_h_sum / g_h_count as f32 } else { 0.0 };
          let g_v = if g_v_count > 0 { g_v_sum / g_v_count as f32 } else { 0.0 };

          let g = if g_h_count == 0 && g_v_count == 0 {
            0.0 // Fallback, should not happen for R/B in X-Trans
          } else if g_h_count == 0 {
            g_v // Only vertical Gs available
          } else if g_v_count == 0 {
            g_h // Only horizontal Gs available
          } else {
            // Both horizontal and vertical Gs exist, use gradient to decide.
            if (h_grad - v_grad).abs() < 0.001 { // Gradients are similar
              (g_h_sum + g_v_sum) / (g_h_count + g_v_count) as f32
            } else if h_grad < v_grad { // Horizontal edge
              g_h
            } else { // Vertical edge
              g_v
            }
          };

          padded.at_mut(y, x)[CFA_COLOR_G] = g;
        }
      }
    }

    // Pass 3: Interpolate R and B at Green locations
    for y in 2..padded.height - 2 {
      for x in 2..padded.width - 2 {
        let roi_y = y - 2;
        let roi_x = x - 2;
        let color_idx = cfa.color_at(roi_y, roi_x);

        if color_idx == CFA_COLOR_G {
          let mut r_sum = 0.0;
          let mut r_count = 0;
          let mut b_sum = 0.0;
          let mut b_count = 0;

          // Left
          if roi_x > 0 {
            match cfa.color_at(roi_y, roi_x - 1) {
              CFA_COLOR_R => { r_sum += padded.at(y, x - 1)[CFA_COLOR_R]; r_count += 1; }
              CFA_COLOR_B => { b_sum += padded.at(y, x - 1)[CFA_COLOR_B]; b_count += 1; }
              _ => {}
            }
          }
          // Right
          match cfa.color_at(roi_y, roi_x + 1) {
            CFA_COLOR_R => { r_sum += padded.at(y, x + 1)[CFA_COLOR_R]; r_count += 1; }
            CFA_COLOR_B => { b_sum += padded.at(y, x + 1)[CFA_COLOR_B]; b_count += 1; }
            _ => {}
          }
          // Top
          if roi_y > 0 {
            match cfa.color_at(roi_y - 1, roi_x) {
              CFA_COLOR_R => { r_sum += padded.at(y - 1, x)[CFA_COLOR_R]; r_count += 1; }
              CFA_COLOR_B => { b_sum += padded.at(y - 1, x)[CFA_COLOR_B]; b_count += 1; }
              _ => {}
            }
          }
          // Bottom
          match cfa.color_at(roi_y + 1, roi_x) {
            CFA_COLOR_R => { r_sum += padded.at(y + 1, x)[CFA_COLOR_R]; r_count += 1; }
            CFA_COLOR_B => { b_sum += padded.at(y + 1, x)[CFA_COLOR_B]; b_count += 1; }
            _ => {}
          }

          if r_count > 0 {
            padded.at_mut(y, x)[CFA_COLOR_R] = r_sum / r_count as f32;
          }
          if b_count > 0 {
            padded.at_mut(y, x)[CFA_COLOR_B] = b_sum / b_count as f32;
          }
        }
      }
    }

    // Pass 4: Interpolate R at B and B at R
    for y in 2..padded.height - 2 {
      for x in 2..padded.width - 2 {
        let roi_y = y - 2;
        let roi_x = x - 2;
        let color_idx = cfa.color_at(roi_y, roi_x);

        if color_idx == CFA_COLOR_R || color_idx == CFA_COLOR_B {
          let mut r_sum = 0.0;
          let mut b_sum = 0.0;
          let mut g_neighbor_count = 0;

          // Look at diagonal Green neighbors and collect their R/B values from Pass 3.
          // Top-Left
          if roi_y > 0 && roi_x > 0 && cfa.color_at(roi_y - 1, roi_x - 1) == CFA_COLOR_G {
            r_sum += padded.at(y - 1, x - 1)[CFA_COLOR_R];
            b_sum += padded.at(y - 1, x - 1)[CFA_COLOR_B];
            g_neighbor_count += 1;
          }
          // Top-Right
          if roi_y > 0 && cfa.color_at(roi_y - 1, roi_x + 1) == CFA_COLOR_G {
            r_sum += padded.at(y - 1, x + 1)[CFA_COLOR_R];
            b_sum += padded.at(y - 1, x + 1)[CFA_COLOR_B];
            g_neighbor_count += 1;
          }
          // Bottom-Left
          if roi_x > 0 && cfa.color_at(roi_y + 1, roi_x - 1) == CFA_COLOR_G {
            r_sum += padded.at(y + 1, x - 1)[CFA_COLOR_R];
            b_sum += padded.at(y + 1, x - 1)[CFA_COLOR_B];
            g_neighbor_count += 1;
          }
          // Bottom-Right
          if cfa.color_at(roi_y + 1, roi_x + 1) == CFA_COLOR_G {
            r_sum += padded.at(y + 1, x + 1)[CFA_COLOR_R];
            b_sum += padded.at(y + 1, x + 1)[CFA_COLOR_B];
            g_neighbor_count += 1;
          }

          if g_neighbor_count > 0 {
            if color_idx == CFA_COLOR_B {
              // Interpolate R at B locations
              padded.at_mut(y, x)[CFA_COLOR_R] = r_sum / g_neighbor_count as f32;
            } else { // color_idx == CFA_COLOR_R
              // Interpolate B at R locations
              padded.at_mut(y, x)[CFA_COLOR_B] = b_sum / g_neighbor_count as f32;
            }
          }
        }
      }
    }

    // Crop the padding off to return the final image
    padded.crop(Rect::new_with_points(
      Point::new(2, 2),
      Point::new(padded.width - 2, padded.height - 2),
    ))
  }
}

#[derive(Default)]
pub struct XTransSuperpixelDemosaic {}

impl XTransSuperpixelDemosaic {
  pub fn new() -> Self {
    Self {}
  }
}

impl Demosaic<f32, 3> for XTransSuperpixelDemosaic {
  fn demosaic(&self, pixels: &PixF32, cfa: &CFA, colors: &PlaneColor, roi: Rect) -> Color2D<f32, 3> {
    let dim = pixels.dim();
    
    // Ensure ROI is within image bounds and adjust to multiple of 6
    let safe_roi = Rect::new(
      roi.p, 
      Dim2::new(
        (roi.width().min(dim.w - roi.p.x) / 6) * 6,
        (roi.height().min(dim.h - roi.p.y) / 6) * 6
      )
    );
    
    if safe_roi.width() == 0 || safe_roi.height() == 0 {
      // Return a minimal image matching the original ROI size
      return Color2D::new_with(vec![[0.0, 0.0, 0.0]; roi.width() * roi.height()], roi.width(), roi.height());
    }

    // The CFA pattern must be shifted according to the ROI's top-left corner.
    let cfa = cfa.shift(safe_roi.p.x, safe_roi.p.y);

    // Create output data that matches the original ROI dimensions
    let mut out_data = vec![[0.0f32; 3]; roi.width() * roi.height()];

    // Process in 6x6 superpixel blocks
    for block_y in 0..(safe_roi.height() / 6) {
      for block_x in 0..(safe_roi.width() / 6) {
        let mut sums = [0.0f32; 3];
        let mut counts = [0u32; 3];

        // Collect values from the 6x6 block
        for y_offset in 0..6 {
          for x_offset in 0..6 {
            let src_y = safe_roi.y() + block_y * 6 + y_offset;
            let src_x = safe_roi.x() + block_x * 6 + x_offset;
            
            if src_y < dim.h && src_x < dim.w {
              let pixel_idx = src_y * dim.w + src_x;
              if pixel_idx < pixels.data.len() {
                let pixel_value = pixels.data[pixel_idx];
                
                // Get the color (R, G, or B) at this position from the shifted CFA.
                let cfa_color_val = cfa.color_at(y_offset, x_offset);
                
                if cfa_color_val == CFA_COLOR_R {
                  sums[0] += pixel_value; // Red channel
                  counts[0] += 1;
                } else if cfa_color_val == CFA_COLOR_G {
                  sums[1] += pixel_value; // Green channel
                  counts[1] += 1;
                } else if cfa_color_val == CFA_COLOR_B {
                  sums[2] += pixel_value; // Blue channel
                  counts[2] += 1;
                }
              }
            }
          }
        }

        // Calculate averages for this superpixel
        let r = if counts[0] > 0 { sums[0] / counts[0] as f32 } else { 0.0 };
        let g = if counts[1] > 0 { sums[1] / counts[1] as f32 } else { 0.0 };
        let b = if counts[2] > 0 { sums[2] / counts[2] as f32 } else { 0.0 };
        let superpixel_color = [r, g, b];

        // Fill the corresponding 6x6 block in the output with this color
        for y_offset in 0..6 {
          for x_offset in 0..6 {
            let out_y = block_y * 6 + y_offset;
            let out_x = block_x * 6 + x_offset;
            
            // Make sure we don't go beyond the safe ROI bounds
            if out_y < safe_roi.height() && out_x < safe_roi.width() {
              let out_idx = out_y * roi.width() + out_x;
              if out_idx < out_data.len() {
                out_data[out_idx] = superpixel_color;
              }
            }
          }
        }
      }
    }

    Color2D::new_with(out_data, roi.width(), roi.height())
  }
}