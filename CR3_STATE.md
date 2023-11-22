# State of CR3 support

CR3 is fully supported.


| Make                 | State                                    | Remarks                                                                     |
|----------------------|------------------------------------------|-----------------------------------------------------------------------------|
| BMFF                 | ✅ Yes                                   |                                                                             |
| EXIF                 | ✅ Yes                                   |                                                                             |
| GPS                  | ✅ Yes                                   |                                                                             |
| Makernotes           | ✅ Yes <sup> only required fields </sup> |                                                                             |
| RAW decoding         | ✅ Yes                                   |                                                                             |
| CRAW decoding        | ✅ Yes                                   | Unknown compression method                                                  |
| Dual Pixel           | ✅ Yes <sup> with restrictions </sup>    | Dual Pixel files can be decoded, but no Dual Pixel corrections are possible |
| Thumb/Preview extraction   | ✅ Yes       | Thumbnail is generated from preview                                                               |
| HDR PQ / HEIF        | ✅ Yes                                   | For HDR-PQ, DNG thumbnail and preview image is generated from RAW           |
| CR3 Filmroll         | ✅ Yes                                   | Filmrolls using encoding type 3                                             |


## Roll files

Some models can write roll files, including up to 70 images in a single CR3 file. dnglab is able to extract all images from a roll.
A single specific image can be extracted with:

    dnglab convert --image-index <id> CSI_2839.CR3 /tmp/img1.dng

All images can be extracted by:

    dnglab convert --image-index all CSI_2839.CR3 /tmp/roll.dng

This created roll_0000.dng, roll_0001.dng and so on. If a CR3 file contains only a single image, no number suffix is applied so
you can always use **--image-index all**.
