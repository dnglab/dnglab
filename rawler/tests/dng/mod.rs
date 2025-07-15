use crate::common::simple_file_check;

mod compression_sets {
  super::simple_file_check!(dng_10bit_packed, "dng/compression-sets/10bit.dng", "6d1e45fe37210b8444d34fe4ccc3f3d2");
  super::simple_file_check!(dng_12bit_packed, "dng/compression-sets/12bit.dng", "04be71fe1169c290f283b35e47f73c35");
  super::simple_file_check!(dng_16bit_bigend, "dng/compression-sets/16bit_bigend.dng", "f3549fafda97fca90b9993c1278bcd90");
}
