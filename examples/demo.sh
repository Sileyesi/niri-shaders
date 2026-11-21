#!/bin/sh
# Runs the window shader demo in a nested niri window.
#
#     examples/demo.sh [wallpaper.png]
#
# Build first with `cargo build --release` (add `--no-default-features --features dbus,systemd`
# if you don't have libclang for screencasting).

cd "$(dirname "$0")/.." || exit 1

if [ -n "$1" ]; then
    NIRI_DEMO_WALLPAPER=$(realpath "$1")
    export NIRI_DEMO_WALLPAPER
fi

exec ./target/release/niri -c examples/demo-config.kdl
