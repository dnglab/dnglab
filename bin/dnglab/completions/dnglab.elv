
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
            cand --profile-path 'Directory with profile TOML files'
            cand --config 'dnglab configuration file'
            cand -v 'Print status for every file'
            cand -h 'Print help (see more with ''--help'')'
            cand --help 'Print help (see more with ''--help'')'
            cand -V 'Print version'
            cand --version 'Print version'
            cand analyze 'Analyze raw image'
            cand process-raw 'process-raw'
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
            cand --export-dcp 'Write the as-shot camera profile as DCP file'
            cand --export-dcp-all 'Write all camera profiles as DCP files to DIR'
            cand -d 'Log level'
            cand --loglevel 'Log level'
            cand --profile-path 'Directory with profile TOML files'
            cand --config 'dnglab configuration file'
            cand --raw-pixel 'raw-pixel'
            cand --full-pixel 'Write uncompressed full pixel data to STDOUT'
            cand --preview-pixel 'Write uncompressed preview pixel data to STDOUT'
            cand --thumbnail-pixel 'Write uncompressed preview pixel data to STDOUT'
            cand --raw-checksum 'Write MD5 checksum of raw pixels to STDOUT'
            cand --full-checksum 'Write MD5 checksum of full pixels to STDOUT'
            cand --preview-checksum 'Write MD5 checksum of preview pixels to STDOUT'
            cand --thumbnail-checksum 'Write MD5 checksum of thumbnail pixels to STDOUT'
            cand --srgb 'Write sRGB 16-bit TIFF to STDOUT'
            cand --meta 'Write metadata to STDOUT'
            cand --structure 'Write file structure to STDOUT'
            cand --summary 'Write summary information for file to STDOUT'
            cand --camera-profiles 'Write camera profiles to STDOUT (plain text, JSON or YAML)'
            cand -f 'Override existing DCP files'
            cand --override 'Override existing DCP files'
            cand --json 'Format metadata as JSON'
            cand --yaml 'Format metadata as YAML'
            cand -v 'Print status for every file'
            cand -h 'Print help (see more with ''--help'')'
            cand --help 'Print help (see more with ''--help'')'
        }
        &'dnglab;process-raw'= {
            cand --remove-dng-opcodes 'Remove DNG opcodes, as comma separated [<list>:]<opcode> (e.g. WarpRectilinear,2:GainMap)'
            cand --bad-pixels 'Bad pixel list in the format of dcraw: column, row and UNIX time per line, relative to the ActiveArea. It is written as FixBadPixelsList to OpcodeList1 and replaces an existing one'
            cand --artist 'Set the artist tag'
            cand --keep-mtime 'Keep mtime, read from EXIF with fallback to original file mtime'
            cand --image-index 'Select a specific image index (or ''all'') if file is a image container'
            cand --crop 'Crop of the image: DefaultCrop (best), ActiveArea or none'
            cand --format 'Output image format, defaults to the OUTPUT file extension or tif. TIFF has 16-bit samples, linear 32-bit floating point samples for High Dynamic Range camera profiles'
            cand --color-space 'Color space of the image'
            cand --highlights 'Highlights above white after the BaselineExposure: compress keeps their color with a smooth shoulder, clip clips each channel like DNG readers'
            cand --bayer-demosaic 'Demosaic algorithm for Bayer RGB sensors: ppg is fast, ahd interpolates along edges with fewer artifacts, vng averages the directions with small gradients, rcd interpolates along edges with ratio corrections, bilinear averages the neighbours of each color'
            cand --xtrans-demosaic 'Demosaic algorithm for X-Trans sensors: markesteijn has the best quality, vng is faster'
            cand --dcp-file 'Camera profile (DCP) to render the image with, replaces the primary camera profile'
            cand --jpeg-quality 'JPEG quality (1-100)'
            cand --denoise-luma 'Strength of the luma noise reduction (0-4), 0 is off, 1 is a full reduction [default: 0.25]'
            cand --denoise-chroma 'Strength of the chroma noise reduction (0-4), 0 is off, 1 is a full reduction [default: 0.5]'
            cand --sharpen 'Strength of the sharpening (0-2), 0 is off, 1 is the classic Laplacian filter [default: 0.2]'
            cand --exposure 'Change the exposure by this value in EV, e.g. -1.2. A negative value keeps white [default: 0]'
            cand --steps 'Apply only these processing steps of the rendering, as comma separated list. Without it the default steps are applied'
            cand --skip-steps 'Skip these processing steps of the rendering, as comma separated list (e.g. tonecurve,denoise,sharpen). Takes priority over --steps'
            cand -d 'Log level'
            cand --loglevel 'Log level'
            cand --profile-path 'Directory with profile TOML files'
            cand --config 'dnglab configuration file'
            cand --crop-transparent 'Crop to the largest rectangle without transparent pixels of the DNG transparency mask'
            cand --auto-exposure 'Correct the exposure by an estimate from the image, --exposure is added to it'
            cand -f 'Override existing files'
            cand --override 'Override existing files'
            cand -r 'Process input directory recursive'
            cand --recursive 'Process input directory recursive'
            cand -v 'Print status for every file'
            cand -h 'Print help (see more with ''--help'')'
            cand --help 'Print help (see more with ''--help'')'
        }
        &'dnglab;convert'= {
            cand -p 'Convert profile name'
            cand --profile 'Convert profile name'
            cand -c 'Compression for raw image'
            cand --compression 'Compression for raw image'
            cand --ljpeg92-predictor 'LJPEG-92 predictor'
            cand --lossy-jpeg-quality 'JPEG quality for lossy compression (1-100)'
            cand --jxl-distance 'JPEG XL distance (0-25), 0 is lossless'
            cand --jxl-effort 'JPEG XL encoder effort (1-9)'
            cand --jxl-decode-speed 'JPEG XL decoding speed for lossy compression (1-4)'
            cand --dng-preview 'DNG include preview image'
            cand --dng-thumbnail 'DNG include thumbnail image'
            cand --dng-force-preview 'Generate DNG preview image from raw data'
            cand --dng-force-thumbnail 'Generate DNG thumbnail image from raw data'
            cand --max-dng-backward-version 'Maximum DNG backward version'
            cand --embed-raw 'Embed the raw file into DNG'
            cand --apply-dng-opcodes 'Apply DNG opcode lists to the image data instead of writing them to the DNG'
            cand --remove-dng-opcodes 'Remove DNG opcodes, as comma separated [<list>:]<opcode> (e.g. WarpRectilinear,2:GainMap)'
            cand --artist 'Set the artist tag'
            cand --copyright 'Set the copyright tag'
            cand --keep-mtime 'Keep mtime, read from EXIF with fallback to original file mtime'
            cand --image-index 'Select a specific image index (or ''all'') if file is a image container'
            cand --crop 'DNG default crop'
            cand -j 'Number of files to convert in parallel (0 = auto)'
            cand --jobs 'Number of files to convert in parallel (0 = auto)'
            cand --dcp-file 'Camera profile (DCP) that replaces the primary camera profile'
            cand --extra-dcp-file 'Camera profile (DCP) to embed as extra camera profile, can be repeated'
            cand --bad-pixels 'Bad pixel list in the format of dcraw: column, row and UNIX time per line, relative to the ActiveArea. It is written as FixBadPixelsList to OpcodeList1 and replaces an existing one'
            cand -d 'Log level'
            cand --loglevel 'Log level'
            cand --profile-path 'Directory with profile TOML files'
            cand --config 'dnglab configuration file'
            cand -f 'Override existing files'
            cand --override 'Override existing files'
            cand -r 'Process input directory recursive'
            cand --recursive 'Process input directory recursive'
            cand -v 'Print status for every file'
            cand -h 'Print help (see more with ''--help'')'
            cand --help 'Print help (see more with ''--help'')'
        }
        &'dnglab;ftpserver'= {
            cand -p 'Convert profile name'
            cand --profile 'Convert profile name'
            cand -c 'Compression for raw image'
            cand --compression 'Compression for raw image'
            cand --ljpeg92-predictor 'LJPEG-92 predictor'
            cand --lossy-jpeg-quality 'JPEG quality for lossy compression (1-100)'
            cand --jxl-distance 'JPEG XL distance (0-25), 0 is lossless'
            cand --jxl-effort 'JPEG XL encoder effort (1-9)'
            cand --jxl-decode-speed 'JPEG XL decoding speed for lossy compression (1-4)'
            cand --dng-preview 'DNG include preview image'
            cand --dng-thumbnail 'DNG include thumbnail image'
            cand --dng-force-preview 'Generate DNG preview image from raw data'
            cand --dng-force-thumbnail 'Generate DNG thumbnail image from raw data'
            cand --max-dng-backward-version 'Maximum DNG backward version'
            cand --embed-raw 'Embed the raw file into DNG'
            cand --apply-dng-opcodes 'Apply DNG opcode lists to the image data instead of writing them to the DNG'
            cand --remove-dng-opcodes 'Remove DNG opcodes, as comma separated [<list>:]<opcode> (e.g. WarpRectilinear,2:GainMap)'
            cand --artist 'Set the artist tag'
            cand --copyright 'Set the copyright tag'
            cand --keep-mtime 'Keep mtime, read from EXIF with fallback to original file mtime'
            cand --image-index 'Select a specific image index (or ''all'') if file is a image container'
            cand --crop 'DNG default crop'
            cand --port 'FTP listen port'
            cand --listen 'FTP listen address'
            cand --keep-original 'Keep original raw'
            cand -d 'Log level'
            cand --loglevel 'Log level'
            cand --profile-path 'Directory with profile TOML files'
            cand --config 'dnglab configuration file'
            cand -f 'Override existing files'
            cand --override 'Override existing files'
            cand -v 'Print status for every file'
            cand -h 'Print help (see more with ''--help'')'
            cand --help 'Print help (see more with ''--help'')'
        }
        &'dnglab;cameras'= {
            cand -d 'Log level'
            cand --loglevel 'Log level'
            cand --profile-path 'Directory with profile TOML files'
            cand --config 'dnglab configuration file'
            cand --md 'Markdown format output'
            cand -v 'Print status for every file'
            cand -h 'Print help (see more with ''--help'')'
            cand --help 'Print help (see more with ''--help'')'
        }
        &'dnglab;lenses'= {
            cand -d 'Log level'
            cand --loglevel 'Log level'
            cand --profile-path 'Directory with profile TOML files'
            cand --config 'dnglab configuration file'
            cand --md 'Markdown format output'
            cand -v 'Print status for every file'
            cand -h 'Print help (see more with ''--help'')'
            cand --help 'Print help (see more with ''--help'')'
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
            cand --profile-path 'Directory with profile TOML files'
            cand --config 'dnglab configuration file'
            cand -f 'Override existing files'
            cand --override 'Override existing files'
            cand -v 'Print status for every file'
            cand -h 'Print help (see more with ''--help'')'
            cand --help 'Print help (see more with ''--help'')'
        }
        &'dnglab;gui'= {
            cand -d 'Log level'
            cand --loglevel 'Log level'
            cand --profile-path 'Directory with profile TOML files'
            cand --config 'dnglab configuration file'
            cand -v 'Print status for every file'
            cand -h 'Print help (see more with ''--help'')'
            cand --help 'Print help (see more with ''--help'')'
        }
        &'dnglab;extract'= {
            cand -d 'Log level'
            cand --loglevel 'Log level'
            cand --profile-path 'Directory with profile TOML files'
            cand --config 'dnglab configuration file'
            cand --skipchecks 'Skip integrity checks'
            cand -r 'Process input directory recursive'
            cand --recursive 'Process input directory recursive'
            cand -f 'Override existing files'
            cand --override 'Override existing files'
            cand -v 'Print status for every file'
            cand -h 'Print help (see more with ''--help'')'
            cand --help 'Print help (see more with ''--help'')'
        }
        &'dnglab;help'= {
            cand analyze 'Analyze raw image'
            cand process-raw 'process-raw'
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
        &'dnglab;help;process-raw'= {
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
