use serde::{Deserialize, Serialize};

use crate::{
  formats::tiff::{Rational, Result, SRational, Value, IFD},
  lens::LensDescription,
  tags::{ExifGpsTag, ExifTag},
};

use std::convert::TryInto;

/// This struct contains the EXIF information.
/// If a property accepts diffent data types, the type with
/// the best accuracy is choosen.
#[derive(Debug, Default, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Exif {
  pub orientation: Option<u16>,
  pub copyright: Option<String>,
  pub artist: Option<String>,
  pub lens_spec: Option<[Rational; 4]>,
  pub exposure_time: Option<Rational>,
  pub fnumber: Option<Rational>,
  pub aperture_value: Option<Rational>,
  pub brightness_value: Option<Rational>,
  pub iso_speed_ratings: Option<u16>,
  pub iso_speed: Option<u32>,
  pub recommended_exposure_index: Option<u32>,
  pub sensitivity_type: Option<u16>,
  pub exposure_bias: Option<SRational>,
  pub date_time_original: Option<String>,
  pub create_date: Option<String>,
  pub modify_date: Option<String>,
  pub exposure_program: Option<u16>,
  pub timezone_offset: Option<i16>,
  pub offset_time: Option<String>,
  pub offset_time_original: Option<String>,
  pub offset_time_digitized: Option<String>,
  pub sub_sec_time: Option<String>,
  pub sub_sec_time_original: Option<String>,
  pub sub_sec_time_digitized: Option<String>,
  pub shutter_speed_value: Option<SRational>,
  pub max_aperture_value: Option<Rational>,
  pub subject_distance: Option<Rational>,
  pub metering_mode: Option<u16>,
  pub light_source: Option<u16>,
  pub flash: Option<u16>,
  pub focal_length: Option<Rational>,
  pub image_number: Option<u32>,
  pub color_space: Option<u16>,
  pub flash_energy: Option<Rational>,
  pub exposure_mode: Option<u16>,
  pub white_balance: Option<u16>,
  pub scene_capture_type: Option<u16>,
  pub subject_distance_range: Option<u16>,
  pub owner_name: Option<String>,
  pub serial_number: Option<String>,
  pub lens_serial_number: Option<String>,
  pub lens_make: Option<String>,
  pub lens_model: Option<String>,
  pub gps: Option<ExifGPS>,
}

#[derive(Debug, Default, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ExifGPS {
  pub gps_version_id: Option<[u8; 4]>,
  pub gps_latitude_ref: Option<String>,
  pub gps_latitude: Option<[Rational; 3]>,
  pub gps_longitude_ref: Option<String>,
  pub gps_longitude: Option<[Rational; 3]>,
  pub gps_altitude_ref: Option<u8>,
  pub gps_altitude: Option<Rational>,
  pub gps_timestamp: Option<[Rational; 3]>,
  pub gps_satellites: Option<String>,
  pub gps_status: Option<String>,
  pub gps_measure_mode: Option<String>,
  pub gps_dop: Option<Rational>,
  pub gps_speed_ref: Option<String>,
  pub gps_speed: Option<Rational>,
  pub gps_track_ref: Option<String>,
  pub gps_track: Option<Rational>,
  pub gps_img_direction_ref: Option<String>,
  pub gps_img_direction: Option<Rational>,
  pub gps_map_datum: Option<String>,
  pub gps_dest_latitude_ref: Option<String>,
  pub gps_dest_latitude: Option<[Rational; 3]>,
  pub gps_dest_longitude_ref: Option<String>,
  pub gps_dest_longitude: Option<[Rational; 3]>,
  pub gps_dest_bearing_ref: Option<String>,
  pub gps_dest_bearing: Option<Rational>,
  pub gps_dest_distance_ref: Option<String>,
  pub gps_dest_distance: Option<Rational>,
  pub gps_processing_method: Option<Vec<u8>>,
  pub gps_area_information: Option<Vec<u8>>,
  pub gps_date_stamp: Option<String>,
  pub gps_differential: Option<u16>,
  pub gps_h_positioning_error: Option<Rational>,
}

impl Exif {
  /// Read EXIF data. As some EXIF tags located in the root IFD,
  /// we accept both IFDs here.
  pub fn new(root_or_exif: &IFD) -> Result<Self> {
    let mut ins = Self::default();
    ins.extend_from_ifd(root_or_exif)?;
    if let Some((_, exif_ifd)) = root_or_exif.sub_ifds().iter().find(|(tag, _)| **tag == ExifTag::ExifOffset as u16) {
      ins.extend_from_ifd(&exif_ifd[0])?;
    }
    if let Some((_, gps_ifd)) = root_or_exif.sub_ifds().iter().find(|(tag, _)| **tag == ExifTag::GPSInfo as u16) {
      ins.extend_from_gps_ifd(&gps_ifd[0])?;
    }
    Ok(ins)
  }

  /// Extend the EXIF info from this IFD. If the IFD contains a ExifIFD,
  /// extend from this IFD, too.
  pub fn extend_from_ifd(&mut self, ifd: &IFD) -> Result<()> {
    let trim = |a: &String| -> String { a.trim().into() };
    for (tag, entry) in ifd.entries().iter() {
      // First try EXIF tags
      if let Ok(tag) = ExifTag::try_from(*tag) {
        match (tag, &entry.value) {
          (ExifTag::Orientation, Value::Short(data)) => self.orientation = data.first().cloned(),
          (ExifTag::Copyright, Value::Ascii(data)) => self.copyright = data.strings().get(0).map(trim),
          (ExifTag::Artist, Value::Ascii(data)) => self.artist = data.strings().get(0).map(trim),
          (ExifTag::ExposureTime, Value::Rational(data)) => self.exposure_time = data.get(0).cloned(),
          (ExifTag::FNumber, Value::Rational(data)) => self.fnumber = data.get(0).cloned(),
          (ExifTag::BrightnessValue, Value::Rational(data)) => self.brightness_value = data.get(0).cloned(),
          (ExifTag::ApertureValue, Value::Rational(data)) => self.aperture_value = data.get(0).cloned(),
          (ExifTag::ISOSpeedRatings, Value::Short(data)) => self.iso_speed_ratings = data.first().cloned(),
          (ExifTag::ISOSpeed, Value::Long(data)) => self.iso_speed = data.first().cloned(),
          (ExifTag::RecommendedExposureIndex, Value::Long(data)) => self.recommended_exposure_index = data.first().cloned(),
          (ExifTag::SensitivityType, Value::Short(data)) => self.sensitivity_type = data.first().cloned(),
          (ExifTag::ExposureBiasValue, Value::SRational(data)) => self.exposure_bias = data.get(0).cloned(),
          (ExifTag::DateTimeOriginal, Value::Ascii(data)) => self.date_time_original = data.strings().get(0).cloned(),
          (ExifTag::CreateDate, Value::Ascii(data)) => self.create_date = data.strings().get(0).cloned(),
          (ExifTag::ModifyDate, Value::Ascii(data)) => self.modify_date = data.strings().get(0).cloned(),
          (ExifTag::ExposureProgram, Value::Short(data)) => self.exposure_program = data.first().cloned(),
          (ExifTag::TimeZoneOffset, Value::SShort(data)) => self.timezone_offset = data.first().cloned(),
          (ExifTag::OffsetTime, Value::Ascii(data)) => self.offset_time = data.strings().get(0).cloned(),
          (ExifTag::OffsetTimeOriginal, Value::Ascii(data)) => self.offset_time_original = data.strings().get(0).cloned(),
          (ExifTag::OffsetTimeDigitized, Value::Ascii(data)) => self.offset_time_digitized = data.strings().get(0).cloned(),
          (ExifTag::SubSecTime, Value::Ascii(data)) => self.sub_sec_time = data.strings().get(0).cloned(),
          (ExifTag::SubSecTimeOriginal, Value::Ascii(data)) => self.sub_sec_time_original = data.strings().get(0).cloned(),
          (ExifTag::SubSecTimeDigitized, Value::Ascii(data)) => self.sub_sec_time_digitized = data.strings().get(0).cloned(),
          (ExifTag::ShutterSpeedValue, Value::SRational(data)) => self.shutter_speed_value = data.get(0).cloned(),
          (ExifTag::MaxApertureValue, Value::Rational(data)) => self.max_aperture_value = data.get(0).cloned(),
          (ExifTag::SubjectDistance, Value::Rational(data)) => self.subject_distance = data.get(0).cloned(),
          (ExifTag::MeteringMode, Value::Short(data)) => self.metering_mode = data.first().cloned(),
          (ExifTag::LightSource, Value::Short(data)) => self.light_source = data.first().cloned(),
          (ExifTag::Flash, Value::Short(data)) => self.flash = data.first().cloned(),
          (ExifTag::FocalLength, Value::Rational(data)) => self.focal_length = data.get(0).cloned(),
          (ExifTag::ImageNumber, Value::Long(data)) => self.image_number = data.first().cloned(),
          (ExifTag::ColorSpace, Value::Short(data)) => self.color_space = data.first().cloned(),
          (ExifTag::FlashEnergy, Value::Rational(data)) => self.flash_energy = data.get(0).cloned(),
          (ExifTag::ExposureMode, Value::Short(data)) => self.exposure_mode = data.first().cloned(),
          (ExifTag::WhiteBalance, Value::Short(data)) => self.white_balance = data.first().cloned(),
          (ExifTag::SceneCaptureType, Value::Short(data)) => self.scene_capture_type = data.first().cloned(),
          (ExifTag::SubjectDistanceRange, Value::Short(data)) => self.subject_distance_range = data.first().cloned(),
          (ExifTag::OwnerName, Value::Ascii(data)) => self.owner_name = data.strings().get(0).map(trim),
          (ExifTag::SerialNumber, Value::Ascii(data)) => self.serial_number = data.strings().get(0).map(trim),
          (ExifTag::LensSerialNumber, Value::Ascii(data)) => self.lens_serial_number = data.strings().get(0).map(trim),
          (tag, _value) => {
            log::debug!("Ignoring EXIF tag: {:?}", tag);
          }
        }
      }
    }
    Ok(())
  }

  /// Extend the EXIF info from this IFD. If the IFD contains a ExifIFD,
  /// extend from this IFD, too.
  pub fn extend_from_gps_ifd(&mut self, ifd: &IFD) -> Result<()> {
    for (tag, entry) in ifd.entries().iter() {
      if let Ok(tag) = ExifGpsTag::try_from(*tag) {
        // We hit a GPS tag, make sure the gps property is initialized.
        if self.gps.is_none() {
          self.gps = Some(ExifGPS::default());
        }
        if let Some(gps) = &mut self.gps {
          match (tag, &entry.value) {
            (ExifGpsTag::GPSVersionID, Value::Byte(data)) => gps.gps_version_id = data.clone().try_into().ok(),
            (ExifGpsTag::GPSLatitudeRef, Value::Ascii(data)) => gps.gps_latitude_ref = data.strings().get(0).cloned(),
            (ExifGpsTag::GPSLatitude, Value::Rational(data)) => gps.gps_latitude = data.clone().try_into().ok(),
            (ExifGpsTag::GPSLongitudeRef, Value::Ascii(data)) => gps.gps_longitude_ref = data.strings().get(0).cloned(),
            (ExifGpsTag::GPSLongitude, Value::Rational(data)) => gps.gps_longitude = data.clone().try_into().ok(),
            (ExifGpsTag::GPSAltitudeRef, Value::Byte(data)) => gps.gps_altitude_ref = data.first().cloned(),
            (ExifGpsTag::GPSAltitude, Value::Rational(data)) => gps.gps_altitude = data.get(0).cloned(),
            (ExifGpsTag::GPSTimeStamp, Value::Rational(data)) => gps.gps_timestamp = data.clone().try_into().ok(),
            (ExifGpsTag::GPSSatellites, Value::Ascii(data)) => gps.gps_satellites = data.strings().get(0).cloned(),
            (ExifGpsTag::GPSStatus, Value::Ascii(data)) => gps.gps_status = data.strings().get(0).cloned(),
            (ExifGpsTag::GPSMeasureMode, Value::Ascii(data)) => gps.gps_measure_mode = data.strings().get(0).cloned(),
            (ExifGpsTag::GPSDOP, Value::Rational(data)) => gps.gps_dop = data.get(0).cloned(),
            (ExifGpsTag::GPSSpeedRef, Value::Ascii(data)) => gps.gps_speed_ref = data.strings().get(0).cloned(),
            (ExifGpsTag::GPSSpeed, Value::Rational(data)) => gps.gps_speed = data.get(0).cloned(),
            (ExifGpsTag::GPSTrackRef, Value::Ascii(data)) => gps.gps_track_ref = data.strings().get(0).cloned(),
            (ExifGpsTag::GPSTrack, Value::Rational(data)) => gps.gps_track = data.get(0).cloned(),
            (ExifGpsTag::GPSImgDirectionRef, Value::Ascii(data)) => gps.gps_img_direction_ref = data.strings().get(0).cloned(),
            (ExifGpsTag::GPSImgDirection, Value::Rational(data)) => gps.gps_img_direction = data.get(0).cloned(),
            (ExifGpsTag::GPSMapDatum, Value::Ascii(data)) => gps.gps_map_datum = data.strings().get(0).cloned(),
            (ExifGpsTag::GPSDestLatitudeRef, Value::Ascii(data)) => gps.gps_dest_latitude_ref = data.strings().get(0).cloned(),
            (ExifGpsTag::GPSDestLatitude, Value::Rational(data)) => gps.gps_dest_latitude = data.clone().try_into().ok(),
            (ExifGpsTag::GPSDestLongitudeRef, Value::Ascii(data)) => gps.gps_dest_longitude_ref = data.strings().get(0).cloned(),
            (ExifGpsTag::GPSDestLongitude, Value::Rational(data)) => gps.gps_dest_longitude = data.clone().try_into().ok(),
            (ExifGpsTag::GPSDestBearingRef, Value::Ascii(data)) => gps.gps_dest_bearing_ref = data.strings().get(0).cloned(),
            (ExifGpsTag::GPSDestBearing, Value::Rational(data)) => gps.gps_dest_bearing = data.get(0).cloned(),
            (ExifGpsTag::GPSDestDistanceRef, Value::Ascii(data)) => gps.gps_dest_distance_ref = data.strings().get(0).cloned(),
            (ExifGpsTag::GPSDestDistance, Value::Rational(data)) => gps.gps_dest_distance = data.get(0).cloned(),
            (ExifGpsTag::GPSProcessingMethod, Value::Undefined(data)) => gps.gps_processing_method = Some(data.clone()),
            (ExifGpsTag::GPSAreaInformation, Value::Undefined(data)) => gps.gps_area_information = Some(data.clone()),
            (ExifGpsTag::GPSDateStamp, Value::Ascii(data)) => gps.gps_date_stamp = data.strings().get(0).cloned(),
            (ExifGpsTag::GPSDifferential, Value::Short(data)) => gps.gps_differential = data.first().cloned(),
            (ExifGpsTag::GPSHPositioningError, Value::Rational(data)) => gps.gps_h_positioning_error = data.get(0).cloned(),
            (tag, _value) => {
              log::debug!("Ignoring EXIF tag: {:?}", tag);
            }
          }
        }
      }
    }
    Ok(())
  }

  pub(crate) fn extend_from_lens(&mut self, lens: &LensDescription) {
    let lens_info: [Rational; 4] = [lens.focal_range[0], lens.focal_range[1], lens.aperture_range[0], lens.aperture_range[1]];
    self.lens_spec = Some(lens_info);
    self.lens_make = Some(lens.lens_make.clone());
    self.lens_model = Some(lens.lens_model.clone());
  }
}
