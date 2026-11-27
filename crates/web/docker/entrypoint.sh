#!/bin/sh
# SPDX-License-Identifier: GPL-3.0-only
# Copies the player into a writable directory (under the optional base path), writes the
# deployment presets and starts the web server. Runs as a non-root user.
set -eu
ROOT=/tmp/infrasonic-www
BASE=$(printf '%s' "${INFRASONIC_BASE_PATH:-}" | sed 's#^/*##; s#/*$##')
case "$BASE" in
  *..*) echo "INFRASONIC_BASE_PATH must not contain .." >&2; exit 1 ;;
esac
DIR="$ROOT${BASE:+/$BASE}"
rm -rf "$ROOT"
mkdir -p "$DIR"
cp -R /srv/infrasonic/. "$DIR/"
# JSON-escape the server address for config.js.
SERVER=$(printf '%s' "${INFRASONIC_SERVER:-}" | sed 's/\\/\\\\/g; s/"/\\"/g')
printf 'window.INFRASONIC_CONFIG = { server: "%s" };\n' "$SERVER" > "$DIR/config.js"
if [ -n "$BASE" ]; then
  # The root sends visitors to the player.
  printf '<!doctype html><meta http-equiv="refresh" content="0; url=/%s/">\n' "$BASE" > "$ROOT/index.html"
fi
exec static-web-server --config-file /etc/infrasonic/sws.toml --root "$ROOT"
