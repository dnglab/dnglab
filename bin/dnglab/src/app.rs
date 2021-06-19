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
      (@arg verbose: --verbose +global "Print more messages")
      (@arg debug: -d ... +global "Sets the level of debugging information")
      /*
      (@subcommand analyze =>
          (about: "Analyze raw image")
          (@arg reset: --reset "Reset the profile (delete all content and keys)")
          (@arg default: --default "Mark the new profile as default")
          (@arg FILE: +required "Input file")
      )
       */
      (@subcommand convert =>
          (about: "Convert raw image(s) into dng format")
          //(@arg profile: -p --profile "Profile file to use")
          //(@arg dng_version: --dng-version +takes_value "DNG version for output file")
          //(@arg uncompressed: --uncompressed "Write uncompressed DNG output file")
          //(@arg recursive: -r --recursive "Process input directory recursive")
          (@arg override: -f --override "Override existing files")
          (@arg INPUT: +required "Input file or directory")
          (@arg OUTPUT: +required "Output file or existing directory")
      )
      (@subcommand extract =>
          (about: "Extract embedded original Raw from DNG")
          //(@arg recursive: -r --recursive "Process input directory recursive")
          (@arg INPUT: +required "Input file or directory")
          (@arg OUTPUT: +required "Output file or existing directory")
      )
      (@subcommand gui =>
          (about: "Start GUI (not implemented)")
      )
  );
  app
}
