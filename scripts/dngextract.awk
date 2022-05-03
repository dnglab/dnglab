#!/bin/gawk -f

# exiftool -S photo.DNG | awk -f dngextract.awk

function map_cfa_color(color) {
    switch(color) {
        case 0: return "R";
        case 1: return "G";
        case 2: return "B";
    }
}

function map_illu(name) {
    split(name, parts, ": ");
    if(parts[2] == "Standard Light A") return "A";
    return parts[2];
}

function parse_matrix(idx) {
    matrix[idx] = substr($2, 0, 7);
    if(matrix[idx] == "0") { matrix[idx] = "0.0"; }
    for(i = 3; i <= NF;++i) {
        tmp = substr($i, 0, 7);
        if(tmp == "0") tmp = "0.0";
        matrix[idx] = matrix[idx] ", " tmp;
    }
}

BEGIN {
    FS = " ";
    cleanmakes["RICOH IMAGING COMPANY, LTD."] = "Ricoh";
    cleanmakes["PENTAX"] = "Pentax";
}

/^Model:/{
    split($0, model, ": ");
}

/^Make:/{
    split($0, make, ": ");
}

/^ActiveArea:/{
    active_area[1] = $2;
    active_area[2] = $3;
    active_area[3] = $4;
    active_area[4] = $5;
}

/^DefaultCropOrigin:/{
    crop_area[1] = $2;
    crop_area[2] = $3;
}

/^DefaultCropSize:/{
    crop_area[3] = $2;
    crop_area[4] = $3;
}

/^BlackLevel:/{
    blacklevel = "[" $2;
    for(i = 3; i <= NF;++i) {
        blacklevel = blacklevel ", " $i;
    }
    blacklevel = blacklevel "]";
}

/^WhiteLevel:/{
    whitelevel = $2;
}

/^CFAPattern2:/{
    colorpattern = map_cfa_color($2) map_cfa_color($3) map_cfa_color($4) map_cfa_color($5);
}

/^CalibrationIlluminant1:/{
    illu[1] = map_illu($0);
}

/^CalibrationIlluminant2:/{
    illu[2] = map_illu($0);
}

/^ColorMatrix1:/{
    parse_matrix(1);
}

/^ColorMatrix2:/{
    parse_matrix(2);
}

/^ImageWidth:/{
    width = $2;
}

/^ImageHeight:/{
    height = $2;
}


END {
    printf("make = \"%s\"\n", make[2]);
    printf("model = \"%s\"\n", model[2]);
    if(cleanmakes[make[2]] == "") {
        printf("clean_make = \"%s\"\n", make[2]);
    } else {
        printf("clean_make = \"%s\"\n", cleanmakes[make[2]]);
    }
    printf("clean_model = \"%s\"\n", model[2]);
    if(blacklevel != "") printf("blackpoint = %s\n", blacklevel);
    if(whitelevel != "") printf("whitepoint = %s\n", whitelevel);
    if(colorpattern != "") printf("color_pattern = \"%s\"\n", colorpattern);

    if(height != 0 && length(active_area) == 4) {
        printf("active_area = [%d, %d, %d, %d]\n", active_area[2], active_area[1], width - active_area[4], height - active_area[3]);
    }
    if(height != 0 && length(crop_area) == 4 && length(active_area) == 4) {
        printf("crop_area = [%d, %d, %d, %d]\n", active_area[2] + crop_area[1], active_area[1] + crop_area[2],
            width - (active_area[2] + crop_area[1] + crop_area[3]),
            height - (active_area[1] + crop_area[2] + crop_area[4]));
    }

    printf("\n[cameras.color_matrix]\n");
    printf("%s = [%s]\n", illu[1], matrix[1]);
    printf("%s = [%s]\n", illu[2], matrix[2]);
}
