// SPDX-License-Identifier: LGPL-2.1
// Copyright 2021 Daniel Vogelbacher <daniel@chaospixel.com>

use clap::{arg, builder::ValueParser, command, value_parser, ArgAction, Command};
use log::debug;
use rawler::dng::{CropMode, DngCompression};
use std::path::PathBuf;

pub fn create_app() -> Command {
  debug!("Creating CLAP app configuration");

  let convert_base = Command::new("")
    .about("Convert raw image(s) into dng format")
    .arg(
      arg!(compression: -c --"compression" <compression> "Compression for raw image")
        .action(ArgAction::SetTrue)
        .required(false)
        .value_parser(value_parser!(DngCompression))
        .default_value("lossless"),
    )
    .arg(
      arg!(predictor: --"ljpeg92-predictor" <predictor> "LJPEG-92 predictor")
        .required(false)
        .value_parser(clap::value_parser!(u8).range(1..=7))
        .default_value("1"),
    )
    .arg(
      arg!(preview: --"dng-preview" <preview> "DNG include preview image")
        .value_parser(ValueParser::bool())
        .required(false)
        .default_value("true")
        .default_missing_value("true"),
    )
    .arg(
      arg!(thumbnail: --"dng-thumbnail" <thumbnail> "DNG include thumbnail image")
        .action(ArgAction::SetTrue)
        .required(false)
        .default_value("true")
        .value_parser(ValueParser::bool()),
    )
    .arg(
      arg!(embedded: --"embed-raw" <embedded> "Embed the raw file into DNG")
        .value_parser(ValueParser::bool())
        .required(false)
        .default_value("true")
        .default_missing_value("true"),
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
        .value_parser(value_parser!(CropMode))
        .default_value("best"),
    )
    .arg(arg!(-f --override "Override existing files").action(ArgAction::SetTrue));

  let app = command!()
    .about("DNGLab - A camera raw utility and DNG converter")
    .subcommand_required(true)
    .arg_required_else_help(true)
    .arg(arg!(debug: -d ... "turns on debugging mode").global(true))
    .arg(arg!(verbose: -v "Print more messages").global(true).action(ArgAction::SetTrue))
    .subcommand(
      Command::new("analyze")
        .about("Analyze raw image")
        .arg_required_else_help(true)
        .arg(arg!(raw_pixel: --"raw-pixel").action(ArgAction::SetTrue))
        .arg(arg!(full_pixel: --"full-pixel" "Write uncompressed full pixel data to STDOUT").action(ArgAction::SetTrue))
        .arg(arg!(preview_pixel: --"preview-pixel" "Write uncompressed preview pixel data to STDOUT").action(ArgAction::SetTrue))
        .arg(arg!(thumbnail_pixel: --"thumbnail-pixel" "Write uncompressed preview pixel data to STDOUT").action(ArgAction::SetTrue))
        .arg(arg!(raw_checksum: --"raw-checksum" "Write MD5 checksum of raw pixels to STDOUT").action(ArgAction::SetTrue))
        .arg(arg!(preview_checksum: --"preview-checksum" "Write MD5 checksum of preview pixels to STDOUT").action(ArgAction::SetTrue))
        .arg(arg!(thumbnail_checksum: --"thumbnail-checksum" "Write MD5 checksum of thumbnail pixels to STDOUT").action(ArgAction::SetTrue))
        .arg(arg!(srgb: --srgb "Write sRGB 16-bit TIFF to STDOUT").action(ArgAction::SetTrue))
        .arg(arg!(meta: --meta "Write metadata to STDOUT").action(ArgAction::SetTrue))
        .arg(arg!(structure: --structure "Write file structure to STDOUT").action(ArgAction::SetTrue))
        .arg(arg!(summary: --summary "Write summary information for file to STDOUT").action(ArgAction::SetTrue))
        .arg(arg!(json: --json "Format metadata as JSON").action(ArgAction::SetTrue))
        .arg(arg!(yaml: --yaml "Format metadata as YAML").action(ArgAction::SetTrue))
        .arg(arg!(<FILE> "Input file").value_parser(clap::value_parser!(PathBuf))),
    )
    .subcommand(
      convert_base
        .clone()
        .name("convert")
        .arg(arg!(-r --recursive "Process input directory recursive").action(ArgAction::SetTrue))
        .arg(arg!(<INPUT> "Input file or directory").value_parser(clap::value_parser!(PathBuf)))
        .arg(arg!(<OUTPUT> "Output file or existing directory").value_parser(clap::value_parser!(PathBuf))),
    )
    .subcommand(
      convert_base
        .clone()
        .name("ftpserver")
        .arg(
          arg!(ftp_port: --port <port> "FTP listen port")
            .required(false)
            .default_value("2121")
            .value_parser(clap::value_parser!(u16)),
        )
        .arg(arg!(ftp_listen: --listen <addr> "FTP listen address").required(false).default_value("0.0.0.0"))
        .arg(
          arg!(keep_orig: --"keep-original" <keep> "Keep original raw")
            .value_parser(ValueParser::bool())
            .required(false)
            .default_value("true")
            .default_missing_value("true"),
        )
        .arg(arg!(<OUTPUT> "Output file or existing directory").value_parser(clap::value_parser!(PathBuf))),
    )
    .subcommand(
      Command::new("cameras")
        .about("List supported cameras")
        .arg_required_else_help(false)
        .arg(arg!(markdown: --md "Markdown format output").action(ArgAction::SetTrue)),
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
        .arg(arg!(skipchecks: --skipchecks "Skip integrity checks").action(ArgAction::SetTrue))
        .arg(arg!(-r --recursive "Process input directory recursive").action(ArgAction::SetTrue))
        .arg(arg!(-f --override "Override existing files").action(ArgAction::SetTrue))
        .arg(arg!(<INPUT> "Input file or directory").value_parser(clap::value_parser!(PathBuf)))
        .arg(arg!(<OUTPUT> "Output file or existing directory").value_parser(clap::value_parser!(PathBuf))),
    );
  app
}
