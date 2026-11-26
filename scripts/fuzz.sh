#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-only
# Run every fuzz target for a while (needs nightly and cargo-fuzz).
#
#   scripts/fuzz.sh [seconds per target] [target...]
#
# Seeds the decoder corpus with the files from testdata/music (prefixed with a format hint
# byte) and the response corpus with typical server answers.
set -euo pipefail
cd "$(dirname "$0")/../fuzz"
SECS=${1:-60}
shift || true
TARGETS=("$@")
if [ ${#TARGETS[@]} -eq 0 ]; then
  mapfile -t TARGETS < <(cargo +nightly fuzz list)
fi

seed_decode() {
  local dir=corpus/decode i=0
  mkdir -p "$dir"
  # Hint byte = index into HINTS in fuzz_targets/decode.rs.
  declare -A hint=([mp3]=1 [flac]=2 [ogg]=3 [opus]=4 [m4a]=5)
  while IFS= read -r -d '' f; do
    local ext=${f##*.}
    printf "\\x$(printf %02x "${hint[$ext]:-0}")" | cat - "$f" > "$dir/seed-$i.$ext"
    i=$((i + 1))
  done < <(find ../testdata/music -type f -print0)
}

seed_responses() {
  local dir=corpus/subsonic_response
  mkdir -p "$dir"
  printf '\x00{"subsonic-response":{"status":"ok","version":"1.16.1","type":"navidrome","openSubsonic":true}}' > "$dir/ping"
  printf '\x00{"subsonic-response":{"status":"failed","version":"1.16.1","error":{"code":40,"message":"Wrong username or password"}}}' > "$dir/error"
  printf '\x06{"subsonic-response":{"status":"ok","version":"1.16.1","searchResult3":{"artist":[{"id":"1","name":"A","albumCount":2}],"album":[{"id":2,"name":"B","title":"B","songCount":3}],"song":[{"id":"3","parent":"2","isDir":false,"title":"C","duration":180,"replayGain":{"trackGain":-6.5}}]}}}' > "$dir/search"
  printf '\x08{"subsonic-response":{"status":"ok","version":"1.16.1","playQueue":{"entry":[{"id":"3","title":"C"}],"current":"3","position":1500}}}' > "$dir/queue"
}

seed_decode
seed_responses
for t in "${TARGETS[@]}"; do
  echo "== $t ($SECS s)"
  opts=()
  # opus-decoder 0.1.1 has an intentional-looking u8 shift overflow (wraps in release builds,
  # see Cargo.toml). Overflow checks would report it on every other Opus input, so the decoder
  # target is fuzzed with release semantics, like the shipped binary.
  if [ "$t" = decode ]; then opts=(-O); fi
  cargo +nightly fuzz run "${opts[@]}" "$t" -- -max_total_time="$SECS" -rss_limit_mb=2048 -timeout=10
done
