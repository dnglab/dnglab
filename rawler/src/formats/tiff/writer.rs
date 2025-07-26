// SPDX-License-Identifier: MIT
// Copyright 2021 Daniel Vogelbacher <daniel@chaospixel.com>

use std::{
  collections::BTreeMap,
  io::{Seek, SeekFrom, Write},
};

use byteorder::{LittleEndian, NativeEndian, WriteBytesExt};
use image::EncodableLayout;
use rayon::prelude::*;
use weezl::LzwError;

use crate::{
  exif::Exif,
  imgop::Dim2,
  tags::{ExifGpsTag, ExifTag, TiffTag},
};

use super::{Entry, Result, TIFF_MAGIC, TiffError, Value};

pub struct TiffWriter<W> {
  ifd_location: u64,
  pub writer: W,
}

impl<W> TiffWriter<W>
where
  W: Write + Seek,
{
  pub fn new(writer: W) -> Result<Self> {
    let mut tmp = Self { writer, ifd_location: 0 };
    tmp.write_header()?;
    Ok(tmp)
  }

  pub fn new_directory(&self) -> DirectoryWriter {
    DirectoryWriter::new()
  }

  fn write_header(&mut self) -> Result<()> {
    #[cfg(target_endian = "little")]
    let boi: u8 = 0x49;
    #[cfg(not(target_endian = "little"))]
    let boi: u8 = 0x4d;

    self.writer.write_all(&[boi, boi])?;
    self.writer.write_u16::<NativeEndian>(TIFF_MAGIC)?;
    self.ifd_location = self.writer.stream_position()?;
    self.writer.write_u32::<NativeEndian>(0_u32)?;

    Ok(())
  }

  pub fn write_strips_lzw(&mut self, data: &[u16], cpp: usize, dim: Dim2, mut strip_lines: usize) -> Result<(u32, Vec<(u32, u32)>)> {
    if strip_lines == 0 {
      if dim.h > 256 {
        strip_lines = 256;
      } else {
        strip_lines = dim.h; // Use single strip
      }
    }
    let mut tag_data = Vec::with_capacity(dim.h / strip_lines + 1);
    let strips = data
      .par_chunks(cpp * dim.w * strip_lines)
      .map(|strip| {
        let mut encoder = weezl::encode::Encoder::with_tiff_size_switch(weezl::BitOrder::Msb, 8);
        encoder.encode(strip.as_bytes())
      })
      .collect::<std::result::Result<Vec<Vec<u8>>, LzwError>>()
      .map_err(|err| TiffError::General(err.to_string()))?;

    for strip in strips {
      let offset = self.write_data(&strip)?;
      let bytes = strip.len() as u32;
      tag_data.push((offset, bytes));
    }
    Ok((strip_lines as u32, tag_data))
  }

  pub fn write_data(&mut self, data: &[u8]) -> Result<u32>
  where
    W: Seek + Write,
  {
    self.pad_word_boundary()?;
    let offset = self.position()?;
    self.writer.write_all(data)?;
    Ok(offset)
  }

  pub fn write_data_u16_le(&mut self, data: &[u16]) -> Result<u32>
  where
    W: Seek + Write,
  {
    self.pad_word_boundary()?;
    let offset = self.position()?;
    for v in data {
      self.writer.write_u16::<LittleEndian>(*v)?;
    }
    Ok(offset)
  }

  pub fn write_data_f32_le(&mut self, data: &[f32]) -> Result<u32>
  where
    W: Seek + Write,
  {
    self.pad_word_boundary()?;
    let offset = self.position()?;
    for v in data {
      self.writer.write_f32::<LittleEndian>(*v)?;
    }
    Ok(offset)
  }

  pub(crate) fn pad_word_boundary(&mut self) -> Result<()> {
    if self.position()? % 4 != 0 {
      let padding = [0, 0, 0];
      let padd_len = 4 - (self.position()? % 4);
      self.writer.write_all(&padding[..padd_len as usize])?;
    }
    Ok(())
  }

  pub fn build(mut self, root_ifd: DirectoryWriter) -> Result<()> {
    let ifd0_offset = root_ifd.build(&mut self)?;
    self.writer.seek(SeekFrom::Start(self.ifd_location))?;
    self.writer.write_u32::<NativeEndian>(ifd0_offset)?;
    Ok(())
  }
}

impl<W> TiffWriter<W>
where
  W: Seek,
{
  pub fn position(&mut self) -> Result<u32> {
    Ok(self.writer.stream_position().map(|v| v as u32)?) // TODO: try_from?
  }
}

#[derive(Default)]
pub struct DirectoryWriter {
  // We use BTreeMap to make sure tags are written in correct order
  entries: BTreeMap<u16, Entry>,
  next_ifd: u32,
}

impl DirectoryWriter {
  pub fn remove_tag<T: TiffTag>(&mut self, tag: T) {
    let tag: u16 = tag.into();
    self.entries.remove(&tag);
  }

  pub fn add_tag<T: TiffTag, V: Into<Value>>(&mut self, tag: T, value: V) {
    let tag: u16 = tag.into();
    self.entries.insert(
      tag,
      Entry {
        tag,
        value: value.into(),
        embedded: None,
      },
    );
  }

  pub fn add_untyped_tag<V: Into<Value>>(&mut self, tag: u16, value: V) {
    self.entries.insert(
      tag,
      Entry {
        tag,
        value: value.into(),
        embedded: None,
      },
    );
  }

  pub fn contains<T: TiffTag>(&self, tag: T) -> bool {
    self.entries.contains_key(&tag.into())
  }

  pub fn copy<'a>(&mut self, iter: impl Iterator<Item = (&'a u16, &'a Value)>) {
    for entry in iter {
      if !self.entries.contains_key(entry.0) {
        self.add_untyped_tag(*entry.0, entry.1.clone());
      }
    }
  }

  pub fn copy_with_override<'a>(&mut self, iter: impl Iterator<Item = (&'a u16, &'a Value)>) {
    for entry in iter {
      self.add_untyped_tag(*entry.0, entry.1.clone());
    }
  }

  pub fn add_tag_undefined<T: TiffTag>(&mut self, tag: T, data: Vec<u8>) {
    let tag: u16 = tag.into();
    //let data = data.as_ref();
    //let offset = self.write_data(data)?;
    self.entries.insert(
      tag,
      Entry {
        tag,
        value: Value::Undefined(data),
        embedded: None,
      },
    );
  }

  pub fn add_value<T: TiffTag>(&mut self, tag: T, value: Value) {
    let tag: u16 = tag.into();
    self.entries.insert(tag, Entry { tag, value, embedded: None });
  }

  pub fn entry_count(&self) -> u16 {
    self.entries.len() as u16
  }

  pub fn new() -> Self {
    Self {
      entries: BTreeMap::new(),
      next_ifd: 0,
    }
  }

  pub fn is_empty(&self) -> bool {
    self.entries.is_empty()
  }

  pub fn build<W>(mut self, tiff: &mut TiffWriter<W>) -> Result<u32>
  where
    W: Seek + Write,
  {
    if self.entries.is_empty() {
      return Err(TiffError::General("IFD is empty, not allowed by TIFF specification".to_string()));
    }
    for &mut Entry {
      ref mut value,
      ref mut embedded,
      ref tag,
    } in self.entries.values_mut()
    {
      let data_bytes = 4;

      if value.byte_size() > data_bytes {
        tiff.pad_word_boundary()?;
        let offset = tiff.position()?;
        value.write(&mut tiff.writer)?;
        embedded.replace(offset as u32);
      } else {
        if value.count() == 0 {
          panic!("TIFF value is empty, tag: {:?}", tag);
        }
        embedded.replace(value.as_embedded()?);
      }
    }

    tiff.pad_word_boundary()?;
    let offset = tiff.position()?;

    tiff.writer.write_all(&self.entry_count().to_ne_bytes())?;

    for (tag, entry) in self.entries {
      tiff.writer.write_u16::<NativeEndian>(tag)?;
      tiff.writer.write_u16::<NativeEndian>(entry.value_type())?;
      tiff.writer.write_u32::<NativeEndian>(entry.count())?;
      tiff
        .writer
        .write_u32::<NativeEndian>(entry.embedded.expect("embedded attribute must contain a value"))?;
    }
    tiff.writer.write_u32::<NativeEndian>(self.next_ifd)?; // Next IFD

    Ok(offset)
  }

  /*
  pub fn add_entry(&mut self, entry: Entry) {
    self.ifd.insert(tag.into(), entry);
  }
   */
}

impl crate::decoders::RawMetadata {
  pub fn write_exif_tags<W>(&self, tiff: &mut TiffWriter<W>, root_ifd: &mut DirectoryWriter, exif_ifd: &mut DirectoryWriter) -> crate::formats::tiff::Result<()>
  where
    W: Write + Seek,
  {
    self.fill_exif_root(tiff, root_ifd)?;
    Self::fill_exif_ifd(&self.exif, exif_ifd)?;
    Ok(())
  }

  pub fn fill_exif_ifd(exif: &Exif, exif_ifd: &mut DirectoryWriter) -> Result<()> {
    transfer_entry(exif_ifd, ExifTag::FNumber, &exif.fnumber)?;
    transfer_entry(exif_ifd, ExifTag::ApertureValue, &exif.aperture_value)?;
    transfer_entry(exif_ifd, ExifTag::BrightnessValue, &exif.brightness_value)?;
    transfer_entry(exif_ifd, ExifTag::ExposureBiasValue, &exif.exposure_bias)?;
    transfer_entry(exif_ifd, ExifTag::RecommendedExposureIndex, &exif.recommended_exposure_index)?;
    transfer_entry(exif_ifd, ExifTag::ExposureTime, &exif.exposure_time)?;
    transfer_entry(exif_ifd, ExifTag::ISOSpeedRatings, &exif.iso_speed_ratings)?;
    transfer_entry(exif_ifd, ExifTag::ISOSpeed, &exif.iso_speed)?;
    transfer_entry(exif_ifd, ExifTag::SensitivityType, &exif.sensitivity_type)?;
    transfer_entry(exif_ifd, ExifTag::ExposureProgram, &exif.exposure_program)?;
    transfer_entry(exif_ifd, ExifTag::TimeZoneOffset, &exif.timezone_offset)?;
    transfer_entry(exif_ifd, ExifTag::DateTimeOriginal, &exif.date_time_original)?;
    transfer_entry(exif_ifd, ExifTag::CreateDate, &exif.create_date)?;
    transfer_entry(exif_ifd, ExifTag::OffsetTime, &exif.offset_time)?;
    transfer_entry(exif_ifd, ExifTag::OffsetTimeOriginal, &exif.offset_time_original)?;
    transfer_entry(exif_ifd, ExifTag::OffsetTimeDigitized, &exif.offset_time_digitized)?;
    transfer_entry(exif_ifd, ExifTag::SubSecTime, &exif.sub_sec_time)?;
    transfer_entry(exif_ifd, ExifTag::SubSecTimeOriginal, &exif.sub_sec_time_original)?;
    transfer_entry(exif_ifd, ExifTag::SubSecTimeDigitized, &exif.sub_sec_time_digitized)?;
    transfer_entry(exif_ifd, ExifTag::ShutterSpeedValue, &exif.shutter_speed_value)?;
    transfer_entry(exif_ifd, ExifTag::MaxApertureValue, &exif.max_aperture_value)?;
    transfer_entry(exif_ifd, ExifTag::SubjectDistance, &exif.subject_distance)?;
    transfer_entry(exif_ifd, ExifTag::MeteringMode, &exif.metering_mode)?;
    transfer_entry(exif_ifd, ExifTag::LightSource, &exif.light_source)?;
    transfer_entry(exif_ifd, ExifTag::Flash, &exif.flash)?;
    transfer_entry(exif_ifd, ExifTag::FocalLength, &exif.focal_length)?;
    transfer_entry(exif_ifd, ExifTag::ImageNumber, &exif.image_number)?;
    transfer_entry(exif_ifd, ExifTag::ColorSpace, &exif.color_space)?;
    transfer_entry(exif_ifd, ExifTag::FlashEnergy, &exif.flash_energy)?;
    transfer_entry(exif_ifd, ExifTag::ExposureMode, &exif.exposure_mode)?;
    transfer_entry(exif_ifd, ExifTag::WhiteBalance, &exif.white_balance)?;
    transfer_entry(exif_ifd, ExifTag::SceneCaptureType, &exif.scene_capture_type)?;
    transfer_entry(exif_ifd, ExifTag::SubjectDistanceRange, &exif.subject_distance_range)?;
    transfer_entry(exif_ifd, ExifTag::OwnerName, &exif.owner_name)?;
    transfer_entry(exif_ifd, ExifTag::SerialNumber, &exif.serial_number)?;
    transfer_entry(exif_ifd, ExifTag::LensSerialNumber, &exif.lens_serial_number)?;
    transfer_entry(exif_ifd, ExifTag::LensSpecification, &exif.lens_spec)?;
    transfer_entry(exif_ifd, ExifTag::LensMake, &exif.lens_make)?;
    transfer_entry(exif_ifd, ExifTag::LensModel, &exif.lens_model)?;
    transfer_entry(exif_ifd, ExifTag::UserComment, &exif.user_comment)?;
    //transfer_entry(exif_ifd, ExifTag::MakerNotes, &exif.makernotes.as_ref().map(|x| Value::Undefined(x.clone())))?;

    Ok(())
  }

  fn fill_exif_root<W>(&self, tiff: &mut TiffWriter<W>, root_ifd: &mut DirectoryWriter) -> Result<()>
  where
    W: Write + Seek,
  {
    transfer_entry(root_ifd, ExifTag::Orientation, &self.exif.orientation)?;
    transfer_entry(root_ifd, ExifTag::ModifyDate, &self.exif.modify_date)?;
    transfer_entry(root_ifd, ExifTag::Copyright, &self.exif.copyright)?;
    transfer_entry(root_ifd, ExifTag::Artist, &self.exif.artist)?;

    if let Some(gps) = &self.exif.gps {
      let gps_offset = {
        let mut gps_ifd = DirectoryWriter::new();
        transfer_entry(&mut gps_ifd, ExifGpsTag::GPSVersionID, &gps.gps_version_id)?;
        transfer_entry(&mut gps_ifd, ExifGpsTag::GPSLatitudeRef, &gps.gps_latitude_ref)?;
        transfer_entry(&mut gps_ifd, ExifGpsTag::GPSLatitude, &gps.gps_latitude)?;
        transfer_entry(&mut gps_ifd, ExifGpsTag::GPSLongitudeRef, &gps.gps_longitude_ref)?;
        transfer_entry(&mut gps_ifd, ExifGpsTag::GPSLongitude, &gps.gps_longitude)?;
        transfer_entry(&mut gps_ifd, ExifGpsTag::GPSAltitudeRef, &gps.gps_altitude_ref)?;
        transfer_entry(&mut gps_ifd, ExifGpsTag::GPSAltitude, &gps.gps_altitude)?;
        transfer_entry(&mut gps_ifd, ExifGpsTag::GPSTimeStamp, &gps.gps_timestamp)?;
        transfer_entry(&mut gps_ifd, ExifGpsTag::GPSSatellites, &gps.gps_satellites)?;
        transfer_entry(&mut gps_ifd, ExifGpsTag::GPSStatus, &gps.gps_status)?;
        transfer_entry(&mut gps_ifd, ExifGpsTag::GPSMeasureMode, &gps.gps_measure_mode)?;
        transfer_entry(&mut gps_ifd, ExifGpsTag::GPSDOP, &gps.gps_dop)?;
        transfer_entry(&mut gps_ifd, ExifGpsTag::GPSSpeedRef, &gps.gps_speed_ref)?;
        transfer_entry(&mut gps_ifd, ExifGpsTag::GPSSpeed, &gps.gps_speed)?;
        transfer_entry(&mut gps_ifd, ExifGpsTag::GPSTrackRef, &gps.gps_track_ref)?;
        transfer_entry(&mut gps_ifd, ExifGpsTag::GPSTrack, &gps.gps_track)?;
        transfer_entry(&mut gps_ifd, ExifGpsTag::GPSImgDirectionRef, &gps.gps_img_direction_ref)?;
        transfer_entry(&mut gps_ifd, ExifGpsTag::GPSImgDirection, &gps.gps_img_direction)?;
        transfer_entry(&mut gps_ifd, ExifGpsTag::GPSMapDatum, &gps.gps_map_datum)?;
        transfer_entry(&mut gps_ifd, ExifGpsTag::GPSDestLatitudeRef, &gps.gps_dest_latitude_ref)?;
        transfer_entry(&mut gps_ifd, ExifGpsTag::GPSDestLatitude, &gps.gps_dest_latitude)?;
        transfer_entry(&mut gps_ifd, ExifGpsTag::GPSDestLongitudeRef, &gps.gps_dest_longitude_ref)?;
        transfer_entry(&mut gps_ifd, ExifGpsTag::GPSDestLongitude, &gps.gps_dest_longitude)?;
        transfer_entry(&mut gps_ifd, ExifGpsTag::GPSDestBearingRef, &gps.gps_dest_bearing_ref)?;
        transfer_entry(&mut gps_ifd, ExifGpsTag::GPSDestBearing, &gps.gps_dest_bearing)?;
        transfer_entry(&mut gps_ifd, ExifGpsTag::GPSDestDistanceRef, &gps.gps_dest_distance_ref)?;
        transfer_entry(&mut gps_ifd, ExifGpsTag::GPSDestDistance, &gps.gps_dest_distance)?;
        transfer_entry_undefined(&mut gps_ifd, ExifGpsTag::GPSProcessingMethod, &gps.gps_processing_method)?;
        transfer_entry_undefined(&mut gps_ifd, ExifGpsTag::GPSAreaInformation, &gps.gps_area_information)?;
        transfer_entry(&mut gps_ifd, ExifGpsTag::GPSDateStamp, &gps.gps_date_stamp)?;
        transfer_entry(&mut gps_ifd, ExifGpsTag::GPSDifferential, &gps.gps_differential)?;
        transfer_entry(&mut gps_ifd, ExifGpsTag::GPSHPositioningError, &gps.gps_h_positioning_error)?;
        if gps_ifd.entry_count() > 0 { Some(gps_ifd.build(tiff)?) } else { None }
      };
      if let Some(gps_offset) = gps_offset {
        root_ifd.add_tag(ExifTag::GPSInfo, [gps_offset]);
      }
    }

    Ok(())
  }
}

pub(crate) fn transfer_entry<T, V>(ifd: &mut DirectoryWriter, tag: T, entry: &Option<V>) -> Result<()>
where
  T: TiffTag,
  V: Into<Value> + Clone,
{
  if let Some(entry) = entry {
    ifd.add_tag(tag, entry.clone());
  }
  Ok(())
}

pub(crate) fn transfer_entry_undefined<T>(ifd: &mut DirectoryWriter, tag: T, entry: &Option<Vec<u8>>) -> Result<()>
where
  T: TiffTag,
{
  if let Some(entry) = entry {
    ifd.add_tag_undefined(tag, entry.clone());
  }
  Ok(())
}
