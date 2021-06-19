# DNGLab - A camera RAW to DNG file format converter

Command line tool to convert camera RAW files to Digital Negative Format (DNG).

## Examples

**Convert a single file:**

    dnglab convert IMG_1234.CR3 IMG_1234.DNG

**Convert whole directory:**

    dnglab convert ~/DCIM/100EOS ~/filmrolls/photos-france


## Supported cameras and file formats

For a list of supported cameras please see [SUPPORTED_CAMERAS.md].

### Supported raw file formats

| Format | Supported                         | Remarks                                |
|--------|-----------------------------------|----------------------------------------|
| CR3    | ✅ Yes<sup>with restrictions</sup> | CRAW (compressed RAW) is not supported |
| CR2    | ❌ No<sup> planned</sup>           |                                        |
| CRW    | ❌ No                              |                                        |

### Supported DNG features

 * DNG lossless compression (LJPEG-92)

## FAQ

### Why a DNG tool if there is already something from Adobe?
The DNG converter from Adobe is free (at cost), but not free in terms of free software. Nobody can add or fix camera support except of Adobe. And it has no support for Linux. That's why I've started writing my own little DNG swiss army knife.

### Why should I use DNG instead of RAW?
Never ask. If you need DNG you will know.


### Will camera/format (...) be added?
Well, depends on developer resources.

### Is a GUI in planning?
Yes, DNGLab should get a GUI in near future.

