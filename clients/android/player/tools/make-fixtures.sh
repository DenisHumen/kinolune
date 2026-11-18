#!/usr/bin/env bash
# SPDX-License-Identifier: MPL-2.0
#
# PLAY-AND-002: test media for the instrumented tests of clients/android/player.
#
#   h264.mkv, hevc.mkv  12 s, 320x240 @ 25 fps, keyframe every second;
#                       audio: AAC "eng" (default) and AAC "rus";
#                       subtitles: SRT "eng", not default
#   external.srt        side-loaded subtitle for add_external_subtitle
#
# PLAY-AND-006 (subtitle overlay, ADR-0135):
#   karaoke.mkv         10 s, 320x240 @ 25 fps, H.264, no audio. The top 40 rows
#                       carry the frame number in 8 bits (40 px blocks, white = 1),
#                       so a screenshot tells which frame is on screen.
#                       ASS "rus" (default): every second one karaoke line of 25
#                       drawn squares, \k4 each (one frame), magenta → green, plus
#                       a rotated sign in DejaVu Sans with Cyrillic and CJK.
#                       DejaVu Sans is attached (when the host has it).
#   karaoke.ass         the same ASS as a side-loaded file
#   pgs.mkv             6 s testsrc2 with a PGS track "eng": event 1 at 1–2.5 s
#                       (one block), event 2 at 3.5–5 s (two blocks), from
#                       tools/qa/media/pgs.py (FFmpeg has no PGS encoder)
#
# Usage: tools/make-fixtures.sh OUT_DIR
# Needs an ffmpeg with libx264 and libx265 (FFMPEG overrides the command; the
# output directory is passed as an absolute path, so a containerised ffmpeg
# needs it mounted at the same path) and python3 for the PGS track.
# The files are generated, never committed: the encoders are only used here,
# on the build machine, and are not part of any client.

set -euo pipefail

OUT="${1:?usage: make-fixtures.sh OUT_DIR}"
read -r -a FF <<<"${FFMPEG:-ffmpeg}"
PA_REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../../.." && pwd)"
export PA_REPO
mkdir -p "$OUT"
OUT="$(cd "$OUT" && pwd)"

srt() {
  printf '1\n00:00:00,500 --> 00:00:05,000\n%s 1\n\n2\n00:00:05,500 --> 00:00:11,500\n%s 2\n' "$1" "$1"
}
srt "Embedded subtitle" >"$OUT/embedded.srt"
srt "External subtitle" >"$OUT/external.srt"

for codec in h264 hevc; do
  case "$codec" in
    h264) video=(-c:v libx264 -preset ultrafast -profile:v main) ;;
    hevc) video=(-c:v libx265 -preset ultrafast -x265-params log-level=error) ;;
  esac
  "${FF[@]}" -y -v error \
    -f lavfi -i "testsrc2=size=320x240:rate=25:duration=12" \
    -f lavfi -i "sine=frequency=440:duration=12" \
    -f lavfi -i "sine=frequency=660:duration=12" \
    -i "$OUT/embedded.srt" \
    -map 0:v -map 1:a -map 2:a -map 3:s \
    "${video[@]}" -pix_fmt yuv420p -g 25 \
    -c:a aac -b:a 64k -ac 2 -c:s srt \
    -metadata:s:a:0 language=eng -metadata:s:a:0 title=English \
    -metadata:s:a:1 language=rus -metadata:s:a:1 title=Russian \
    -metadata:s:s:0 language=eng \
    -disposition:a:0 default -disposition:a:1 0 -disposition:s:0 0 \
    "$OUT/$codec.mkv"
done
# ---- PLAY-AND-006 -----------------------------------------------------------

karaoke_ass() {
  cat <<'ASS'
[Script Info]
; Kinolune test fixture (PLAY-AND-006)
ScriptType: v4.00+
PlayResX: 320
PlayResY: 240
ScaledBorderAndShadow: yes
WrapStyle: 2

[V4+ Styles]
Format: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, BackColour, Bold, Italic, Underline, StrikeOut, ScaleX, ScaleY, Spacing, Angle, BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding
Style: Bar,DejaVu Sans,20,&H0000FF00,&H00FF00FF,&H00000000,&H00000000,0,0,0,0,100,100,0,0,1,0,0,7,0,0,0,1
Style: Sign,DejaVu Sans,18,&H0000FFFF,&H000000FF,&H00000000,&H80000000,-1,0,0,0,100,100,0,0,1,1,0,5,0,0,0,1

[Events]
Format: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text
ASS
  local sec square line
  square='m 0 0 l 12 0 12 24 0 24'
  for sec in 0 1 2 3 4 5 6 7 8 9; do
    line="{\\pos(10,150)}"
    for _ in $(seq 25); do line="$line{\\k4\\p1}$square{\\p0}"; done
    printf 'Dialogue: 0,0:00:%02d.00,0:00:%02d.00,Bar,,0,0,0,,%s\n' "$sec" "$((sec + 1))" "$line"
  done
  printf 'Dialogue: 1,0:00:00.00,0:00:10.00,Sign,,0,0,0,,{\\an5\\pos(160,80)\\frz8\\fad(300,0)}Кинолуна 字幕\n'
}
karaoke_ass >"$OUT/karaoke.ass"

# Frame number in binary in the top band, mid grey below.
bits="if(lt(Y,40),255*bitand(floor(N/pow(2,floor(X/40))),1),128)"
font=""
for f in /usr/share/fonts/truetype/dejavu/DejaVuSans.ttf /usr/share/fonts/TTF/DejaVuSans.ttf \
  /usr/share/fonts/dejavu/DejaVuSans.ttf /usr/share/fonts/dejavu-sans-fonts/DejaVuSans.ttf; do
  [ -f "$f" ] && { font=$f; break; }
done
attach=()
if [ -n "$font" ]; then
  cp "$font" "$OUT/DejaVuSans.ttf"
  attach=(-attach "$OUT/DejaVuSans.ttf" -metadata:s:t mimetype=application/x-truetype-font)
fi
"${FF[@]}" -y -v error \
  -f lavfi -i "color=c=black:size=320x240:rate=25:duration=10,format=yuv420p,geq=lum='$bits':cb=128:cr=128" \
  -i "$OUT/karaoke.ass" "${attach[@]}" \
  -map 0:v -map 1:s -c:v libx264 -preset ultrafast -crf 16 -pix_fmt yuv420p -g 25 \
  -c:s copy -metadata:s:s:0 language=rus -metadata:s:s:0 title=Karaoke -disposition:s:0 default \
  "$OUT/karaoke.mkv"
rm -f "$OUT/DejaVuSans.ttf"

python3 - "$OUT/pgs.sup" <<'PY'
import os, sys
sys.path.insert(0, os.path.join(os.environ["PA_REPO"], "tools", "qa", "media"))
import pgs
pgs.write_sup(sys.argv[1], [(1.0, 2.5), (3.5, 5.0)], (320, 240))
PY
"${FF[@]}" -y -v error \
  -f lavfi -i "testsrc2=size=320x240:rate=25:duration=6" \
  -fix_sub_duration -i "$OUT/pgs.sup" \
  -map 0:v -map 1:s -c:v libx264 -preset ultrafast -pix_fmt yuv420p -g 25 \
  -c:s copy -metadata:s:s:0 language=eng \
  "$OUT/pgs.mkv"
rm -f "$OUT/pgs.sup"

echo "fixtures: $OUT"
