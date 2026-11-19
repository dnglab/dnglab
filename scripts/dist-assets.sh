#!/usr/bin/env bash
# Generate the shell completions and man pages for the dnglab packages
# into bin/dnglab/completions and bin/dnglab/manpages.
# Usage: scripts/dist-assets.sh <dnglab binary for the build host>
set -euo pipefail

if [ $# -ne 1 ]; then
  echo "Usage: $0 <dnglab binary>" >&2
  exit 1
fi
dnglab=$(realpath "$1")
cd "$(dirname "$0")/../bin/dnglab"

mkdir -p completions
"$dnglab" completions bash > completions/dnglab
"$dnglab" completions zsh > completions/_dnglab
"$dnglab" completions fish > completions/dnglab.fish

cargo run --release -p dnglab-mangen -- manpages
