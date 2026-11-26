#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-only
# Start Navidrome and gonic with the synthetic music from testdata/music, run a command with
# INFRASONIC_E2E_SERVERS pointing at them, then remove the containers.
#
#   scripts/e2e-servers.sh cargo test -p infrasonic-e2e
#
# Images can be pinned with NAVIDROME_IMAGE / GONIC_IMAGE.
set -euo pipefail
cd "$(dirname "$0")/.."
NAVIDROME_IMAGE=${NAVIDROME_IMAGE:-deluan/navidrome:latest}
GONIC_IMAGE=${GONIC_IMAGE:-sentriz/gonic:latest}
MUSIC="$PWD/testdata/music"
SUFFIX="$$"
ND="infrasonic-e2e-navidrome-$SUFFIX"
GO="infrasonic-e2e-gonic-$SUFFIX"

cleanup() { docker rm -f "$ND" "$GO" >/dev/null 2>&1 || true; }
trap cleanup EXIT

docker run -d --rm --name "$ND" -p 127.0.0.1::4533 -v "$MUSIC:/music:ro" \
  -e ND_LOGLEVEL=warn -e ND_ENABLEEXTERNALSERVICES=false -e ND_AGENTS= \
  -e ND_ENABLEINSIGHTSCOLLECTOR=false "$NAVIDROME_IMAGE" >/dev/null
docker run -d --rm --name "$GO" -p 127.0.0.1::80 -v "$MUSIC:/music:ro" \
  --tmpfs /podcasts --tmpfs /playlists --tmpfs /cache --tmpfs /data \
  -e GONIC_SCAN_AT_START_ENABLED=true "$GONIC_IMAGE" >/dev/null

port() { docker port "$1" "$2/tcp" | head -1 | sed 's/.*://'; }
ND_URL="http://127.0.0.1:$(port "$ND" 4533)"
GO_URL="http://127.0.0.1:$(port "$GO" 80)"
API="u=admin&p=admin&v=1.16.1&c=e2e&f=json"

wait_for() { # url
  for _ in $(seq 1 120); do
    if curl -fs "$1" >/dev/null 2>&1; then return 0; fi
    sleep 0.5
  done
  echo "server at $1 did not come up" >&2
  return 1
}

# Navidrome: create the admin user on first start.
wait_for "$ND_URL/ping"
curl -fs -X POST -H 'Content-Type: application/json' \
  -d '{"username":"admin","password":"admin"}' "$ND_URL/auth/createAdmin" >/dev/null || true
wait_for "$GO_URL/rest/ping?$API"

# Wait until both libraries are scanned (9 songs).
for url in "$ND_URL" "$GO_URL"; do
  for _ in $(seq 1 120); do
    n=$(curl -fs "$url/rest/getRandomSongs?size=100&$API" 2>/dev/null \
      | grep -o '"isDir":false' | wc -l || true)
    [ "${n:-0}" -ge 9 ] && break
    curl -fs "$url/rest/startScan?$API" >/dev/null 2>&1 || true
    sleep 1
  done
done

export INFRASONIC_E2E_SERVERS="navidrome=$ND_URL;gonic=$GO_URL"
echo "INFRASONIC_E2E_SERVERS=$INFRASONIC_E2E_SERVERS"
"$@"
