# Print an optspec for argparse to handle cmd's options that are independent of any subcommand.
function __fish_dnglab_global_optspecs
	string join \n d/loglevel= v h/help V/version
end

function __fish_dnglab_needs_command
	# Figure out if the current invocation already has a command.
	set -l cmd (commandline -opc)
	set -e cmd[1]
	argparse -s (__fish_dnglab_global_optspecs) -- $cmd 2>/dev/null
	or return
	if set -q argv[1]
		# Also print the command, so this can be used to figure out what it is.
		echo $argv[1]
		return 1
	end
	return 0
end

function __fish_dnglab_using_subcommand
	set -l cmd (__fish_dnglab_needs_command)
	test -z "$cmd"
	and return 1
	contains -- $cmd[1] $argv
end

complete -c dnglab -n "__fish_dnglab_needs_command" -s d -l loglevel -d 'Log level' -r -f -a "error\t''
warn\t''
info\t''
debug\t''
trace\t''"
complete -c dnglab -n "__fish_dnglab_needs_command" -s v -d 'Print status for every file'
complete -c dnglab -n "__fish_dnglab_needs_command" -s h -l help -d 'Print help'
complete -c dnglab -n "__fish_dnglab_needs_command" -s V -l version -d 'Print version'
complete -c dnglab -n "__fish_dnglab_needs_command" -f -a "analyze" -d 'Analyze raw image'
complete -c dnglab -n "__fish_dnglab_needs_command" -f -a "convert" -d 'Convert raw image(s) into dng format'
complete -c dnglab -n "__fish_dnglab_needs_command" -f -a "ftpserver" -d 'Convert raw image(s) into dng format'
complete -c dnglab -n "__fish_dnglab_needs_command" -f -a "cameras" -d 'List supported cameras'
complete -c dnglab -n "__fish_dnglab_needs_command" -f -a "lenses" -d 'List supported lenses'
complete -c dnglab -n "__fish_dnglab_needs_command" -f -a "makedng" -d 'Lowlevel command to make a DNG file'
complete -c dnglab -n "__fish_dnglab_needs_command" -f -a "gui" -d 'Start GUI (not implemented)'
complete -c dnglab -n "__fish_dnglab_needs_command" -f -a "extract" -d 'Extract embedded original Raw from DNG'
complete -c dnglab -n "__fish_dnglab_needs_command" -f -a "help" -d 'Print this message or the help of the given subcommand(s)'
complete -c dnglab -n "__fish_dnglab_using_subcommand analyze" -s d -l loglevel -d 'Log level' -r -f -a "error\t''
warn\t''
info\t''
debug\t''
trace\t''"
complete -c dnglab -n "__fish_dnglab_using_subcommand analyze" -l raw-pixel
complete -c dnglab -n "__fish_dnglab_using_subcommand analyze" -l full-pixel -d 'Write uncompressed full pixel data to STDOUT'
complete -c dnglab -n "__fish_dnglab_using_subcommand analyze" -l preview-pixel -d 'Write uncompressed preview pixel data to STDOUT'
complete -c dnglab -n "__fish_dnglab_using_subcommand analyze" -l thumbnail-pixel -d 'Write uncompressed preview pixel data to STDOUT'
complete -c dnglab -n "__fish_dnglab_using_subcommand analyze" -l raw-checksum -d 'Write MD5 checksum of raw pixels to STDOUT'
complete -c dnglab -n "__fish_dnglab_using_subcommand analyze" -l preview-checksum -d 'Write MD5 checksum of preview pixels to STDOUT'
complete -c dnglab -n "__fish_dnglab_using_subcommand analyze" -l thumbnail-checksum -d 'Write MD5 checksum of thumbnail pixels to STDOUT'
complete -c dnglab -n "__fish_dnglab_using_subcommand analyze" -l srgb -d 'Write sRGB 16-bit TIFF to STDOUT'
complete -c dnglab -n "__fish_dnglab_using_subcommand analyze" -l meta -d 'Write metadata to STDOUT'
complete -c dnglab -n "__fish_dnglab_using_subcommand analyze" -l structure -d 'Write file structure to STDOUT'
complete -c dnglab -n "__fish_dnglab_using_subcommand analyze" -l summary -d 'Write summary information for file to STDOUT'
complete -c dnglab -n "__fish_dnglab_using_subcommand analyze" -l json -d 'Format metadata as JSON'
complete -c dnglab -n "__fish_dnglab_using_subcommand analyze" -l yaml -d 'Format metadata as YAML'
complete -c dnglab -n "__fish_dnglab_using_subcommand analyze" -s v -d 'Print status for every file'
complete -c dnglab -n "__fish_dnglab_using_subcommand analyze" -s h -l help -d 'Print help'
complete -c dnglab -n "__fish_dnglab_using_subcommand convert" -s c -l compression -d 'Compression for raw image' -r -f -a "lossless\t''
uncompressed\t''"
complete -c dnglab -n "__fish_dnglab_using_subcommand convert" -l ljpeg92-predictor -d 'LJPEG-92 predictor' -r
complete -c dnglab -n "__fish_dnglab_using_subcommand convert" -l dng-preview -d 'DNG include preview image' -r -f -a "true\t''
false\t''"
complete -c dnglab -n "__fish_dnglab_using_subcommand convert" -l dng-thumbnail -d 'DNG include thumbnail image' -r -f -a "true\t''
false\t''"
complete -c dnglab -n "__fish_dnglab_using_subcommand convert" -l embed-raw -d 'Embed the raw file into DNG' -r -f -a "true\t''
false\t''"
complete -c dnglab -n "__fish_dnglab_using_subcommand convert" -l artist -d 'Set the artist tag' -r
complete -c dnglab -n "__fish_dnglab_using_subcommand convert" -l keep-mtime -d 'Keep mtime, read from EXIF with fallback to original file mtime' -r -f -a "true\t''
false\t''"
complete -c dnglab -n "__fish_dnglab_using_subcommand convert" -l image-index -d 'Select a specific image index (or \'all\') if file is a image container' -r
complete -c dnglab -n "__fish_dnglab_using_subcommand convert" -l crop -d 'DNG default crop' -r -f -a "best\t''
activearea\t''
none\t''"
complete -c dnglab -n "__fish_dnglab_using_subcommand convert" -s d -l loglevel -d 'Log level' -r -f -a "error\t''
warn\t''
info\t''
debug\t''
trace\t''"
complete -c dnglab -n "__fish_dnglab_using_subcommand convert" -s f -l override -d 'Override existing files'
complete -c dnglab -n "__fish_dnglab_using_subcommand convert" -s r -l recursive -d 'Process input directory recursive'
complete -c dnglab -n "__fish_dnglab_using_subcommand convert" -s v -d 'Print status for every file'
complete -c dnglab -n "__fish_dnglab_using_subcommand convert" -s h -l help -d 'Print help'
complete -c dnglab -n "__fish_dnglab_using_subcommand ftpserver" -s c -l compression -d 'Compression for raw image' -r -f -a "lossless\t''
uncompressed\t''"
complete -c dnglab -n "__fish_dnglab_using_subcommand ftpserver" -l ljpeg92-predictor -d 'LJPEG-92 predictor' -r
complete -c dnglab -n "__fish_dnglab_using_subcommand ftpserver" -l dng-preview -d 'DNG include preview image' -r -f -a "true\t''
false\t''"
complete -c dnglab -n "__fish_dnglab_using_subcommand ftpserver" -l dng-thumbnail -d 'DNG include thumbnail image' -r -f -a "true\t''
false\t''"
complete -c dnglab -n "__fish_dnglab_using_subcommand ftpserver" -l embed-raw -d 'Embed the raw file into DNG' -r -f -a "true\t''
false\t''"
complete -c dnglab -n "__fish_dnglab_using_subcommand ftpserver" -l artist -d 'Set the artist tag' -r
complete -c dnglab -n "__fish_dnglab_using_subcommand ftpserver" -l keep-mtime -d 'Keep mtime, read from EXIF with fallback to original file mtime' -r -f -a "true\t''
false\t''"
complete -c dnglab -n "__fish_dnglab_using_subcommand ftpserver" -l image-index -d 'Select a specific image index (or \'all\') if file is a image container' -r
complete -c dnglab -n "__fish_dnglab_using_subcommand ftpserver" -l crop -d 'DNG default crop' -r -f -a "best\t''
activearea\t''
none\t''"
complete -c dnglab -n "__fish_dnglab_using_subcommand ftpserver" -l port -d 'FTP listen port' -r
complete -c dnglab -n "__fish_dnglab_using_subcommand ftpserver" -l listen -d 'FTP listen address' -r
complete -c dnglab -n "__fish_dnglab_using_subcommand ftpserver" -l keep-original -d 'Keep original raw' -r -f -a "true\t''
false\t''"
complete -c dnglab -n "__fish_dnglab_using_subcommand ftpserver" -s d -l loglevel -d 'Log level' -r -f -a "error\t''
warn\t''
info\t''
debug\t''
trace\t''"
complete -c dnglab -n "__fish_dnglab_using_subcommand ftpserver" -s f -l override -d 'Override existing files'
complete -c dnglab -n "__fish_dnglab_using_subcommand ftpserver" -s v -d 'Print status for every file'
complete -c dnglab -n "__fish_dnglab_using_subcommand ftpserver" -s h -l help -d 'Print help'
complete -c dnglab -n "__fish_dnglab_using_subcommand cameras" -s d -l loglevel -d 'Log level' -r -f -a "error\t''
warn\t''
info\t''
debug\t''
trace\t''"
complete -c dnglab -n "__fish_dnglab_using_subcommand cameras" -l md -d 'Markdown format output'
complete -c dnglab -n "__fish_dnglab_using_subcommand cameras" -s v -d 'Print status for every file'
complete -c dnglab -n "__fish_dnglab_using_subcommand cameras" -s h -l help -d 'Print help'
complete -c dnglab -n "__fish_dnglab_using_subcommand lenses" -s d -l loglevel -d 'Log level' -r -f -a "error\t''
warn\t''
info\t''
debug\t''
trace\t''"
complete -c dnglab -n "__fish_dnglab_using_subcommand lenses" -l md -d 'Markdown format output'
complete -c dnglab -n "__fish_dnglab_using_subcommand lenses" -s v -d 'Print status for every file'
complete -c dnglab -n "__fish_dnglab_using_subcommand lenses" -s h -l help -d 'Print help'
complete -c dnglab -n "__fish_dnglab_using_subcommand makedng" -s o -l output -d 'Output DNG file path' -r -F
complete -c dnglab -n "__fish_dnglab_using_subcommand makedng" -s i -l input -d 'Input files (raw, preview, exif, ...), index for map starts with 0' -r -F
complete -c dnglab -n "__fish_dnglab_using_subcommand makedng" -l map -d 'Input usage map' -r
complete -c dnglab -n "__fish_dnglab_using_subcommand makedng" -l dng-backward-version -d 'DNG specification version' -r -f -a "1.0\t''
1.1\t''
1.2\t''
1.3\t''
1.4\t''
1.5\t''
1.6\t''"
complete -c dnglab -n "__fish_dnglab_using_subcommand makedng" -l colorimetric-reference -d 'Reference for XYZ values' -r -f -a "scene\t''
output\t''"
complete -c dnglab -n "__fish_dnglab_using_subcommand makedng" -l unique-camera-model -d 'Unique camera model' -r
complete -c dnglab -n "__fish_dnglab_using_subcommand makedng" -l artist -d 'Set the Artist tag' -r
complete -c dnglab -n "__fish_dnglab_using_subcommand makedng" -l make -d 'Set the Make tag' -r
complete -c dnglab -n "__fish_dnglab_using_subcommand makedng" -l model -d 'Set the Model tag' -r
complete -c dnglab -n "__fish_dnglab_using_subcommand makedng" -l matrix1 -d 'Matrix 1' -r -f -a "XYZ_sRGB_D50\t''
XYZ_sRGB_D65\t''
XYZ_AdobeRGB_D50\t''
XYZ_AdobeRGB_D65\t''
custom 3x3 matrix (comma seperated)\t''"
complete -c dnglab -n "__fish_dnglab_using_subcommand makedng" -l matrix2 -d 'Matrix 2' -r -f -a "XYZ_sRGB_D50\t''
XYZ_sRGB_D65\t''
XYZ_AdobeRGB_D50\t''
XYZ_AdobeRGB_D65\t''
custom 3x3 matrix (comma seperated)\t''"
complete -c dnglab -n "__fish_dnglab_using_subcommand makedng" -l matrix3 -d 'Matrix 3' -r -f -a "XYZ_sRGB_D50\t''
XYZ_sRGB_D65\t''
XYZ_AdobeRGB_D50\t''
XYZ_AdobeRGB_D65\t''
custom 3x3 matrix (comma seperated)\t''"
complete -c dnglab -n "__fish_dnglab_using_subcommand makedng" -l illuminant1 -d 'Illuminant 1' -r -f -a "Unknown\t''
A\t''
B\t''
C\t''
D50\t''
D55\t''
D65\t''
D75\t''"
complete -c dnglab -n "__fish_dnglab_using_subcommand makedng" -l illuminant2 -d 'Illuminant 2' -r -f -a "Unknown\t''
A\t''
B\t''
C\t''
D50\t''
D55\t''
D65\t''
D75\t''"
complete -c dnglab -n "__fish_dnglab_using_subcommand makedng" -l illuminant3 -d 'Illuminant 3' -r -f -a "Unknown\t''
A\t''
B\t''
C\t''
D50\t''
D55\t''
D65\t''
D75\t''"
complete -c dnglab -n "__fish_dnglab_using_subcommand makedng" -l linearization -d 'Linearization table' -r -f -a "8bit_sRGB\t''
8bit_sRGB_invert\t''
16bit_sRGB\t''
16bit_sRGB_invert\t''
8bit_gamma1.8\t''
8bit_gamma1.8_invert\t''
8bit_gamma2.0\t''
8bit_gamma2.0_invert\t''
8bit_gamma2.2\t''
8bit_gamma2.2_invert\t''
8bit_gamma2.4\t''
8bit_gamma2.4_invert\t''
16bit_gamma1.8\t''
16bit_gamma1.8_invert\t''
16bit_gamma2.0\t''
16bit_gamma2.0_invert\t''
16bit_gamma2.2\t''
16bit_gamma2.2_invert\t''
16bit_gamma2.4\t''
16bit_gamma2.4_invert\t''
custom table (comma seperated)\t''"
complete -c dnglab -n "__fish_dnglab_using_subcommand makedng" -l wb -d 'Whitebalance as-shot' -r
complete -c dnglab -n "__fish_dnglab_using_subcommand makedng" -l white-xy -d 'Whitebalance as-shot encoded as xy chromaticity coordinates' -r -f -a "D50\t''
D65\t''
custom x\,y value (comma seperated)\t''"
complete -c dnglab -n "__fish_dnglab_using_subcommand makedng" -s d -l loglevel -d 'Log level' -r -f -a "error\t''
warn\t''
info\t''
debug\t''
trace\t''"
complete -c dnglab -n "__fish_dnglab_using_subcommand makedng" -s f -l override -d 'Override existing files'
complete -c dnglab -n "__fish_dnglab_using_subcommand makedng" -s v -d 'Print status for every file'
complete -c dnglab -n "__fish_dnglab_using_subcommand makedng" -s h -l help -d 'Print help (see more with \'--help\')'
complete -c dnglab -n "__fish_dnglab_using_subcommand gui" -s d -l loglevel -d 'Log level' -r -f -a "error\t''
warn\t''
info\t''
debug\t''
trace\t''"
complete -c dnglab -n "__fish_dnglab_using_subcommand gui" -s v -d 'Print status for every file'
complete -c dnglab -n "__fish_dnglab_using_subcommand gui" -s h -l help -d 'Print help'
complete -c dnglab -n "__fish_dnglab_using_subcommand extract" -s d -l loglevel -d 'Log level' -r -f -a "error\t''
warn\t''
info\t''
debug\t''
trace\t''"
complete -c dnglab -n "__fish_dnglab_using_subcommand extract" -l skipchecks -d 'Skip integrity checks'
complete -c dnglab -n "__fish_dnglab_using_subcommand extract" -s r -l recursive -d 'Process input directory recursive'
complete -c dnglab -n "__fish_dnglab_using_subcommand extract" -s f -l override -d 'Override existing files'
complete -c dnglab -n "__fish_dnglab_using_subcommand extract" -s v -d 'Print status for every file'
complete -c dnglab -n "__fish_dnglab_using_subcommand extract" -s h -l help -d 'Print help'
complete -c dnglab -n "__fish_dnglab_using_subcommand help; and not __fish_seen_subcommand_from analyze convert ftpserver cameras lenses makedng gui extract help" -f -a "analyze" -d 'Analyze raw image'
complete -c dnglab -n "__fish_dnglab_using_subcommand help; and not __fish_seen_subcommand_from analyze convert ftpserver cameras lenses makedng gui extract help" -f -a "convert" -d 'Convert raw image(s) into dng format'
complete -c dnglab -n "__fish_dnglab_using_subcommand help; and not __fish_seen_subcommand_from analyze convert ftpserver cameras lenses makedng gui extract help" -f -a "ftpserver" -d 'Convert raw image(s) into dng format'
complete -c dnglab -n "__fish_dnglab_using_subcommand help; and not __fish_seen_subcommand_from analyze convert ftpserver cameras lenses makedng gui extract help" -f -a "cameras" -d 'List supported cameras'
complete -c dnglab -n "__fish_dnglab_using_subcommand help; and not __fish_seen_subcommand_from analyze convert ftpserver cameras lenses makedng gui extract help" -f -a "lenses" -d 'List supported lenses'
complete -c dnglab -n "__fish_dnglab_using_subcommand help; and not __fish_seen_subcommand_from analyze convert ftpserver cameras lenses makedng gui extract help" -f -a "makedng" -d 'Lowlevel command to make a DNG file'
complete -c dnglab -n "__fish_dnglab_using_subcommand help; and not __fish_seen_subcommand_from analyze convert ftpserver cameras lenses makedng gui extract help" -f -a "gui" -d 'Start GUI (not implemented)'
complete -c dnglab -n "__fish_dnglab_using_subcommand help; and not __fish_seen_subcommand_from analyze convert ftpserver cameras lenses makedng gui extract help" -f -a "extract" -d 'Extract embedded original Raw from DNG'
complete -c dnglab -n "__fish_dnglab_using_subcommand help; and not __fish_seen_subcommand_from analyze convert ftpserver cameras lenses makedng gui extract help" -f -a "help" -d 'Print this message or the help of the given subcommand(s)'
