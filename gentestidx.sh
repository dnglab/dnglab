#!/bin/bash

# README
# You need to export $RAWDB to point to the rawdb.

function process_rawfile() {
	rawtype=$1
        rawfile=$2
        echo $rawfile;
	sample=$RAWDB/"$rawfile"
        analyze=${rawfile}".analyze.yaml"
        digest=${rawfile}".digest.txt"
	full_path_analyze=rawler/data/testdata/"$analyze";
        mkdir -p "${full_path_analyze%/*}";
        if [ ! -f rawler/data/testdata/"$analyze" ]; then
                echo "Processing ${rawfile}";
                echo "  analyze file: $analyze"
                echo "  digest file:  $digest"
                ./target/release/dnglab analyze --meta --yaml "$sample" > rawler/data/testdata/"$analyze";
                ./target/release/dnglab analyze --raw-checksum "$sample" > rawler/data/testdata/"$digest";
        fi
	if [ "$rawtype" == "cameras" ]; then
	        MAKE=`echo $rawfile | cut -d/ -f2`;
	        MODEL=`echo $rawfile | cut -d/ -f3`;
	        TESTNAME=`basename "${rawfile@L}" | sed -e 's,[[:space:][:punct:]],_,g' -e 's,_+,_,g'`;
		echo -e "\tsuper::camera_file_check!(\"$MAKE\", \"$MODEL\", "cam_"$TESTNAME, \"`echo $rawfile | cut -d'/' -f4-`\");" >> "rawler/tests/cameras/mod.rs";
	else
		SAMPLESET=`echo $rawfile | cut -d/ -f2`;
		TESTNAME=`basename "${rawfile@L}" | sed -e 's,[[:space:][:punct:]],_,g' -e 's,_+,_,g'`;
		echo -e "\tsuper::sample_file_check!(\"$SAMPLESET\", "sample_"$TESTNAME, \"`echo $rawfile | cut -d'/' -f3-`\");" >> "rawler/tests/samples/mod.rs";
	fi
}


cat rawler/tests/supported_rawdb_sets.txt | while read setdir; do mkdir -v -p "rawler/data/testdata/$setdir"; done

#find "$RAWDB" -type f -name RAWLER_SUPPORTED -printf "%h\n" | while read searchdir; do find "$searchdir" -type f -not -name "RAWLER_SUPPORTED" -and -not -name "*.txt" -exec realpath --relative-to $RAWDB '{}' \;; done > rawler/tests/testfiles.idx

#find $RAWDB  -type f -name "*.CR3" -exec realpath --relative-to $RAWDB '{}' \; > rawler/tests/testfiles.idx

#while read line; do dirname "$line"; done < rawler/tests/testfiles.idx | sort | uniq | while read setdir; do mkdir -p "rawler/tests/testfiles/$setdir"; done

#exit;

cargo build --release;


#echo "" > "rawler/tests/generated_tests.inc";

echo "use crate::common::camera_file_check;" > "rawler/tests/cameras/mod.rs";
cat rawler/tests/supported_rawdb_sets.txt | grep -v "^$" | while read setdir; do
	echo "Processing: $setdir";
	modname="camera_"`echo $setdir | cut -d'/' -f3- | sed -e 's/\+/plus/g' | sed -e 's,[^[:alnum:]]\+,_,g'`;
	echo "mod ${modname@L} {" >> "rawler/tests/cameras/mod.rs";
	find "$RAWDB/$setdir" -type f -not -name "*.txt" -exec realpath --relative-to $RAWDB '{}' \; | while read rawfile; do
		process_rawfile "cameras" "$rawfile";
	done;
	echo "}" >> "rawler/tests/cameras/mod.rs";
done;
echo "" >> "rawler/tests/cameras/mod.rs";

echo "use crate::common::sample_file_check;" > "rawler/tests/samples/mod.rs";
cat rawler/tests/supported_sample_sets.txt | grep -v "^$" | while read setdir; do
        echo "Processing: $setdir";
        modname=`echo $setdir | cut -d'/' -f2- | sed -e 's/\+/plus/g' | sed -e 's,[^[:alnum:]]\+,_,g'`;
        echo "mod ${modname@L} {" >> "rawler/tests/samples/mod.rs";
        find "$RAWDB/$setdir" -type f -not -name "*.txt" -exec realpath --relative-to $RAWDB '{}' \; | while read rawfile; do
                process_rawfile "samples" "$rawfile";
        done;
        echo "}" >> "rawler/tests/samples/mod.rs";
done;
echo "" >> "rawler/tests/samples/mod.rs";



cargo fmt;

exit;

echo "use crate::common::camera_file_check;" > "rawler/tests/cameras/mod.rs";

while read rawfile; do
        sample=$RAWDB/"$rawfile"
        analyze=${rawfile}".analyze.yaml"
        digest=${rawfile}".digest.txt"
	dir=`dirname rawler/tests/testdata/"$analyze"`;
	mkdir -p "$dir";
	if [ ! -f rawler/tests/testdata/"$analyze" ]; then
		echo "Processing ${rawfile}";
        	echo "  analyze file: $analyze"
	        echo "  digest file:  $digest"
	        ./target/release/dnglab analyze --meta --yaml "$sample" > rawler/tests/testdata/"$analyze";
	        ./target/release/dnglab analyze --checksum "$sample" > rawler/tests/testdata/"$digest";
	fi
	MAKE=`echo $rawfile | cut -d/ -f2`;
	MODEL=`echo $rawfile | cut -d/ -f3`;
	TESTNAME=`basename "${rawfile@L}" | sed -e 's,[[:space:][:punct:]],_,g' -e 's,_+,_,g'`;
	echo "camera_file_check!(\"$MAKE\", \"$MODEL\", $TESTNAME, \"`echo $rawfile | cut -d'/' -f4-`\");" >> "rawler/tests/cameras/mod.rs";
        #file "$pixel";
done < rawler/tests/testfiles.idx;


