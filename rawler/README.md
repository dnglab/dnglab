# Rawler
Digital camera raw format library

## What is Rawler?
Rawler parses raw file formats and provides access to the pixels
and all metadata.

## Why is it named rawler?
It's a (c)rawler, just for raw file structures.

## Why another raw library?

 * There are libraries dealing with metadata (exiv2) and for pixels (rawspeed, libraw),
   but not for both. Each library ships their own file structure parsers.
 * rawloader, which is basically the fundament of this library, is a pretty good library,
   but lacks of support for metadata.
 * For fun.

## Examples and sample files

The examples in `examples/` use sample files of
[rawdb.dnglab.org](https://rawdb.dnglab.org). The `rawler::rawdb` module
downloads them into a local cache when they are used the first time. It
is part of the `rawdb` feature, which is off by default and on for the
examples, tests and benchmarks of rawler. Other crates enable it with
`features = ["rawdb"]`.

```sh
cargo run -p rawler --release --example <name>
```

* `develop`: develops a Canon EOS 5D Mark III CR2 to a JPEG image.
* `exif`: prints the camera, exposure time, ISO and aperture of the CR2, or
  of a raw file given as argument.
* `makernote`: prints the Canon makernote of the CR2.

The cache is the directory in the environment variable `RAWDB_CACHE`, or
`dnglab/rawdb` in the cache directory of the user: `~/.cache/dnglab/rawdb`
on Linux, `~/Library/Caches/dnglab/rawdb` on macOS and
`%LOCALAPPDATA%\dnglab\rawdb` on Windows. Downloads keep 5% of the file
system free.

## Testing

`cargo test` runs the tests that need no sample files. The `rawdb-tests`
feature adds the tests of all rawdb samples against the snapshots in
`data/testdata/rawdb`. They download about 100 GiB, so `RAWDB_CACHE`
should point to a directory with enough space:

```sh
RAWDB_CACHE=/path/to/rawdb cargo test -p rawler --release --features rawdb-tests
```
