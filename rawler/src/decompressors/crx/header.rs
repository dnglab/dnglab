use byteorder::{BigEndian, ReadBytesExt};
use log::debug;
use std::io::Cursor;

use super::{CodecParams, Plane, Result, Subband, Tile};
use crate::{decompressors::crx::CrxError, formats::bmff::ext_cr3::cmp1::Cmp1Box};

impl CodecParams {
  /// Create new codec parameters
  pub fn new(cmp1: &Cmp1Box) -> Result<Self> {
    const INCR_BIT_TABLE: [u8; 16] = [0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 0, 0, 0, 1, 0];

    if cmp1.n_planes != 4 {
      return Err(CrxError::General(format!("Plane configration {} is not supported", cmp1.n_planes)));
    }

    let tile_cols: usize = (cmp1.f_width / cmp1.tile_width) as usize;
    let tile_rows: usize = (cmp1.f_height / cmp1.tile_height) as usize;
    assert!(tile_cols > 0);
    assert!(tile_rows > 0);

    let params = Self {
      sample_precision: cmp1.n_bits as u8 + INCR_BIT_TABLE[4 * cmp1.enc_type as usize + 2] + 1,
      image_width: cmp1.f_width as usize,
      image_height: cmp1.f_height as usize,
      plane_count: cmp1.n_planes as u8,
      plane_width: if cmp1.n_planes == 4 {
        cmp1.f_width as usize / tile_cols / 2
      } else {
        cmp1.f_width as usize / tile_cols
      },
      plane_height: if cmp1.n_planes == 4 {
        cmp1.f_height as usize / tile_rows / 2
      } else {
        cmp1.f_height as usize / tile_rows
      },
      // 3 bands per level + one last LL
      // only 1 band for zero levels (uncompressed)
      subband_count: 3 * cmp1.image_levels as u8 + 1,
      levels: cmp1.image_levels as u8,
      n_bits: cmp1.n_bits as u8,
      enc_type: cmp1.enc_type as u8,
      tile_cols,
      tile_rows,
      tile_width: cmp1.tile_width as usize,
      tile_height: cmp1.tile_height as usize,
      mdat_hdr_size: cmp1.mdat_hdr_size,
    };

    if params.tile_cols > 0xff {
      return Err(CrxError::General(format!("Tile column count {} is not supported", tile_cols)));
    }
    if params.tile_rows > 0xff {
      return Err(CrxError::General(format!("Tile row count {} is not supported", tile_rows)));
    }
    if params.tile_width < 0x16 || params.tile_height < 0x16 || params.plane_width > 0x7FFF || params.plane_height > 0x7FFF {
      return Err(CrxError::General(format!("Invalid params for band decoding")));
    }

    Ok(params)
  }

  /// Process tiles and update values
  fn process_tiles(&mut self, tiles: &mut Vec<Tile>) {
    for cur_tile in 0..tiles.len() {
      if (cur_tile + 1) % self.tile_cols != 0 {
        // not the last tile in a tile row
        tiles[cur_tile].width = self.tile_width;
        debug!("D1 {}", tiles[cur_tile].width);
        if self.tile_cols > 1 {
          //tile->tileFlag = E_HAS_TILES_ON_THE_RIGHT;
          if cur_tile % self.tile_cols != 0 {
            // not the first tile in tile row
            //tile->tileFlag |= E_HAS_TILES_ON_THE_LEFT;
          }
        }
      } else {
        // last tile in a tile row
        tiles[cur_tile].width = self.tile_width;
        //tiles[curTile].width = self.plane_width - self.tile_width * (self.tile_cols - 1);
        debug!("D2 {}", tiles[cur_tile].width);
        if self.tile_cols > 1 {
          //tile->tileFlag = E_HAS_TILES_ON_THE_LEFT;
        }
      }

      if (cur_tile) < (tiles.len() - self.tile_cols) {
        // in first tile row
        tiles[cur_tile].height = self.tile_height;
        debug!("D3 {}", tiles[cur_tile].height);
        /*
        if (img->tileRows > 1) {
          tile->tileFlag |= E_HAS_TILES_ON_THE_BOTTOM;
          if (curTile >= img->tileCols)
            tile->tileFlag |= E_HAS_TILES_ON_THE_TOP;
        }
        */
      } else {
        // non first tile row
        tiles[cur_tile].height = self.tile_height;
        //tiles[curTile].height = self.plane_height - self.tile_height * (self.tile_rows - 1);
        debug!("D4 {}", tiles[cur_tile].height);
        if self.tile_rows > 1 {
          //  tile->tileFlag |= E_HAS_TILES_ON_THE_TOP;
        }
      }
    }
    // process subbands
    for tile in tiles {
      debug!("{}", tile.descriptor_line());
      debug!("Tw: {}, Th: {}", tile.width, tile.height);
      let mut plane_sizes = 0;
      for plane in &mut tile.planes {
        debug!("{}", plane.descriptor_line());
        let mut band_sizes = 0;
        for band in &mut plane.subbands {
          band_sizes += band.subband_size;
          //band.width = tile.width;
          //band.height = tile.height;
          band.width = self.plane_width;
          band.height = self.plane_height;
          // FIXME: ExCoef
          debug!("{}", band.descriptor_line());
          debug!("    Bw: {}, Bh: {}", band.width, band.height);
        }
        assert_eq!(plane.plane_size, band_sizes);
        plane_sizes += plane.plane_size;
      }
      assert_eq!(tile.tile_size, plane_sizes);
    }
  }

  /// Parse header information
  pub(crate) fn parse_header<'a>(&mut self, mdat: &'a [u8]) -> Result<Vec<Tile>> {
    let mut tiles = Vec::new();
    let hdr = self.get_header(mdat);
    let mut hdr = Cursor::new(hdr);

    let len = hdr.get_ref().len();

    let mut tile_offset: usize = 0;
    let mut plane_offset: usize = 0;
    let mut band_offset: usize = 0;

    while (hdr.position() as usize) < len - 1 {
      let ind = hdr
        .read_u16::<BigEndian>()
        .map_err(|_| CrxError::General("Header indicator read failed".into()))?;
      match ind {
        0xff01 | 0xff11 => {
          let tile = Tile::new(tiles.len(), &mut hdr, ind, tile_offset)?;
          tile_offset += tile.tile_size as usize;
          plane_offset = 0; // reset plane offset
          tiles.push(tile);
        }
        0xff02 | 0xff12 => {
          let plane = Plane::new(
            tiles.last_mut().unwrap().planes.len(),
            &mut hdr,
            ind,
            tiles.last().unwrap().data_offset,
            plane_offset,
          )?;
          plane_offset += plane.plane_size as usize;
          band_offset = 0; // reset band offset
          tiles.last_mut().unwrap().planes.push(plane);
        }
        0xff03 | 0xff13 => {
          let subband = Subband::new(
            tiles.last_mut().unwrap().planes.last_mut().unwrap().subbands.len(),
            &mut hdr,
            ind,
            tiles.last().unwrap().data_offset + tiles.last().unwrap().planes.last().unwrap().data_offset,
            band_offset,
          )?;
          band_offset += subband.subband_size as usize;
          tiles.last_mut().unwrap().planes.last_mut().unwrap().subbands.push(subband);
        }
        0x0000 => {
          debug!("end reached?");
          break;
        }
        _ => {
          return Err(CrxError::General(format!("Invalid header marker")));
        }
      }
    }
    //self.tiles = tiles;
    self.process_tiles(&mut tiles);

    Ok(tiles)
  }
}
