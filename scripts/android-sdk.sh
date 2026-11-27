#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-only
# Install what the Android app build needs on a CI runner (Android SDK already present):
# the SDK platform and build tools, the pinned NDK, the Rust Android targets and cargo-ndk.
set -euo pipefail
cd "$(dirname "$0")/.."
: "${ANDROID_HOME:?ANDROID_HOME not set}"
ndk=$(sed -n 's/^val ndkVersionPinned = "\(.*\)"/\1/p' android/app/build.gradle.kts)
sdkmanager=$(ls "$ANDROID_HOME"/cmdline-tools/*/bin/sdkmanager | sort -V | tail -1)
yes | "$sdkmanager" --licenses >/dev/null || true
"$sdkmanager" --install "platforms;android-37.2" "build-tools;37.0.0" "ndk;$ndk" "platform-tools" >/dev/null
rustup target add aarch64-linux-android armv7-linux-androideabi x86_64-linux-android
command -v cargo-ndk >/dev/null || cargo install cargo-ndk --version 4.1.2 --locked
