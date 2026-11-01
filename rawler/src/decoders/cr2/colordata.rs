use crate::{
  RawlerError, Result,
  formats::tiff::{Entry, Rational, Value},
};

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub(crate) struct ColorData {
  pub(crate) version: i16,
  pub(crate) wb: [f32; 4],
  pub(crate) blacklevel: Option<[u16; 4]>,
  pub(crate) normal_whitelevel: Option<u16>,
  pub(crate) specular_whitelevel: Option<u16>,
}

impl ColorData {
  fn new(data: &[u16], version: i16, wb_off: usize, black_off: Option<usize>, norm_white_off: Option<usize>, specular_white_off: Option<usize>) -> Self {
    log::debug!("Found Canon COLORDATA version: {}, data len: {}", version, data.len());
    let wb = [data[wb_off] as f32, data[wb_off + 1] as f32, data[wb_off + 2] as f32, data[wb_off + 3] as f32];
    let blacklevel = black_off.map(|off| [data[off], data[off + 1], data[off + 2], data[off + 3]]);
    let normal_whitelevel = norm_white_off.map(|off| data[off]);
    let specular_whitelevel = specular_white_off.map(|off| data[off]);

    Self {
      version,
      wb,
      blacklevel,
      normal_whitelevel,
      specular_whitelevel,
    }
  }

  /// Bits per sample of the raw data the levels are for, if they don't
  /// follow the raw data
  ///
  /// Version 5 is used by the 450D with 14 bit and the 1000D with 12 bit raw
  /// data, the levels are for 14 bit data.
  fn level_bits(&self) -> Option<usize> {
    (self.version == 5).then_some(14)
  }

  /// Divisor that maps the levels to raw data with `precision` bits per sample
  fn level_divisor(&self, precision: usize) -> u32 {
    match self.level_bits() {
      Some(bits) if precision < bits => 1 << (bits - precision),
      _ => 1,
    }
  }

  /// Black levels for raw data with `precision` bits per sample
  pub(crate) fn blacklevel_for(&self, precision: usize) -> Option<[Rational; 4]> {
    let divisor = self.level_divisor(precision);
    self.blacklevel.map(|levels| {
      levels.map(|level| {
        let gcd = num::integer::gcd(u32::from(level), divisor);
        Rational::new(u32::from(level) / gcd, divisor / gcd)
      })
    })
  }

  /// Specular white level for raw data with `precision` bits per sample
  pub(crate) fn specular_whitelevel_for(&self, precision: usize) -> Option<u32> {
    self.specular_whitelevel.map(|level| u32::from(level) / self.level_divisor(precision))
  }
}

pub(crate) fn parse_colordata(colordata: &Entry) -> Result<ColorData> {
  match &colordata.value {
    Value::Undefined(undef) => {
      let transmuted: Vec<u16> = undef.chunks(2).map(|v| ((v[1] as u16) << 8) | (v[0] as u16)).collect();
      let data = &transmuted;
      let version: i16 = data[0] as i16;

      Ok(match version {
        // -4 (M100/M5/M6)
        -4 => ColorData::new(data, version, 0x47, Some(0x14d), Some(0x0569), Some(0x056a)),
        // -3 (M10/M3)
        -3 => ColorData::new(data, version, 0x47, Some(0x108), None, None),

        _ => return Err(format!("Unknown2 COLORDATA version: {}", data[0]).into()),
      })

      /*
      Ok(match data.len() {
          // 20D and 350D
          582 => ColorData::new(&data, version, 0x19, None, None, None),
          // 1DmkII and 1DSmkII
          653 => ColorData::new(&data, version, 0x22, None, None, None),
          // 1DmkIIN, 5D, 30D, 400D
          796 => ColorData::new(&data, version, 0x3f, Some(0xc4), None, None),
          _ => panic!("COLORDATA count of {} is unknown", data.len())
      })
       */
    }
    Value::Short(data) => {
      match data.len() {
        // 20D and 350D
        582 => return Ok(ColorData::new(data, 0, 0x19, None, None, None)),
        // 1DmkII and 1DSmkII
        653 => return Ok(ColorData::new(data, 0, 0x22, None, None, None)),
        // 1DmkIIN, 5D, 30D, 400D
        796 => return Ok(ColorData::new(data, 0, 0x3f, Some(0xc4), None, None)),
        _ => log::debug!("COLORDATA count of {} is unknown, continue with version matching", data.len()),
      }

      let version: i16 = data[0] as i16;
      /*
      for (i, item) in data.iter().enumerate() {
        if *item > 127 {
        eprintln!("0x{:>4x}: {}", i, item);
        }
      }
       */

      Ok(match version {
        // 1 = (1DmkIIN/5D/30D/400D)
        1 => ColorData::new(data, version, 63, Some(196), None, None),
        // 2 (1DmkIII)
        // 3 (40D)
        2 | 3 => ColorData::new(data, version, 0x3f, Some(0xe7), None, None),
        // 4 (1DSmkIII)
        // 5 (450D/1000D)
        4 | 5 => ColorData::new(data, version, 0x3f, Some(0x2b4), Some(0x2b8), Some(0x2b9)),
        // 6 (50D/5DmkII)
        // 7 (500D/550D/7D/1DmkIV)
        6 | 7 => ColorData::new(data, version, 0x3f, Some(0x2cb), Some(0x2cf), Some(0x2d0)),
        // 9 (60D/1100D)
        9 => ColorData::new(data, version, 0x3f, Some(0x2cf), Some(0x2d3), Some(0x2d4)),
        // -4 (M100/M5/M6)
        -4 => ColorData::new(data, version, 0x47, Some(0x14d), Some(0x0569), Some(0x056a)),
        // -3 (M10/M3)
        -3 => ColorData::new(data, version, 0x47, Some(0x108), None, None),

        // 10 (600D/1200D)
        // 10 (1DX/5DmkIII/6D/70D/100D/650D/700D/M/M2)
        10 => {
          if data.len() == 1273 || data.len() == 1275 {
            ColorData::new(data, version, 0x3f, Some(0x1df), Some(0x1e3), Some(0x1e4))
          } else {
            ColorData::new(data, version, 0x3f, Some(0x1f8), Some(0x1fc), Some(0x1fd))
          }
        }
        // 11 (7DmkII/750D/760D/8000D)
        11 => ColorData::new(data, version, 0x3f, Some(0x2d8), Some(0x2dc), Some(0x2dd)),

        // 12 (1DXmkII/5DS/5DSR)
        12 => ColorData::new(data, version, 0x3f, Some(0x30a), Some(0x30e), Some(0x30f)),
        // 13 (80D/5DmkIV)
        13 => ColorData::new(data, version, 0x3f, Some(0x30a), Some(0x30e), Some(0x30f)),
        // 14 (1300D/2000D/4000D)
        14 => ColorData::new(data, version, 0x3f, Some(0x22c), Some(0x230), Some(0x231)),
        // 15 (6DmkII/77D/200D/800D,9000D)
        15 => ColorData::new(data, version, 0x3f, Some(0x30a), Some(0x30e), Some(0x30f)),
        // 16 (M50)
        // 17 (EOS R)
        // 18 (EOS RP/250D)
        // 19 (90D/850D/M6mkII/M200)
        16 | 17 | 18 | 19 => ColorData::new(data, version, 0x47, Some(0x149), Some(0x31c), Some(0x31d)),
        // 32 (1DXmkIII)
        // 33 (R5/R6)
        32 | 33 => ColorData::new(data, version, 0x55, Some(0x157), Some(0x32a), Some(0x32b)),
        // 34 (R3)
        34 => ColorData::new(data, version, 0x69, Some(0x16b), Some(0x280), Some(0x281)),
        // 48 (R6 Mark II/R7/R8/R10/R50)
        48 => ColorData::new(data, version, 0x69, Some(0x16b), Some(0x280), Some(0x281)),
        // 64 (R5MK2), 65 (R50V), 66 (R6MK3)
        64 | 65 | 66 => ColorData::new(data, version, 0x69, Some(0x17f), Some(0x294), Some(0x295)),

        _ => return Err(format!("Unknown COLORDATA version: {}", data[0]).into()),
      })
    }

    _ => Err(RawlerError::DecoderFailed(format!(
      "Invalid COLORDATA tag: type {}",
      colordata.value_type_name()
    ))),
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  /// COLORDATA of `version` with the black levels at 0x2b4 and the specular white level at 0x2b9
  fn colordata(version: i16) -> ColorData {
    let mut data = vec![0_u16; 0x2c0];
    data[0x2b4..0x2b8].copy_from_slice(&[1023, 1024, 1023, 1023]);
    data[0x2b9] = 15120;
    ColorData::new(&data, version, 0x3f, Some(0x2b4), Some(0x2b8), Some(0x2b9))
  }

  #[test]
  fn version_5_levels_are_scaled_to_the_precision() {
    // 1000D with 12 bit raw data
    let colordata = colordata(5);
    let black = [Rational::new(1023, 4), Rational::new(256, 1), Rational::new(1023, 4), Rational::new(1023, 4)];
    assert_eq!(colordata.blacklevel_for(12), Some(black));
    assert_eq!(colordata.specular_whitelevel_for(12), Some(3780));
    // 450D with 14 bit raw data
    let black = [1023_u32, 1024, 1023, 1023].map(Rational::from);
    assert_eq!(colordata.blacklevel_for(14), Some(black));
    assert_eq!(colordata.specular_whitelevel_for(14), Some(15120));
  }

  #[test]
  fn other_versions_follow_the_raw_data() {
    let colordata = colordata(7);
    let black = [1023_u32, 1024, 1023, 1023].map(Rational::from);
    assert_eq!(colordata.blacklevel_for(12), Some(black));
    assert_eq!(colordata.specular_whitelevel_for(12), Some(15120));
  }
}
