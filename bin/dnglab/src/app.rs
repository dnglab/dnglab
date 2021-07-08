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
          (@arg pixel: --pixel "Write uncompressed pixel data to STDOUT")
          (@arg meta: --meta "Write metadata to STDOUT")
          (@arg summary: --summary "Write summary information for file to STDOUT")
          (@arg json: --json "Format metadata as JSON")
          (@arg yaml: --yaml "Format metadata as YAML")
          (@arg FILE: +required "Input file")
      )
      (@subcommand convert =>
          (about: "Convert raw image(s) into dng format")
          //(@arg profile: -p --profile "Profile file to use")
          //(@arg dng_version: --dng-version +takes_value "DNG version for output file")
          (@arg compression: -c --compression +takes_value "'lossless' (default) or 'none'")
          (@arg nocrop: --nocrop "Do not crop black areas, output full sensor data")
          (@arg noembedded: --noembedded "Do not embed original raw file")
          (@arg recursive: -r --recursive "Process input directory recursive")
          (@arg override: -f --override "Override existing files")
          (@arg INPUT: +required "Input file or directory")
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
      (@subcommand gui =>
          (about: "Start GUI (not implemented)")
      )
  );
  app
}
