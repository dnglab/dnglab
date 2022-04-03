// SPDX-License-Identifier: LGPL-2.1
// Copyright 2021 Daniel Vogelbacher <daniel@chaospixel.com>

use clap::{clap_app, crate_version, App};
use log::debug;

pub fn create_app() -> App<'static, 'static> {
  debug!("Creating CLAP app configuration");
  let app = clap_app!(dnglab =>
      (version: crate_version!())
      (author: "Daniel V. <daniel@chaospixel.com>")
      (about: "DNGLab - A camera raw utility and DNG converter")
      (@arg verbose: -v --verbose +global "Print more messages")
      (@arg debug: -d ... +global "Sets the level of debugging information")
      (@subcommand analyze =>
          (about: "Analyze raw image")
          (@arg raw_pixel: --("raw-pixel") "Write uncompressed raw pixel data to STDOUT")
          (@arg full_pixel: --("full-pixel") "Write uncompressed full pixel data to STDOUT")
          (@arg preview_pixel: --("preview-pixel") "Write uncompressed preview pixel data to STDOUT")
          (@arg thumbnail_pixel: --("thumbnail-pixel") "Write uncompressed preview pixel data to STDOUT")
          (@arg raw_checksum: --("raw-checksum") "Write MD5 checksum of raw pixels to STDOUT")
          (@arg preview_checksum: --("preview-checksum") "Write MD5 checksum of preview pixels to STDOUT")
          (@arg thumbnail_checksum: --("thumbnail-checksum") "Write MD5 checksum of thumbnail pixels to STDOUT")
          (@arg srgb: --srgb "Write sRGB 16-bit TIFF to STDOUT")
          (@arg meta: --meta "Write metadata to STDOUT")
          (@arg summary: --summary "Write summary information for file to STDOUT")
          (@arg json: --json "Format metadata as JSON")
          (@arg yaml: --yaml "Format metadata as YAML")
          (@arg FILE: +required "Input file")
      )
      (@subcommand metadata =>
        (about: "Metadata for raw image")
        (@arg json: --json "Format metadata as JSON")
        (@arg yaml: --yaml "Format metadata as YAML")
        (@arg text: --text "Format metadata as TEXT")
        (@arg FILE: +required "Input file")
    )
      (@subcommand convert =>
          (about: "Convert raw image(s) into dng format")
          //(@arg profile: -p --profile "Profile file to use")
          //(@arg dng_version: --dng-version +takes_value "DNG version for output file")
          (@arg compression: -c --compression default_value[lossless] {validate_compression} "'lossless' or 'none'")
          (@arg predictor: --("ljpeg92-predictor") +takes_value #{1, 7} "LJPEG-92 predictor (1-7)")
          (@arg preview: --("dng-preview") default_value[yes] {validate_bool} "Include a DNG preview image")
          (@arg thumbnail: --("dng-thumbnail") default_value[yes] {validate_bool} "Include a DNG thumbnail image")
          (@arg embedded: --("dng-embedded") default_value[yes] {validate_bool} "Embed the raw file into DNG")
          (@arg artist: --("artist") +takes_value "Set the artist tag")
          (@arg index: --("image-index") +takes_value "Select a specific image index (or 'all') if file is a image container")
          (@arg crop: --("crop") default_value[yes] {validate_bool} "Apply crop to ActiveArea")
          (@arg recursive: -r --recursive "Process input directory recursive")
          (@arg override: -f --override "Override existing files")
          (@arg INPUT: +required "Input file or directory")
          (@arg OUTPUT: +required "Output file or existing directory")
      )
      (@subcommand ftpconvert =>
        (about: "Convert raw image(s) into dng format")
        //(@arg profile: -p --profile "Profile file to use")
        //(@arg dng_version: --dng-version +takes_value "DNG version for output file")
        (@arg compression: -c --compression default_value[lossless] {validate_compression} "'lossless' or 'none'")
        (@arg predictor: --("ljpeg92-predictor") +takes_value #{1, 7} "LJPEG-92 predictor (1-7)")
        (@arg preview: --("dng-preview") default_value[yes] {validate_bool} "Include a DNG preview image")
        (@arg thumbnail: --("dng-thumbnail") default_value[yes] {validate_bool} "Include a DNG thumbnail image")
        (@arg embedded: --("dng-embedded") default_value[yes] {validate_bool} "Embed the raw file into DNG")
        (@arg artist: --("artist") +takes_value "Set the artist tag")
        (@arg index: --("image-index") +takes_value "Select a specific image index (or 'all') if file is a image container")
        (@arg crop: --("crop") default_value[yes] {validate_bool} "Apply crop to ActiveArea")
        (@arg override: -f --override "Override existing files")
        (@arg ftp_port: --("port") +takes_value "Include a DNG thumbnail image")
        (@arg ftp_listen: --("listen") +takes_value "Include a DNG thumbnail image")
        (@arg keep_orig: --("keep-original") default_value[yes] {validate_bool} "Include a DNG thumbnail image")
        (@arg OUTPUT: +required "Output file or existing directory")
    )
      (@subcommand extract =>
          (about: "Extract embedded original Raw from DNG")
          (@arg recursive: -r --recursive "Process input directory recursive")
          (@arg override: -f --override "Override existing files")
          (@arg skipchecks: --skipchecks "Skip integrity checks")
          (@arg INPUT: +required "Input file or directory")
          (@arg OUTPUT: +required "Output file or existing directory")
      )
      (@subcommand cameras =>
        (about: "List supported cameras")
        (@arg markdown: --md "Format table as Markdown")
    )
      (@subcommand gui =>
          (about: "Start GUI (not implemented)")
      )
  );
  app
}

fn validate_bool(v: String) -> Result<(), String> {
  convert_bool(Some(&v), false).map(|_| ())
}

fn validate_compression(v: String) -> Result<(), String> {
  if v.eq("lossless") || v.eq("none") {
    Ok(())
  } else {
    Err(format!("'{}' is not a valid compression method", v))
  }
}

pub fn convert_bool(v: Option<&str>, default: bool) -> Result<bool, String> {
  const T: [&str; 3] = ["1", "true", "yes"];
  const F: [&str; 3] = ["0", "false", "no"];
  match &v {
    Some(v) => {
      if T.contains(v) {
        Ok(true)
      } else if F.contains(v) {
        Ok(false)
      } else {
        return Err(format!("{} is not a valid option", v));
      }
    }
    None => Ok(default),
  }
}
