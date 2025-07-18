use crate::common::simple_file_check;

mod compression_sets {
  super::simple_file_check!(dng_10bit_packed, "dng/compression-sets/10bit.dng", "6d1e45fe37210b8444d34fe4ccc3f3d2");
  super::simple_file_check!(dng_12bit_packed, "dng/compression-sets/12bit.dng", "04be71fe1169c290f283b35e47f73c35");
  super::simple_file_check!(dng_16bit_bigend, "dng/compression-sets/16bit_bigend.dng", "f3549fafda97fca90b9993c1278bcd90");
  super::simple_file_check!(dng_jpegxl_lossless_16bit_linear_tiles, "dng/compression-sets/dng_jpegxl_lossless_16bit_linear_tiles.dng", "3ba3dd02d41e2ecee42587ff243d5412");
  super::simple_file_check!(dng_jpegxl_lossless_16bit_mosaic_tiles, "dng/compression-sets/dng_jpegxl_lossless_16bit_mosaic_tiles.dng", "bdc9398c5279766c69acf2ac8e2571b6");
  super::simple_file_check!(dng_jpegxl_lossy_16bit_linear_tiles, "dng/compression-sets/dng_jpegxl_lossy_16bit_linear_tiles.dng", "eaeefb71e3037d4e5c64cbfa05043c9e");
  super::simple_file_check!(dng_jpegxl_lossy_16bit_mosaic_tiles, "dng/compression-sets/dng_jpegxl_lossy_16bit_mosaic_tiles.dng", "7e20b78a9c37f5a63f14b8a936fc4f46");
  super::simple_file_check!(dng_fp16_pred_deflate, "dng/compression-sets/dng-fp16-w-pred-deflate.dng", "f759f8933aa3127d5d24ba993d2487e1");
  super::simple_file_check!(dng_fp24_pred_deflate, "dng/compression-sets/dng-fp24-w-pred-deflate.dng", "a6f642172a8c6a19b5710dbc43afebed");
  super::simple_file_check!(dng_fp32_pred_deflate, "dng/compression-sets/dng-fp32-w-pred-deflate.dng", "01edf993269a7673c6a8e864c4210651");
  super::simple_file_check!(dng_fp16_uncompr, "dng/compression-sets/dng-fp16-uncompressed.dng", "bdb66cda7819302364b1e8eac0bb7e18");
  super::simple_file_check!(dng_fp24_uncompr, "dng/compression-sets/dng-fp24-uncompressed.dng", "fdc7bec705076b99a895c5de647c23fd");
  super::simple_file_check!(dng_fp32_uncompr, "dng/compression-sets/dng-fp32-uncompressed.dng", "fdc7bec705076b99a895c5de647c23fd");
  super::simple_file_check!(dng_multistrip_16rows, "dng/compression-sets/uncompressed_multistrip_16row.dng", "8ac07f2aff6980e58b82546612c2558d");
  super::simple_file_check!(dng_multistrip_1row, "dng/compression-sets/uncompressed_multistrip_1row.dng", "e80f72a241615e0deeb16c6de8cfa1d5");
}
