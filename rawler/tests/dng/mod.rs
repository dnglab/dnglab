use crate::common::simple_file_check;

mod compression_sets {
  super::simple_file_check!(dng_10bit_packed, "dng/compression-sets/10bit.dng", "6d1e45fe37210b8444d34fe4ccc3f3d2");
  super::simple_file_check!(dng_12bit_packed, "dng/compression-sets/12bit.dng", "04be71fe1169c290f283b35e47f73c35");
  super::simple_file_check!(dng_16bit_bigend, "dng/compression-sets/16bit_bigend.dng", "f3549fafda97fca90b9993c1278bcd90");
  super::simple_file_check!(dng_jpegxl_lossless_16bit_linear_tiles, "dng/compression-sets/dng_jpegxl_lossless_16bit_linear_tiles.dng", "3ba3dd02d41e2ecee42587ff243d5412");
  super::simple_file_check!(dng_jpegxl_lossless_16bit_mosaic_tiles, "dng/compression-sets/dng_jpegxl_lossless_16bit_mosaic_tiles.dng", "bdc9398c5279766c69acf2ac8e2571b6");
  super::simple_file_check!(dng_jpegxl_lossy_16bit_linear_tiles, "dng/compression-sets/dng_jpegxl_lossy_16bit_linear_tiles.dng", "6bf406c65445c79cfb739219d5dfd4d5");
  super::simple_file_check!(dng_jpegxl_lossy_16bit_mosaic_tiles, "dng/compression-sets/dng_jpegxl_lossy_16bit_mosaic_tiles.dng", "45be39aa3d59f8be1869de5babd2b06e");
}
