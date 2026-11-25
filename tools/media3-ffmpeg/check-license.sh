#!/usr/bin/env bash
# SPDX-License-Identifier: MPL-2.0
#
# Licence gate for the Media3 FFmpeg audio extension (PLAY-AND-004).
#
#   check-license.sh <extension-dir> <ffmpeg-out>
#
# Fails (exit 1) unless:
#   - the FFmpeg it links passes tools/ffmpeg/check-license.sh --artifact
#     ("LGPL version 2.1 or later", no GPL/nonfree/v3 option);
#   - every libffmpegJNI.so links FFmpeg dynamically (NEEDED libavcodec.so,
#     libswresample.so, libavutil.so), defines no FFmpeg symbol and embeds no
#     FFmpeg licence string (no static copy: users can swap the LGPL libraries);
#   - its other NEEDED libraries are NDK system libraries;
#   - libavcodec.so has the decoders the extension is for (TrueHD, MLP, DTS,
#     E-AC-3, AC-3; versions.sh) and FFmpeg's configure line does not drop
#     decoders;
#   - the Java side is the audio part only (no video renderer).
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=versions.sh
. "$ROOT/versions.sh"

fail=0
bad() { printf 'licence check: FAIL: %s\n' "$*" >&2; fail=1; }
ok() { printf 'licence check: ok: %s\n' "$*" >&2; }

[ $# -eq 2 ] || { sed -n '4,17p' "$0"; exit 2; }
ext="$1" ff="$2"

tool() {
    local name="$1" ndk="${ANDROID_NDK_HOME:-}/toolchains/llvm/prebuilt/linux-x86_64/bin/llvm-$1"
    if [ -x "$ndk" ]; then printf '%s' "$ndk"
    elif command -v "llvm-$name" >/dev/null 2>&1; then printf 'llvm-%s' "$name"
    else printf '%s' "$name"; fi
}
READELF="${READELF:-$(tool readelf)}"
NM="${NM:-$(tool nm)}"

SYSTEM_LIBS_RE='^lib(c|m|dl|log|android)\.so$'
FFMPEG_LIBS_RE='^lib(avcodec|swresample|avutil)\.so$'

# 1. The FFmpeg we link.
if "$ROOT/../ffmpeg/check-license.sh" --artifact "$ff"; then ok "linked FFmpeg is LGPL v2.1+"
else bad "linked FFmpeg at $ff failed tools/ffmpeg/check-license.sh"; fi

# 2–3. Each libffmpegJNI.so.
found=0
while IFS= read -r -d '' so; do
    found=1
    name="${so#"$ext"/}"
    needed="$("$READELF" -d "$so" | sed -n 's/.*(NEEDED).*\[\(.*\)\]/\1/p')"
    for lib in libavcodec.so libswresample.so libavutil.so; do
        if printf '%s\n' "$needed" | grep -qx "$lib"; then ok "$name links $lib dynamically"
        else bad "$name does not link $lib dynamically"; fi
    done
    while IFS= read -r lib; do
        [ -n "$lib" ] || continue
        if ! printf '%s\n' "$lib" | grep -Eq "$SYSTEM_LIBS_RE|$FFMPEG_LIBS_RE"; then
            bad "$name needs $lib (neither FFmpeg nor an NDK system library)"
        fi
    done <<< "$needed"
    defined="$("$NM" -D --defined-only "$so" 2>/dev/null | awk '{print $NF}' | grep -E '^(av_|avcodec_|swr_|ff_)' || true)"
    if [ -n "$defined" ]; then bad "$name defines FFmpeg symbols (static copy?): $(printf '%s' "$defined" | head -5 | tr '\n' ' ')"
    else ok "$name defines no FFmpeg symbols"; fi
    if LC_ALL=C grep -aqE 'license: (LGPL|GPL|nonfree)' "$so"; then bad "$name embeds an FFmpeg licence string (static copy?)"
    else ok "$name embeds no FFmpeg code"; fi
done < <(find "$ext/jniLibs" -name libffmpegJNI.so -type f -print0 2>/dev/null)
[ "$found" = 1 ] || bad "no jniLibs/<abi>/libffmpegJNI.so under $ext"

# 4. The decoders.
avcodec="$ff/lib/libavcodec.so"
if [ -f "$avcodec" ]; then
    while IFS=: read -r dec longname; do
        [ -n "$dec" ] || continue
        if LC_ALL=C grep -aqF "$longname" "$avcodec"; then ok "decoder $dec ($longname)"
        else bad "libavcodec.so has no $dec decoder ($longname)"; fi
    done <<< "$REQUIRED_DECODERS"
else
    bad "no $avcodec"
fi
if grep -Eq -- '--disable-(decoders|decoder=)' "$ff/BUILDINFO.txt" 2>/dev/null; then
    bad "FFmpeg was configured with decoders disabled: $(grep -Eo -- '--disable-decoders?[^ ]*' "$ff/BUILDINFO.txt" | tr '\n' ' ')"
fi

# 5. Audio only.
if [ -e "$ext/java/androidx/media3/decoder/ffmpeg/ExperimentalFfmpegVideoRenderer.java" ]; then
    bad "the video renderer is not part of the audio extension"
fi
[ -f "$ext/java/androidx/media3/decoder/ffmpeg/FfmpegAudioRenderer.java" ] || bad "no FfmpegAudioRenderer.java in $ext/java"

exit "$fail"
