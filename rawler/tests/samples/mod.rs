use crate::common::sample_file_check;
mod dng_compression_variants {
  super::sample_file_check!("dng-compression-variants", sample_10bit_dng, "10bit.dng");
  super::sample_file_check!("dng-compression-variants", sample_dng_jpegxl_lossless_16bit_mosaic_tiles_dng, "dng_jpegxl_lossless_16bit_mosaic_tiles.dng");
  super::sample_file_check!("dng-compression-variants", sample_lossy_tiles_dng, "lossy_tiles.dng");
  super::sample_file_check!("dng-compression-variants", sample_ljpeg_tiles_dng, "ljpeg_tiles.dng");
  super::sample_file_check!("dng-compression-variants", sample_12bit_dng, "12bit.dng");
  super::sample_file_check!("dng-compression-variants", sample_dng_fp24_w_pred_deflate_dng, "dng-fp24-w-pred-deflate.dng");
  super::sample_file_check!("dng-compression-variants", sample_uncompressed_multistrip_16row_dng, "uncompressed_multistrip_16row.dng");
  super::sample_file_check!("dng-compression-variants", sample_8bit_lintable_dng, "8bit_lintable.dng");
  super::sample_file_check!("dng-compression-variants", sample_dng_fp16_w_pred_deflate_dng, "dng-fp16-w-pred-deflate.dng");
  super::sample_file_check!("dng-compression-variants", sample_dng_fp16_uncompressed_dng, "dng-fp16-uncompressed.dng");
  super::sample_file_check!("dng-compression-variants", sample_ljpeg_singlestrip_dng, "ljpeg_singlestrip.dng");
  super::sample_file_check!("dng-compression-variants", sample_dng_fp32_w_pred_deflate_dng, "dng-fp32-w-pred-deflate.dng");
  super::sample_file_check!("dng-compression-variants", sample_dng_jpegxl_lossy_16bit_mosaic_tiles_dng, "dng_jpegxl_lossy_16bit_mosaic_tiles.dng");
  super::sample_file_check!("dng-compression-variants", sample_origin_cr3, "origin.CR3");
  super::sample_file_check!("dng-compression-variants", sample_dng_jpegxl_lossless_16bit_linear_tiles_dng, "dng_jpegxl_lossless_16bit_linear_tiles.dng");
  super::sample_file_check!("dng-compression-variants", sample_uncompressed_multistrip_1row_dng, "uncompressed_multistrip_1row.dng");
  super::sample_file_check!("dng-compression-variants", sample_dng_fp24_uncompressed_dng, "dng-fp24-uncompressed.dng");
  super::sample_file_check!("dng-compression-variants", sample_16bit_bigend_dng, "16bit_bigend.dng");
  super::sample_file_check!("dng-compression-variants", sample_dng_fp32_uncompressed_dng, "dng-fp32-uncompressed.dng");
  super::sample_file_check!("dng-compression-variants", sample_dng_jpegxl_lossy_16bit_linear_tiles_dng, "dng_jpegxl_lossy_16bit_linear_tiles.dng");
}
