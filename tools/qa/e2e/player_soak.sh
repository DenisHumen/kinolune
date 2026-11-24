#!/usr/bin/env bash
# SPDX-License-Identifier: MPL-2.0
#
# BUG-001: the player memory soak without the live stand — no SMB, no
# network, no secrets. Builds clients/flutter/app/tool/soak/player_soak.dart
# as a release (or profile) app, makes a few local test films with ffmpeg
# and runs N cycles «open → play 6 s → seek → play 4 s → close» under Xvfb
# in the e2e runner image. soak_memory.py gives the verdict (exit 1 when the
# memory keeps growing: more than PA_E2E_SOAK_LIMIT_MB per 50 cycles).
#
#   tools/qa/e2e/player_soak.sh [--cycles N] [--mode app|player] [--profile]
#                               [--bundle DIR] [--env K=V]... [--wrap CMD]
#
# --cycles N    soak cycles (40)
# --mode M      app: what «Смотреть» does (library item, trickplay, skip
#               segments, progress); player: the bare PlayerScreen (app)
# --profile     build a profile bundle instead of release
# --bundle DIR  run this prebuilt bundle instead of building one
# --env K=V     extra environment for the app (e.g. MALLOC_ARENA_MAX=8)
# --wrap CMD    run the app under CMD (e.g. "heaptrack -o /abs/out"; the
#               image PA_SOAK_IMAGE must have the tool)
#
# Results: $PA_E2E_HOME/soak/<stamp>/{run.log,report.md} (PA_E2E_HOME,
# default ~/.cache/kinolune-e2e, as run.sh). Runs on the build server or
# any Linux with Docker; `run.sh --soak` covers the same with SMB and the
# real network.
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
repo="$(cd "$here/../../.." && pwd)"
home="${PA_E2E_HOME:-$HOME/.cache/kinolune-e2e}"
runner_image=${PA_SOAK_IMAGE:-pa-test-e2e-runner:2}

cycles=40 mode=app build=release bundle="" wrap="" appenv=()
while [ $# -gt 0 ]; do
    case "$1" in
        --cycles) cycles="${2:?--cycles needs a number}"; shift ;;
        --mode) mode="${2:?--mode needs app or player}"; shift ;;
        --profile) build=profile ;;
        --bundle) bundle="${2:?--bundle needs a directory}"; shift ;;
        --wrap) wrap="${2:?--wrap needs a command}"; shift ;;
        --env) appenv+=(-e "${2:?--env needs K=V}"); shift ;;
        *) echo "usage: player_soak.sh [--cycles N] [--mode app|player] [--profile] [--bundle DIR] [--env K=V]... [--wrap CMD]" >&2; exit 64 ;;
    esac
    shift
done

if ! docker image inspect "$runner_image" >/dev/null 2>&1; then
    docker build -t "$runner_image" -f "$here/runner.Dockerfile" "$here"
fi

stamp="$(date +%Y%m%d-%H%M%S)"
out="$home/soak/$stamp"
data="$home/soak/$stamp-data"
films="$home/soak-films"
mkdir -p "$out" "$data"
echo "player_soak: results in $out" >&2

app="$repo/clients/flutter/app"
set +e
docker run --rm --name "pa-test-soak-$$" --network none \
    -u "$(id -u):$(id -g)" -e HOME="$HOME" -v "$HOME:$HOME" -w "$app" --cpus 8 \
    -e XDG_DATA_HOME="$data/data" -e XDG_CONFIG_HOME="$data/config" -e XDG_CACHE_HOME="$data/cache" \
    -e KINOLUNE_SOAK_DIR="$films" -e KINOLUNE_SOAK_CYCLES="$cycles" -e KINOLUNE_SOAK_MODE="$mode" \
    -e PA_SOAK_BUILD="$build" -e PA_SOAK_BUNDLE="$bundle" -e PA_SOAK_DATA="$data" -e PA_SOAK_WRAP="$wrap" \
    -e CARGO_BUILD_JOBS=4 "${appenv[@]}" \
    "$runner_image" bash -c '
        set -e
        export PATH=$HOME/opt/flutter/bin:$HOME/.cargo/bin:$PATH
        # Three short local films (ffmpeg here only makes test files).
        if [ ! -f "$KINOLUNE_SOAK_DIR/Soak Charlie (2003).mkv" ]; then
            mkdir -p "$KINOLUNE_SOAK_DIR"
            mk() { nice ffmpeg -loglevel error -y -f lavfi -i "testsrc2=size=$2:rate=24" \
                -f lavfi -i "sine=frequency=$3:sample_rate=48000" -t "$4" \
                -c:v libx264 -preset veryfast -pix_fmt yuv420p -c:a aac -shortest \
                "$KINOLUNE_SOAK_DIR/$1"; }
            mk "Soak Alpha (2001).mkv" 1280x720 440 120
            mk "Soak Bravo (2002).mkv" 640x360 660 60
            mk "Soak Charlie (2003).mkv" 1920x1080 880 300
        fi
        bundle="$PA_SOAK_BUNDLE"
        if [ -z "$bundle" ]; then
            flutter pub get >/dev/null
            nice flutter build linux --"$PA_SOAK_BUILD" -t tool/soak/player_soak.dart >&2
            cp -a "build/linux/x64/$PA_SOAK_BUILD/bundle" "$PA_SOAK_DATA/bundle"
            bundle="$PA_SOAK_DATA/bundle"
        fi
        Xvfb :99 -screen 0 1600x1000x24 -nolisten tcp >/dev/null 2>&1 &
        for _ in $(seq 50); do [ -e /tmp/.X11-unix/X99 ] && break; sleep 0.2; done
        export DISPLAY=:99
        export XDG_RUNTIME_DIR=/tmp/pa-soak-run && mkdir -p -m 700 "$XDG_RUNTIME_DIR"
        pulseaudio --daemonize=yes --exit-idle-time=-1 >/dev/null 2>&1 || echo "soak: no PulseAudio" >&2
        # shellcheck disable=SC2086 # the wrapper is a command line
        exec nice timeout 10h $PA_SOAK_WRAP "$bundle/kinolune"
    ' 2>&1 | tee "$out/run.log"
rc=${PIPESTATUS[0]}
set -e

{
    echo "# BUG-001 player soak — $stamp"
    echo
    echo "mode: $mode; build: ${bundle:-$build}; cycles: $cycles; exit code: $rc"
} >"$out/report.md"
set +e
python3 "$here/soak_memory.py" "$out/run.log" >>"$out/report.md"
mem_rc=$?
set -e
[ "$mem_rc" -ne 0 ] && [ "$rc" -eq 0 ] && rc=$mem_rc
cat "$out/report.md"
rm -rf "$data"
exit "$rc"
