use std::path::PathBuf;

use clap_mangen::Man;

use clap_complete::{
  generate_to,
  Shell::{Bash, Elvish, Fish, PowerShell, Zsh},
};

fn main() -> std::io::Result<()> {
  build_manpages()?;
  build_completions()?;
  Ok(())
}

fn build_completions() -> std::io::Result<()> {
  let outdir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("completions");
  let mut cmd = dnglab_lib::app::create_app().name("dnglab");
  // Generate shell completions.
  for shell in [Bash, Elvish, Fish, PowerShell, Zsh] {
    generate_to(shell, &mut cmd, "dnglab", &outdir).expect("completions build failed");
  }
  Ok(())
}

fn build_manpages() -> std::io::Result<()> {
  let outdir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("manpages");
  let name = "dnglab";
  let cmd = dnglab_lib::app::create_app().name("dnglab");
  let man = clap_mangen::Man::new(cmd.clone());
  let mut buffer: Vec<u8> = Default::default();
  man.render(&mut buffer)?;

  std::fs::write(outdir.join("dnglab.1"), buffer)?;

  for subcommand in cmd.get_subcommands() {
    let subcommand_name = subcommand.get_name();
    let subcommand_name = format!("{name}-{subcommand_name}");
    let mut buffer: Vec<u8> = Default::default();
    let man = Man::new(subcommand.clone().name(&subcommand_name));
    man.render(&mut buffer)?;
    std::fs::write(PathBuf::from(&outdir).join(format!("{}{}", &subcommand_name, ".1")), buffer)?;
  }
  Ok(())
}
