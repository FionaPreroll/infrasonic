#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-only
# Build the web player into target/web (static files, ready for any web server).
#
#   scripts/web.sh           build
#   scripts/web.sh --serve   build and serve on http://127.0.0.1:8098
#
# Needs: rustup target wasm32-unknown-unknown and wasm-bindgen-cli matching Cargo.lock.
set -euo pipefail
cd "$(dirname "$0")/.."
OUT=${OUT:-target/web}
T=wasm32-unknown-unknown
# The page is built for size; the audio worker for speed, since it decodes.
cargo build -p infrasonic-web --target $T --profile web --bin infrasonic-web
cargo build -p infrasonic-web --target $T --profile web-worker --bin infrasonic-engine-worker
rm -rf "$OUT"
mkdir -p "$OUT/pkg"
wasm-bindgen --target web --no-typescript --out-dir "$OUT/pkg" "target/$T/web/infrasonic-web.wasm"
wasm-bindgen --target web --no-typescript --out-dir "$OUT/pkg" \
  "target/$T/web-worker/infrasonic-engine-worker.wasm"
python3 scripts/wasm-strip.py "$OUT"/pkg/*.wasm
cp crates/web/www/* "$OUT/"
VERSION=${VERSION:-$(git describe --tags --always --dirty 2>/dev/null || echo dev)}
sed -i.bak "s/__VERSION__/$VERSION/" "$OUT/sw.js" && rm "$OUT/sw.js.bak"

size() { gzip -9 -c "$1" | wc -c | awk '{printf "%.0f kB", $1 / 1000}'; }
first=0
for f in index.html style.css config.js pkg/infrasonic-web.js pkg/infrasonic-web_bg.wasm; do
  first=$((first + $(gzip -9 -c "$OUT/$f" | wc -c)))
done
echo "first start (gzip): $((first / 1000)) kB; audio worker on first play: $(size "$OUT/pkg/infrasonic-engine-worker_bg.wasm")"

if [ "${1:-}" = --serve ]; then
  exec python3 -m http.server 8098 --bind 127.0.0.1 --directory "$OUT"
fi
