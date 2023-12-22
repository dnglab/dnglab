
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
            [CompletionResult]::new('-d', 'd', [CompletionResultType]::ParameterName, 'turns on debugging mode')
            [CompletionResult]::new('-v', 'v', [CompletionResultType]::ParameterName, 'Print more messages')
            [CompletionResult]::new('-h', 'h', [CompletionResultType]::ParameterName, 'Print help')
            [CompletionResult]::new('--help', 'help', [CompletionResultType]::ParameterName, 'Print help')
            [CompletionResult]::new('-V', 'V ', [CompletionResultType]::ParameterName, 'Print version')
            [CompletionResult]::new('--version', 'version', [CompletionResultType]::ParameterName, 'Print version')
            [CompletionResult]::new('analyze', 'analyze', [CompletionResultType]::ParameterValue, 'Analyze raw image')
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
            [CompletionResult]::new('--raw-pixel', 'raw-pixel', [CompletionResultType]::ParameterName, 'raw-pixel')
            [CompletionResult]::new('--full-pixel', 'full-pixel', [CompletionResultType]::ParameterName, 'Write uncompressed full pixel data to STDOUT')
            [CompletionResult]::new('--preview-pixel', 'preview-pixel', [CompletionResultType]::ParameterName, 'Write uncompressed preview pixel data to STDOUT')
            [CompletionResult]::new('--thumbnail-pixel', 'thumbnail-pixel', [CompletionResultType]::ParameterName, 'Write uncompressed preview pixel data to STDOUT')
            [CompletionResult]::new('--raw-checksum', 'raw-checksum', [CompletionResultType]::ParameterName, 'Write MD5 checksum of raw pixels to STDOUT')
            [CompletionResult]::new('--preview-checksum', 'preview-checksum', [CompletionResultType]::ParameterName, 'Write MD5 checksum of preview pixels to STDOUT')
            [CompletionResult]::new('--thumbnail-checksum', 'thumbnail-checksum', [CompletionResultType]::ParameterName, 'Write MD5 checksum of thumbnail pixels to STDOUT')
            [CompletionResult]::new('--srgb', 'srgb', [CompletionResultType]::ParameterName, 'Write sRGB 16-bit TIFF to STDOUT')
            [CompletionResult]::new('--meta', 'meta', [CompletionResultType]::ParameterName, 'Write metadata to STDOUT')
            [CompletionResult]::new('--structure', 'structure', [CompletionResultType]::ParameterName, 'Write file structure to STDOUT')
            [CompletionResult]::new('--summary', 'summary', [CompletionResultType]::ParameterName, 'Write summary information for file to STDOUT')
            [CompletionResult]::new('--json', 'json', [CompletionResultType]::ParameterName, 'Format metadata as JSON')
            [CompletionResult]::new('--yaml', 'yaml', [CompletionResultType]::ParameterName, 'Format metadata as YAML')
            [CompletionResult]::new('-d', 'd', [CompletionResultType]::ParameterName, 'turns on debugging mode')
            [CompletionResult]::new('-v', 'v', [CompletionResultType]::ParameterName, 'Print more messages')
            [CompletionResult]::new('-h', 'h', [CompletionResultType]::ParameterName, 'Print help')
            [CompletionResult]::new('--help', 'help', [CompletionResultType]::ParameterName, 'Print help')
            break
        }
        'dnglab;convert' {
            [CompletionResult]::new('-c', 'c', [CompletionResultType]::ParameterName, 'Compression for raw image')
            [CompletionResult]::new('--compression', 'compression', [CompletionResultType]::ParameterName, 'Compression for raw image')
            [CompletionResult]::new('--ljpeg92-predictor', 'ljpeg92-predictor', [CompletionResultType]::ParameterName, 'LJPEG-92 predictor')
            [CompletionResult]::new('--dng-preview', 'dng-preview', [CompletionResultType]::ParameterName, 'DNG include preview image')
            [CompletionResult]::new('--dng-thumbnail', 'dng-thumbnail', [CompletionResultType]::ParameterName, 'DNG include thumbnail image')
            [CompletionResult]::new('--embed-raw', 'embed-raw', [CompletionResultType]::ParameterName, 'Embed the raw file into DNG')
            [CompletionResult]::new('--artist', 'artist', [CompletionResultType]::ParameterName, 'Set the artist tag')
            [CompletionResult]::new('--image-index', 'image-index', [CompletionResultType]::ParameterName, 'Select a specific image index (or ''all'') if file is a image container')
            [CompletionResult]::new('--crop', 'crop', [CompletionResultType]::ParameterName, 'DNG default crop')
            [CompletionResult]::new('-f', 'f', [CompletionResultType]::ParameterName, 'Override existing files')
            [CompletionResult]::new('--override', 'override', [CompletionResultType]::ParameterName, 'Override existing files')
            [CompletionResult]::new('-r', 'r', [CompletionResultType]::ParameterName, 'Process input directory recursive')
            [CompletionResult]::new('--recursive', 'recursive', [CompletionResultType]::ParameterName, 'Process input directory recursive')
            [CompletionResult]::new('-d', 'd', [CompletionResultType]::ParameterName, 'turns on debugging mode')
            [CompletionResult]::new('-v', 'v', [CompletionResultType]::ParameterName, 'Print more messages')
            [CompletionResult]::new('-h', 'h', [CompletionResultType]::ParameterName, 'Print help')
            [CompletionResult]::new('--help', 'help', [CompletionResultType]::ParameterName, 'Print help')
            break
        }
        'dnglab;ftpserver' {
            [CompletionResult]::new('-c', 'c', [CompletionResultType]::ParameterName, 'Compression for raw image')
            [CompletionResult]::new('--compression', 'compression', [CompletionResultType]::ParameterName, 'Compression for raw image')
            [CompletionResult]::new('--ljpeg92-predictor', 'ljpeg92-predictor', [CompletionResultType]::ParameterName, 'LJPEG-92 predictor')
            [CompletionResult]::new('--dng-preview', 'dng-preview', [CompletionResultType]::ParameterName, 'DNG include preview image')
            [CompletionResult]::new('--dng-thumbnail', 'dng-thumbnail', [CompletionResultType]::ParameterName, 'DNG include thumbnail image')
            [CompletionResult]::new('--embed-raw', 'embed-raw', [CompletionResultType]::ParameterName, 'Embed the raw file into DNG')
            [CompletionResult]::new('--artist', 'artist', [CompletionResultType]::ParameterName, 'Set the artist tag')
            [CompletionResult]::new('--image-index', 'image-index', [CompletionResultType]::ParameterName, 'Select a specific image index (or ''all'') if file is a image container')
            [CompletionResult]::new('--crop', 'crop', [CompletionResultType]::ParameterName, 'DNG default crop')
            [CompletionResult]::new('--port', 'port', [CompletionResultType]::ParameterName, 'FTP listen port')
            [CompletionResult]::new('--listen', 'listen', [CompletionResultType]::ParameterName, 'FTP listen address')
            [CompletionResult]::new('--keep-original', 'keep-original', [CompletionResultType]::ParameterName, 'Keep original raw')
            [CompletionResult]::new('-f', 'f', [CompletionResultType]::ParameterName, 'Override existing files')
            [CompletionResult]::new('--override', 'override', [CompletionResultType]::ParameterName, 'Override existing files')
            [CompletionResult]::new('-d', 'd', [CompletionResultType]::ParameterName, 'turns on debugging mode')
            [CompletionResult]::new('-v', 'v', [CompletionResultType]::ParameterName, 'Print more messages')
            [CompletionResult]::new('-h', 'h', [CompletionResultType]::ParameterName, 'Print help')
            [CompletionResult]::new('--help', 'help', [CompletionResultType]::ParameterName, 'Print help')
            break
        }
        'dnglab;cameras' {
            [CompletionResult]::new('--md', 'md', [CompletionResultType]::ParameterName, 'Markdown format output')
            [CompletionResult]::new('-d', 'd', [CompletionResultType]::ParameterName, 'turns on debugging mode')
            [CompletionResult]::new('-v', 'v', [CompletionResultType]::ParameterName, 'Print more messages')
            [CompletionResult]::new('-h', 'h', [CompletionResultType]::ParameterName, 'Print help')
            [CompletionResult]::new('--help', 'help', [CompletionResultType]::ParameterName, 'Print help')
            break
        }
        'dnglab;lenses' {
            [CompletionResult]::new('--md', 'md', [CompletionResultType]::ParameterName, 'Markdown format output')
            [CompletionResult]::new('-d', 'd', [CompletionResultType]::ParameterName, 'turns on debugging mode')
            [CompletionResult]::new('-v', 'v', [CompletionResultType]::ParameterName, 'Print more messages')
            [CompletionResult]::new('-h', 'h', [CompletionResultType]::ParameterName, 'Print help')
            [CompletionResult]::new('--help', 'help', [CompletionResultType]::ParameterName, 'Print help')
            break
        }
        'dnglab;makedng' {
            [CompletionResult]::new('-o', 'o', [CompletionResultType]::ParameterName, 'Output DNG file path')
            [CompletionResult]::new('--output', 'output', [CompletionResultType]::ParameterName, 'Output DNG file path')
            [CompletionResult]::new('-i', 'i', [CompletionResultType]::ParameterName, 'Input files (raw, preview, exif, ...), index for map starts with 0')
            [CompletionResult]::new('--input', 'input', [CompletionResultType]::ParameterName, 'Input files (raw, preview, exif, ...), index for map starts with 0')
            [CompletionResult]::new('--map', 'map', [CompletionResultType]::ParameterName, 'Input usage map')
            [CompletionResult]::new('--dng-backward-version', 'dng-backward-version', [CompletionResultType]::ParameterName, 'DNG specification version')
            [CompletionResult]::new('--colorimetric-reference', 'colorimetric-reference', [CompletionResultType]::ParameterName, 'Reference for XYZ values')
            [CompletionResult]::new('--unique-camera-model', 'unique-camera-model', [CompletionResultType]::ParameterName, 'Unique camera model')
            [CompletionResult]::new('--artist', 'artist', [CompletionResultType]::ParameterName, 'Set the Artist tag')
            [CompletionResult]::new('--make', 'make', [CompletionResultType]::ParameterName, 'Set the Make tag')
            [CompletionResult]::new('--model', 'model', [CompletionResultType]::ParameterName, 'Set the Model tag')
            [CompletionResult]::new('--matrix1', 'matrix1', [CompletionResultType]::ParameterName, 'Matrix 1')
            [CompletionResult]::new('--matrix2', 'matrix2', [CompletionResultType]::ParameterName, 'Matrix 2')
            [CompletionResult]::new('--matrix3', 'matrix3', [CompletionResultType]::ParameterName, 'Matrix 3')
            [CompletionResult]::new('--illuminant1', 'illuminant1', [CompletionResultType]::ParameterName, 'Illuminant 1')
            [CompletionResult]::new('--illuminant2', 'illuminant2', [CompletionResultType]::ParameterName, 'Illuminant 2')
            [CompletionResult]::new('--illuminant3', 'illuminant3', [CompletionResultType]::ParameterName, 'Illuminant 3')
            [CompletionResult]::new('--linearization', 'linearization', [CompletionResultType]::ParameterName, 'Linearization table')
            [CompletionResult]::new('--wb', 'wb', [CompletionResultType]::ParameterName, 'Whitebalance as-shot')
            [CompletionResult]::new('--white-xy', 'white-xy', [CompletionResultType]::ParameterName, 'Whitebalance as-shot encoded as xy chromaticity coordinates')
            [CompletionResult]::new('-f', 'f', [CompletionResultType]::ParameterName, 'Override existing files')
            [CompletionResult]::new('--override', 'override', [CompletionResultType]::ParameterName, 'Override existing files')
            [CompletionResult]::new('-d', 'd', [CompletionResultType]::ParameterName, 'turns on debugging mode')
            [CompletionResult]::new('-v', 'v', [CompletionResultType]::ParameterName, 'Print more messages')
            [CompletionResult]::new('-h', 'h', [CompletionResultType]::ParameterName, 'Print help (see more with ''--help'')')
            [CompletionResult]::new('--help', 'help', [CompletionResultType]::ParameterName, 'Print help (see more with ''--help'')')
            break
        }
        'dnglab;gui' {
            [CompletionResult]::new('-d', 'd', [CompletionResultType]::ParameterName, 'turns on debugging mode')
            [CompletionResult]::new('-v', 'v', [CompletionResultType]::ParameterName, 'Print more messages')
            [CompletionResult]::new('-h', 'h', [CompletionResultType]::ParameterName, 'Print help')
            [CompletionResult]::new('--help', 'help', [CompletionResultType]::ParameterName, 'Print help')
            break
        }
        'dnglab;extract' {
            [CompletionResult]::new('--skipchecks', 'skipchecks', [CompletionResultType]::ParameterName, 'Skip integrity checks')
            [CompletionResult]::new('-r', 'r', [CompletionResultType]::ParameterName, 'Process input directory recursive')
            [CompletionResult]::new('--recursive', 'recursive', [CompletionResultType]::ParameterName, 'Process input directory recursive')
            [CompletionResult]::new('-f', 'f', [CompletionResultType]::ParameterName, 'Override existing files')
            [CompletionResult]::new('--override', 'override', [CompletionResultType]::ParameterName, 'Override existing files')
            [CompletionResult]::new('-d', 'd', [CompletionResultType]::ParameterName, 'turns on debugging mode')
            [CompletionResult]::new('-v', 'v', [CompletionResultType]::ParameterName, 'Print more messages')
            [CompletionResult]::new('-h', 'h', [CompletionResultType]::ParameterName, 'Print help')
            [CompletionResult]::new('--help', 'help', [CompletionResultType]::ParameterName, 'Print help')
            break
        }
        'dnglab;help' {
            [CompletionResult]::new('analyze', 'analyze', [CompletionResultType]::ParameterValue, 'Analyze raw image')
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
