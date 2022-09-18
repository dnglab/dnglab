# DNGLab - A camera RAW to DNG file format converter

[![CI](https://github.com/dnglab/dnglab/actions/workflows/ci.yaml/badge.svg)](https://github.com/dnglab/dnglab/actions/workflows/ci.yaml)
[![Matrix](https://img.shields.io/matrix/dnglab:matrix.org?server_fqdn=matrix.org)](https://app.element.io/#/room/#dnglab:matrix.org)

Command line tool to convert camera RAW files to Digital Negative Format (DNG).


 It is currently in alpha state, so don't expect a polished and bugfree application.
 Please report bugs in our [issue tracker](https://github.com/dnglab/dnglab/issues).


## Installation

There are pre-built binary packages for each release which can be downloaded from
the asset section under [latest release](https://github.com/dnglab/dnglab/releases/latest).
The **.deb** packages are for Debian based systems (amd64 and arm64), which can be installed
with `dpkg -i dnglab_x.x.x_amd64.deb`. For non-Debian systems, you can use the single-binary file,
for example `./dnglab_linux_x64 convert IMG_1234.CR2 IMG_1234.dng`.

Windows is not officially supported, but the release assets contains a **dnglab-win-x64_vx.x.x.zip**
file with pre-built Windows binary. Please be aware that this build is untested.

## Build from source
Dnglab is written in Rust, so you can compile it by your own on your target machine.
You need the Rust toolchain installed on your machine, see https://rustup.rs/ for that.
Once the toolchain is installed, you can simply compile Dnglab with:

````
git checkout https://github.com/dnglab/dnglab.git
cd dnglab
cargo build --release
````

The dnglab binary is found at `./target/release/dnglab`.


## Examples

**Convert a single file:**

    dnglab convert IMG_1234.CR3 IMG_1234.DNG

**Convert whole directory:**

    dnglab convert ~/DCIM/100EOS ~/filmrolls/photos-france


## Supported cameras and file formats

For a list of supported cameras please see [SUPPORTED_CAMERAS.md](SUPPORTED_CAMERAS.md).

### Supported raw file formats

|Manufacturer | Format | Supported                         | Remarks                                |
|-------------|--------|-----------------------------------|----------------------------------------|
|ARRI         | ARI    | ✅ Yes                            |                                        |
|Canon        | CR3    | ✅ Yes                            | [CR3_STATE.md](CR3_STATE.md)           |
|Canon        | CR2    | ✅ Yes                            |                                        |
|Canon        | CRW    | ✅ Yes                            |                                        |
|Epson        | ERF    | ✅ Yes                            |                                        |
|Fujifilm     | RAF    | ✅ Yes                            |                                        |
|Hasselblad   | 3FR    | ✅ Yes                            |                                        |
|Kodak        | KDC    | ✅ Yes                            |                                        |
|Kodak        | DCS    | ✅ Yes                            |                                        |
|Kodak        | DCR    | ✅ Yes                            |                                        |
|Leaf         | IIQ    | ✅ Yes                            |                                        |
|Leaf         | MOS    | ✅ Yes                            |                                        |
|Mamiya       | MEF    | ✅ Yes                            |                                        |
|Minolta      | MRW    | ✅ Yes                            |                                        |
|Nikon        | NEF    | ✅ Yes                            |                                        |
|Nikon        | NRW    | ✅ Yes                            |                                        |
|Olympus      | ORF    | ✅ Yes                            |                                        |
|Panasonic/Leica| RW2  | ✅ Yes                            |                                        |
|Pentax/Ricoh | PEF    | ✅ Yes                            |                                        |
|Phase One    | IIQ    | ✅ Yes                            |                                        |
|Samsung      | SRW    | ✅ Yes                            |                                        |
|Sony         | ARW    | ✅ Yes                            |                                        |
|Sony         | SRF    | ✅ Yes                            |                                        |
|Sony         | SR2    | ✅ Yes                            |                                        |

### Supported DNG features

 * DNG lossless compression (LJPEG-92)

## Command line help

### convert subcommand

````
dnglab-convert
Convert raw image(s) into dng format

USAGE:
    dnglab convert [OPTIONS] <INPUT> <OUTPUT>

ARGS:
    <INPUT>     Input file or directory
    <OUTPUT>    Output file or existing directory

OPTIONS:
        --artist <artist>
            Set the artist tag

    -c, --compression <compression>
            Compression for raw image [default: lossless] [possible values: lossless, none]

        --crop <crop>
            DNG default crop [default: best] [possible values: best, activearea, none]

    -d
            turns on debugging mode

        --dng-preview <preview>
            DNG include preview image [default: true]

        --dng-thumbnail <thumbnail>
            DNG include thumbnail image [default: true]

        --embed-raw <embedded>
            Embed the raw file into DNG [default: true]

    -f, --override
            Override existing files

    -h, --help
            Print help information

        --image-index <index>
            Select a specific image index (or 'all') if file is a image container [default: 0]

        --ljpeg92-predictor <predictor>
            LJPEG-92 predictor [default: 1] [possible values: 1, 2, 3, 4, 5, 6, 7]

    -r, --recursive
            Process input directory recursive

    -v
            Print more messages
````

### analyze subcommand

````
dnglab-analyze
Analyze raw image

USAGE:
    dnglab analyze [OPTIONS] <FILE>

ARGS:
    <FILE>    Input file

OPTIONS:
    -d                          turns on debugging mode
        --full-pixel            Write uncompressed full pixel data to STDOUT
    -h, --help                  Print help information
        --json                  Format metadata as JSON
        --meta                  Write metadata to STDOUT
        --preview-checksum      Write MD5 checksum of preview pixels to STDOUT
        --preview-pixel         Write uncompressed preview pixel data to STDOUT
        --raw-checksum          Write MD5 checksum of raw pixels to STDOUT
        --raw-pixel
        --srgb                  Write sRGB 16-bit TIFF to STDOUT
        --structure             Write file structure to STDOUT
        --summary               Write summary information for file to STDOUT
        --thumbnail-checksum    Write MD5 checksum of thumbnail pixels to STDOUT
        --thumbnail-pixel       Write uncompressed preview pixel data to STDOUT
    -v                          Print more messages
        --yaml                  Format metadata as YAML

````

With **analyze**, you can get a full dump of the internal file structure
as YAML or JSON. With JSON output, it's possible to filter and transform
the data with **jq**.
For example, to get the *cfa_layout* from the CMP1 box for CR3 files, you can
write:

````
find /cr3samples/ -type f -name "*.CR3" -exec dnglab analyze --structure '{}' --json \; | \
  jq ". | { file: .file.fileName, cfa_layout: .format.cr3.moov.trak[1].mdia.minf.stbl.stsd.craw.cmp1.cfa_layout}"
````

The output is:

```json
{
  "file": "Canon EOS 90D_CRAW_ISO_250_nocrop_nodual.CR3",
  "cfa_layout": 1
}
{
  "file": "Canon EOS 90D_CRAW_ISO_100_nocrop_nodual.CR3",
  "cfa_layout": 1
}
```

### extract subcommand

````
dnglab-extract
Extract embedded original Raw from DNG

USAGE:
    dnglab extract [OPTIONS] <FILE> <INPUT> <OUTPUT>

ARGS:
    <FILE>      Input file
    <INPUT>     Input file or directory
    <OUTPUT>    Output file or existing directory

OPTIONS:
    -d                  turns on debugging mode
    -f, --override      Override existing files
    -h, --help          Print help information
    -r, --recursive     Process input directory recursive
        --skipchecks    Skip integrity checks
    -v                  Print more messages
````


## FAQ

### Why a DNG tool if there is already something from Adobe?
The DNG converter from Adobe is free (at cost), but not free in terms of free software. Nobody can add or fix camera support except of Adobe. And it has no support for Linux. That's why I've started writing my own little DNG swiss army knife.

### Why should I use DNG instead of RAW?
Never ask. If you need DNG you will know.


### Will camera/format (...) be added?
Well, depends on developer resources.

### Is a GUI in planning?
Yes, DNGLab should get a GUI in near future.

## Credits

Special thanks goes to:

 * Darktable developer team [www.darktable.org](https://www.darktable.org)
 * Laurent Clévy [CR3 documentation](https://github.com/lclevy/canon_cr3)
 * Kostya Shishkov
 * Hubert Kowalski
 * Rawloader development team [rawloader](https://github.com/pedrocr/rawloader)
 * All volunteers who have contributed samples.

Without the support and engagement from these people the development of
dnglab would not have been possible.
