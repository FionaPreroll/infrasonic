#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-only
# Regenerates the synthetic test music in testdata/music. Needs ffmpeg.
# The files are sine tones generated here, so there are no third-party rights in them.
set -euo pipefail
cd "$(dirname "$0")/.."
OUT=testdata/music
rm -rf "$OUT"
mkdir -p "$OUT"

# tone <file> <freq> <seconds> <artist> <album> <title> <track> <year> <genre> [extra ffmpeg args]
tone() {
  local file=$1 freq=$2 secs=$3 artist=$4 album=$5 title=$6 track=$7 year=$8 genre=$9
  shift 9
  mkdir -p "$(dirname "$file")"
  ffmpeg -nostdin -loglevel error -y \
    -f lavfi -i "sine=frequency=${freq}:sample_rate=44100:duration=${secs}" \
    -f lavfi -i "sine=frequency=$((freq * 3 / 2)):sample_rate=44100:duration=${secs}" \
    -filter_complex "[0:a][1:a]join=inputs=2:channel_layout=stereo,volume=0.5[a]" -map "[a]" \
    -metadata artist="$artist" -metadata album_artist="$artist" -metadata album="$album" \
    -metadata title="$title" -metadata track="$track" -metadata date="$year" \
    -metadata genre="$genre" "$@" "$file"
}

A="$OUT/Infrasonic Test Ensemble/Sine Studies (2024)"
tone "$A/01 - Low A.flac" 220 4 "Infrasonic Test Ensemble" "Sine Studies" "Low A" 1 2024 Electronic -c:a flac
tone "$A/02 - Middle A.flac" 440 4 "Infrasonic Test Ensemble" "Sine Studies" "Middle A" 2 2024 Electronic -c:a flac
tone "$A/03 - High A.flac" 880 4 "Infrasonic Test Ensemble" "Sine Studies" "High A" 3 2024 Electronic -c:a flac

B="$OUT/The Codec Quartet/Formats Live (2025)"
tone "$B/01 - MP3 Song.mp3" 330 5 "The Codec Quartet" "Formats Live" "MP3 Song" 1 2025 Jazz -c:a libmp3lame -b:a 128k
tone "$B/02 - Vorbis Song.ogg" 350 5 "The Codec Quartet" "Formats Live" "Vorbis Song" 2 2025 Jazz -c:a libvorbis -q:a 3
tone "$B/03 - Opus Song.opus" 370 5 "The Codec Quartet" "Formats Live" "Opus Song" 3 2025 Jazz -c:a libopus -b:a 96k
tone "$B/04 - AAC Song.m4a" 390 5 "The Codec Quartet" "Formats Live" "AAC Song" 4 2025 Jazz -c:a aac -b:a 128k

C="$OUT/Black Noise Collective/Black Album (2023)"
tone "$C/01 - Black Hole.mp3" 262 3 "Black Noise Collective" "Black Album" "Black Hole" 1 2023 Ambient -c:a libmp3lame -b:a 96k
tone "$C/02 - Blackout.mp3" 294 3 "Black Noise Collective" "Black Album" "Blackout" 2 2023 Ambient -c:a libmp3lame -b:a 96k

echo "generated:"; find "$OUT" -type f | sort
