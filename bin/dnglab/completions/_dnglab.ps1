
using namespace System.Management.Automation
using namespace System.Management.Automation.Language

Register-ArgumentCompleter -Native -CommandName 'dnglab' -ScriptBlock {
    param($wordToComplete, $commandAst, $cursorPosition)

    $commandElements = $commandAst.CommandElements
    $command = @(
        'dnglab'
        for ($i = 1; $i -lt $commandElements.Count; $i++) {
            $element = $commandElements[$i]
            if ($element -isnot [StringConstantExpressionAst] -or
                $element.StringConstantType -ne [StringConstantType]::BareWord -or
                $element.Value.StartsWith('-') -or
                $element.Value -eq $wordToComplete) {
                break
        }
        $element.Value
    }) -join ';'

    $completions = @(switch ($command) {
        'dnglab' {
            [CompletionResult]::new('-d', '-d', [CompletionResultType]::ParameterName, 'Log level')
            [CompletionResult]::new('--loglevel', '--loglevel', [CompletionResultType]::ParameterName, 'Log level')
            [CompletionResult]::new('--profile-path', '--profile-path', [CompletionResultType]::ParameterName, 'Directory with profile TOML files')
            [CompletionResult]::new('--config', '--config', [CompletionResultType]::ParameterName, 'dnglab configuration file')
            [CompletionResult]::new('-v', '-v', [CompletionResultType]::ParameterName, 'Print status for every file')
            [CompletionResult]::new('-h', '-h', [CompletionResultType]::ParameterName, 'Print help (see more with ''--help'')')
            [CompletionResult]::new('--help', '--help', [CompletionResultType]::ParameterName, 'Print help (see more with ''--help'')')
            [CompletionResult]::new('-V', '-V ', [CompletionResultType]::ParameterName, 'Print version')
            [CompletionResult]::new('--version', '--version', [CompletionResultType]::ParameterName, 'Print version')
            [CompletionResult]::new('analyze', 'analyze', [CompletionResultType]::ParameterValue, 'Analyze raw image')
            [CompletionResult]::new('process-raw', 'process-raw', [CompletionResultType]::ParameterValue, 'process-raw')
            [CompletionResult]::new('convert', 'convert', [CompletionResultType]::ParameterValue, 'Convert raw image(s) into dng format')
            [CompletionResult]::new('ftpserver', 'ftpserver', [CompletionResultType]::ParameterValue, 'Convert raw image(s) into dng format')
            [CompletionResult]::new('cameras', 'cameras', [CompletionResultType]::ParameterValue, 'List supported cameras')
            [CompletionResult]::new('lenses', 'lenses', [CompletionResultType]::ParameterValue, 'List supported lenses')
            [CompletionResult]::new('makedng', 'makedng', [CompletionResultType]::ParameterValue, 'Lowlevel command to make a DNG file')
            [CompletionResult]::new('gui', 'gui', [CompletionResultType]::ParameterValue, 'Start GUI (not implemented)')
            [CompletionResult]::new('extract', 'extract', [CompletionResultType]::ParameterValue, 'Extract embedded original Raw from DNG')
            [CompletionResult]::new('help', 'help', [CompletionResultType]::ParameterValue, 'Print this message or the help of the given subcommand(s)')
            break
        }
        'dnglab;analyze' {
            [CompletionResult]::new('--export-dcp', '--export-dcp', [CompletionResultType]::ParameterName, 'Write the as-shot camera profile as DCP file')
            [CompletionResult]::new('--export-dcp-all', '--export-dcp-all', [CompletionResultType]::ParameterName, 'Write all camera profiles as DCP files to DIR')
            [CompletionResult]::new('-d', '-d', [CompletionResultType]::ParameterName, 'Log level')
            [CompletionResult]::new('--loglevel', '--loglevel', [CompletionResultType]::ParameterName, 'Log level')
            [CompletionResult]::new('--profile-path', '--profile-path', [CompletionResultType]::ParameterName, 'Directory with profile TOML files')
            [CompletionResult]::new('--config', '--config', [CompletionResultType]::ParameterName, 'dnglab configuration file')
            [CompletionResult]::new('--raw-pixel', '--raw-pixel', [CompletionResultType]::ParameterName, 'raw-pixel')
            [CompletionResult]::new('--full-pixel', '--full-pixel', [CompletionResultType]::ParameterName, 'Write uncompressed full pixel data to STDOUT')
            [CompletionResult]::new('--preview-pixel', '--preview-pixel', [CompletionResultType]::ParameterName, 'Write uncompressed preview pixel data to STDOUT')
            [CompletionResult]::new('--thumbnail-pixel', '--thumbnail-pixel', [CompletionResultType]::ParameterName, 'Write uncompressed preview pixel data to STDOUT')
            [CompletionResult]::new('--raw-checksum', '--raw-checksum', [CompletionResultType]::ParameterName, 'Write MD5 checksum of raw pixels to STDOUT')
            [CompletionResult]::new('--full-checksum', '--full-checksum', [CompletionResultType]::ParameterName, 'Write MD5 checksum of full pixels to STDOUT')
            [CompletionResult]::new('--preview-checksum', '--preview-checksum', [CompletionResultType]::ParameterName, 'Write MD5 checksum of preview pixels to STDOUT')
            [CompletionResult]::new('--thumbnail-checksum', '--thumbnail-checksum', [CompletionResultType]::ParameterName, 'Write MD5 checksum of thumbnail pixels to STDOUT')
            [CompletionResult]::new('--srgb', '--srgb', [CompletionResultType]::ParameterName, 'Write sRGB 16-bit TIFF to STDOUT')
            [CompletionResult]::new('--meta', '--meta', [CompletionResultType]::ParameterName, 'Write metadata to STDOUT')
            [CompletionResult]::new('--structure', '--structure', [CompletionResultType]::ParameterName, 'Write file structure to STDOUT')
            [CompletionResult]::new('--summary', '--summary', [CompletionResultType]::ParameterName, 'Write summary information for file to STDOUT')
            [CompletionResult]::new('--camera-profiles', '--camera-profiles', [CompletionResultType]::ParameterName, 'Write camera profiles to STDOUT (plain text, JSON or YAML)')
            [CompletionResult]::new('-f', '-f', [CompletionResultType]::ParameterName, 'Override existing DCP files')
            [CompletionResult]::new('--override', '--override', [CompletionResultType]::ParameterName, 'Override existing DCP files')
            [CompletionResult]::new('--json', '--json', [CompletionResultType]::ParameterName, 'Format metadata as JSON')
            [CompletionResult]::new('--yaml', '--yaml', [CompletionResultType]::ParameterName, 'Format metadata as YAML')
            [CompletionResult]::new('-v', '-v', [CompletionResultType]::ParameterName, 'Print status for every file')
            [CompletionResult]::new('-h', '-h', [CompletionResultType]::ParameterName, 'Print help (see more with ''--help'')')
            [CompletionResult]::new('--help', '--help', [CompletionResultType]::ParameterName, 'Print help (see more with ''--help'')')
            break
        }
        'dnglab;process-raw' {
            [CompletionResult]::new('--remove-dng-opcodes', '--remove-dng-opcodes', [CompletionResultType]::ParameterName, 'Remove DNG opcodes, as comma separated [<list>:]<opcode> (e.g. WarpRectilinear,2:GainMap)')
            [CompletionResult]::new('--bad-pixels', '--bad-pixels', [CompletionResultType]::ParameterName, 'Bad pixel list in the format of dcraw: column, row and UNIX time per line, relative to the ActiveArea. It is written as FixBadPixelsList to OpcodeList1 and replaces an existing one')
            [CompletionResult]::new('--artist', '--artist', [CompletionResultType]::ParameterName, 'Set the artist tag')
            [CompletionResult]::new('--keep-mtime', '--keep-mtime', [CompletionResultType]::ParameterName, 'Keep mtime, read from EXIF with fallback to original file mtime')
            [CompletionResult]::new('--image-index', '--image-index', [CompletionResultType]::ParameterName, 'Select a specific image index (or ''all'') if file is a image container')
            [CompletionResult]::new('--crop', '--crop', [CompletionResultType]::ParameterName, 'Crop of the image: DefaultCrop (best), ActiveArea or none')
            [CompletionResult]::new('--format', '--format', [CompletionResultType]::ParameterName, 'Output image format, defaults to the OUTPUT file extension or tif. TIFF has 16-bit samples, linear 32-bit floating point samples for High Dynamic Range camera profiles. PNM is a 16-bit PGM for one color plane or PPM for three')
            [CompletionResult]::new('--color-space', '--color-space', [CompletionResultType]::ParameterName, 'Color space of the image')
            [CompletionResult]::new('--highlights', '--highlights', [CompletionResultType]::ParameterName, 'Highlights above white after the BaselineExposure: compress keeps their color with a smooth shoulder, clip clips each channel like DNG readers')
            [CompletionResult]::new('--bayer-demosaic', '--bayer-demosaic', [CompletionResultType]::ParameterName, 'Demosaic algorithm for Bayer RGB sensors: ppg is fast, ahd interpolates along edges with fewer artifacts, vng averages the directions with small gradients, rcd interpolates along edges with ratio corrections, bilinear averages the neighbours of each color')
            [CompletionResult]::new('--xtrans-demosaic', '--xtrans-demosaic', [CompletionResultType]::ParameterName, 'Demosaic algorithm for X-Trans sensors: markesteijn has the best quality, vng is faster')
            [CompletionResult]::new('--dcp-file', '--dcp-file', [CompletionResultType]::ParameterName, 'Camera profile (DCP) to render the image with, replaces the primary camera profile')
            [CompletionResult]::new('--jpeg-quality', '--jpeg-quality', [CompletionResultType]::ParameterName, 'JPEG quality (1-100)')
            [CompletionResult]::new('--denoise-luma', '--denoise-luma', [CompletionResultType]::ParameterName, 'Strength of the luma noise reduction (0-4), 0 is off, 1 is a full reduction [default: 0.25]')
            [CompletionResult]::new('--denoise-chroma', '--denoise-chroma', [CompletionResultType]::ParameterName, 'Strength of the chroma noise reduction (0-4), 0 is off, 1 is a full reduction [default: 0.5]')
            [CompletionResult]::new('--sharpen', '--sharpen', [CompletionResultType]::ParameterName, 'Strength of the sharpening (0-2), 0 is off, 1 is the classic Laplacian filter [default: 0.2]')
            [CompletionResult]::new('--exposure', '--exposure', [CompletionResultType]::ParameterName, 'Change the exposure by this value in EV, e.g. -1.2. A negative value keeps white [default: 0]')
            [CompletionResult]::new('--steps', '--steps', [CompletionResultType]::ParameterName, 'Apply only these processing steps of the rendering, as comma separated list, ''all'' for all steps. Without it the default steps are applied')
            [CompletionResult]::new('--skip-steps', '--skip-steps', [CompletionResultType]::ParameterName, 'Skip these processing steps of the rendering, as comma separated list (e.g. tonecurve,denoise,sharpen). ''all'' skips every step and writes the raw values of the whole raw image. Takes priority over --steps')
            [CompletionResult]::new('-d', '-d', [CompletionResultType]::ParameterName, 'Log level')
            [CompletionResult]::new('--loglevel', '--loglevel', [CompletionResultType]::ParameterName, 'Log level')
            [CompletionResult]::new('--profile-path', '--profile-path', [CompletionResultType]::ParameterName, 'Directory with profile TOML files')
            [CompletionResult]::new('--config', '--config', [CompletionResultType]::ParameterName, 'dnglab configuration file')
            [CompletionResult]::new('--crop-transparent', '--crop-transparent', [CompletionResultType]::ParameterName, 'Crop to the largest rectangle without transparent pixels of the DNG transparency mask')
            [CompletionResult]::new('--auto-exposure', '--auto-exposure', [CompletionResultType]::ParameterName, 'Correct the exposure by an estimate from the image, --exposure is added to it')
            [CompletionResult]::new('-f', '-f', [CompletionResultType]::ParameterName, 'Override existing files')
            [CompletionResult]::new('--override', '--override', [CompletionResultType]::ParameterName, 'Override existing files')
            [CompletionResult]::new('-r', '-r', [CompletionResultType]::ParameterName, 'Process input directory recursive')
            [CompletionResult]::new('--recursive', '--recursive', [CompletionResultType]::ParameterName, 'Process input directory recursive')
            [CompletionResult]::new('-v', '-v', [CompletionResultType]::ParameterName, 'Print status for every file')
            [CompletionResult]::new('-h', '-h', [CompletionResultType]::ParameterName, 'Print help (see more with ''--help'')')
            [CompletionResult]::new('--help', '--help', [CompletionResultType]::ParameterName, 'Print help (see more with ''--help'')')
            break
        }
        'dnglab;convert' {
            [CompletionResult]::new('-p', '-p', [CompletionResultType]::ParameterName, 'Convert profile name')
            [CompletionResult]::new('--profile', '--profile', [CompletionResultType]::ParameterName, 'Convert profile name')
            [CompletionResult]::new('-c', '-c', [CompletionResultType]::ParameterName, 'Compression for raw image')
            [CompletionResult]::new('--compression', '--compression', [CompletionResultType]::ParameterName, 'Compression for raw image')
            [CompletionResult]::new('--ljpeg92-predictor', '--ljpeg92-predictor', [CompletionResultType]::ParameterName, 'LJPEG-92 predictor')
            [CompletionResult]::new('--lossy-jpeg-quality', '--lossy-jpeg-quality', [CompletionResultType]::ParameterName, 'JPEG quality for lossy compression (1-100)')
            [CompletionResult]::new('--jxl-distance', '--jxl-distance', [CompletionResultType]::ParameterName, 'JPEG XL distance (0-25), 0 is lossless')
            [CompletionResult]::new('--jxl-effort', '--jxl-effort', [CompletionResultType]::ParameterName, 'JPEG XL encoder effort (1-9)')
            [CompletionResult]::new('--jxl-decode-speed', '--jxl-decode-speed', [CompletionResultType]::ParameterName, 'JPEG XL decoding speed for lossy compression (1-4)')
            [CompletionResult]::new('--dng-preview', '--dng-preview', [CompletionResultType]::ParameterName, 'DNG include preview image')
            [CompletionResult]::new('--dng-thumbnail', '--dng-thumbnail', [CompletionResultType]::ParameterName, 'DNG include thumbnail image')
            [CompletionResult]::new('--dng-force-preview', '--dng-force-preview', [CompletionResultType]::ParameterName, 'Generate DNG preview image from raw data')
            [CompletionResult]::new('--dng-force-thumbnail', '--dng-force-thumbnail', [CompletionResultType]::ParameterName, 'Generate DNG thumbnail image from raw data')
            [CompletionResult]::new('--max-dng-backward-version', '--max-dng-backward-version', [CompletionResultType]::ParameterName, 'Maximum DNG backward version')
            [CompletionResult]::new('--embed-raw', '--embed-raw', [CompletionResultType]::ParameterName, 'Embed the raw file into DNG')
            [CompletionResult]::new('--apply-dng-opcodes', '--apply-dng-opcodes', [CompletionResultType]::ParameterName, 'Apply DNG opcode lists to the image data instead of writing them to the DNG')
            [CompletionResult]::new('--remove-dng-opcodes', '--remove-dng-opcodes', [CompletionResultType]::ParameterName, 'Remove DNG opcodes, as comma separated [<list>:]<opcode> (e.g. WarpRectilinear,2:GainMap)')
            [CompletionResult]::new('--artist', '--artist', [CompletionResultType]::ParameterName, 'Set the artist tag')
            [CompletionResult]::new('--copyright', '--copyright', [CompletionResultType]::ParameterName, 'Set the copyright tag')
            [CompletionResult]::new('--keep-mtime', '--keep-mtime', [CompletionResultType]::ParameterName, 'Keep mtime, read from EXIF with fallback to original file mtime')
            [CompletionResult]::new('--image-index', '--image-index', [CompletionResultType]::ParameterName, 'Select a specific image index (or ''all'') if file is a image container')
            [CompletionResult]::new('--crop', '--crop', [CompletionResultType]::ParameterName, 'DNG default crop')
            [CompletionResult]::new('-j', '-j', [CompletionResultType]::ParameterName, 'Number of files to convert in parallel (0 = auto)')
            [CompletionResult]::new('--jobs', '--jobs', [CompletionResultType]::ParameterName, 'Number of files to convert in parallel (0 = auto)')
            [CompletionResult]::new('--dcp-file', '--dcp-file', [CompletionResultType]::ParameterName, 'Camera profile (DCP) that replaces the primary camera profile')
            [CompletionResult]::new('--extra-dcp-file', '--extra-dcp-file', [CompletionResultType]::ParameterName, 'Camera profile (DCP) to embed as extra camera profile, can be repeated')
            [CompletionResult]::new('--bad-pixels', '--bad-pixels', [CompletionResultType]::ParameterName, 'Bad pixel list in the format of dcraw: column, row and UNIX time per line, relative to the ActiveArea. It is written as FixBadPixelsList to OpcodeList1 and replaces an existing one')
            [CompletionResult]::new('-d', '-d', [CompletionResultType]::ParameterName, 'Log level')
            [CompletionResult]::new('--loglevel', '--loglevel', [CompletionResultType]::ParameterName, 'Log level')
            [CompletionResult]::new('--profile-path', '--profile-path', [CompletionResultType]::ParameterName, 'Directory with profile TOML files')
            [CompletionResult]::new('--config', '--config', [CompletionResultType]::ParameterName, 'dnglab configuration file')
            [CompletionResult]::new('-f', '-f', [CompletionResultType]::ParameterName, 'Override existing files')
            [CompletionResult]::new('--override', '--override', [CompletionResultType]::ParameterName, 'Override existing files')
            [CompletionResult]::new('-r', '-r', [CompletionResultType]::ParameterName, 'Process input directory recursive')
            [CompletionResult]::new('--recursive', '--recursive', [CompletionResultType]::ParameterName, 'Process input directory recursive')
            [CompletionResult]::new('-v', '-v', [CompletionResultType]::ParameterName, 'Print status for every file')
            [CompletionResult]::new('-h', '-h', [CompletionResultType]::ParameterName, 'Print help (see more with ''--help'')')
            [CompletionResult]::new('--help', '--help', [CompletionResultType]::ParameterName, 'Print help (see more with ''--help'')')
            break
        }
        'dnglab;ftpserver' {
            [CompletionResult]::new('-p', '-p', [CompletionResultType]::ParameterName, 'Convert profile name')
            [CompletionResult]::new('--profile', '--profile', [CompletionResultType]::ParameterName, 'Convert profile name')
            [CompletionResult]::new('-c', '-c', [CompletionResultType]::ParameterName, 'Compression for raw image')
            [CompletionResult]::new('--compression', '--compression', [CompletionResultType]::ParameterName, 'Compression for raw image')
            [CompletionResult]::new('--ljpeg92-predictor', '--ljpeg92-predictor', [CompletionResultType]::ParameterName, 'LJPEG-92 predictor')
            [CompletionResult]::new('--lossy-jpeg-quality', '--lossy-jpeg-quality', [CompletionResultType]::ParameterName, 'JPEG quality for lossy compression (1-100)')
            [CompletionResult]::new('--jxl-distance', '--jxl-distance', [CompletionResultType]::ParameterName, 'JPEG XL distance (0-25), 0 is lossless')
            [CompletionResult]::new('--jxl-effort', '--jxl-effort', [CompletionResultType]::ParameterName, 'JPEG XL encoder effort (1-9)')
            [CompletionResult]::new('--jxl-decode-speed', '--jxl-decode-speed', [CompletionResultType]::ParameterName, 'JPEG XL decoding speed for lossy compression (1-4)')
            [CompletionResult]::new('--dng-preview', '--dng-preview', [CompletionResultType]::ParameterName, 'DNG include preview image')
            [CompletionResult]::new('--dng-thumbnail', '--dng-thumbnail', [CompletionResultType]::ParameterName, 'DNG include thumbnail image')
            [CompletionResult]::new('--dng-force-preview', '--dng-force-preview', [CompletionResultType]::ParameterName, 'Generate DNG preview image from raw data')
            [CompletionResult]::new('--dng-force-thumbnail', '--dng-force-thumbnail', [CompletionResultType]::ParameterName, 'Generate DNG thumbnail image from raw data')
            [CompletionResult]::new('--max-dng-backward-version', '--max-dng-backward-version', [CompletionResultType]::ParameterName, 'Maximum DNG backward version')
            [CompletionResult]::new('--embed-raw', '--embed-raw', [CompletionResultType]::ParameterName, 'Embed the raw file into DNG')
            [CompletionResult]::new('--apply-dng-opcodes', '--apply-dng-opcodes', [CompletionResultType]::ParameterName, 'Apply DNG opcode lists to the image data instead of writing them to the DNG')
            [CompletionResult]::new('--remove-dng-opcodes', '--remove-dng-opcodes', [CompletionResultType]::ParameterName, 'Remove DNG opcodes, as comma separated [<list>:]<opcode> (e.g. WarpRectilinear,2:GainMap)')
            [CompletionResult]::new('--artist', '--artist', [CompletionResultType]::ParameterName, 'Set the artist tag')
            [CompletionResult]::new('--copyright', '--copyright', [CompletionResultType]::ParameterName, 'Set the copyright tag')
            [CompletionResult]::new('--keep-mtime', '--keep-mtime', [CompletionResultType]::ParameterName, 'Keep mtime, read from EXIF with fallback to original file mtime')
            [CompletionResult]::new('--image-index', '--image-index', [CompletionResultType]::ParameterName, 'Select a specific image index (or ''all'') if file is a image container')
            [CompletionResult]::new('--crop', '--crop', [CompletionResultType]::ParameterName, 'DNG default crop')
            [CompletionResult]::new('--port', '--port', [CompletionResultType]::ParameterName, 'FTP listen port')
            [CompletionResult]::new('--listen', '--listen', [CompletionResultType]::ParameterName, 'FTP listen address')
            [CompletionResult]::new('--user', '--user', [CompletionResultType]::ParameterName, 'FTP user name')
            [CompletionResult]::new('--password', '--password', [CompletionResultType]::ParameterName, 'FTP password')
            [CompletionResult]::new('--max-upload-size', '--max-upload-size', [CompletionResultType]::ParameterName, 'Maximum size of an uploaded file in MiB')
            [CompletionResult]::new('--keep-original', '--keep-original', [CompletionResultType]::ParameterName, 'Keep original raw')
            [CompletionResult]::new('-d', '-d', [CompletionResultType]::ParameterName, 'Log level')
            [CompletionResult]::new('--loglevel', '--loglevel', [CompletionResultType]::ParameterName, 'Log level')
            [CompletionResult]::new('--profile-path', '--profile-path', [CompletionResultType]::ParameterName, 'Directory with profile TOML files')
            [CompletionResult]::new('--config', '--config', [CompletionResultType]::ParameterName, 'dnglab configuration file')
            [CompletionResult]::new('-f', '-f', [CompletionResultType]::ParameterName, 'Override existing files')
            [CompletionResult]::new('--override', '--override', [CompletionResultType]::ParameterName, 'Override existing files')
            [CompletionResult]::new('-v', '-v', [CompletionResultType]::ParameterName, 'Print status for every file')
            [CompletionResult]::new('-h', '-h', [CompletionResultType]::ParameterName, 'Print help (see more with ''--help'')')
            [CompletionResult]::new('--help', '--help', [CompletionResultType]::ParameterName, 'Print help (see more with ''--help'')')
            break
        }
        'dnglab;cameras' {
            [CompletionResult]::new('-d', '-d', [CompletionResultType]::ParameterName, 'Log level')
            [CompletionResult]::new('--loglevel', '--loglevel', [CompletionResultType]::ParameterName, 'Log level')
            [CompletionResult]::new('--profile-path', '--profile-path', [CompletionResultType]::ParameterName, 'Directory with profile TOML files')
            [CompletionResult]::new('--config', '--config', [CompletionResultType]::ParameterName, 'dnglab configuration file')
            [CompletionResult]::new('--md', '--md', [CompletionResultType]::ParameterName, 'Markdown format output')
            [CompletionResult]::new('-v', '-v', [CompletionResultType]::ParameterName, 'Print status for every file')
            [CompletionResult]::new('-h', '-h', [CompletionResultType]::ParameterName, 'Print help (see more with ''--help'')')
            [CompletionResult]::new('--help', '--help', [CompletionResultType]::ParameterName, 'Print help (see more with ''--help'')')
            break
        }
        'dnglab;lenses' {
            [CompletionResult]::new('-d', '-d', [CompletionResultType]::ParameterName, 'Log level')
            [CompletionResult]::new('--loglevel', '--loglevel', [CompletionResultType]::ParameterName, 'Log level')
            [CompletionResult]::new('--profile-path', '--profile-path', [CompletionResultType]::ParameterName, 'Directory with profile TOML files')
            [CompletionResult]::new('--config', '--config', [CompletionResultType]::ParameterName, 'dnglab configuration file')
            [CompletionResult]::new('--md', '--md', [CompletionResultType]::ParameterName, 'Markdown format output')
            [CompletionResult]::new('-v', '-v', [CompletionResultType]::ParameterName, 'Print status for every file')
            [CompletionResult]::new('-h', '-h', [CompletionResultType]::ParameterName, 'Print help (see more with ''--help'')')
            [CompletionResult]::new('--help', '--help', [CompletionResultType]::ParameterName, 'Print help (see more with ''--help'')')
            break
        }
        'dnglab;makedng' {
            [CompletionResult]::new('-o', '-o', [CompletionResultType]::ParameterName, 'Output DNG file path')
            [CompletionResult]::new('--output', '--output', [CompletionResultType]::ParameterName, 'Output DNG file path')
            [CompletionResult]::new('-i', '-i', [CompletionResultType]::ParameterName, 'Input files (raw, preview, exif, ...), index for map starts with 0')
            [CompletionResult]::new('--input', '--input', [CompletionResultType]::ParameterName, 'Input files (raw, preview, exif, ...), index for map starts with 0')
            [CompletionResult]::new('--map', '--map', [CompletionResultType]::ParameterName, 'Input usage map')
            [CompletionResult]::new('--dng-backward-version', '--dng-backward-version', [CompletionResultType]::ParameterName, 'DNG specification version')
            [CompletionResult]::new('--colorimetric-reference', '--colorimetric-reference', [CompletionResultType]::ParameterName, 'Reference for XYZ values')
            [CompletionResult]::new('--unique-camera-model', '--unique-camera-model', [CompletionResultType]::ParameterName, 'Unique camera model')
            [CompletionResult]::new('--artist', '--artist', [CompletionResultType]::ParameterName, 'Set the Artist tag')
            [CompletionResult]::new('--make', '--make', [CompletionResultType]::ParameterName, 'Set the Make tag')
            [CompletionResult]::new('--model', '--model', [CompletionResultType]::ParameterName, 'Set the Model tag')
            [CompletionResult]::new('--matrix1', '--matrix1', [CompletionResultType]::ParameterName, 'Matrix 1')
            [CompletionResult]::new('--matrix2', '--matrix2', [CompletionResultType]::ParameterName, 'Matrix 2')
            [CompletionResult]::new('--matrix3', '--matrix3', [CompletionResultType]::ParameterName, 'Matrix 3')
            [CompletionResult]::new('--illuminant1', '--illuminant1', [CompletionResultType]::ParameterName, 'Illuminant 1')
            [CompletionResult]::new('--illuminant2', '--illuminant2', [CompletionResultType]::ParameterName, 'Illuminant 2')
            [CompletionResult]::new('--illuminant3', '--illuminant3', [CompletionResultType]::ParameterName, 'Illuminant 3')
            [CompletionResult]::new('--linearization', '--linearization', [CompletionResultType]::ParameterName, 'Linearization table')
            [CompletionResult]::new('--wb', '--wb', [CompletionResultType]::ParameterName, 'Whitebalance as-shot')
            [CompletionResult]::new('--white-xy', '--white-xy', [CompletionResultType]::ParameterName, 'Whitebalance as-shot encoded as xy chromaticity coordinates')
            [CompletionResult]::new('-d', '-d', [CompletionResultType]::ParameterName, 'Log level')
            [CompletionResult]::new('--loglevel', '--loglevel', [CompletionResultType]::ParameterName, 'Log level')
            [CompletionResult]::new('--profile-path', '--profile-path', [CompletionResultType]::ParameterName, 'Directory with profile TOML files')
            [CompletionResult]::new('--config', '--config', [CompletionResultType]::ParameterName, 'dnglab configuration file')
            [CompletionResult]::new('-f', '-f', [CompletionResultType]::ParameterName, 'Override existing files')
            [CompletionResult]::new('--override', '--override', [CompletionResultType]::ParameterName, 'Override existing files')
            [CompletionResult]::new('-v', '-v', [CompletionResultType]::ParameterName, 'Print status for every file')
            [CompletionResult]::new('-h', '-h', [CompletionResultType]::ParameterName, 'Print help (see more with ''--help'')')
            [CompletionResult]::new('--help', '--help', [CompletionResultType]::ParameterName, 'Print help (see more with ''--help'')')
            break
        }
        'dnglab;gui' {
            [CompletionResult]::new('-d', '-d', [CompletionResultType]::ParameterName, 'Log level')
            [CompletionResult]::new('--loglevel', '--loglevel', [CompletionResultType]::ParameterName, 'Log level')
            [CompletionResult]::new('--profile-path', '--profile-path', [CompletionResultType]::ParameterName, 'Directory with profile TOML files')
            [CompletionResult]::new('--config', '--config', [CompletionResultType]::ParameterName, 'dnglab configuration file')
            [CompletionResult]::new('-v', '-v', [CompletionResultType]::ParameterName, 'Print status for every file')
            [CompletionResult]::new('-h', '-h', [CompletionResultType]::ParameterName, 'Print help (see more with ''--help'')')
            [CompletionResult]::new('--help', '--help', [CompletionResultType]::ParameterName, 'Print help (see more with ''--help'')')
            break
        }
        'dnglab;extract' {
            [CompletionResult]::new('-d', '-d', [CompletionResultType]::ParameterName, 'Log level')
            [CompletionResult]::new('--loglevel', '--loglevel', [CompletionResultType]::ParameterName, 'Log level')
            [CompletionResult]::new('--profile-path', '--profile-path', [CompletionResultType]::ParameterName, 'Directory with profile TOML files')
            [CompletionResult]::new('--config', '--config', [CompletionResultType]::ParameterName, 'dnglab configuration file')
            [CompletionResult]::new('--skipchecks', '--skipchecks', [CompletionResultType]::ParameterName, 'Skip integrity checks')
            [CompletionResult]::new('-r', '-r', [CompletionResultType]::ParameterName, 'Process input directory recursive')
            [CompletionResult]::new('--recursive', '--recursive', [CompletionResultType]::ParameterName, 'Process input directory recursive')
            [CompletionResult]::new('-f', '-f', [CompletionResultType]::ParameterName, 'Override existing files')
            [CompletionResult]::new('--override', '--override', [CompletionResultType]::ParameterName, 'Override existing files')
            [CompletionResult]::new('-v', '-v', [CompletionResultType]::ParameterName, 'Print status for every file')
            [CompletionResult]::new('-h', '-h', [CompletionResultType]::ParameterName, 'Print help (see more with ''--help'')')
            [CompletionResult]::new('--help', '--help', [CompletionResultType]::ParameterName, 'Print help (see more with ''--help'')')
            break
        }
        'dnglab;help' {
            [CompletionResult]::new('analyze', 'analyze', [CompletionResultType]::ParameterValue, 'Analyze raw image')
            [CompletionResult]::new('process-raw', 'process-raw', [CompletionResultType]::ParameterValue, 'process-raw')
            [CompletionResult]::new('convert', 'convert', [CompletionResultType]::ParameterValue, 'Convert raw image(s) into dng format')
            [CompletionResult]::new('ftpserver', 'ftpserver', [CompletionResultType]::ParameterValue, 'Convert raw image(s) into dng format')
            [CompletionResult]::new('cameras', 'cameras', [CompletionResultType]::ParameterValue, 'List supported cameras')
            [CompletionResult]::new('lenses', 'lenses', [CompletionResultType]::ParameterValue, 'List supported lenses')
            [CompletionResult]::new('makedng', 'makedng', [CompletionResultType]::ParameterValue, 'Lowlevel command to make a DNG file')
            [CompletionResult]::new('gui', 'gui', [CompletionResultType]::ParameterValue, 'Start GUI (not implemented)')
            [CompletionResult]::new('extract', 'extract', [CompletionResultType]::ParameterValue, 'Extract embedded original Raw from DNG')
            [CompletionResult]::new('help', 'help', [CompletionResultType]::ParameterValue, 'Print this message or the help of the given subcommand(s)')
            break
        }
        'dnglab;help;analyze' {
            break
        }
        'dnglab;help;process-raw' {
            break
        }
        'dnglab;help;convert' {
            break
        }
        'dnglab;help;ftpserver' {
            break
        }
        'dnglab;help;cameras' {
            break
        }
        'dnglab;help;lenses' {
            break
        }
        'dnglab;help;makedng' {
            break
        }
        'dnglab;help;gui' {
            break
        }
        'dnglab;help;extract' {
            break
        }
        'dnglab;help;help' {
            break
        }
    })

    $completions.Where{ $_.CompletionText -like "$wordToComplete*" } |
        Sort-Object -Property ListItemText
}
