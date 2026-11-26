#!/bin/sh
# Dev-time packaging of the factory content (not part of the program).
# Copies the legacy samples as they are into <out>/samples; presets (JSON, converted by a separate dev tool) go to
# <out>/presets/<plugin>/<name>.json and projects to <out>/projects.
# Usage: tools/factory.sh <legacy data folder> <out folder>
set -eu
data="${1:?legacy data folder (legado/data)}"
out="${2:?output folder (the program finds it as 'factory' beside itself, or in LINUXMUSIC_FACTORY)}"
mkdir -p "$out/samples" "$out/presets" "$out/projects"
cp -R "$data/samples/." "$out/samples/"
# the drum-synth sources and text files are not samples
find "$out/samples" -name '*.txt' -delete
echo "factory content in $out"
