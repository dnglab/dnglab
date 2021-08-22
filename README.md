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
    -d                 Sets the level of debugging information
    -h, --help         Prints help information
    -f, --override     Override existing files
    -r, --recursive    Process input directory recursive
    -V, --version      Prints version information
    -v, --verbose      Print more messages

OPTIONS:
        --artist <artist>                  Set the artist tag
    -c, --compression <compression>        'lossless' or 'none' [default: lossless]
        --crop <crop>                      Apply crop to ActiveArea [default: yes]
        --dng-embedded <embedded>          Embed the raw file into DNG [default: yes]
        --ljpeg92-predictor <predictor>    LJPEG-92 predictor (1-7)
        --dng-preview <preview>            Include a DNG preview image [default: yes]
        --dng-thumbnail <thumbnail>        Include a DNG thumbnail image [default: yes]

ARGS:
    <INPUT>     Input file or directory
    <OUTPUT>    Output file or existing directory
````

### analyze subcommand

````
dnglab-analyze
Analyze raw image

USAGE:
    dnglab analyze [FLAGS] <FILE>

FLAGS:
    -d               Sets the level of debugging information
    -h, --help       Prints help information
        --json       Format metadata as JSON
        --meta       Write metadata to STDOUT
        --pixel      Write uncompressed pixel data to STDOUT
        --summary    Write summary information for file to STDOUT
    -V, --version    Prints version information
    -v, --verbose    Print more messages
        --yaml       Format metadata as YAML

ARGS:
    <FILE>    Input file
````

With **analyze**, you can get a full dump of the internal file structure
as YAML or JSON. With JSON output, it's possible to filter and transform
the data with **jq**.
For example, to get the *cfa_layout* from the CMP1 box for CR3 files, you can
write:

````
find /cr3samples/ -type f -name "*.CR3" -exec dnglab analyze --meta '{}' --json \; | \
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
    dnglab extract [FLAGS] <INPUT> <OUTPUT>

FLAGS:
    -d                  Sets the level of debugging information
    -h, --help          Prints help information
    -f, --override      Override existing files
    -r, --recursive     Process input directory recursive
        --skipchecks    Skip integrity checks
    -V, --version       Prints version information
    -v, --verbose       Print more messages

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
