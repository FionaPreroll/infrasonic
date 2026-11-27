#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-only
# Builds the Rust library for the app. Gradle runs this; it can also be run by hand.
#
#   android/build-rust.sh android <debug|release> <jniLibs dir> [ABIs...]
#       libinfrasonic_ffi.so for each Android ABI (default: arm64-v8a armeabi-v7a x86_64)
#   android/build-rust.sh host
#       the library for this machine, for JVM unit tests (target/debug)
#   android/build-rust.sh bindings <dir>
#       the Kotlin bindings, generated from the host library
#
# Needs cargo-ndk and the NDK (ANDROID_NDK_HOME, or the NDK in ANDROID_HOME). Paths in the
# binaries are made independent of the checkout location so that builds are reproducible.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

mode="${1:-}"
case "$mode" in
  android)
    profile="$2"; out="$3"; shift 3
    abis=("$@")
    [ ${#abis[@]} -eq 0 ] && abis=(arm64-v8a armeabi-v7a x86_64)
    # Debug libraries keep line tables only; full debug info makes them 200 MB.
    export CARGO_PROFILE_DEV_DEBUG=line-tables-only
    [ -n "${ANDROID_NDK_HOME:-}" ] && [ ! -d "$ANDROID_NDK_HOME" ] && unset ANDROID_NDK_HOME
    if [ -z "${ANDROID_HOME:-}" ] && [ -f android/local.properties ]; then
      ANDROID_HOME=$(sed -n 's/^sdk\.dir=//p' android/local.properties)
    fi
    if [ -z "${ANDROID_NDK_HOME:-}" ] && [ -n "${ANDROID_HOME:-}" ] && [ -d "$ANDROID_HOME/ndk" ]; then
      ANDROID_NDK_HOME="$ANDROID_HOME/ndk/$(ls "$ANDROID_HOME/ndk" | sort -V | tail -1)"
      export ANDROID_NDK_HOME
    fi
    # Only for the Android targets: other flags would rebuild the host target directory.
    CARGO_HOME_DIR="${CARGO_HOME:-$HOME/.cargo}"
    for t in AARCH64_LINUX_ANDROID ARMV7_LINUX_ANDROIDEABI X86_64_LINUX_ANDROID; do
      export "CARGO_TARGET_${t}_RUSTFLAGS=--remap-path-prefix=$ROOT=/infrasonic --remap-path-prefix=$CARGO_HOME_DIR=/cargo -C link-arg=-Wl,--build-id=none"
    done
    export SOURCE_DATE_EPOCH="${SOURCE_DATE_EPOCH:-$(git log -1 --format=%ct 2>/dev/null || echo 0)}"
    if [ -z "${ANDROID_NDK_HOME:-}" ]; then
      echo "no Android NDK: set ANDROID_NDK_HOME or ANDROID_HOME (or sdk.dir in local.properties)" >&2
      exit 1
    fi
    flags=()
    [ "$profile" = release ] && flags=(--release)
    targets=()
    for a in "${abis[@]}"; do targets+=(-t "$a"); done
    rm -rf "$out"
    mkdir -p "$out"
    cargo ndk "${targets[@]}" -P 26 -o "$out" build -p infrasonic-ffi --lib "${flags[@]}"
    ;;
  host)
    cargo build -p infrasonic-ffi --lib
    ;;
  bindings)
    out="$2"
    cargo build -p infrasonic-ffi --lib
    lib="$ROOT/target/debug/libinfrasonic_ffi.so"
    [ -f "$lib" ] || lib="$ROOT/target/debug/libinfrasonic_ffi.dylib"
    rm -rf "$out"
    cargo run -q -p infrasonic-ffi --features bindgen --bin uniffi-bindgen -- \
      generate --library "$lib" --language kotlin --no-format --out-dir "$out"
    ;;
  *)
    sed -n '3,12p' "$0" >&2
    exit 2
    ;;
esac
