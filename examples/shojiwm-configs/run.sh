#!/bin/sh
# Runs one of the ShojiWM config demos in a nested niri window.
#
#     examples/shojiwm-configs/run.sh rainy-terminal|liquid-terminal|frozen-desktop|true-liquid-glass|liquid-desktop|water-window [wallpaper]
#
# Build niri first with `cargo build --release`.

dir=$(cd "$(dirname "$0")" && pwd)
name=${1:-liquid-desktop}

if [ ! -f "$dir/$name.kdl" ]; then
    echo "usage: $0 rainy-terminal|liquid-terminal|frozen-desktop|true-liquid-glass|liquid-desktop|water-window [wallpaper]" >&2
    exit 1
fi

# The configs start helpers from here, like the QuickShell bar of true-liquid-glass.
NIRI_SHOJIWM_DIR=$dir
export NIRI_SHOJIWM_DIR

if [ -n "$2" ]; then
    NIRI_DEMO_WALLPAPER=$(realpath "$2")
    export NIRI_DEMO_WALLPAPER
fi

exec "$dir/../../target/release/niri" -c "$dir/$name.kdl"
