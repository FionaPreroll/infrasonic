#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-only
# Build the web spike (engine decoders in WebAssembly) and check it in headless Chromium.
#
#   scripts/web-spike.sh          build into target/web-spike and run the check
#   scripts/web-spike.sh --serve  build and serve on http://127.0.0.1:8099 to try it by hand
#
# Needs: rustup target wasm32-unknown-unknown, wasm-bindgen-cli matching Cargo.lock, Python 3,
# and Node with Playwright for the check.
set -euo pipefail
cd "$(dirname "$0")/.."
OUT=target/web-spike
cargo build -p infrasonic-web-spike --target wasm32-unknown-unknown --release
rm -rf "$OUT"
mkdir -p "$OUT"
wasm-bindgen --target web --no-typescript --out-dir "$OUT/pkg" \
  target/wasm32-unknown-unknown/release/infrasonic_web_spike.wasm
cp crates/web-spike/www/index.html "$OUT/"
cp -r testdata/music "$OUT/music"
echo "wasm: $(du -h "$OUT/pkg/infrasonic_web_spike_bg.wasm" | cut -f1)"

if [ "${1:-}" = --serve ]; then
  exec python3 -m http.server 8099 --bind 127.0.0.1 --directory "$OUT"
fi
python3 -m http.server 8099 --bind 127.0.0.1 --directory "$OUT" >/dev/null 2>&1 &
SERVER=$!
trap 'kill $SERVER' EXIT
sleep 1
node crates/web-spike/check.mjs http://127.0.0.1:8099/
