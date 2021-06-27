# DNGLab - A camera RAW to DNG file format converter

[![CI](https://github.com/dnglab/dnglab/actions/workflows/ci.yaml/badge.svg)](https://github.com/dnglab/dnglab/actions/workflows/ci.yaml)
[![Matrix](https://img.shields.io/matrix/dnglab:matrix.org?server_fqdn=matrix.org)](https://app.element.io/#/room/#dnglab:matrix.org)

Command line tool to convert camera RAW files to Digital Negative Format (DNG).


 It is currently in alpha state, so don't expect a polished and bugfree application.
 Please report bugs in our [issue tracker](https://github.com/dnglab/dnglab/issues).

## Examples

**Convert a single file:**

    dnglab convert IMG_1234.CR3 IMG_1234.DNG

**Convert whole directory:**

    dnglab convert ~/DCIM/100EOS ~/filmrolls/photos-france


## Supported cameras and file formats

For a list of supported cameras please see [SUPPORTED_CAMERAS.md](SUPPORTED_CAMERAS.md).

### Supported raw file formats

| Format | Supported                         | Remarks                                |
|--------|-----------------------------------|----------------------------------------|
| CR3    | ✅ Yes<sup>with restrictions</sup> | [CR3_STATE.md](CR3_STATE.md)           |
| CR2    | ❌ No<sup> planned</sup>           |                                        |
| CRW    | ❌ No                              |                                        |


### Supported DNG features

 * DNG lossless compression (LJPEG-92)

## Command line help

### convert subcommand

````
dnglab-convert
Convert raw image(s) into dng format

USAGE:
    dnglab convert [FLAGS] [OPTIONS] <INPUT> <OUTPUT>

FLAGS:
    -d                  Sets the level of debugging information
    -h, --help          Prints help information
        --nocrop        Do not crop black areas, output full sensor data
        --noembedded    Do not embed original raw file
    -f, --override      Override existing files
    -V, --version       Prints version information
        --verbose       Print more messages

OPTIONS:
    -c, --compression <compression>    'lossless' (default) or 'none'

ARGS:
    <INPUT>     Input file or directory
    <OUTPUT>    Output file or existing directory
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

