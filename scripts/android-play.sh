#!/bin/sh
# SPDX-License-Identifier: GPL-3.0-only
# Copy the headless player to an Android device and run it there (needs adb and USB debugging).
#
#   scripts/android-play.sh [--bin PATH] [infrasonic-headless arguments]
#   scripts/android-play.sh --url https://me:secret@music.example.org --random 5
#
# The binary is built by the CI job "Android build" (artifact
# infrasonic-headless-aarch64-linux-android) or locally with the NDK:
#   cargo build --release --target aarch64-linux-android -p infrasonic-headless
set -eu

bin=target/aarch64-linux-android/release/infrasonic-headless
if [ "${1:-}" = "--bin" ]; then
    bin=$2
    shift 2
fi
[ -f "$bin" ] || { echo "no binary at $bin" >&2; exit 1; }

dest=/data/local/tmp/infrasonic-headless
adb push "$bin" "$dest" >/dev/null
adb shell chmod 755 "$dest"
# Quote every argument for the device shell.
args=""
for a in "$@"; do
    args="$args '$(printf %s "$a" | sed "s/'/'\\\\''/g")'"
done
exec adb shell "$dest$args"
