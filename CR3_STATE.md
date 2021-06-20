# State of CR3 support

| Make                 | State                                    | Remarks                                                                     |
|----------------------|------------------------------------------|-----------------------------------------------------------------------------|
| BMFF                 | ✅ Yes                                   |                                                                             |
| EXIF                 | ✅ Yes                                   |                                                                             |
| GPS                  | ✅ Yes                                   |                                                                             |
| Makernotes           | ✅ Yes <sup> only required fields </sup> |                                                                             |
| RAW decoding         | ✅ Yes                                   |                                                                             |
| CRAW decoding        | ❌ No                                    | Unknown compression method                                                  |
| Dual Pixel           | ✅ Yes <sup> with restrictions </sup>    | Dual Pixel files can be decoded, but no Dual Pixel corrections are possible |
| Thumbnail extraction | ❌ No<sup>not required</sup>             |                                                                             |
| Preview extraction   | ✅ Yes<sup>used as thumbnail</sup>       |                                                                             |
| HDR PQ / HEIF        | ❌ No                                    | HDR PQ has no effect for RAW data, but makes CR3 more difficult to handle.  |
