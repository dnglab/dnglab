#!/usr/bin/env python3
"""
Regenerate the rawler rawdb test index.

For every entry in rawler/tests/supported_rawdb_sets.txt this script:

  1. Queries https://rawdb.dnglab.org/api/sets/{maker}/{model} to learn which
     files exist in the requested subfolder.
  2. Downloads missing or stale files into $RAWDB_CACHE/{maker}/{model}/...
     (freshness: file size from listing, Last-Modified only when size matches).
  3. Runs `dnglab analyze` five times per file to produce the .analyze.yaml +
     four .digest.*.txt files under rawler/data/testdata/rawdb/.
  4. After every file is processed successfully, writes
     rawler/tests/rawdb/mod.rs once with one `mod <maker>` per maker, holding
     `super::rawdb_test_file!(...)` invocations for every file from that
     maker.

Env vars:
  RAWDB_CACHE      Required. Local cache root.
  RAWDB_API_KEY    Optional. Sent as `X-API-Key` to bypass anonymous limits.
"""

from __future__ import annotations

import argparse
import email.utils
import os
import re
import subprocess
import sys
import time
from concurrent.futures import ThreadPoolExecutor, as_completed
from dataclasses import dataclass
from pathlib import Path
from typing import Iterable

try:
    import requests
except ImportError:
    sys.stderr.write("error: the `requests` package is required. Install with: pip install requests\n")
    sys.exit(2)


REPO_ROOT = Path(__file__).resolve().parent
DEFAULT_SETS = REPO_ROOT / "rawler/tests/supported_rawdb_sets.txt"
DEFAULT_TESTDATA = REPO_ROOT / "rawler/data/testdata"
DEFAULT_MODULE_OUT = REPO_ROOT / "rawler/tests/rawdb/mod.rs"
DEFAULT_DNGLAB = REPO_ROOT / "target/release/dnglab"
DEFAULT_BASE_URL = "https://rawdb.dnglab.org"

MAX_RETRIES = 3
ANALYZE_OUTPUTS = [
    (["--meta", "--yaml"], ".analyze.yaml"),
    (["--raw-checksum"], ".digest.txt"),
    (["--full-checksum"], ".digest.full.txt"),
    (["--preview-checksum"], ".digest.preview.txt"),
    (["--thumbnail-checksum"], ".digest.thumbnail.txt"),
]


# ---------------------------------------------------------------- data types


@dataclass(frozen=True)
class SetLine:
    maker: str
    model: str
    subfolder: str


@dataclass(frozen=True)
class FileJob:
    maker: str
    model: str
    subfolder: str
    rel_path: str       # e.g. "raw_modes/Canon EOS R6_RAW.CR3"
    expected_size: int


@dataclass(frozen=True)
class TestEntry:
    maker: str
    model: str
    rel_path: str
    test_ident: str
    download_status: str  # "created" | "unchanged"
    analyze_status: str   # "regenerated" | "skipped"


# ---------------------------------------------------------------- helpers


def log(msg: str) -> None:
    print(msg, file=sys.stderr, flush=True)


def slug(s: str) -> str:
    """Lowercase, non-alnum runs → '_', strip surrounding '_'."""
    return re.sub(r"[^a-z0-9]+", "_", s.lower()).strip("_")


def test_ident(rel_path: str) -> str:
    """slug(basename(rel_path)); prefixed with `sample_` if it would start with a non-letter."""
    s = slug(os.path.basename(rel_path))
    return s if s and s[0].isalpha() else "sample_" + s


def parse_set_line(raw: str) -> SetLine | None:
    line = raw.strip()
    if not line or line.startswith("#"):
        return None
    parts = line.split("/")
    if len(parts) < 3:
        log(f"warn: skipping malformed set line: {line!r}")
        return None
    maker = parts[0]
    model = parts[1]
    subfolder = "/".join(parts[2:])
    return SetLine(maker=maker, model=model, subfolder=subfolder)


def read_sets(path: Path) -> list[SetLine]:
    entries: list[SetLine] = []
    with path.open() as f:
        for raw in f:
            entry = parse_set_line(raw)
            if entry is not None:
                entries.append(entry)
    return entries


def retry_delay(attempt: int) -> float:
    """10 s before retry #1, 60 s before #2, 120 s before #3."""
    if attempt == 1:
        return 10.0
    return 60.0 * (2 ** (attempt - 2))


def parse_retry_after(value: str | None) -> float | None:
    if value is None:
        return None
    value = value.strip()
    try:
        return float(value)
    except ValueError:
        pass
    try:
        ts = email.utils.parsedate_to_datetime(value)
        if ts is not None:
            return max(0.0, ts.timestamp() - time.time())
    except (TypeError, ValueError):
        pass
    return None


def parse_last_modified(value: str | None) -> float | None:
    if value is None:
        return None
    try:
        ts = email.utils.parsedate_to_datetime(value)
        return ts.timestamp() if ts is not None else None
    except (TypeError, ValueError):
        return None


def make_session(api_key: str | None) -> requests.Session:
    s = requests.Session()
    s.headers["User-Agent"] = "rawdb_regen_tests.py/1.0"
    if api_key:
        s.headers["X-API-Key"] = api_key
    return s


# ---------------------------------------------------------------- API


def _get_json_with_retries(session: requests.Session, url: str, allow_404: bool, allow_401: bool) -> dict | None:
    """GET `url` and decode JSON, with the shared retry policy.

    Returns None when allow_404 and the server replies 404, or when
    allow_401 and the server replies 401. Other 4xx/5xx are retried and
    eventually raise RuntimeError.
    """
    for attempt in range(1, MAX_RETRIES + 2):
        try:
            r = session.get(url, timeout=60)
        except requests.RequestException as e:
            if attempt > MAX_RETRIES:
                raise RuntimeError(f"network error fetching {url}: {e}") from e
            time.sleep(retry_delay(attempt))
            continue
        if r.status_code == 200:
            return r.json()
        if r.status_code == 404 and allow_404:
            return None
        if r.status_code == 401 and allow_401:
            return None
        if attempt > MAX_RETRIES:
            raise RuntimeError(f"HTTP {r.status_code} for {url}")
        if r.status_code == 429:
            delay = parse_retry_after(r.headers.get("Retry-After")) or retry_delay(attempt)
        else:
            delay = retry_delay(attempt)
        time.sleep(delay)
    raise RuntimeError(f"unreachable: exhausted retries for {url}")


def fetch_set(session: requests.Session, base_url: str, maker: str, model: str) -> dict | None:
    """Return SetDetailResponse dict or None on 404."""
    url = f"{base_url}/api/sets/{requests.utils.quote(maker, safe='')}/{requests.utils.quote(model, safe='')}"
    return _get_json_with_retries(session, url, allow_404=True, allow_401=False)


def fetch_export(session: requests.Session, base_url: str) -> dict | None:
    """Return ExportResponse dict, or None if the API key is missing/invalid (401)."""
    return _get_json_with_retries(session, f"{base_url}/api/export", allow_404=False, allow_401=True)


def build_catalogue_from_export(export: dict) -> dict[tuple[str, str], dict[str, list[dict]]]:
    """Group each ExportSetEnvelope.files entry by category."""
    catalogue: dict[tuple[str, str], dict[str, list[dict]]] = {}
    for s in export.get("sets", []):
        key = (s["maker"], s["model"])
        by_cat = catalogue.setdefault(key, {})
        for f in s.get("files", []):
            by_cat.setdefault(f["category"], []).append(f)
    for by_cat in catalogue.values():
        for files in by_cat.values():
            files.sort(key=lambda f: f["path"])
    return catalogue


def build_catalogue_from_per_set(
    session: requests.Session,
    base_url: str,
    pairs: Iterable[tuple[str, str]],
    verbose: bool,
) -> dict[tuple[str, str], dict[str, list[dict]]]:
    catalogue: dict[tuple[str, str], dict[str, list[dict]]] = {}
    for maker, model in sorted(set(pairs)):
        if verbose:
            log(f"fetch set {maker}/{model}")
        detail = fetch_set(session, base_url, maker, model)
        if detail is None:
            log(f"warn: set not found on server: {maker}/{model}")
            continue
        catalogue[(maker, model)] = detail.get("categories", {})
    return catalogue


def head_remote(session: requests.Session, base_url: str, maker: str, model: str, rel_path: str) -> tuple[int | None, float | None]:
    url = download_url(base_url, maker, model, rel_path)
    try:
        r = session.head(url, allow_redirects=True, timeout=30)
    except requests.RequestException:
        return (None, None)
    if r.status_code >= 400:
        return (None, None)
    size = None
    if "Content-Length" in r.headers:
        try:
            size = int(r.headers["Content-Length"])
        except ValueError:
            pass
    return (size, parse_last_modified(r.headers.get("Last-Modified")))


def download_url(base_url: str, maker: str, model: str, rel_path: str) -> str:
    parts = [requests.utils.quote(maker, safe=""), requests.utils.quote(model, safe="")]
    parts += [requests.utils.quote(seg, safe="") for seg in rel_path.split("/") if seg]
    return f"{base_url}/api/download/" + "/".join(parts)


def download_file(session: requests.Session, base_url: str, job: FileJob, dest: Path) -> float | None:
    """Stream-download `job` to `dest` atomically. Return remote Last-Modified ts or None."""
    url = download_url(base_url, job.maker, job.model, job.rel_path)
    tmp = dest.with_suffix(dest.suffix + ".part")
    dest.parent.mkdir(parents=True, exist_ok=True)
    for attempt in range(1, MAX_RETRIES + 2):
        try:
            with session.get(url, stream=True, allow_redirects=True, timeout=60) as r:
                if r.status_code == 404:
                    raise RuntimeError(f"404 not found: {url}")
                if r.status_code >= 400:
                    if attempt > MAX_RETRIES:
                        raise RuntimeError(f"HTTP {r.status_code} for {url}")
                    if r.status_code == 429:
                        delay = parse_retry_after(r.headers.get("Retry-After")) or retry_delay(attempt)
                    else:
                        delay = retry_delay(attempt)
                    time.sleep(delay)
                    continue
                with tmp.open("wb") as out:
                    for chunk in r.iter_content(chunk_size=1 << 20):
                        if chunk:
                            out.write(chunk)
                lm = parse_last_modified(r.headers.get("Last-Modified"))
            tmp.replace(dest)
            if lm is not None:
                os.utime(dest, (lm, lm))
            return lm
        except requests.RequestException as e:
            tmp.unlink(missing_ok=True)
            if attempt > MAX_RETRIES:
                raise RuntimeError(f"network error downloading {url}: {e}") from e
            time.sleep(retry_delay(attempt))
    raise RuntimeError(f"unreachable: exhausted retries for {url}")


# ---------------------------------------------------------------- per-file work


def ensure_file(session: requests.Session, base_url: str, job: FileJob, cache_root: Path, verbose: bool) -> tuple[Path, str]:
    """Download `job` if missing/stale. Returns (local_path, "created" | "unchanged")."""
    local = cache_root / job.maker / job.model / job.rel_path
    needs_download = True
    if local.is_file():
        if local.stat().st_size == job.expected_size:
            # Size matches; consult Last-Modified only.
            _, remote_mtime = head_remote(session, base_url, job.maker, job.model, job.rel_path)
            if remote_mtime is None or remote_mtime <= local.stat().st_mtime + 1:
                needs_download = False
        else:
            if verbose:
                log(f"  size mismatch ({local.stat().st_size} != {job.expected_size}), re-downloading {job.rel_path}")
    if needs_download:
        log(f"  download {job.maker}/{job.model}/{job.rel_path}")
        download_file(session, base_url, job, local)
        return local, "created"
    return local, "unchanged"


def run_analyze(dnglab: Path, raw_file: Path, out_base: Path, verbose: bool, override: bool) -> str:
    """Run the 5 dnglab analyze invocations. Returns "regenerated" or "skipped"."""
    out_base.parent.mkdir(parents=True, exist_ok=True)
    out_paths = [out_base.with_name(out_base.name + suffix) for _, suffix in ANALYZE_OUTPUTS]
    if not override and all(p.is_file() and p.stat().st_size > 0 for p in out_paths):
        if verbose:
            log(f"  analyze skipped (all 5 testdata files present): {raw_file.name}")
        return "skipped"
    for (flags, _suffix), out_path in zip(ANALYZE_OUTPUTS, out_paths):
        if verbose:
            log(f"  analyze {' '.join(flags)} -> {out_path.name}")
        with out_path.open("wb") as out:
            res = subprocess.run(
                [str(dnglab), "analyze", *flags, str(raw_file)],
                stdout=out, stderr=subprocess.PIPE,
            )
        if res.returncode != 0:
            raise RuntimeError(
                f"dnglab analyze {flags} failed for {raw_file}:\n{res.stderr.decode(errors='replace')}"
            )
    return "regenerated"


def process_job(
    job: FileJob,
    session: requests.Session,
    base_url: str,
    cache_root: Path,
    testdata_root: Path,
    dnglab: Path,
    verbose: bool,
    override: bool,
) -> TestEntry:
    raw_file, download_status = ensure_file(session, base_url, job, cache_root, verbose)
    out_base = testdata_root / "rawdb" / job.maker / job.model / job.rel_path
    analyze_status = run_analyze(dnglab, raw_file, out_base, verbose, override)
    return TestEntry(
        maker=job.maker,
        model=job.model,
        rel_path=job.rel_path,
        test_ident=test_ident(job.rel_path),
        download_status=download_status,
        analyze_status=analyze_status,
    )


# ---------------------------------------------------------------- planning


def plan_jobs(
    catalogue: dict[tuple[str, str], dict[str, list[dict]]],
    set_lines: Iterable[SetLine],
) -> list[FileJob]:
    # Group requested subfolders per (maker, model).
    wanted: dict[tuple[str, str], set[str]] = {}
    for entry in set_lines:
        wanted.setdefault((entry.maker, entry.model), set()).add(entry.subfolder)

    jobs: list[FileJob] = []
    seen: set[tuple[str, str, str]] = set()
    for (maker, model), subfolders in sorted(wanted.items()):
        categories = catalogue.get((maker, model))
        if categories is None:
            log(f"warn: set not in catalogue: {maker}/{model}")
            continue
        for sub in sorted(subfolders):
            envelopes = categories.get(sub)
            if not envelopes:
                log(f"warn: subfolder {sub!r} absent for {maker}/{model}")
                continue
            for env in envelopes:
                rel = env["path"]
                key = (maker, model, rel)
                if key in seen:
                    continue
                seen.add(key)
                jobs.append(FileJob(
                    maker=maker, model=model, subfolder=sub,
                    rel_path=rel, expected_size=int(env["size"]),
                ))
    return jobs


# ---------------------------------------------------------------- output


def inner_mod_name(model: str, subfolder: str) -> str:
    """Inner per-set module name from `model/subfolder`: lowercase, '+' → 'plus',
       runs of non-alnum collapsed to '_'. Prefixed with `sample_` if it would
       otherwise start with a non-letter."""
    raw = f"{model}/{subfolder}".replace("+", "plus")
    s = re.sub(r"[^a-zA-Z0-9]+", "_", raw).strip("_").lower()
    return s if s and s[0].isalpha() else "sample_" + s


def render_module(entries: list[TestEntry]) -> str:
    by_maker: dict[str, list[TestEntry]] = {}
    for e in entries:
        by_maker.setdefault(slug(e.maker), []).append(e)

    lines: list[str] = []
    lines.append("// @generated by rawdb_regen_tests.py — DO NOT EDIT BY HAND")
    lines.append("use crate::common::rawdb_test_file;")
    lines.append("")
    for maker_slug in sorted(by_maker):
        bucket = by_maker[maker_slug]
        # Group entries by (model, subfolder) for the inner per-set module.
        inner: dict[tuple[str, str], list[TestEntry]] = {}
        for e in bucket:
            subfolder = e.rel_path.split("/", 1)[0]
            inner.setdefault((e.model, subfolder), []).append(e)

        lines.append(f"mod {maker_slug} {{")
        for (model, subfolder), files in sorted(inner.items()):
            mod_name = inner_mod_name(model, subfolder)
            lines.append(f"  mod {mod_name} {{")
            for e in sorted(files, key=lambda e: e.rel_path):
                lines.append(
                    f'    super::super::rawdb_test_file!("{e.maker}", "{e.model}", '
                    f'{e.test_ident}, "{e.rel_path}");'
                )
            lines.append("  }")
        lines.append("}")
        lines.append("")
    return "\n".join(lines)


def write_module(path: Path, content: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    tmp = path.with_suffix(path.suffix + ".tmp")
    tmp.write_text(content)
    tmp.replace(path)


# ---------------------------------------------------------------- main


def main(argv: list[str] | None = None) -> int:
    p = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    p.add_argument("-j", "--jobs", type=int, default=os.cpu_count() or 4)
    p.add_argument("--sets", type=Path, default=DEFAULT_SETS)
    p.add_argument("--testdata-dir", type=Path, default=DEFAULT_TESTDATA)
    p.add_argument("--module-out", type=Path, default=DEFAULT_MODULE_OUT)
    p.add_argument("--dnglab", type=Path, default=DEFAULT_DNGLAB)
    p.add_argument("--base-url", default=DEFAULT_BASE_URL)
    p.add_argument("--build", dest="build", action="store_true", default=True)
    p.add_argument("--no-build", dest="build", action="store_false")
    p.add_argument("--no-export-api", action="store_true",
                   help="Force per-pair /api/sets calls even when RAWDB_API_KEY is set.")
    p.add_argument("--override", action="store_true",
                   help="Re-run dnglab analyze even when testdata files already exist.")
    p.add_argument("--dry-run", action="store_true")
    p.add_argument("-v", "--verbose", action="store_true")
    args = p.parse_args(argv)

    cache_env = os.environ.get("RAWDB_CACHE")
    if not cache_env:
        log("error: RAWDB_CACHE must be set")
        return 2
    cache_root = Path(cache_env)
    if not cache_root.exists():
        cache_root.mkdir(parents=True)
        log(f"note: created RAWDB_CACHE={cache_root} (did not exist)")
    if not cache_root.is_dir():
        log(f"error: RAWDB_CACHE={cache_root} exists but is not a directory")
        return 2

    if args.build and not args.dry_run:
        log("building dnglab (release, features=rawdb)")
        res = subprocess.run(
            ["cargo", "build", "--release", "--features", "rawdb"],
            cwd=REPO_ROOT,
        )
        if res.returncode != 0:
            log("error: cargo build failed")
            return res.returncode
    if not args.dnglab.exists():
        log(f"error: dnglab binary not found at {args.dnglab} (use --dnglab or --build)")
        return 2

    set_lines = read_sets(args.sets)
    if not set_lines:
        log(f"error: no usable lines in {args.sets}")
        return 2
    log(f"loaded {len(set_lines)} set entries from {args.sets}")

    api_key = os.environ.get("RAWDB_API_KEY")
    session = make_session(api_key)

    catalogue: dict[tuple[str, str], dict[str, list[dict]]] | None = None
    if api_key and not args.no_export_api:
        export = fetch_export(session, args.base_url)
        if export is not None:
            catalogue = build_catalogue_from_export(export)
            log(f"loaded catalogue from /api/export ({export.get('set_count', len(catalogue))} sets)")
        else:
            log("warn: /api/export rejected the API key, falling back to /api/sets per pair")
    if catalogue is None:
        pairs = {(e.maker, e.model) for e in set_lines}
        catalogue = build_catalogue_from_per_set(session, args.base_url, pairs, args.verbose)
        log(f"loaded catalogue from /api/sets ({len(catalogue)} sets)")

    jobs = plan_jobs(catalogue, set_lines)
    log(f"planned {len(jobs)} file jobs")

    if args.dry_run:
        for job in jobs:
            print(f"{job.maker}/{job.model}/{job.rel_path}  ({job.expected_size} bytes)")
        return 0

    entries: list[TestEntry] = []
    failures: list[tuple[FileJob, str]] = []

    with ThreadPoolExecutor(max_workers=args.jobs) as pool:
        # requests.Session is thread-safe for parallel GETs; share one.
        futures = {
            pool.submit(
                process_job, job, session, args.base_url,
                cache_root, args.testdata_dir, args.dnglab, args.verbose, args.override,
            ): job
            for job in jobs
        }
        done = 0
        for fut in as_completed(futures):
            job = futures[fut]
            done += 1
            try:
                entry = fut.result()
            except Exception as e:
                failures.append((job, str(e)))
                log(f"[{done}/{len(jobs)}] FAIL {job.maker}/{job.model}/{job.rel_path}: {e}")
            else:
                entries.append(entry)
                log(f"[{done}/{len(jobs)}] ok   {entry.maker}/{entry.model}/{entry.rel_path}")

    def print_summary(module_written: Path | None) -> None:
        created = sum(1 for e in entries if e.download_status == "created")
        unchanged = sum(1 for e in entries if e.download_status == "unchanged")
        regenerated = sum(1 for e in entries if e.analyze_status == "regenerated")
        skipped = sum(1 for e in entries if e.analyze_status == "skipped")
        log("summary:")
        log(f"  downloads:   {created} created, {unchanged} unchanged")
        log(f"  analyze:     {regenerated} regenerated, {skipped} skipped (--override to force)")
        if module_written is not None:
            makers = len({slug(e.maker) for e in entries})
            log(f"  test module: {module_written}  ({len(entries)} entries across {makers} makers)")
        if failures:
            log(f"  failures:    {len(failures)}")

    if failures:
        log(f"error: {len(failures)} job(s) failed; mod.rs not written")
        for job, msg in failures[:10]:
            log(f"  - {job.maker}/{job.model}/{job.rel_path}: {msg}")
        print_summary(module_written=None)
        return 1

    content = render_module(entries)
    write_module(args.module_out, content)
    log(f"wrote {args.module_out} ({len(entries)} test entries, "
        f"{len({slug(e.maker) for e in entries})} makers)")

    # Best-effort formatting; failure is not fatal.
    fmt = subprocess.run(["cargo", "fmt", "-p", "rawler"], cwd=REPO_ROOT)
    if fmt.returncode != 0:
        log("warn: cargo fmt failed (non-fatal)")

    print_summary(module_written=args.module_out)
    return 0


if __name__ == "__main__":
    sys.exit(main())
