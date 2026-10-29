// SPDX-License-Identifier: LGPL-2.1
// Copyright 2021 Daniel Vogelbacher <daniel@chaospixel.com>

use std::path::PathBuf;

use clap::{
  Arg, ArgAction, Command, arg,
  builder::{NonEmptyStringValueParser, ValueParser},
  command, value_parser,
};
use log::debug;
use rawler::dng::{ApplyDngOpcodes, CropMode, DngCompression, DngVersion, MAX_JXL_DISTANCE, opcodes::OpcodeSelector};
use rawler::imgop::develop::ProcessedImageFormat;

use crate::makedng::{
  CalibrationIlluminantArgParser, ColorMatrixArgParser, DngColorimetricReference, InputSourceUsageMap, LinearizationTableArgParser, WhiteBalanceInput,
  WhitePointArgParser,
};

#[derive(Clone, Debug, Copy, Eq, PartialEq, PartialOrd, Ord)]
pub enum LogLevel {
  Error,
  Warn,
  Info,
  Debug,
  Trace,
}

impl clap::ValueEnum for LogLevel {
  fn value_variants<'a>() -> &'a [Self] {
    &[Self::Error, Self::Warn, Self::Info, Self::Debug, Self::Trace]
  }

  fn to_possible_value(&self) -> Option<clap::builder::PossibleValue> {
    Some(match self {
      Self::Error => clap::builder::PossibleValue::new("error"),
      Self::Warn => clap::builder::PossibleValue::new("warn"),
      Self::Info => clap::builder::PossibleValue::new("info"),
      Self::Debug => clap::builder::PossibleValue::new("debug"),
      Self::Trace => clap::builder::PossibleValue::new("trace"),
    })
  }
}

/// Parse a JPEG XL distance in the range 0 to 25
fn jxl_distance_parser(value: &str) -> std::result::Result<f32, String> {
  let distance: f32 = value.parse().map_err(|err| format!("{}", err))?;
  if (0.0..=MAX_JXL_DISTANCE).contains(&distance) {
    Ok(distance)
  } else {
    Err(format!("{} is not in 0..={}", value, MAX_JXL_DISTANCE))
  }
}

/// Argument to remove DNG opcodes, shared by convert and process-raw
fn remove_dng_opcodes_arg() -> Arg {
  arg!(remove_dng_opcodes: --"remove-dng-opcodes" <opcodes> "Remove DNG opcodes, as comma separated [<list>:]<opcode> (e.g. WarpRectilinear,2:GainMap)")
    .required(false)
    .value_parser(value_parser!(OpcodeSelector))
    .value_delimiter(',')
    .action(ArgAction::Append)
}

// The subcommands are built in separate functions. A single builder chain for the whole app
// needs a huge stack frame in debug builds, which overflows the 1 MB main thread stack on Windows.
pub fn create_app() -> Command {
  debug!("Creating CLAP app configuration");

  command!()
    .about("DNGLab - A camera raw utility and DNG converter")
    .subcommand_required(true)
    .arg_required_else_help(true)
    .arg(
      arg!(loglevel: -d --loglevel <level> "Log level")
        .global(true)
        .required(false)
        .value_parser(value_parser!(LogLevel))
        .default_value("warn"),
    )
    .arg(arg!(verbose: -v "Print status for every file").global(true).action(ArgAction::SetTrue))
    .arg(
      arg!(profile_path: --"profile-path" <dir> "Directory with profile TOML files")
        .global(true)
        .required(false)
        .value_parser(clap::value_parser!(PathBuf))
        .long_help(
          "Directory with profile TOML files. All *.toml files in this directory are loaded.
If not given, 'profile_path' from the dnglab configuration file is used. Otherwise profiles are loaded
from the 'rawler/profiles' and 'dnglab/profiles' directories in the user configuration directory
(e.g. ~/.config on Linux, ~/Library/Application Support on macOS, %APPDATA% on Windows).",
        ),
    )
    .arg(
      arg!(config: --config <file> "dnglab configuration file")
        .global(true)
        .required(false)
        .value_parser(clap::value_parser!(PathBuf))
        .long_help(
          "dnglab configuration file. If not given, 'dnglab/config.toml' in the user configuration directory
(e.g. ~/.config on Linux, ~/Library/Application Support on macOS, %APPDATA% on Windows) is used if it exists.
Example:

  version = 1
  profile_path = \"profiles\"   # relative to the configuration file

  [convert]
  default_profile = \"builtin_latest\"",
        ),
    )
    .subcommand(analyze_command())
    .subcommand(process_raw_command())
    .subcommand(convert_command())
    .subcommand(ftpserver_command())
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
        .arg(arg!(markdown: --md "Markdown format output").action(ArgAction::SetTrue)),
    )
    .subcommand(makedng_command())
    .subcommand(Command::new("gui").about("Start GUI (not implemented)").arg_required_else_help(false))
    .subcommand(extract_command())
}

/// Arguments shared by convert and ftpserver
fn convert_base_command() -> Command {
  Command::new("dnglab")
    .about("Convert raw image(s) into dng format")
    .arg(
      arg!(profile: -p --"profile" <name> "Convert profile name")
        .required(false)
        .value_parser(NonEmptyStringValueParser::new())
        .long_help(
          "Convert profile to use for conversion. Built-in profiles are 'builtin_compat' (DNG 1.4.0), 'builtin_latest' (DNG 1.6.0),
'builtin_jxl' (JPEG XL lossless, DNG 1.7.1) and 'builtin_archive'.
If not given, 'convert.default_profile' from the dnglab configuration file is used.
Otherwise no profile is used and the command line switches with their defaults apply.
Command line switches override the values of the profile.",
        ),
    )
    .arg(
      arg!(compression: -c --"compression" <compression> "Compression for raw image")
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
      arg!(lossy_jpeg_quality: --"lossy-jpeg-quality" <quality> "JPEG quality for lossy compression (1-100)")
        .required(false)
        .value_parser(clap::value_parser!(u8).range(1..=100))
        .default_value("94"),
    )
    .arg(
      arg!(jxl_distance: --"jxl-distance" <distance> "JPEG XL distance (0-25), 0 is lossless")
        .required(false)
        .value_parser(jxl_distance_parser)
        .default_value("0")
        .long_help(
          "JPEG XL distance (0-25). 0 is lossless, larger values are lossy and give smaller files.
Lossy compression applies OpcodeList1 to the image data.",
        ),
    )
    .arg(
      arg!(jxl_effort: --"jxl-effort" <effort> "JPEG XL encoder effort (1-9)")
        .required(false)
        .value_parser(clap::value_parser!(u8).range(1..=9))
        .default_value("7"),
    )
    .arg(
      arg!(jxl_decode_speed: --"jxl-decode-speed" <speed> "JPEG XL decoding speed for lossy compression (1-4)")
        .required(false)
        .value_parser(clap::value_parser!(u8).range(1..=4))
        .default_value("4"),
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
        .value_parser(ValueParser::bool())
        .required(false)
        .default_value("true")
        .default_missing_value("true"),
    )
    .arg(
      arg!(force_preview: --"dng-force-preview" <force_preview> "Generate DNG preview image from raw data")
        .value_parser(ValueParser::bool())
        .required(false)
        .default_value("false")
        .default_missing_value("true"),
    )
    .arg(
      arg!(force_thumbnail: --"dng-force-thumbnail" <force_thumbnail> "Generate DNG thumbnail image from raw data")
        .value_parser(ValueParser::bool())
        .required(false)
        .default_value("false")
        .default_missing_value("true"),
    )
    .arg(
      arg!(max_dng_backward_version: --"max-dng-backward-version" <version> "Maximum DNG backward version")
        .required(false)
        .value_parser(value_parser!(DngVersion))
        .default_value("1.4"),
    )
    .arg(
      arg!(embedded: --"embed-raw" <embedded> "Embed the raw file into DNG")
        .value_parser(ValueParser::bool())
        .required(false)
        .default_value("true")
        .default_missing_value("true"),
    )
    .arg(
      arg!(apply_dng_opcodes: --"apply-dng-opcodes" <mode> "Apply DNG opcode lists to the image data instead of writing them to the DNG")
        .required(false)
        .value_parser(value_parser!(ApplyDngOpcodes))
        .default_value("auto"),
    )
    .arg(remove_dng_opcodes_arg())
    .arg(
      arg!(--"artist" <artist> "Set the artist tag")
        .required(false)
        .value_parser(NonEmptyStringValueParser::new()),
    )
    .arg(
      arg!(--"copyright" <copyright> "Set the copyright tag")
        .required(false)
        .value_parser(NonEmptyStringValueParser::new()),
    )
    .arg(
      arg!(keep_mtime: --"keep-mtime" <keepmtime> "Keep mtime, read from EXIF with fallback to original file mtime")
        .value_parser(ValueParser::bool())
        .required(false)
        .default_value("false")
        .default_missing_value("false"),
    )
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
    .arg(arg!(-f --override "Override existing files").action(ArgAction::SetTrue))
}

/// Subcommand analyze
fn analyze_command() -> Command {
  Command::new("analyze")
    .about("Analyze raw image")
    .arg_required_else_help(true)
    .arg(arg!(raw_pixel: --"raw-pixel").action(ArgAction::SetTrue))
    .arg(arg!(full_pixel: --"full-pixel" "Write uncompressed full pixel data to STDOUT").action(ArgAction::SetTrue))
    .arg(arg!(preview_pixel: --"preview-pixel" "Write uncompressed preview pixel data to STDOUT").action(ArgAction::SetTrue))
    .arg(arg!(thumbnail_pixel: --"thumbnail-pixel" "Write uncompressed preview pixel data to STDOUT").action(ArgAction::SetTrue))
    .arg(arg!(raw_checksum: --"raw-checksum" "Write MD5 checksum of raw pixels to STDOUT").action(ArgAction::SetTrue))
    .arg(arg!(full_checksum: --"full-checksum" "Write MD5 checksum of full pixels to STDOUT").action(ArgAction::SetTrue))
    .arg(arg!(preview_checksum: --"preview-checksum" "Write MD5 checksum of preview pixels to STDOUT").action(ArgAction::SetTrue))
    .arg(arg!(thumbnail_checksum: --"thumbnail-checksum" "Write MD5 checksum of thumbnail pixels to STDOUT").action(ArgAction::SetTrue))
    .arg(arg!(srgb: --srgb "Write sRGB 16-bit TIFF to STDOUT").action(ArgAction::SetTrue))
    .arg(arg!(meta: --meta "Write metadata to STDOUT").action(ArgAction::SetTrue))
    .arg(arg!(structure: --structure "Write file structure to STDOUT").action(ArgAction::SetTrue))
    .arg(arg!(summary: --summary "Write summary information for file to STDOUT").action(ArgAction::SetTrue))
    .arg(arg!(camera_profiles: --"camera-profiles" "Write camera profiles to STDOUT (plain text, JSON or YAML)").action(ArgAction::SetTrue))
    .arg(arg!(json: --json "Format metadata as JSON").action(ArgAction::SetTrue))
    .arg(arg!(yaml: --yaml "Format metadata as YAML").action(ArgAction::SetTrue))
    .arg(arg!(<FILE> "Input file").value_parser(clap::value_parser!(PathBuf)))
}

/// Subcommand process-raw
fn process_raw_command() -> Command {
  Command::new("process-raw")
    .arg(remove_dng_opcodes_arg())
    .arg(
      arg!(--"artist" <artist> "Set the artist tag")
        .required(false)
        .value_parser(NonEmptyStringValueParser::new()),
    )
    .arg(
      arg!(keep_mtime: --"keep-mtime" <keepmtime> "Keep mtime, read from EXIF with fallback to original file mtime")
        .value_parser(ValueParser::bool())
        .required(false)
        .default_value("false")
        .default_missing_value("false"),
    )
    .arg(
      arg!(index: --"image-index" <index> "Select a specific image index (or 'all') if file is a image container")
        .required(false)
        .default_value("0"),
    )
    .arg(
      arg!(--"crop" <crop> "Crop of the image: DefaultCrop (best), ActiveArea or none")
        .required(false)
        .value_parser(value_parser!(CropMode))
        .default_value("best"),
    )
    .arg(
      arg!(--"format" <format> "Output image format, defaults to the OUTPUT file extension or tif")
        .required(false)
        .value_parser(value_parser!(ProcessedImageFormat)),
    )
    .arg(
      arg!(jpeg_quality: --"jpeg-quality" <quality> "JPEG quality (1-100)")
        .required(false)
        .value_parser(clap::value_parser!(u8).range(1..=100))
        .default_value("94"),
    )
    .arg(arg!(-f --override "Override existing files").action(ArgAction::SetTrue))
    .arg(arg!(-r --recursive "Process input directory recursive").action(ArgAction::SetTrue))
    .arg(arg!(<INPUT> "Input file or directory").value_parser(clap::value_parser!(PathBuf)))
    .arg(arg!(<OUTPUT> "Output file or existing directory").value_parser(clap::value_parser!(PathBuf)))
}

/// Subcommand convert
fn convert_command() -> Command {
  convert_base_command()
    .name("convert")
    .arg(arg!(-r --recursive "Process input directory recursive").action(ArgAction::SetTrue))
    .arg(
      arg!(jobs: -j --jobs <N> "Number of files to convert in parallel (0 = auto)")
        .required(false)
        .value_parser(clap::value_parser!(usize))
        .default_value("0"),
    )
    .arg(arg!(<INPUT> "Input file or directory").value_parser(clap::value_parser!(PathBuf)))
    .arg(arg!(<OUTPUT> "Output file or existing directory").value_parser(clap::value_parser!(PathBuf)))
}

/// Subcommand ftpserver
fn ftpserver_command() -> Command {
  convert_base_command()
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
    .arg(arg!(<OUTPUT> "Output file or existing directory").value_parser(clap::value_parser!(PathBuf)))
}

/// Subcommand makedng
fn makedng_command() -> Command {
  Command::new("makedng")
    .about("Lowlevel command to make a DNG file")
    .arg_required_else_help(true)
    .arg(arg!(OUTPUT: -o --"output" <OUTPUT> "Output DNG file path").value_parser(clap::value_parser!(PathBuf)))
    .arg(
      arg!(inputs: -i --"input" <INPUT> "Input files (raw, preview, exif, ...), index for map starts with 0")
        .required(true)
        .long_help(
          "Input files to merge into a single DNG file. Usually only a single input file is used.
If multiple input files are given, --map should be used to specifiy how to interpret each intput file.",
        )
        .value_parser(clap::value_parser!(PathBuf))
        .num_args(1..),
    )
    .arg(
      arg!(map: --map <map> "Input usage map")
        .required(false)
        .num_args(1..)
        .default_values(["0:raw", "0:preview", "0:thumbnail", "0:exif", "0:xmp"])
        .long_help(
          "When multiple input files given, each input file should be mapped to a specific type of data.
First input file starts with index 0. Possible types are 'raw', 'preview', 'thumbnail', 'exif', 'xmp'.
Input files which are not mapped are ignored.",
        )
        .value_parser(value_parser!(InputSourceUsageMap)),
    )
    /*
    .arg(
      arg!(dng_version: --"dng-version" <VERSION> "DNG specification version")
        .required(false)
        .default_value("1.6")
        .value_parser(value_parser!(DngVersion)),
    )
     */
    .arg(
      arg!(dng_backward_version: --"dng-backward-version" <version> "DNG specification version")
        .required(false)
        .default_value("1.4")
        .value_parser(value_parser!(DngVersion)),
    )
    .arg(
      arg!(colorimetric_reference: --"colorimetric-reference" <reference> "Reference for XYZ values")
        .required(false)
        .default_value("scene")
        .value_parser(value_parser!(DngColorimetricReference)),
    )
    .arg(
      arg!(unique_camera_model: --"unique-camera-model" <id> "Unique camera model")
        .required(false)
        .value_parser(NonEmptyStringValueParser::new()),
    )
    .arg(
      arg!(artist: --"artist" <artist> "Set the Artist tag")
        .required(false)
        .value_parser(NonEmptyStringValueParser::new()),
    )
    .arg(
      arg!(make: --"make" <make> "Set the Make tag")
        .required(false)
        .value_parser(NonEmptyStringValueParser::new()),
    )
    .arg(
      arg!(model: --"model" <model> "Set the Model tag")
        .required(false)
        .value_parser(NonEmptyStringValueParser::new()),
    )
    .arg(
      arg!(matrix1: --matrix1 <MATRIX> "Matrix 1")
        .required(false)
        .requires("illuminant1")
        .value_parser(ColorMatrixArgParser),
    )
    .arg(
      arg!(matrix2: --matrix2 <MATRIX> "Matrix 2")
        .required(false)
        .requires("illuminant2")
        .value_parser(ColorMatrixArgParser),
    )
    .arg(
      arg!(matrix3: --matrix3 <MATRIX> "Matrix 3")
        .required(false)
        .requires("illuminant3")
        .value_parser(ColorMatrixArgParser),
    )
    .arg(
      arg!(illuminant1: --illuminant1 <ILLUMINANT> "Illuminant 1")
        .required(false)
        .value_parser(CalibrationIlluminantArgParser),
    )
    .arg(
      arg!(illuminant2: --illuminant2 <ILLUMINANT> "Illuminant 2")
        .required(false)
        .value_parser(CalibrationIlluminantArgParser),
    )
    .arg(
      arg!(illuminant3: --illuminant3 <ILLUMINANT> "Illuminant 3")
        .required(false)
        .value_parser(CalibrationIlluminantArgParser),
    )
    .arg(
      arg!(linearization: --linearization <TABLE> "Linearization table")
        .required(false)
        .value_parser(LinearizationTableArgParser {}),
    )
    .arg(
      arg!(as_shot_neutral: --wb <"R,G,B"> "Whitebalance as-shot")
        .required(false)
        .conflicts_with("as_shot_white_xy")
        .value_parser(value_parser!(WhiteBalanceInput)),
    )
    .arg(
      arg!(as_shot_white_xy: --"white-xy" <"x,y"> "Whitebalance as-shot encoded as xy chromaticity coordinates")
        .required(false)
        .value_parser(WhitePointArgParser),
    )
    .arg(arg!(-f --override "Override existing files"))
}

/// Subcommand extract
fn extract_command() -> Command {
  Command::new("extract")
    .about("Extract embedded original Raw from DNG")
    .arg_required_else_help(true)
    .arg(arg!(skipchecks: --skipchecks "Skip integrity checks").action(ArgAction::SetTrue))
    .arg(arg!(-r --recursive "Process input directory recursive").action(ArgAction::SetTrue))
    .arg(arg!(-f --override "Override existing files").action(ArgAction::SetTrue))
    .arg(arg!(<INPUT> "Input file or directory").value_parser(clap::value_parser!(PathBuf)))
    .arg(arg!(<OUTPUT> "Output file or existing directory").value_parser(clap::value_parser!(PathBuf)))
}
