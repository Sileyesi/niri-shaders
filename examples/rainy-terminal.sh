#!/bin/sh
# Runs the rainy terminal look in a nested niri window.
#
#     examples/rainy-terminal.sh [wallpaper.png]

cd "$(dirname "$0")/.." || exit 1

if [ -n "$1" ]; then
    NIRI_DEMO_WALLPAPER=$(realpath "$1")
    export NIRI_DEMO_WALLPAPER
fi

exec ./target/release/niri -c examples/rainy-terminal.kdl
