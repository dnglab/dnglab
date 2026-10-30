// SPDX-License-Identifier: LGPL-2.1
// Copyright 2026 Daniel Vogelbacher <daniel@chaospixel.com>

//! Sample files of [rawdb.dnglab.org](https://rawdb.dnglab.org)
//!
//! Samples are downloaded into a local cache when they are used the first
//! time. The cache is the directory in the environment variable `RAWDB_CACHE`
//! or `dnglab/rawdb` in the cache directory of the user, e.g.
//! `~/.cache/dnglab/rawdb` on Linux, `~/Library/Caches/dnglab/rawdb` on macOS
//! and `%LOCALAPPDATA%\dnglab\rawdb` on Windows.
//!
//! ```no_run
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let path = rawler::rawdb::sample("Canon", "EOS 5D Mark III", "raw_modes/Canon EOS 5D Mark III_RAW_ISO_200.CR2")?;
//! let rawimage = rawler::decode_file(&path)?;
//! # Ok(())
//! # }
//! ```

use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
use std::ffi::OsString;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::thread::sleep;
use std::time::Duration;

// RFC 3986 path segment: encode everything except unreserved (alphanumeric + - _ . ~).
const SEGMENT: &AsciiSet = &NON_ALPHANUMERIC.remove(b'-').remove(b'_').remove(b'.').remove(b'~');

const BASE_URL: &str = "https://rawdb.dnglab.org/api/download";
const MAX_RETRIES: u32 = 3;
/// Share of the file system of the cache that downloads keep free, in percent
const RESERVED_SPACE_PERCENT: u64 = 5;
/// Bytes written between two checks of the free space
const SPACE_CHECK_INTERVAL: u64 = 16 << 20;

/// Returns the absolute path of the sample cache.
///
/// The cache is the directory in `RAWDB_CACHE`, or `dnglab/rawdb` in the
/// cache directory of the user if `RAWDB_CACHE` is unset or empty. If the
/// directory does not exist yet it is created, and a note is printed to
/// stderr.
pub fn cache_dir() -> Result<PathBuf, RawdbError> {
  let path = resolve_cache_dir(std::env::var_os("RAWDB_CACHE"), dirs::cache_dir()).ok_or(RawdbError::NoCacheDir)?;
  if !path.exists() {
    fs::create_dir_all(&path)?;
    eprintln!("note: created rawdb cache {} (did not exist)", path.display());
  }
  if !path.is_dir() {
    return Err(io::Error::new(io::ErrorKind::NotADirectory, format!("rawdb cache {} is not a directory", path.display())).into());
  }
  Ok(fs::canonicalize(&path)?)
}

/// Cache directory from the value of `RAWDB_CACHE` or the cache directory of the user
fn resolve_cache_dir(env: Option<OsString>, user_cache: Option<PathBuf>) -> Option<PathBuf> {
  match env.filter(|value| !value.is_empty()) {
    Some(value) => Some(PathBuf::from(value)),
    None => user_cache.map(|dir| dir.join("dnglab").join("rawdb")),
  }
}

#[derive(Debug, thiserror::Error)]
pub enum RawdbError {
  #[error("io error: {0}")]
  Io(#[from] io::Error),
  #[error("network error: {0}")]
  Network(String),
  #[error("http {status} for {url}")]
  Http { status: u16, url: String },
  #[error("no cache directory: RAWDB_CACHE is not set and the user has no cache directory")]
  NoCacheDir,
  #[error("RAWDB_API_KEY_REQUIRED is set but RAWDB_API_KEY is not, refusing to download {url} anonymously")]
  ApiKeyRequired { url: String },
  #[error("not enough space in {}: {available} bytes available, {reserve} bytes are kept free", path.display())]
  InsufficientSpace { path: PathBuf, available: u64, reserve: u64 },
}

/// Ensure a sample file exists in the cache, downloading it from rawdb.dnglab.org if missing.
///
/// Returns the absolute path of the sample file, see [`ensure_file`].
pub fn sample(make: &str, model: &str, subpath: &str) -> Result<PathBuf, RawdbError> {
  ensure_file(&cache_dir()?, make, model, subpath)
}

/// Ensure a sample file exists locally, downloading it from rawdb.dnglab.org if missing.
///
/// Returns the absolute path to the local file. The local layout mirrors the
/// remote one: `{cache}/{make}/{model}/{subpath}`.
///
/// A download keeps 5% of the file system of the cache free: it doesn't start
/// or is stopped with [`RawdbError::InsufficientSpace`] otherwise.
///
/// If `RAWDB_API_KEY` is set in the environment, it is sent as the
/// `X-API-Key` header to bypass anonymous rate limits.
pub fn ensure_file(cache: &Path, make: &str, model: &str, subpath: &str) -> Result<PathBuf, RawdbError> {
  let local = cache.join(make).join(model).join(subpath);
  if local.is_file() {
    return Ok(fs::canonicalize(&local)?);
  }
  let url = build_url(make, model, subpath);
  let api_key = std::env::var("RAWDB_API_KEY").ok();
  if api_key.is_none() && api_key_required() {
    return Err(RawdbError::ApiKeyRequired { url });
  }
  let dir = local.parent().unwrap_or(cache);
  fs::create_dir_all(dir)?;
  ensure_space(dir, 0)?;
  eprintln!("Downloading sample: {}/{}/{}", make, model, subpath);

  let mut attempt: u32 = 0;
  loop {
    attempt += 1;
    let mut req = ureq::get(&url);
    if let Some(k) = &api_key {
      req = req.set("X-API-Key", k);
    }
    match req.call() {
      Ok(resp) => {
        let length = resp.header("Content-Length").and_then(|v| v.trim().parse::<u64>().ok());
        ensure_space(dir, length.unwrap_or(0))?;
        write_atomic(&local, resp.into_reader())?;
        break;
      }
      Err(ureq::Error::Status(code, resp)) => {
        if code == 404 || attempt > MAX_RETRIES {
          return Err(RawdbError::Http { status: code, url });
        }
        let delay = if code == 429 {
          parse_retry_after(&resp).unwrap_or_else(|| retry_delay(attempt))
        } else {
          retry_delay(attempt)
        };
        sleep(delay);
      }
      Err(ureq::Error::Transport(t)) => {
        if attempt > MAX_RETRIES {
          return Err(RawdbError::Network(t.to_string()));
        }
        sleep(retry_delay(attempt));
      }
    }
  }

  Ok(fs::canonicalize(&local)?)
}

// 10s before retry #1, 60s before #2, 120s before #3.
/// Returns true if `RAWDB_API_KEY_REQUIRED` is set to a truthy value
/// (`"true"`, `"1"`, `"yes"`; case-insensitive). When true, downloads must
/// not proceed without `RAWDB_API_KEY` — used by CI to guarantee tests use
/// the rate-limit-exempt key rather than silently falling back to anonymous
/// requests.
fn api_key_required() -> bool {
  matches!(
    std::env::var("RAWDB_API_KEY_REQUIRED").as_deref().map(str::trim),
    Ok("true" | "TRUE" | "True" | "1" | "yes" | "YES" | "Yes")
  )
}

fn retry_delay(attempt: u32) -> Duration {
  match attempt {
    1 => Duration::from_secs(10),
    n => Duration::from_secs(60u64.saturating_mul(1u64 << (n - 2))),
  }
}

fn build_url(make: &str, model: &str, subpath: &str) -> String {
  let mut s = String::from(BASE_URL);
  s.push('/');
  s.push_str(&utf8_percent_encode(make, SEGMENT).to_string());
  s.push('/');
  s.push_str(&utf8_percent_encode(model, SEGMENT).to_string());
  for seg in subpath.split('/').filter(|p| !p.is_empty()) {
    s.push('/');
    s.push_str(&utf8_percent_encode(seg, SEGMENT).to_string());
  }
  s
}

// Only the integer-seconds form of Retry-After is honored.
fn parse_retry_after(resp: &ureq::Response) -> Option<Duration> {
  resp.header("Retry-After").and_then(|v| v.trim().parse::<u64>().ok()).map(Duration::from_secs)
}

/// Checks that the file system of `dir` keeps its reserve free after writing
/// `needed` bytes. The check is skipped if the file system doesn't report its
/// space.
fn ensure_space(dir: &Path, needed: u64) -> Result<(), RawdbError> {
  match fs4::statvfs(dir) {
    Ok(stats) => check_space(dir, stats.available_space(), stats.total_space(), needed),
    Err(err) => {
      log::warn!("Free space of {} is unknown, it is not checked: {}", dir.display(), err);
      Ok(())
    }
  }
}

/// Checks that `available` bytes of a file system with `total` bytes keep the
/// reserve free after writing `needed` bytes
fn check_space(dir: &Path, available: u64, total: u64, needed: u64) -> Result<(), RawdbError> {
  let reserve = total / 100 * RESERVED_SPACE_PERCENT;
  if available < needed.saturating_add(reserve) {
    return Err(RawdbError::InsufficientSpace {
      path: dir.to_path_buf(),
      available,
      reserve,
    });
  }
  Ok(())
}

/// Writes the data of `reader` to `final_path`, through a `.part` file that is
/// removed if writing fails or the free space of the file system runs out
fn write_atomic(final_path: &Path, mut reader: impl io::Read) -> Result<(), RawdbError> {
  let mut tmp = final_path.as_os_str().to_owned();
  tmp.push(".part");
  let tmp = PathBuf::from(tmp);
  let dir = final_path.parent().unwrap_or(Path::new("."));
  let mut write = || -> Result<(), RawdbError> {
    let mut f = fs::File::create(&tmp)?;
    while io::copy(&mut io::Read::take(&mut reader, SPACE_CHECK_INTERVAL), &mut f)? > 0 {
      ensure_space(dir, 0)?;
    }
    f.sync_all()?;
    Ok(())
  };
  if let Err(err) = write() {
    fs::remove_file(&tmp).ok();
    return Err(err);
  }
  Ok(fs::rename(&tmp, final_path)?)
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn short_circuits_when_file_already_present() {
    let dir = std::env::temp_dir().join(format!("rawdb_test_{}", std::process::id()));
    let target = dir.join("Make").join("Model").join("sub/dir/file.raw");
    fs::create_dir_all(target.parent().unwrap()).unwrap();
    fs::write(&target, b"hello").unwrap();

    let got = ensure_file(&dir, "Make", "Model", "sub/dir/file.raw").expect("must succeed without network");
    assert_eq!(fs::canonicalize(&target).unwrap(), got);

    fs::remove_dir_all(&dir).ok();
  }

  #[test]
  fn write_atomic_removes_part_file_on_error() -> Result<(), RawdbError> {
    let dir = std::env::temp_dir().join(format!("rawdb_write_test_{}", std::process::id()));
    fs::create_dir_all(&dir)?;
    let target = dir.join("file.raw");
    write_atomic(&target, &b"hello"[..])?;
    assert_eq!(fs::read(&target)?, b"hello");
    // A download that fails after some data
    let failing = io::Read::chain(&b"partial"[..], FailingReader);
    assert!(matches!(write_atomic(&dir.join("failed.raw"), failing), Err(RawdbError::Io(_))));
    assert!(!dir.join("failed.raw").exists() && !dir.join("failed.raw.part").exists());
    fs::remove_dir_all(&dir).ok();
    Ok(())
  }

  struct FailingReader;

  impl io::Read for FailingReader {
    fn read(&mut self, _buf: &mut [u8]) -> io::Result<usize> {
      Err(io::Error::new(io::ErrorKind::ConnectionReset, "connection reset"))
    }
  }

  #[test]
  fn cache_dir_from_env_or_user_cache() {
    let user_cache = Some(PathBuf::from("/home/user/.cache"));
    assert_eq!(
      resolve_cache_dir(Some("/data/rawdb".into()), user_cache.clone()),
      Some(PathBuf::from("/data/rawdb"))
    );
    // An empty RAWDB_CACHE is ignored
    let default = Some(PathBuf::from("/home/user/.cache/dnglab/rawdb"));
    assert_eq!(resolve_cache_dir(Some(OsString::new()), user_cache.clone()), default);
    assert_eq!(resolve_cache_dir(None, user_cache), default);
    assert_eq!(resolve_cache_dir(None, None), None);
  }

  #[test]
  fn downloads_keep_space_free() {
    let dir = Path::new("/cache");
    let gib = 1 << 30;
    // 5 GiB of 100 GiB are kept free
    assert!(check_space(dir, 10 * gib, 100 * gib, 5 * gib).is_ok());
    assert!(check_space(dir, 10 * gib, 100 * gib, 0).is_ok());
    assert!(matches!(
      check_space(dir, 10 * gib, 100 * gib, 5 * gib + 1),
      Err(RawdbError::InsufficientSpace { available, reserve, .. }) if available == 10 * gib && reserve == 5 * gib
    ));
    assert!(check_space(dir, 4 * gib, 100 * gib, 0).is_err());
    // A sample larger than the available space
    assert!(check_space(dir, 100, 100, u64::MAX).is_err());
    let temp_dir = std::env::temp_dir();
    assert!(ensure_space(&temp_dir, 0).is_ok());
    assert!(matches!(ensure_space(&temp_dir, u64::MAX), Err(RawdbError::InsufficientSpace { .. })));
  }

  #[test]
  fn url_encoding_handles_spaces_and_keeps_separators() {
    let url = build_url("Samsung", "NX3000", "raw_modes/NX3000_ISO_100_Samsung SRW Compressed 2.SRW");
    assert_eq!(
      url,
      "https://rawdb.dnglab.org/api/download/Samsung/NX3000/raw_modes/NX3000_ISO_100_Samsung%20SRW%20Compressed%202.SRW"
    );
  }

  #[test]
  fn retry_delay_schedule() {
    assert_eq!(retry_delay(1), Duration::from_secs(10));
    assert_eq!(retry_delay(2), Duration::from_secs(60));
    assert_eq!(retry_delay(3), Duration::from_secs(120));
  }
}
