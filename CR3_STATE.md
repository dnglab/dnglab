# State of CR3 support

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
| HDR PQ / HEIF        | ✅ Yes                                    | For HDR-PQ, DNG thumbnail and preview image is generated from RAW  |
| CR3 Filmroll | ❌ No            | Filmrolls using encoding type 3                                                                            |
