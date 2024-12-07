
use builtin;
use str;

set edit:completion:arg-completer[dnglab] = {|@words|
    fn spaces {|n|
        builtin:repeat $n ' ' | str:join ''
    }
    fn cand {|text desc|
        edit:complex-candidate $text &display=$text' '(spaces (- 14 (wcswidth $text)))$desc
    }
    var command = 'dnglab'
    for word $words[1..-1] {
        if (str:has-prefix $word '-') {
            break
        }
        set command = $command';'$word
    }
    var completions = [
        &'dnglab'= {
            cand -d 'Log level'
            cand --loglevel 'Log level'
            cand -v 'Print status for every file'
            cand -h 'Print help'
            cand --help 'Print help'
            cand -V 'Print version'
            cand --version 'Print version'
            cand analyze 'Analyze raw image'
            cand convert 'Convert raw image(s) into dng format'
            cand ftpserver 'Convert raw image(s) into dng format'
            cand cameras 'List supported cameras'
            cand lenses 'List supported lenses'
            cand makedng 'Lowlevel command to make a DNG file'
            cand gui 'Start GUI (not implemented)'
            cand extract 'Extract embedded original Raw from DNG'
            cand help 'Print this message or the help of the given subcommand(s)'
        }
        &'dnglab;analyze'= {
            cand -d 'Log level'
            cand --loglevel 'Log level'
            cand --raw-pixel 'raw-pixel'
            cand --full-pixel 'Write uncompressed full pixel data to STDOUT'
            cand --preview-pixel 'Write uncompressed preview pixel data to STDOUT'
            cand --thumbnail-pixel 'Write uncompressed preview pixel data to STDOUT'
            cand --raw-checksum 'Write MD5 checksum of raw pixels to STDOUT'
            cand --preview-checksum 'Write MD5 checksum of preview pixels to STDOUT'
            cand --thumbnail-checksum 'Write MD5 checksum of thumbnail pixels to STDOUT'
            cand --srgb 'Write sRGB 16-bit TIFF to STDOUT'
            cand --meta 'Write metadata to STDOUT'
            cand --structure 'Write file structure to STDOUT'
            cand --summary 'Write summary information for file to STDOUT'
            cand --json 'Format metadata as JSON'
            cand --yaml 'Format metadata as YAML'
            cand -v 'Print status for every file'
            cand -h 'Print help'
            cand --help 'Print help'
        }
        &'dnglab;convert'= {
            cand -c 'Compression for raw image'
            cand --compression 'Compression for raw image'
            cand --ljpeg92-predictor 'LJPEG-92 predictor'
            cand --dng-preview 'DNG include preview image'
            cand --dng-thumbnail 'DNG include thumbnail image'
            cand --embed-raw 'Embed the raw file into DNG'
            cand --artist 'Set the artist tag'
            cand --image-index 'Select a specific image index (or ''all'') if file is a image container'
            cand --crop 'DNG default crop'
            cand -d 'Log level'
            cand --loglevel 'Log level'
            cand -f 'Override existing files'
            cand --override 'Override existing files'
            cand -r 'Process input directory recursive'
            cand --recursive 'Process input directory recursive'
            cand -v 'Print status for every file'
            cand -h 'Print help'
            cand --help 'Print help'
        }
        &'dnglab;ftpserver'= {
            cand -c 'Compression for raw image'
            cand --compression 'Compression for raw image'
            cand --ljpeg92-predictor 'LJPEG-92 predictor'
            cand --dng-preview 'DNG include preview image'
            cand --dng-thumbnail 'DNG include thumbnail image'
            cand --embed-raw 'Embed the raw file into DNG'
            cand --artist 'Set the artist tag'
            cand --image-index 'Select a specific image index (or ''all'') if file is a image container'
            cand --crop 'DNG default crop'
            cand --port 'FTP listen port'
            cand --listen 'FTP listen address'
            cand --keep-original 'Keep original raw'
            cand -d 'Log level'
            cand --loglevel 'Log level'
            cand -f 'Override existing files'
            cand --override 'Override existing files'
            cand -v 'Print status for every file'
            cand -h 'Print help'
            cand --help 'Print help'
        }
        &'dnglab;cameras'= {
            cand -d 'Log level'
            cand --loglevel 'Log level'
            cand --md 'Markdown format output'
            cand -v 'Print status for every file'
            cand -h 'Print help'
            cand --help 'Print help'
        }
        &'dnglab;lenses'= {
            cand -d 'Log level'
            cand --loglevel 'Log level'
            cand --md 'Markdown format output'
            cand -v 'Print status for every file'
            cand -h 'Print help'
            cand --help 'Print help'
        }
        &'dnglab;makedng'= {
            cand -o 'Output DNG file path'
            cand --output 'Output DNG file path'
            cand -i 'Input files (raw, preview, exif, ...), index for map starts with 0'
            cand --input 'Input files (raw, preview, exif, ...), index for map starts with 0'
            cand --map 'Input usage map'
            cand --dng-backward-version 'DNG specification version'
            cand --colorimetric-reference 'Reference for XYZ values'
            cand --unique-camera-model 'Unique camera model'
            cand --artist 'Set the Artist tag'
            cand --make 'Set the Make tag'
            cand --model 'Set the Model tag'
            cand --matrix1 'Matrix 1'
            cand --matrix2 'Matrix 2'
            cand --matrix3 'Matrix 3'
            cand --illuminant1 'Illuminant 1'
            cand --illuminant2 'Illuminant 2'
            cand --illuminant3 'Illuminant 3'
            cand --linearization 'Linearization table'
            cand --wb 'Whitebalance as-shot'
            cand --white-xy 'Whitebalance as-shot encoded as xy chromaticity coordinates'
            cand -d 'Log level'
            cand --loglevel 'Log level'
            cand -f 'Override existing files'
            cand --override 'Override existing files'
            cand -v 'Print status for every file'
            cand -h 'Print help (see more with ''--help'')'
            cand --help 'Print help (see more with ''--help'')'
        }
        &'dnglab;gui'= {
            cand -d 'Log level'
            cand --loglevel 'Log level'
            cand -v 'Print status for every file'
            cand -h 'Print help'
            cand --help 'Print help'
        }
        &'dnglab;extract'= {
            cand -d 'Log level'
            cand --loglevel 'Log level'
            cand --skipchecks 'Skip integrity checks'
            cand -r 'Process input directory recursive'
            cand --recursive 'Process input directory recursive'
            cand -f 'Override existing files'
            cand --override 'Override existing files'
            cand -v 'Print status for every file'
            cand -h 'Print help'
            cand --help 'Print help'
        }
        &'dnglab;help'= {
            cand analyze 'Analyze raw image'
            cand convert 'Convert raw image(s) into dng format'
            cand ftpserver 'Convert raw image(s) into dng format'
            cand cameras 'List supported cameras'
            cand lenses 'List supported lenses'
            cand makedng 'Lowlevel command to make a DNG file'
            cand gui 'Start GUI (not implemented)'
            cand extract 'Extract embedded original Raw from DNG'
            cand help 'Print this message or the help of the given subcommand(s)'
        }
        &'dnglab;help;analyze'= {
        }
        &'dnglab;help;convert'= {
        }
        &'dnglab;help;ftpserver'= {
        }
        &'dnglab;help;cameras'= {
        }
        &'dnglab;help;lenses'= {
        }
        &'dnglab;help;makedng'= {
        }
        &'dnglab;help;gui'= {
        }
        &'dnglab;help;extract'= {
        }
        &'dnglab;help;help'= {
        }
    ]
    $completions[$command]
}
