#!/usr/bin/env python3
"""
Select the rawdb tests that a pull request affects.

The rawdb tests check every rawdb sample, which takes about two hours. A pull
request that changes only camera definitions and test data needs only the
tests of the samples it touches. This script tells the CI workflow which ones,
as lines for $GITHUB_OUTPUT:

  mode=full       run all tests: the pull request changes other files, or the
                  selection is not certain
  mode=selected   run only the tests in `tests`
  tests=...       exact test names, separated by spaces, possibly none

With $GITHUB_STEP_SUMMARY set, it appends the decision and its reason.

The tests of a pull request that changes only test data are the union of the
tests that are new or changed in rawler/tests/rawdb/mod.rs, the tests whose
snapshot files are added or changed, and the tests whose snapshots report the
clean make and model of a changed camera definition.

Usage: select_rawdb_tests.py --base <commit> [--head <commit>]
"""

from __future__ import annotations

import argparse
import os
import re
import subprocess
import sys
from collections import Counter

REL_MODRS = "rawler/tests/rawdb/mod.rs"
REL_SETS = "rawler/tests/supported_rawdb_sets.txt"
REL_TESTDATA = "rawler/data/testdata/rawdb/"
REL_CAMERAS = "rawler/data/cameras/"
SNAPSHOT_SUFFIXES = (".analyze.yaml", ".digest.txt", ".digest.full.txt", ".digest.preview.txt", ".digest.thumbnail.txt")
TEST_NAME = re.compile(r"^rawdb::\w+::\w+::\w+$")
# The tokens of the generated mod.rs: module starts, test macros and closing braces
MODRS_TOKEN = re.compile(
    r'mod\s+(?P<mod>\w+)\s*\{'
    r'|rawdb_test_file!\(\s*"(?P<maker>[^"]*)"\s*,\s*"(?P<model>[^"]*)"\s*,\s*(?P<ident>\w+)\s*,\s*"(?P<rel>[^"]*)"\s*,?\s*\)'
    r'|(?P<close>\})'
)
# The clean make and model in an .analyze.yaml, below data.metadata
ANALYZE_NAME = re.compile(r"^      (make|model): (.*)$", re.MULTILINE)


class Full(Exception):
    """The whole suite must run, for the reason in the message."""


def git(*args: str) -> str:
    return subprocess.run(["git", *args], check=True, capture_output=True, text=True).stdout


def read_blobs(rev: str, paths: list[str]) -> dict[str, str | None]:
    """The content of `paths` at `rev`, None for a path that doesn't exist."""
    if not paths:
        return {}
    res = subprocess.run(
        ["git", "cat-file", "--batch"],
        input="".join(f"{rev}:{path}\n" for path in paths).encode(),
        check=True,
        capture_output=True,
    )
    out, pos, blobs = res.stdout, 0, {}
    for path in paths:
        end = out.index(b"\n", pos)
        header = out[pos:end].split()
        pos = end + 1
        if header[-1] == b"missing":
            blobs[path] = None
            continue
        size = int(header[2])
        blobs[path] = out[pos : pos + size].decode("utf-8", errors="replace")
        pos += size + 1
    return blobs


def changed_files(merge_base: str, head: str) -> list[tuple[str, str]]:
    """(status, path) of the files the pull request changes."""
    fields = git("diff", "--name-status", "--no-renames", "-z", merge_base, head).split("\0")
    return [(fields[i], fields[i + 1]) for i in range(0, len(fields) - 1, 2)]


def parse_tests(modrs: str | None) -> dict[tuple[str, str, str], str]:
    """(maker, model, sample path) of each test in mod.rs, with its full name."""
    tests: dict[tuple[str, str, str], str] = {}
    modules: list[str] = []
    for token in MODRS_TOKEN.finditer(modrs or ""):
        if token["mod"]:
            modules.append(token["mod"])
        elif token["close"]:
            if modules:
                modules.pop()
        else:
            if len(modules) != 2:
                raise Full(f"{REL_MODRS} has a test outside of a maker and a set module: {token['ident']}")
            name = f"rawdb::{modules[0]}::{modules[1]}::{token['ident']}"
            tests[(token["maker"], token["model"], token["rel"])] = name
    return tests


def toml_names(text: str) -> set[tuple[str, str]]:
    """The clean names of a camera definition: its own and those of its aliases."""
    import tomllib

    data = tomllib.loads(text)
    make = data.get("clean_make")
    names = set()
    if make and data.get("clean_model"):
        names.add((make, data["clean_model"]))
    for alias in data.get("model_aliases", []):
        if make and len(alias) >= 2:
            names.add((make, alias[1]))
    return names


def duplicate_keys(rev: str) -> set[tuple[str, str, str]]:
    """Lookup keys (make, model, mode) that more than one camera definition uses."""
    import tomllib

    paths = [p for p in git("ls-tree", "-r", "--name-only", rev, REL_CAMERAS).splitlines() if p.endswith(".toml")]
    keys: Counter[tuple[str, str, str]] = Counter()
    for path, text in read_blobs(rev, paths).items():
        try:
            data = tomllib.loads(text or "")
        except tomllib.TOMLDecodeError as e:
            raise Full(f"{path} at {rev[:9]} is no valid TOML: {e}") from e
        models = [data.get("model", "")] + [alias[0] for alias in data.get("model_aliases", []) if alias]
        modes = [""] + [mode.get("mode", "") for mode in data.get("cameras", {}).get("modes", [])]
        keys.update((data.get("make", ""), model, mode) for model in models for mode in modes)
    return {key for key, count in keys.items() if count > 1}


def analyze_names(head: str, tests: dict[tuple[str, str, str], str]) -> dict[tuple[str, str, str], tuple[str, str]]:
    """The clean make and model that the .analyze.yaml snapshot of each test reports."""
    paths = {key: f"{REL_TESTDATA}{key[0]}/{key[1]}/{key[2]}.analyze.yaml" for key in tests}
    blobs = read_blobs(head, list(paths.values()))
    names = {}
    for key, path in paths.items():
        found = dict(ANALYZE_NAME.findall(blobs[path] or ""))
        if "make" in found and "model" in found:
            names[key] = tuple(value.strip().strip("'\"") for value in (found["make"], found["model"]))
    return names


def select(base: str, head: str) -> tuple[list[str], list[str]]:
    """The selected tests and notes for the summary. Raises Full when all tests must run."""
    merge_base = git("merge-base", base, head).strip()
    changes = changed_files(merge_base, head)
    tomls, snapshots, others = [], [], []
    for status, path in changes:
        if path.startswith(REL_CAMERAS) and path.endswith(".toml"):
            tomls.append(path)
        elif path.startswith(REL_TESTDATA):
            snapshots.append((status, path))
        elif path in (REL_MODRS, REL_SETS) or path.endswith(".md"):
            pass
        else:
            others.append(path)
    if others:
        listed = ", ".join(f"`{p}`" for p in others[:5]) + (f" and {len(others) - 5} more" if len(others) > 5 else "")
        raise Full(f"the pull request changes files outside the test data: {listed}")

    head_modrs, base_modrs = read_blobs(head, [REL_MODRS])[REL_MODRS], read_blobs(merge_base, [REL_MODRS])[REL_MODRS]
    head_tests, base_tests = parse_tests(head_modrs), parse_tests(base_modrs)
    selected: set[str] = set()
    notes: list[str] = []

    # Tests that are new or changed in mod.rs
    selected.update(name for key, name in head_tests.items() if base_tests.get(key) != name)

    # Tests whose snapshot files are added or changed
    for status, path in snapshots:
        if status.startswith("D"):
            continue
        parts = path[len(REL_TESTDATA) :].split("/", 2)
        if len(parts) < 3:
            continue
        maker, model, file = parts
        suffix = next((s for s in SNAPSHOT_SUFFIXES if file.endswith(s)), None)
        if suffix and (maker, model, file[: -len(suffix)]) in head_tests:
            selected.add(head_tests[(maker, model, file[: -len(suffix)])])

    # Tests of the cameras whose definitions changed
    if tomls:
        new_duplicates = duplicate_keys(head) - duplicate_keys(merge_base)
        if new_duplicates:
            raise Full(f"a camera definition adds lookup keys that another one uses: {sorted(new_duplicates)[:5]}")
        names: set[tuple[str, str]] = set()
        for rev in (head, merge_base):
            for path, text in read_blobs(rev, tomls).items():
                if text is not None:
                    names |= toml_names(text)
        by_camera = {head_tests[key] for key, name in analyze_names(head, head_tests).items() if name in names}
        selected |= by_camera
        if not by_camera:
            notes.append(f"No snapshot reports the cameras of the changed definitions: {', '.join(f'{m} {n}' for m, n in sorted(names))}.")

    invalid = [name for name in selected if not TEST_NAME.match(name)]
    if invalid:
        raise Full(f"unexpected test names: {invalid[:5]}")
    return sorted(selected), notes


def main() -> int:
    p = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    p.add_argument("--base", required=True, help="Commit of the base branch")
    p.add_argument("--head", default="HEAD", help="Commit of the pull request")
    args = p.parse_args()

    try:
        tests, notes = select(args.base, args.head)
        mode = "selected"
        reason = "The pull request changes only test data."
    except Full as e:
        tests, notes, mode, reason = [], [], "full", f"All tests run: {e}."
    except Exception as e:  # noqa: BLE001 - any failure falls back to the whole suite
        tests, notes, mode, reason = [], [], "full", f"All tests run, the selection failed: {e!r}."

    print(f"mode={mode}")
    print(f"tests={' '.join(tests)}")
    summary = os.environ.get("GITHUB_STEP_SUMMARY")
    if summary:
        with open(summary, "a") as out:
            out.write(f"### rawdb tests: {mode}\n\n{reason}\n\n")
            for note in notes:
                out.write(f"{note}\n\n")
            if mode == "selected":
                out.write(f"{len(tests)} selected tests:\n\n" + "".join(f"- `{t}`\n" for t in tests) + "\n")
    print(f"{mode}: {reason} {len(tests)} tests", file=sys.stderr)
    return 0


if __name__ == "__main__":
    sys.exit(main())
