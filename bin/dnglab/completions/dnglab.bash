_dnglab() {
    local i cur prev opts cmd
    COMPREPLY=()
    cur="${COMP_WORDS[COMP_CWORD]}"
    prev="${COMP_WORDS[COMP_CWORD-1]}"
    cmd=""
    opts=""

    for i in ${COMP_WORDS[@]}
    do
        case "${cmd},${i}" in
            ",$1")
                cmd="dnglab"
                ;;
            dnglab,analyze)
                cmd="dnglab__analyze"
                ;;
            dnglab,cameras)
                cmd="dnglab__cameras"
                ;;
            dnglab,convert)
                cmd="dnglab__convert"
                ;;
            dnglab,extract)
                cmd="dnglab__extract"
                ;;
            dnglab,ftpserver)
                cmd="dnglab__ftpserver"
                ;;
            dnglab,gui)
                cmd="dnglab__gui"
                ;;
            dnglab,help)
                cmd="dnglab__help"
                ;;
            dnglab,lenses)
                cmd="dnglab__lenses"
                ;;
            dnglab,makedng)
                cmd="dnglab__makedng"
                ;;
            dnglab__help,analyze)
                cmd="dnglab__help__analyze"
                ;;
            dnglab__help,cameras)
                cmd="dnglab__help__cameras"
                ;;
            dnglab__help,convert)
                cmd="dnglab__help__convert"
                ;;
            dnglab__help,extract)
                cmd="dnglab__help__extract"
                ;;
            dnglab__help,ftpserver)
                cmd="dnglab__help__ftpserver"
                ;;
            dnglab__help,gui)
                cmd="dnglab__help__gui"
                ;;
            dnglab__help,help)
                cmd="dnglab__help__help"
                ;;
            dnglab__help,lenses)
                cmd="dnglab__help__lenses"
                ;;
            dnglab__help,makedng)
                cmd="dnglab__help__makedng"
                ;;
            *)
                ;;
        esac
    done

    case "${cmd}" in
        dnglab)
            opts="-d -v -h -V --help --version analyze convert ftpserver cameras lenses makedng gui extract help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 1 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        dnglab__analyze)
            opts="-d -v -h --raw-pixel --full-pixel --preview-pixel --thumbnail-pixel --raw-checksum --preview-checksum --thumbnail-checksum --srgb --meta --structure --summary --json --yaml --help <FILE>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        dnglab__cameras)
            opts="-d -v -h --md --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        dnglab__convert)
            opts="-c -f -r -d -v -h --compression --ljpeg92-predictor --dng-preview --dng-thumbnail --embed-raw --artist --image-index --crop --override --recursive --help <INPUT> <OUTPUT>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --compression)
                    COMPREPLY=($(compgen -W "lossless uncompressed" -- "${cur}"))
                    return 0
                    ;;
                -c)
                    COMPREPLY=($(compgen -W "lossless uncompressed" -- "${cur}"))
                    return 0
                    ;;
                --ljpeg92-predictor)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --dng-preview)
                    COMPREPLY=($(compgen -W "true false" -- "${cur}"))
                    return 0
                    ;;
                --dng-thumbnail)
                    COMPREPLY=($(compgen -W "true false" -- "${cur}"))
                    return 0
                    ;;
                --embed-raw)
                    COMPREPLY=($(compgen -W "true false" -- "${cur}"))
                    return 0
                    ;;
                --artist)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --image-index)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --crop)
                    COMPREPLY=($(compgen -W "best activearea none" -- "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        dnglab__extract)
            opts="-r -f -d -v -h --skipchecks --recursive --override --help <INPUT> <OUTPUT>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        dnglab__ftpserver)
            opts="-c -f -d -v -h --compression --ljpeg92-predictor --dng-preview --dng-thumbnail --embed-raw --artist --image-index --crop --override --port --listen --keep-original --help <OUTPUT>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --compression)
                    COMPREPLY=($(compgen -W "lossless uncompressed" -- "${cur}"))
                    return 0
                    ;;
                -c)
                    COMPREPLY=($(compgen -W "lossless uncompressed" -- "${cur}"))
                    return 0
                    ;;
                --ljpeg92-predictor)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --dng-preview)
                    COMPREPLY=($(compgen -W "true false" -- "${cur}"))
                    return 0
                    ;;
                --dng-thumbnail)
                    COMPREPLY=($(compgen -W "true false" -- "${cur}"))
                    return 0
                    ;;
                --embed-raw)
                    COMPREPLY=($(compgen -W "true false" -- "${cur}"))
                    return 0
                    ;;
                --artist)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --image-index)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --crop)
                    COMPREPLY=($(compgen -W "best activearea none" -- "${cur}"))
                    return 0
                    ;;
                --port)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --listen)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --keep-original)
                    COMPREPLY=($(compgen -W "true false" -- "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        dnglab__gui)
            opts="-d -v -h --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        dnglab__help)
            opts="analyze convert ftpserver cameras lenses makedng gui extract help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        dnglab__help__analyze)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        dnglab__help__cameras)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        dnglab__help__convert)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        dnglab__help__extract)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        dnglab__help__ftpserver)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        dnglab__help__gui)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        dnglab__help__help)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        dnglab__help__lenses)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        dnglab__help__makedng)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        dnglab__lenses)
            opts="-d -v -h --md --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        dnglab__makedng)
            opts="-o -i -f -d -v -h --output --input --map --dng-backward-version --colorimetric-reference --unique-camera-model --artist --make --model --matrix1 --matrix2 --matrix3 --illuminant1 --illuminant2 --illuminant3 --linearization --wb --white-xy --override --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --output)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                -o)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --input)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                -i)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --map)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --dng-backward-version)
                    COMPREPLY=($(compgen -W "1.0 1.1 1.2 1.3 1.4 1.5 1.6" -- "${cur}"))
                    return 0
                    ;;
                --colorimetric-reference)
                    COMPREPLY=($(compgen -W "scene output" -- "${cur}"))
                    return 0
                    ;;
                --unique-camera-model)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --artist)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --make)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --model)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --matrix1)
                    COMPREPLY=($(compgen -W "XYZ_sRGB_D50 XYZ_sRGB_D65 XYZ_AdobeRGB_D50 XYZ_AdobeRGB_D65 custom 3x3 matrix (comma seperated)" -- "${cur}"))
                    return 0
                    ;;
                --matrix2)
                    COMPREPLY=($(compgen -W "XYZ_sRGB_D50 XYZ_sRGB_D65 XYZ_AdobeRGB_D50 XYZ_AdobeRGB_D65 custom 3x3 matrix (comma seperated)" -- "${cur}"))
                    return 0
                    ;;
                --matrix3)
                    COMPREPLY=($(compgen -W "XYZ_sRGB_D50 XYZ_sRGB_D65 XYZ_AdobeRGB_D50 XYZ_AdobeRGB_D65 custom 3x3 matrix (comma seperated)" -- "${cur}"))
                    return 0
                    ;;
                --illuminant1)
                    COMPREPLY=($(compgen -W "Unknown A B C D50 D55 D65 D75" -- "${cur}"))
                    return 0
                    ;;
                --illuminant2)
                    COMPREPLY=($(compgen -W "Unknown A B C D50 D55 D65 D75" -- "${cur}"))
                    return 0
                    ;;
                --illuminant3)
                    COMPREPLY=($(compgen -W "Unknown A B C D50 D55 D65 D75" -- "${cur}"))
                    return 0
                    ;;
                --linearization)
                    COMPREPLY=($(compgen -W "8bit_sRGB 8bit_sRGB_invert 16bit_sRGB 16bit_sRGB_invert 8bit_gamma1.8 8bit_gamma1.8_invert 8bit_gamma2.0 8bit_gamma2.0_invert 8bit_gamma2.2 8bit_gamma2.2_invert 8bit_gamma2.4 8bit_gamma2.4_invert 16bit_gamma1.8 16bit_gamma1.8_invert 16bit_gamma2.0 16bit_gamma2.0_invert 16bit_gamma2.2 16bit_gamma2.2_invert 16bit_gamma2.4 16bit_gamma2.4_invert custom table (comma seperated)" -- "${cur}"))
                    return 0
                    ;;
                --wb)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --white-xy)
                    COMPREPLY=($(compgen -W "D50 D65 custom x,y value (comma seperated)" -- "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
    esac
}

if [[ "${BASH_VERSINFO[0]}" -eq 4 && "${BASH_VERSINFO[1]}" -ge 4 || "${BASH_VERSINFO[0]}" -gt 4 ]]; then
    complete -F _dnglab -o nosort -o bashdefault -o default dnglab
else
    complete -F _dnglab -o bashdefault -o default dnglab
fi
