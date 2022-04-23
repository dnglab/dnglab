use crate::Result;
use crate::{decoders::nef::NikonMakernote, formats::tiff::IFD};

const ERRMSG: &str = "Lens composite buffer error: EOF";

#[derive(Default, Clone)]
#[allow(dead_code)]
pub struct NefLensData {
  version: u32,
  exit_pupil_position: u8,
  af_aperture: u8,
  focus_position: u8,
  focus_distance: u8,
  lens_id_number: u8,
  lens_fstops: u8,
  min_focal_len: u8,
  max_focal_len: u8,
  max_aperture_at_min_focal: u8,
  max_aperture_at_max_focal: u8,
  mcu_version: u8,
  effective_max_aperture: u8,
  lens_model: Option<String>,
}

impl NefLensData {
  pub fn composite_id(&self, lens_type: u8) -> String {
    format!(
      "{:02X} {:02X} {:02X} {:02X} {:02X} {:02X} {:02X} {:02X}",
      self.lens_id_number,
      self.lens_fstops,
      self.min_focal_len,
      self.max_focal_len,
      self.max_aperture_at_min_focal,
      self.max_aperture_at_max_focal,
      self.mcu_version,
      lens_type,
    )
  }
}

pub(super) fn from_makernote(makernote: &IFD) -> Result<Option<NefLensData>> {
  if let Some(levels) = makernote.get_entry(NikonMakernote::LensData) {
    let mut buf = levels.get_data().clone();

    let mut version: u32 = 0;
    for i in 0..4 {
      version = (version << 4) + (buf[i] - b'0') as u32;
    }

    let lensdata = match version {
      0x100 => parse_lensdata_0x100(version, &buf)?,
      0x101 => parse_lensdata_0x101(version, &buf)?,
      0x201 | 0x202 | 0x203 => {
        super::decrypt::nef_decrypt(&mut buf, 4, makernote)?;
        parse_lensdata_0x101(version, &buf)?
      }
      0x204 => {
        super::decrypt::nef_decrypt(&mut buf, 4, makernote)?;
        parse_lensdata_0x204(version, &buf)?
      }
      0x400 => {
        super::decrypt::nef_decrypt(&mut buf, 4, makernote)?;
        parse_lensdata_0x4xx(version, &buf, 0x18a)?
      }
      0x401 => {
        super::decrypt::nef_decrypt(&mut buf, 4, makernote)?;
        parse_lensdata_0x4xx(version, &buf, 0x18a)?
      }
      0x402 => {
        super::decrypt::nef_decrypt(&mut buf, 4, makernote)?;
        parse_lensdata_0x4xx(version, &buf, 0x18b)?
      }
      0x403 => {
        super::decrypt::nef_decrypt(&mut buf, 4, makernote)?;
        parse_lensdata_0x4xx(version, &buf, 0x2ac)?
      }
      0x800 => {
        super::decrypt::nef_decrypt(&mut buf, 4, makernote)?;
        parse_lensdata_0x800(version, &buf)?
      }

      _ => todo!("Lensdata version: 0x{:x} not implemented", version),
    };

    log::debug!("NEF lens data version: 0x{:x}", version);

    Ok(Some(lensdata))
  } else {
    Ok(None)
  }
}

fn parse_lensdata_0x100(version: u32, buf: &[u8]) -> Result<NefLensData> {
  Ok(NefLensData {
    version,
    exit_pupil_position: 0,
    af_aperture: 0,
    focus_position: 0,
    focus_distance: 0,
    lens_id_number: *buf.get(NefLensData00::LensIDNumber as usize).ok_or(ERRMSG)?,
    lens_fstops: *buf.get(NefLensData00::LensFStops as usize).ok_or(ERRMSG)?,
    min_focal_len: *buf.get(NefLensData00::MinFocalLength as usize).ok_or(ERRMSG)?,
    max_focal_len: *buf.get(NefLensData00::MaxFocalLength as usize).ok_or(ERRMSG)?,
    max_aperture_at_min_focal: *buf.get(NefLensData00::MaxApertureAtMinFocal as usize).ok_or(ERRMSG)?,
    max_aperture_at_max_focal: *buf.get(NefLensData00::MaxApertureAtMaxFocal as usize).ok_or(ERRMSG)?,
    mcu_version: *buf.get(NefLensData00::MCUVersion as usize).ok_or(ERRMSG)?,
    effective_max_aperture: 0,
    lens_model: None,
  })
}

fn parse_lensdata_0x101(version: u32, buf: &[u8]) -> Result<NefLensData> {
  Ok(NefLensData {
    version,
    exit_pupil_position: *buf.get(NefLensData01::ExitPupilPosition as usize).ok_or(ERRMSG)?,
    af_aperture: *buf.get(NefLensData01::AFAperture as usize).ok_or(ERRMSG)?,
    focus_position: *buf.get(NefLensData01::FocusPosition as usize).ok_or(ERRMSG)?,
    focus_distance: *buf.get(NefLensData01::FocusDistance as usize).ok_or(ERRMSG)?,
    lens_id_number: *buf.get(NefLensData01::LensIDNumber as usize).ok_or(ERRMSG)?,
    lens_fstops: *buf.get(NefLensData01::LensFStops as usize).ok_or(ERRMSG)?,
    min_focal_len: *buf.get(NefLensData01::MinFocalLength as usize).ok_or(ERRMSG)?,
    max_focal_len: *buf.get(NefLensData01::MaxFocalLength as usize).ok_or(ERRMSG)?,
    max_aperture_at_min_focal: *buf.get(NefLensData01::MaxApertureAtMinFocal as usize).ok_or(ERRMSG)?,
    max_aperture_at_max_focal: *buf.get(NefLensData01::MaxApertureAtMaxFocal as usize).ok_or(ERRMSG)?,
    mcu_version: *buf.get(NefLensData01::MCUVersion as usize).ok_or(ERRMSG)?,
    effective_max_aperture: *buf.get(NefLensData01::EffectiveMaxAperture as usize).ok_or(ERRMSG)?,
    lens_model: None,
  })
}

fn parse_lensdata_0x204(version: u32, buf: &[u8]) -> Result<NefLensData> {
  Ok(NefLensData {
    version,
    exit_pupil_position: *buf.get(NefLensData204::ExitPupilPosition as usize).ok_or(ERRMSG)?,
    af_aperture: *buf.get(NefLensData204::AFAperture as usize).ok_or(ERRMSG)?,
    focus_position: *buf.get(NefLensData204::FocusPosition as usize).ok_or(ERRMSG)?,
    focus_distance: *buf.get(NefLensData204::FocusDistance as usize).ok_or(ERRMSG)?,
    lens_id_number: *buf.get(NefLensData204::LensIDNumber as usize).ok_or(ERRMSG)?,
    lens_fstops: *buf.get(NefLensData204::LensFStops as usize).ok_or(ERRMSG)?,
    min_focal_len: *buf.get(NefLensData204::MinFocalLength as usize).ok_or(ERRMSG)?,
    max_focal_len: *buf.get(NefLensData204::MaxFocalLength as usize).ok_or(ERRMSG)?,
    max_aperture_at_min_focal: *buf.get(NefLensData204::MaxApertureAtMinFocal as usize).ok_or(ERRMSG)?,
    max_aperture_at_max_focal: *buf.get(NefLensData204::MaxApertureAtMaxFocal as usize).ok_or(ERRMSG)?,
    mcu_version: *buf.get(NefLensData204::MCUVersion as usize).ok_or(ERRMSG)?,
    effective_max_aperture: *buf.get(NefLensData204::EffectiveMaxAperture as usize).ok_or(ERRMSG)?,
    lens_model: None,
  })
}

fn parse_lensdata_0x4xx(version: u32, buf: &[u8], model_offset: usize) -> Result<NefLensData> {
  let mut data = NefLensData { version, ..Default::default() };
  if buf.len() >= model_offset + 64 {
    let str = String::from_utf8_lossy(&buf[model_offset..model_offset + 64]);
    data.lens_model = Some(str.trim().into());
  }
  Ok(data)
}

fn parse_lensdata_0x800(version: u32, buf: &[u8]) -> Result<NefLensData> {
  if buf[0x03] != 0 {
    log::debug!("Found old lens data");
    Ok(NefLensData {
      version,
      exit_pupil_position: *buf.get(NefLensData800::ExitPupilPosition as usize).ok_or(ERRMSG)?,
      af_aperture: *buf.get(NefLensData800::AFAperture as usize).ok_or(ERRMSG)?,
      focus_position: *buf.get(NefLensData800::FocusPosition as usize).ok_or(ERRMSG)?,
      focus_distance: *buf.get(NefLensData800::FocusDistance as usize).ok_or(ERRMSG)?,
      lens_id_number: *buf.get(NefLensData800::LensIDNumber as usize).ok_or(ERRMSG)?,
      lens_fstops: *buf.get(NefLensData800::LensFStops as usize).ok_or(ERRMSG)?,
      min_focal_len: *buf.get(NefLensData800::MinFocalLength as usize).ok_or(ERRMSG)?,
      max_focal_len: *buf.get(NefLensData800::MaxFocalLength as usize).ok_or(ERRMSG)?,
      max_aperture_at_min_focal: *buf.get(NefLensData800::MaxApertureAtMinFocal as usize).ok_or(ERRMSG)?,
      max_aperture_at_max_focal: *buf.get(NefLensData800::MaxApertureAtMaxFocal as usize).ok_or(ERRMSG)?,
      mcu_version: *buf.get(NefLensData800::MCUVersion as usize).ok_or(ERRMSG)?,
      effective_max_aperture: *buf.get(NefLensData800::EffectiveMaxAperture as usize).ok_or(ERRMSG)?,
      lens_model: None,
    })
  } else {
    log::debug!("Found new lens data");
    todo!();
    //let data = NefLensData::default();
    // TODO: Add Z Lens decoding
    // Some values here are u16 in little endian!
    //Ok(data)
  }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[allow(non_camel_case_types)]
#[allow(dead_code)]
enum NefLensData00 {
  Version = 0x00,
  LensIDNumber = 0x06,
  LensFStops = 0x07,
  MinFocalLength = 0x08,
  MaxFocalLength = 0x09,
  MaxApertureAtMinFocal = 0x0a,
  MaxApertureAtMaxFocal = 0x0b,
  MCUVersion = 0x0c,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[allow(non_camel_case_types)]
#[allow(dead_code)]
enum NefLensData01 {
  Version = 0x00,
  ExitPupilPosition = 0x04,
  AFAperture = 0x05,
  FocusPosition = 0x08,
  FocusDistance = 0x09,
  FocalLength = 0x0a,
  LensIDNumber = 0x0b,
  LensFStops = 0x0c,
  MinFocalLength = 0x0d,
  MaxFocalLength = 0x0e,
  MaxApertureAtMinFocal = 0x0f,
  MaxApertureAtMaxFocal = 0x10,
  MCUVersion = 0x11,
  EffectiveMaxAperture = 0x12,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[allow(non_camel_case_types)]
#[allow(dead_code)]
enum NefLensData204 {
  Version = 0x00,
  ExitPupilPosition = 0x04,
  AFAperture = 0x05,
  FocusPosition = 0x08,
  FocusDistance = 0x0a,
  FocalLength = 0x0b,
  LensIDNumber = 0x0c,
  LensFStops = 0x0d,
  MinFocalLength = 0x0e,
  MaxFocalLength = 0x0f,
  MaxApertureAtMinFocal = 0x10,
  MaxApertureAtMaxFocal = 0x11,
  MCUVersion = 0x12,
  EffectiveMaxAperture = 0x13,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[allow(non_camel_case_types)]
#[allow(dead_code)]
enum NefLensData800 {
  Version = 0x00,
  OldLensDataFlag = 0x03,
  ExitPupilPosition = 0x04,
  AFAperture = 0x05,
  FocusPosition = 0x09,
  FocusDistance = 0x0b,
  FocalLength = 0x0c,
  LensIDNumber = 0x0d,
  LensFStops = 0x0e,
  MinFocalLength = 0x0f,
  MaxFocalLength = 0x10,
  MaxApertureAtMinFocal = 0x11,
  MaxApertureAtMaxFocal = 0x12,
  MCUVersion = 0x13,
  EffectiveMaxAperture = 0x14,
}
