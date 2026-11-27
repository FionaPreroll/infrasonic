#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-only
# Run the app's device tests on a connected emulator against a Navidrome on this machine.
# The emulator reaches this machine's 127.0.0.1 as 10.0.2.2. Screenshots end up in
# android/app/build/outputs/connected_android_test_additional_output.
#
#   scripts/android-device-test.sh      (starts the servers with scripts/e2e-servers.sh)
set -euo pipefail
cd "$(dirname "$0")/.."
if [ -z "${INFRASONIC_E2E_SERVERS:-}" ]; then
  exec scripts/e2e-servers.sh "$0" "$@"
fi
url=$(tr ';' '\n' <<<"$INFRASONIC_E2E_SERVERS" | sed -n 's/^navidrome=//p' | sed 's#//127\.0\.0\.1:#//10.0.2.2:#')
abi=$(adb shell getprop ro.product.cpu.abi | tr -d '\r')
cd android
./gradlew :app:connectedDebugAndroidTest --console=plain \
  -Pinfrasonic.abis="$abi" \
  -Pandroid.testInstrumentationRunnerArguments.server="$url" "$@"
