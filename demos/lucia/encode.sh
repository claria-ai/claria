#!/bin/bash
# Encode demos/output/lucia/raw/*.webm into the MP4s and poster frames that
# claria-ai.github.io/fun.html embeds. See README.md here.
#   usage: demos/lucia/encode.sh [<claria-ai.github.io checkout>]
# With a site checkout, also copies videos/lucia-*.mp4 and img/lucia-*-poster.jpg into it.
set -euo pipefail

out="$(cd "$(dirname "$0")/../output/lucia" && pwd)"
site="${1:-}"

for raw in "$out"/raw/lucia-*.webm; do
  name="$(basename "$raw" .webm)"
  ffmpeg -v error -y -i "$raw" -vf "scale=1280:-2:flags=lanczos" \
    -c:v libx264 -preset slow -crf 25 -pix_fmt yuv420p -movflags +faststart -an \
    "$out/$name.mp4"
  # 0.8s in is each video's opening title card.
  ffmpeg -v error -y -ss 0.8 -i "$out/$name.mp4" -frames:v 1 -vf scale=640:-1 -q:v 4 \
    "$out/$name-poster.jpg"
  dur="$(ffprobe -v error -show_entries format=duration -of csv=p=0 "$out/$name.mp4")"
  echo "$name.mp4  ${dur}s  $(du -h "$out/$name.mp4" | cut -f1)"
  if [ -n "$site" ]; then
    cp "$out/$name.mp4" "$site/videos/$name.mp4"
    cp "$out/$name-poster.jpg" "$site/img/$name-poster.jpg"
  fi
done
[ -n "$site" ] && echo "Copied into $site; run its build.py and commit."
exit 0
