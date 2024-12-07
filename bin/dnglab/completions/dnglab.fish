complete -c dnglab -n "__fish_use_subcommand" -s d -l loglevel -d 'Log level' -r -f -a "{error	'',warn	'',info	'',debug	'',trace	''}"
complete -c dnglab -n "__fish_use_subcommand" -s v -d 'Print status for every file'
complete -c dnglab -n "__fish_use_subcommand" -s h -l help -d 'Print help'
complete -c dnglab -n "__fish_use_subcommand" -s V -l version -d 'Print version'
complete -c dnglab -n "__fish_use_subcommand" -f -a "analyze" -d 'Analyze raw image'
complete -c dnglab -n "__fish_use_subcommand" -f -a "convert" -d 'Convert raw image(s) into dng format'
complete -c dnglab -n "__fish_use_subcommand" -f -a "ftpserver" -d 'Convert raw image(s) into dng format'
complete -c dnglab -n "__fish_use_subcommand" -f -a "cameras" -d 'List supported cameras'
complete -c dnglab -n "__fish_use_subcommand" -f -a "lenses" -d 'List supported lenses'
complete -c dnglab -n "__fish_use_subcommand" -f -a "makedng" -d 'Lowlevel command to make a DNG file'
complete -c dnglab -n "__fish_use_subcommand" -f -a "gui" -d 'Start GUI (not implemented)'
complete -c dnglab -n "__fish_use_subcommand" -f -a "extract" -d 'Extract embedded original Raw from DNG'
complete -c dnglab -n "__fish_use_subcommand" -f -a "help" -d 'Print this message or the help of the given subcommand(s)'
complete -c dnglab -n "__fish_seen_subcommand_from analyze" -s d -l loglevel -d 'Log level' -r -f -a "{error	'',warn	'',info	'',debug	'',trace	''}"
complete -c dnglab -n "__fish_seen_subcommand_from analyze" -l raw-pixel
complete -c dnglab -n "__fish_seen_subcommand_from analyze" -l full-pixel -d 'Write uncompressed full pixel data to STDOUT'
complete -c dnglab -n "__fish_seen_subcommand_from analyze" -l preview-pixel -d 'Write uncompressed preview pixel data to STDOUT'
complete -c dnglab -n "__fish_seen_subcommand_from analyze" -l thumbnail-pixel -d 'Write uncompressed preview pixel data to STDOUT'
complete -c dnglab -n "__fish_seen_subcommand_from analyze" -l raw-checksum -d 'Write MD5 checksum of raw pixels to STDOUT'
complete -c dnglab -n "__fish_seen_subcommand_from analyze" -l preview-checksum -d 'Write MD5 checksum of preview pixels to STDOUT'
complete -c dnglab -n "__fish_seen_subcommand_from analyze" -l thumbnail-checksum -d 'Write MD5 checksum of thumbnail pixels to STDOUT'
complete -c dnglab -n "__fish_seen_subcommand_from analyze" -l srgb -d 'Write sRGB 16-bit TIFF to STDOUT'
complete -c dnglab -n "__fish_seen_subcommand_from analyze" -l meta -d 'Write metadata to STDOUT'
complete -c dnglab -n "__fish_seen_subcommand_from analyze" -l structure -d 'Write file structure to STDOUT'
complete -c dnglab -n "__fish_seen_subcommand_from analyze" -l summary -d 'Write summary information for file to STDOUT'
complete -c dnglab -n "__fish_seen_subcommand_from analyze" -l json -d 'Format metadata as JSON'
complete -c dnglab -n "__fish_seen_subcommand_from analyze" -l yaml -d 'Format metadata as YAML'
complete -c dnglab -n "__fish_seen_subcommand_from analyze" -s v -d 'Print status for every file'
complete -c dnglab -n "__fish_seen_subcommand_from analyze" -s h -l help -d 'Print help'
complete -c dnglab -n "__fish_seen_subcommand_from convert" -s c -l compression -d 'Compression for raw image' -r -f -a "{lossless	'',uncompressed	''}"
complete -c dnglab -n "__fish_seen_subcommand_from convert" -l ljpeg92-predictor -d 'LJPEG-92 predictor' -r
complete -c dnglab -n "__fish_seen_subcommand_from convert" -l dng-preview -d 'DNG include preview image' -r -f -a "{true	'',false	''}"
complete -c dnglab -n "__fish_seen_subcommand_from convert" -l dng-thumbnail -d 'DNG include thumbnail image' -r -f -a "{true	'',false	''}"
complete -c dnglab -n "__fish_seen_subcommand_from convert" -l embed-raw -d 'Embed the raw file into DNG' -r -f -a "{true	'',false	''}"
complete -c dnglab -n "__fish_seen_subcommand_from convert" -l artist -d 'Set the artist tag' -r
complete -c dnglab -n "__fish_seen_subcommand_from convert" -l image-index -d 'Select a specific image index (or \'all\') if file is a image container' -r
complete -c dnglab -n "__fish_seen_subcommand_from convert" -l crop -d 'DNG default crop' -r -f -a "{best	'',activearea	'',none	''}"
complete -c dnglab -n "__fish_seen_subcommand_from convert" -s d -l loglevel -d 'Log level' -r -f -a "{error	'',warn	'',info	'',debug	'',trace	''}"
complete -c dnglab -n "__fish_seen_subcommand_from convert" -s f -l override -d 'Override existing files'
complete -c dnglab -n "__fish_seen_subcommand_from convert" -s r -l recursive -d 'Process input directory recursive'
complete -c dnglab -n "__fish_seen_subcommand_from convert" -s v -d 'Print status for every file'
complete -c dnglab -n "__fish_seen_subcommand_from convert" -s h -l help -d 'Print help'
complete -c dnglab -n "__fish_seen_subcommand_from ftpserver" -s c -l compression -d 'Compression for raw image' -r -f -a "{lossless	'',uncompressed	''}"
complete -c dnglab -n "__fish_seen_subcommand_from ftpserver" -l ljpeg92-predictor -d 'LJPEG-92 predictor' -r
complete -c dnglab -n "__fish_seen_subcommand_from ftpserver" -l dng-preview -d 'DNG include preview image' -r -f -a "{true	'',false	''}"
complete -c dnglab -n "__fish_seen_subcommand_from ftpserver" -l dng-thumbnail -d 'DNG include thumbnail image' -r -f -a "{true	'',false	''}"
complete -c dnglab -n "__fish_seen_subcommand_from ftpserver" -l embed-raw -d 'Embed the raw file into DNG' -r -f -a "{true	'',false	''}"
complete -c dnglab -n "__fish_seen_subcommand_from ftpserver" -l artist -d 'Set the artist tag' -r
complete -c dnglab -n "__fish_seen_subcommand_from ftpserver" -l image-index -d 'Select a specific image index (or \'all\') if file is a image container' -r
complete -c dnglab -n "__fish_seen_subcommand_from ftpserver" -l crop -d 'DNG default crop' -r -f -a "{best	'',activearea	'',none	''}"
complete -c dnglab -n "__fish_seen_subcommand_from ftpserver" -l port -d 'FTP listen port' -r
complete -c dnglab -n "__fish_seen_subcommand_from ftpserver" -l listen -d 'FTP listen address' -r
complete -c dnglab -n "__fish_seen_subcommand_from ftpserver" -l keep-original -d 'Keep original raw' -r -f -a "{true	'',false	''}"
complete -c dnglab -n "__fish_seen_subcommand_from ftpserver" -s d -l loglevel -d 'Log level' -r -f -a "{error	'',warn	'',info	'',debug	'',trace	''}"
complete -c dnglab -n "__fish_seen_subcommand_from ftpserver" -s f -l override -d 'Override existing files'
complete -c dnglab -n "__fish_seen_subcommand_from ftpserver" -s v -d 'Print status for every file'
complete -c dnglab -n "__fish_seen_subcommand_from ftpserver" -s h -l help -d 'Print help'
complete -c dnglab -n "__fish_seen_subcommand_from cameras" -s d -l loglevel -d 'Log level' -r -f -a "{error	'',warn	'',info	'',debug	'',trace	''}"
complete -c dnglab -n "__fish_seen_subcommand_from cameras" -l md -d 'Markdown format output'
complete -c dnglab -n "__fish_seen_subcommand_from cameras" -s v -d 'Print status for every file'
complete -c dnglab -n "__fish_seen_subcommand_from cameras" -s h -l help -d 'Print help'
complete -c dnglab -n "__fish_seen_subcommand_from lenses" -s d -l loglevel -d 'Log level' -r -f -a "{error	'',warn	'',info	'',debug	'',trace	''}"
complete -c dnglab -n "__fish_seen_subcommand_from lenses" -l md -d 'Markdown format output'
complete -c dnglab -n "__fish_seen_subcommand_from lenses" -s v -d 'Print status for every file'
complete -c dnglab -n "__fish_seen_subcommand_from lenses" -s h -l help -d 'Print help'
complete -c dnglab -n "__fish_seen_subcommand_from makedng" -s o -l output -d 'Output DNG file path' -r -F
complete -c dnglab -n "__fish_seen_subcommand_from makedng" -s i -l input -d 'Input files (raw, preview, exif, ...), index for map starts with 0' -r -F
complete -c dnglab -n "__fish_seen_subcommand_from makedng" -l map -d 'Input usage map' -r
complete -c dnglab -n "__fish_seen_subcommand_from makedng" -l dng-backward-version -d 'DNG specification version' -r -f -a "{1.0	'',1.1	'',1.2	'',1.3	'',1.4	'',1.5	'',1.6	''}"
complete -c dnglab -n "__fish_seen_subcommand_from makedng" -l colorimetric-reference -d 'Reference for XYZ values' -r -f -a "{scene	'',output	''}"
complete -c dnglab -n "__fish_seen_subcommand_from makedng" -l unique-camera-model -d 'Unique camera model' -r
complete -c dnglab -n "__fish_seen_subcommand_from makedng" -l artist -d 'Set the Artist tag' -r
complete -c dnglab -n "__fish_seen_subcommand_from makedng" -l make -d 'Set the Make tag' -r
complete -c dnglab -n "__fish_seen_subcommand_from makedng" -l model -d 'Set the Model tag' -r
complete -c dnglab -n "__fish_seen_subcommand_from makedng" -l matrix1 -d 'Matrix 1' -r -f -a "{XYZ_sRGB_D50	'',XYZ_sRGB_D65	'',XYZ_AdobeRGB_D50	'',XYZ_AdobeRGB_D65	'',custom 3x3 matrix (comma seperated)	''}"
complete -c dnglab -n "__fish_seen_subcommand_from makedng" -l matrix2 -d 'Matrix 2' -r -f -a "{XYZ_sRGB_D50	'',XYZ_sRGB_D65	'',XYZ_AdobeRGB_D50	'',XYZ_AdobeRGB_D65	'',custom 3x3 matrix (comma seperated)	''}"
complete -c dnglab -n "__fish_seen_subcommand_from makedng" -l matrix3 -d 'Matrix 3' -r -f -a "{XYZ_sRGB_D50	'',XYZ_sRGB_D65	'',XYZ_AdobeRGB_D50	'',XYZ_AdobeRGB_D65	'',custom 3x3 matrix (comma seperated)	''}"
complete -c dnglab -n "__fish_seen_subcommand_from makedng" -l illuminant1 -d 'Illuminant 1' -r -f -a "{Unknown	'',A	'',B	'',C	'',D50	'',D55	'',D65	'',D75	''}"
complete -c dnglab -n "__fish_seen_subcommand_from makedng" -l illuminant2 -d 'Illuminant 2' -r -f -a "{Unknown	'',A	'',B	'',C	'',D50	'',D55	'',D65	'',D75	''}"
complete -c dnglab -n "__fish_seen_subcommand_from makedng" -l illuminant3 -d 'Illuminant 3' -r -f -a "{Unknown	'',A	'',B	'',C	'',D50	'',D55	'',D65	'',D75	''}"
complete -c dnglab -n "__fish_seen_subcommand_from makedng" -l linearization -d 'Linearization table' -r -f -a "{8bit_sRGB	'',8bit_sRGB_invert	'',16bit_sRGB	'',16bit_sRGB_invert	'',8bit_gamma1.8	'',8bit_gamma1.8_invert	'',8bit_gamma2.0	'',8bit_gamma2.0_invert	'',8bit_gamma2.2	'',8bit_gamma2.2_invert	'',8bit_gamma2.4	'',8bit_gamma2.4_invert	'',16bit_gamma1.8	'',16bit_gamma1.8_invert	'',16bit_gamma2.0	'',16bit_gamma2.0_invert	'',16bit_gamma2.2	'',16bit_gamma2.2_invert	'',16bit_gamma2.4	'',16bit_gamma2.4_invert	'',custom table (comma seperated)	''}"
complete -c dnglab -n "__fish_seen_subcommand_from makedng" -l wb -d 'Whitebalance as-shot' -r
complete -c dnglab -n "__fish_seen_subcommand_from makedng" -l white-xy -d 'Whitebalance as-shot encoded as xy chromaticity coordinates' -r -f -a "{D50	'',D65	'',custom x\,y value (comma seperated)	''}"
complete -c dnglab -n "__fish_seen_subcommand_from makedng" -s d -l loglevel -d 'Log level' -r -f -a "{error	'',warn	'',info	'',debug	'',trace	''}"
complete -c dnglab -n "__fish_seen_subcommand_from makedng" -s f -l override -d 'Override existing files'
complete -c dnglab -n "__fish_seen_subcommand_from makedng" -s v -d 'Print status for every file'
complete -c dnglab -n "__fish_seen_subcommand_from makedng" -s h -l help -d 'Print help (see more with \'--help\')'
complete -c dnglab -n "__fish_seen_subcommand_from gui" -s d -l loglevel -d 'Log level' -r -f -a "{error	'',warn	'',info	'',debug	'',trace	''}"
complete -c dnglab -n "__fish_seen_subcommand_from gui" -s v -d 'Print status for every file'
complete -c dnglab -n "__fish_seen_subcommand_from gui" -s h -l help -d 'Print help'
complete -c dnglab -n "__fish_seen_subcommand_from extract" -s d -l loglevel -d 'Log level' -r -f -a "{error	'',warn	'',info	'',debug	'',trace	''}"
complete -c dnglab -n "__fish_seen_subcommand_from extract" -l skipchecks -d 'Skip integrity checks'
complete -c dnglab -n "__fish_seen_subcommand_from extract" -s r -l recursive -d 'Process input directory recursive'
complete -c dnglab -n "__fish_seen_subcommand_from extract" -s f -l override -d 'Override existing files'
complete -c dnglab -n "__fish_seen_subcommand_from extract" -s v -d 'Print status for every file'
complete -c dnglab -n "__fish_seen_subcommand_from extract" -s h -l help -d 'Print help'
complete -c dnglab -n "__fish_seen_subcommand_from help; and not __fish_seen_subcommand_from analyze; and not __fish_seen_subcommand_from convert; and not __fish_seen_subcommand_from ftpserver; and not __fish_seen_subcommand_from cameras; and not __fish_seen_subcommand_from lenses; and not __fish_seen_subcommand_from makedng; and not __fish_seen_subcommand_from gui; and not __fish_seen_subcommand_from extract; and not __fish_seen_subcommand_from help" -f -a "analyze" -d 'Analyze raw image'
complete -c dnglab -n "__fish_seen_subcommand_from help; and not __fish_seen_subcommand_from analyze; and not __fish_seen_subcommand_from convert; and not __fish_seen_subcommand_from ftpserver; and not __fish_seen_subcommand_from cameras; and not __fish_seen_subcommand_from lenses; and not __fish_seen_subcommand_from makedng; and not __fish_seen_subcommand_from gui; and not __fish_seen_subcommand_from extract; and not __fish_seen_subcommand_from help" -f -a "convert" -d 'Convert raw image(s) into dng format'
complete -c dnglab -n "__fish_seen_subcommand_from help; and not __fish_seen_subcommand_from analyze; and not __fish_seen_subcommand_from convert; and not __fish_seen_subcommand_from ftpserver; and not __fish_seen_subcommand_from cameras; and not __fish_seen_subcommand_from lenses; and not __fish_seen_subcommand_from makedng; and not __fish_seen_subcommand_from gui; and not __fish_seen_subcommand_from extract; and not __fish_seen_subcommand_from help" -f -a "ftpserver" -d 'Convert raw image(s) into dng format'
complete -c dnglab -n "__fish_seen_subcommand_from help; and not __fish_seen_subcommand_from analyze; and not __fish_seen_subcommand_from convert; and not __fish_seen_subcommand_from ftpserver; and not __fish_seen_subcommand_from cameras; and not __fish_seen_subcommand_from lenses; and not __fish_seen_subcommand_from makedng; and not __fish_seen_subcommand_from gui; and not __fish_seen_subcommand_from extract; and not __fish_seen_subcommand_from help" -f -a "cameras" -d 'List supported cameras'
complete -c dnglab -n "__fish_seen_subcommand_from help; and not __fish_seen_subcommand_from analyze; and not __fish_seen_subcommand_from convert; and not __fish_seen_subcommand_from ftpserver; and not __fish_seen_subcommand_from cameras; and not __fish_seen_subcommand_from lenses; and not __fish_seen_subcommand_from makedng; and not __fish_seen_subcommand_from gui; and not __fish_seen_subcommand_from extract; and not __fish_seen_subcommand_from help" -f -a "lenses" -d 'List supported lenses'
complete -c dnglab -n "__fish_seen_subcommand_from help; and not __fish_seen_subcommand_from analyze; and not __fish_seen_subcommand_from convert; and not __fish_seen_subcommand_from ftpserver; and not __fish_seen_subcommand_from cameras; and not __fish_seen_subcommand_from lenses; and not __fish_seen_subcommand_from makedng; and not __fish_seen_subcommand_from gui; and not __fish_seen_subcommand_from extract; and not __fish_seen_subcommand_from help" -f -a "makedng" -d 'Lowlevel command to make a DNG file'
complete -c dnglab -n "__fish_seen_subcommand_from help; and not __fish_seen_subcommand_from analyze; and not __fish_seen_subcommand_from convert; and not __fish_seen_subcommand_from ftpserver; and not __fish_seen_subcommand_from cameras; and not __fish_seen_subcommand_from lenses; and not __fish_seen_subcommand_from makedng; and not __fish_seen_subcommand_from gui; and not __fish_seen_subcommand_from extract; and not __fish_seen_subcommand_from help" -f -a "gui" -d 'Start GUI (not implemented)'
complete -c dnglab -n "__fish_seen_subcommand_from help; and not __fish_seen_subcommand_from analyze; and not __fish_seen_subcommand_from convert; and not __fish_seen_subcommand_from ftpserver; and not __fish_seen_subcommand_from cameras; and not __fish_seen_subcommand_from lenses; and not __fish_seen_subcommand_from makedng; and not __fish_seen_subcommand_from gui; and not __fish_seen_subcommand_from extract; and not __fish_seen_subcommand_from help" -f -a "extract" -d 'Extract embedded original Raw from DNG'
complete -c dnglab -n "__fish_seen_subcommand_from help; and not __fish_seen_subcommand_from analyze; and not __fish_seen_subcommand_from convert; and not __fish_seen_subcommand_from ftpserver; and not __fish_seen_subcommand_from cameras; and not __fish_seen_subcommand_from lenses; and not __fish_seen_subcommand_from makedng; and not __fish_seen_subcommand_from gui; and not __fish_seen_subcommand_from extract; and not __fish_seen_subcommand_from help" -f -a "help" -d 'Print this message or the help of the given subcommand(s)'
