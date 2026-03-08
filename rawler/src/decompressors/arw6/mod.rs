use crate::Result;
use crate::bits::LookupTable;
use crate::pixarray::PixU16;

pub(crate) fn decompress_arw6(_buf: &[u8], _width: usize, _height: usize, _curve: &LookupTable, _dummy: bool) -> Result<PixU16> {
  unimplemented!()
}
