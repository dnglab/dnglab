#!/bin/bash

# README
# You need to export $RAWDB to point to the rawdb.
if [[ -z "$RAWLER_RAWDB" ]]; then
    echo "Error: RAWLER_RAWDB environment variable is not set."
    exit 1
fi

RAWDB=$RAWLER_RAWDB

function process_rawfile() {
	rawtype=$1
    rawfile=$2
    echo $rawfile >&2;
	sample=$RAWDB/"$rawfile"
    analyze=${rawfile}".analyze.yaml"
    digest_raw=${rawfile}".digest.txt"
	digest_full=${rawfile}".digest.full.txt"
	digest_preview=${rawfile}".digest.preview.txt"
	digest_thumbnail=${rawfile}".digest.thumbnail.txt"

	full_path_analyze=rawler/data/testdata/"$analyze";
	mkdir -p "${full_path_analyze%/*}";
	#if [ ! -f rawler/data/testdata/"$analyze" ]; then
			echo "Processing ${rawfile}" >&2
			echo "  analyze file: $analyze" >&2
			echo "  digest file:  $digest_raw" >&2
			./target/release/dnglab analyze --meta --yaml "$sample" > rawler/data/testdata/"$analyze";
			./target/release/dnglab analyze --raw-checksum "$sample" > rawler/data/testdata/"$digest_raw";
	./target/release/dnglab analyze --full-checksum "$sample" > rawler/data/testdata/"$digest_full";
	./target/release/dnglab analyze --preview-checksum "$sample" > rawler/data/testdata/"$digest_preview";
	./target/release/dnglab analyze --thumbnail-checksum "$sample" > rawler/data/testdata/"$digest_thumbnail";
	#fi
	if [ "$rawtype" == "cameras" ]; then
	        MAKE=`echo $rawfile | cut -d/ -f2`;
	        MODEL=`echo $rawfile | cut -d/ -f3`;
	        TESTNAME=`basename "${rawfile@L}" | sed -e 's,[[:space:][:punct:]],_,g' -e 's,_+,_,g'`;
		echo -e "\tsuper::camera_file_check!(\"$MAKE\", \"$MODEL\", "cam_"$TESTNAME, \"`echo $rawfile | cut -d'/' -f4-`\");";
	else
		SAMPLESET=`echo $rawfile | cut -d/ -f2`;
		TESTNAME=`basename "${rawfile@L}" | sed -e 's,[[:space:][:punct:]],_,g' -e 's,_+,_,g'`;
		echo -e "\tsuper::sample_file_check!(\"$SAMPLESET\", "sample_"$TESTNAME, \"`echo $rawfile | cut -d'/' -f3-`\");";
	fi
}

function process_camera_set() {
	setdir=$1
	echo "Processing: $setdir" >&2;
	modname="camera_"`echo $setdir | cut -d'/' -f3- | sed -e 's/\+/plus/g' | sed -e 's,[^[:alnum:]]\+,_,g'`;
	echo "mod ${modname@L} {";
	find "$RAWDB/$setdir" -type f -not -name "*.txt" -not -name "*.xmp" -exec realpath --relative-to $RAWDB '{}' \; | while read rawfile; do
		process_rawfile "cameras" "$rawfile";
	done;
	echo "}";
}

function process_sample_set() {
	setdir=$1
    echo "Processing: $setdir" >&2;
    modname=`echo $setdir | cut -d'/' -f2- | sed -e 's/\+/plus/g' | sed -e 's,[^[:alnum:]]\+,_,g'`;
	echo "mod ${modname@L} {";
	find "$RAWDB/$setdir" -type f -not -name "*.txt" -not -name "*.xmp" -exec realpath --relative-to $RAWDB '{}' \; | while read rawfile; do
			process_rawfile "samples" "$rawfile";
	done;
	echo "}";
}


cargo build --release;

export -f process_rawfile
export -f process_camera_set
export -f process_sample_set


echo "use crate::common::camera_file_check;" > "rawler/tests/cameras/mod.rs";
cat rawler/tests/supported_rawdb_sets.txt | grep -v "^$" | parallel -k process_camera_set >> "rawler/tests/cameras/mod.rs";
echo "" >> "rawler/tests/cameras/mod.rs";

echo "use crate::common::sample_file_check;" > "rawler/tests/samples/mod.rs";
cat rawler/tests/supported_sample_sets.txt | grep -v "^$" | parallel -k process_sample_set >> "rawler/tests/samples/mod.rs";
echo "" >> "rawler/tests/samples/mod.rs";


cargo fmt;

