// SPDX-License-Identifier: LGPL-2.1
// Copyright 2026 Daniel Vogelbacher <daniel@chaospixel.com>

use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::thread::sleep;
use std::time::Duration;

// RFC 3986 path segment: encode everything except unreserved (alphanumeric + - _ . ~).
const SEGMENT: &AsciiSet = &NON_ALPHANUMERIC.remove(b'-').remove(b'_').remove(b'.').remove(b'~');

const BASE_URL: &str = "https://rawdb.dnglab.org/api/download";
const MAX_RETRIES: u32 = 3;

/// Returns the absolute path of the directory referenced by `RAWDB_CACHE`.
///
/// If the directory does not exist yet it is created, and a note is printed to
/// stderr. Panics if `RAWDB_CACHE` is unset or the path exists but is not a
/// directory.
pub fn get_rawdb_cache() -> PathBuf {
  let raw = std::env::var("RAWDB_CACHE").expect("RAWDB_CACHE environment variable must be set (~100 GiB data to download!)");
  let path = PathBuf::from(&raw);
  if !path.exists() {
    fs::create_dir_all(&path).unwrap_or_else(|e| panic!("Failed to create RAWDB_CACHE={raw:?}: {e}"));
    eprintln!("note: created RAWDB_CACHE={raw:?} (did not exist)");
  }
  assert!(path.is_dir(), "RAWDB_CACHE={raw:?} exists but is not a directory");
  fs::canonicalize(&path).expect("RAWDB_CACHE must be canonicalizable to an absolute path")
}

#[derive(Debug, thiserror::Error)]
pub enum RawdbError {
  #[error("io error: {0}")]
  Io(#[from] io::Error),
  #[error("network error: {0}")]
  Network(String),
  #[error("http {status} for {url}")]
  Http { status: u16, url: String },
}

/// Ensure a sample file exists locally, downloading it from rawdb.dnglab.org if missing.
///
/// Returns the absolute path to the local file. The local layout mirrors the
/// remote one: `{rawdb_cache}/{make}/{model}/{subpath}`.
///
/// If `RAWDB_API_KEY` is set in the environment, it is sent as the
/// `X-API-Key` header to bypass anonymous rate limits.
pub fn rawdb_ensure_file(rawdb_cache: &Path, make: &str, model: &str, subpath: &str) -> Result<PathBuf, RawdbError> {
  let local = rawdb_cache.join(make).join(model).join(subpath);
  if local.is_file() {
    return Ok(fs::canonicalize(&local)?);
  }
  eprintln!("Downloading sample: {}/{}/{}", make, model, subpath);
  if let Some(parent) = local.parent() {
    fs::create_dir_all(parent)?;
  }

  let url = build_url(make, model, subpath);
  let api_key = std::env::var("RAWDB_API_KEY").ok();

  let mut attempt: u32 = 0;
  loop {
    attempt += 1;
    let mut req = ureq::get(&url);
    if let Some(k) = &api_key {
      req = req.set("X-API-Key", k);
    }
    match req.call() {
      Ok(resp) => {
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

fn write_atomic(final_path: &Path, mut reader: impl io::Read) -> io::Result<()> {
  let mut tmp = final_path.as_os_str().to_owned();
  tmp.push(".part");
  let tmp = PathBuf::from(tmp);
  {
    let mut f = fs::File::create(&tmp)?;
    io::copy(&mut reader, &mut f)?;
    f.sync_all()?;
  }
  fs::rename(&tmp, final_path)
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

    let got = rawdb_ensure_file(&dir, "Make", "Model", "sub/dir/file.raw").expect("must succeed without network");
    assert_eq!(fs::canonicalize(&target).unwrap(), got);

    fs::remove_dir_all(&dir).ok();
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
