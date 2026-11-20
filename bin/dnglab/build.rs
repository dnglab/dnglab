use std::path::PathBuf;

fn main() {
  emit_version();
}

/// Run git in the package directory, its trimmed output on success
fn git(args: &[&str]) -> Option<String> {
  let output = std::process::Command::new("git")
    .args(args)
    .current_dir(env!("CARGO_MANIFEST_DIR"))
    .output()
    .ok()?;
  if !output.status.success() {
    return None;
  }
  Some(String::from_utf8(output.stdout).ok()?.trim().to_owned())
}

/// Version from `git describe --long --dirty` output, the distance to the tag,
/// the short commit hash and the dirty state as SemVer build metadata:
/// `v0.8.0-484-g3a3425398-dirty` is `0.8.0+484.g3a3425398.dirty`
fn describe_to_version(describe: &str) -> Option<String> {
  let (describe, dirty) = match describe.strip_suffix("-dirty") {
    Some(describe) => (describe, true),
    None => (describe, false),
  };
  let (rest, hash) = describe.rsplit_once('-')?;
  let (tag, distance) = rest.rsplit_once('-')?;
  let tag = tag.strip_prefix('v')?;
  let distance: u32 = distance.parse().ok()?;
  let mut metadata = Vec::new();
  if distance > 0 {
    metadata.push(distance.to_string());
    metadata.push(hash.to_owned());
  }
  if dirty {
    metadata.push("dirty".to_owned());
  }
  if metadata.is_empty() {
    Some(tag.to_owned())
  } else {
    Some(format!("{}+{}", tag, metadata.join(".")))
  }
}

/// Set DNGLAB_VERSION to the version from git, or the package version
/// without a git checkout or version tag
fn emit_version() {
  println!("cargo:rerun-if-changed=build.rs");
  if let (Some(git_dir), Some(common_dir)) = (
    git(&["rev-parse", "--absolute-git-dir"]),
    git(&["rev-parse", "--path-format=absolute", "--git-common-dir"]),
  ) {
    let git_dir = PathBuf::from(git_dir);
    let common_dir = PathBuf::from(common_dir);
    let mut watched = vec![
      git_dir.join("HEAD"),
      git_dir.join("index"),
      common_dir.join("packed-refs"),
      common_dir.join("refs/tags"),
    ];
    if let Some(head_ref) = git(&["symbolic-ref", "-q", "HEAD"]) {
      watched.push(common_dir.join(head_ref));
    }
    // A path that doesn't exist reruns the script on every build
    for path in watched.iter().filter(|path| path.exists()) {
      println!("cargo:rerun-if-changed={}", path.display());
    }
  }
  let version = git(&["describe", "--tags", "--long", "--match", "v[0-9]*", "--dirty"])
    .and_then(|describe| describe_to_version(&describe))
    .unwrap_or_else(|| env!("CARGO_PKG_VERSION").to_owned());
  println!("cargo:rustc-env=DNGLAB_VERSION={version}");
}
