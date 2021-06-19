use std::io::{Read, Seek, SeekFrom};


use log::debug;
use serde::{Serialize};

use super::{ dinf::DinfBox, stbl::StblBox, vendor::VendorBox,
  vmhd::VmhdBox, BmffError, BoxHeader, FourCC, ReadBox, Result,
};

#[derive(Debug, Clone, PartialEq, Default, Serialize)]
pub struct MinfBox {
  pub header: BoxHeader,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub vmhd: Option<VmhdBox>,

  //#[serde(skip_serializing_if = "Option::is_none")]
  //pub smhd: Option<SmhdBox>,
  pub dinf: DinfBox,
  pub stbl: StblBox,
  pub vendor: Vec<VendorBox>,
}

impl MinfBox {
  pub const TYP: FourCC = FourCC::with(['m', 'i', 'n', 'f']);
}

impl<R: Read + Seek> ReadBox<&mut R> for MinfBox {
  fn read_box(mut reader: &mut R, header: BoxHeader) -> Result<Self> {
    let mut vmhd = None;
    //let mut smhd = None;
    let mut dinf = None;
    let mut stbl = None;

    let mut vendors = Vec::new();

    let mut current = reader.seek(SeekFrom::Current(0))?;

    while current < header.end_offset() {
      // get box?

      let header = BoxHeader::parse(&mut reader)?;

      match header.typ {
        VmhdBox::TYP => {
          vmhd = Some(VmhdBox::read_box(&mut reader, header)?);
        }
        DinfBox::TYP => {
          dinf = Some(DinfBox::read_box(&mut reader, header)?);
        }
        StblBox::TYP => {
          stbl = Some(StblBox::read_box(&mut reader, header)?);
        }
        _ => {
          debug!("Vendor box found in minf: {:?}", header.typ);
          let vendor = VendorBox::read_box(&mut reader, header)?;
          vendors.push(vendor);
        }
      }

      current = reader.seek(SeekFrom::Current(0))?;
    }

    reader.seek(SeekFrom::Start(header.end_offset()))?;

    Ok(Self {
      header,
      vmhd,
      dinf: dinf.ok_or(BmffError::Parse("dinf box not found, corrupt file?".into()))?,
      stbl: stbl.ok_or(BmffError::Parse("stbl box not found, corrupt file?".into()))?,
      vendor: vendors,
    })
  }
}
