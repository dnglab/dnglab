// SPDX-License-Identifier: LGPL-2.1
// Copyright 2021 Daniel Vogelbacher <daniel@chaospixel.com>

use clap::{arg, command, Command};
use log::debug;

pub fn create_app() -> Command<'static> {
  debug!("Creating CLAP app configuration");

  let convert_base = Command::new("")
    .about("Convert raw image(s) into dng format")
    .arg(
      arg!(compression: -c --"compression" <compression> "Compression for raw image")
        .required(false)
        .possible_values(&["lossless", "none"])
        .default_value("lossless"),
    )
    .arg(
      arg!(predictor: --"ljpeg92-predictor" <predictor> "LJPEG-92 predictor")
        .required(false)
        .possible_values(&["1", "2", "3", "4", "5", "6", "7"])
        .default_value("1"),
    )
    .arg(
      arg!(preview: --"dng-preview" <preview> "DNG preview image generator")
        .required(false)
        .validator(validate_bool),
    )
    .arg(
      arg!(thumbnail: --"dng-thumbnail" <thumbnail> "DNG thumbnail image generator")
        .required(false)
        .validator(validate_bool),
    )
    .arg(
      arg!(embedded: --"embed-raw" <embedded> "Embed the raw file into DNG")
        .required(false)
        .default_value("false")
        .validator(validate_bool),
    )
    .arg(arg!(--"artist" <artist> "Set the artist tag").required(false))
    .arg(
      arg!(index: --"image-index" <index> "Select a specific image index (or 'all') if file is a image container")
        .required(false)
        .default_value("0"),
    )
    .arg(
      arg!(--"crop" <crop> "DNG default crop")
        .required(false)
        .possible_values(&["best", "activearea", "none"])
        .default_value("best"),
    )
    .arg(arg!(-r --recursive "Process input directory recursive"))
    .arg(arg!(-f --override "Override existing files"));

  let app = command!()
    .about("DNGLab - A camera raw utility and DNG converter")
    .subcommand_required(true)
    .arg_required_else_help(true)
    .arg(arg!(debug: -d ... "turns on debugging mode").global(true))
    .arg(arg!(verbose: -v "Print more messages").global(true))
    .subcommand(
      Command::new("analyze")
        .about("Analyze raw image")
        .arg_required_else_help(true)
        .arg(arg!(raw_pixel: --"raw-pixel"))
        .arg(arg!(full_pixel: --"full-pixel" "Write uncompressed full pixel data to STDOUT"))
        .arg(arg!(preview_pixel: --"preview-pixel" "Write uncompressed preview pixel data to STDOUT"))
        .arg(arg!(thumbnail_pixel: --"thumbnail-pixel" "Write uncompressed preview pixel data to STDOUT"))
        .arg(arg!(raw_checksum: --"raw-checksum" "Write MD5 checksum of raw pixels to STDOUT"))
        .arg(arg!(preview_checksum: --"preview-checksum" "Write MD5 checksum of preview pixels to STDOUT"))
        .arg(arg!(thumbnail_checksum: --"thumbnail-checksum" "Write MD5 checksum of thumbnail pixels to STDOUT"))
        .arg(arg!(srgb: --srgb "Write sRGB 16-bit TIFF to STDOUT"))
        .arg(arg!(meta: --meta "Write metadata to STDOUT"))
        .arg(arg!(structure: --structure "Write file structure to STDOUT"))
        .arg(arg!(summary: --summary "Write summary information for file to STDOUT"))
        .arg(arg!(json: --json "Format metadata as JSON"))
        .arg(arg!(yaml: --yaml "Format metadata as YAML"))
        .arg(arg!(<FILE> "Input file")),
    )
    .subcommand(
      convert_base
        .clone()
        .name("convert")
        .arg(arg!(<INPUT> "Input file or directory"))
        .arg(arg!(<OUTPUT> "Output file or existing directory")),
    )
    .subcommand(
      convert_base
        .clone()
        .name("ftpserver")
        .arg(arg!(ftp_port: --port <port> "FTP listen port").required(false).default_value("2121"))
        .arg(arg!(ftp_listen: --listen <addr> "FTP listen address").required(false).default_value("0.0.0.0"))
        .arg(arg!(keep_orig: --"keep-original" "Keep original raw"))
        .arg(arg!(<OUTPUT> "Output file or existing directory")),
    )
    .subcommand(
      Command::new("cameras")
        .about("List supported cameras")
        .arg_required_else_help(false)
        .arg(arg!(markdown: --md "Markdown format output")),
    )
    .subcommand(
      Command::new("lenses")
        .about("List supported lenses")
        .arg_required_else_help(false)
        .arg(arg!(--md "Markdown format output")),
    )
    .subcommand(Command::new("gui").about("Start GUI (not implemented)").arg_required_else_help(false))
    .subcommand(
      Command::new("extract")
        .about("Extract embedded original Raw from DNG")
        .arg_required_else_help(true)
        .arg(arg!(<FILE> "Input file"))
        .arg(arg!(skipchecks: --skipchecks "Skip integrity checks"))
        .arg(arg!(-r --recursive "Process input directory recursive"))
        .arg(arg!(-f --override "Override existing files"))
        .arg(arg!(<INPUT> "Input file or directory"))
        .arg(arg!(<OUTPUT> "Output file or existing directory")),
    );
  app
}

fn validate_bool(v: &str) -> Result<(), String> {
  convert_bool(Some(v), false).map(|_| ())
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
