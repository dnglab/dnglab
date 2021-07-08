#!/bin/bash

find testdata/ -type f -name "*.raw" | while read rawfile; do
	sample="${rawfile%.*}"
	analyze=${sample}".analyze"
	pixel=${sample}".pixel"

	echo "Processing ${rawfile}";
	echo "  analyze file: $analyze"
	echo "  pixel file:   $pixel"
	cargo run --release --bin dnglab analyze --meta --yaml "$rawfile" > "$analyze";
	cargo run --release --bin dnglab analyze --pixel "$rawfile" > "$pixel";
	file "$pixel";
done;

