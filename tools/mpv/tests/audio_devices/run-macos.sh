#!/usr/bin/env bash
# SPDX-License-Identifier: MPL-2.0
#
# PKG-021: builds check_audio_devices.c against the Mpv.framework the macOS
# app ships and runs it on a generated AVI, once with CoreAudio and once
# with the null output (both list the CoreAudio devices).
#
#   run-macos.sh <Frameworks dir> <libmpv include dir> <work dir> [seconds]
#
# <Frameworks dir> holds Mpv.framework and the FFmpeg frameworks it links
# (Kinolune.app/Contents/Frameworks). Exit code 0 = OK; a crash (signal) of
# the test program fails with its exit status.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
fw="$(cd "$1" && pwd)"
inc="$2"
work="$3"
secs="${4:-10}"
mkdir -p "$work"

python3 "$here/make_sample.py" "$work/sample.avi" --seconds 40
# The host architecture only: that is the slice that runs here.
cc -O1 -g -Wall -Wextra "$here/check_audio_devices.c" -o "$work/check_audio_devices" \
    -I"$inc" -F"$fw" -framework Mpv -Wl,-rpath,"$fw"

rc=0
for ao in coreaudio null; do
    echo "== ao=$ao"
    set +e
    "$work/check_audio_devices" "$work/sample.avi" "$ao" "$secs" 2>&1 | tee "$work/ao-$ao.log"
    st=${PIPESTATUS[0]}
    set -e
    if [ "$st" -gt 128 ]; then
        echo "::error::check_audio_devices (ao=$ao) crashed with signal $((st - 128))"
        rc=$st
    elif [ "$st" -ne 0 ]; then
        echo "::error::check_audio_devices (ao=$ao) failed ($st)"
        rc=$st
    fi
done
exit "$rc"
