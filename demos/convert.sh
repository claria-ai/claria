#!/bin/bash
# Convert Playwright WebM recordings to iOS-compatible MP4 (H.264 + AAC).
# Requires ffmpeg installed on the system.

set -euo pipefail

for f in output/*.webm; do
  [ -f "$f" ] || continue
  out="${f%.webm}.mp4"
  echo "Converting $(basename "$f") → $(basename "$out")"
  ffmpeg -y -i "$f" \
    -c:v libx264 -preset slow -crf 22 \
    -c:a aac -b:a 128k \
    -movflags +faststart \
    "$out"
done

echo "Done. MP4 files in output/"
