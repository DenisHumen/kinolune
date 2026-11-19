#!/usr/bin/env bash
# SPDX-License-Identifier: MPL-2.0
#
# QA-016: live e2e scenarios. The real app (real Rust core, real libmpv,
# real network) starts from scratch on a Linux desktop under Xvfb, adds
# the SMB share of the e2e stand (tools/qa/e2e/stand.sh), scans it and
# gets clicked through: home, a film, playback, seek, «Продолжить», the
# next episode, music, search, settings, diagnostics. Screenshots and a
# report go to $PA_E2E_HOME/runs/<time>/.
#
#   tools/qa/e2e/run.sh [--force] [--soak N] [--keep-data] [-- extra flutter test args]
#
# --soak N     after the scenarios, repeat browse → play → seek → back N
#              times and report memory, stalls and errors
# --force      run outside the allowed window (only for runs the owner
#              approved)
#
# Owner's rule: live runs only on weekdays 10:00–18:00 Europe/Kyiv. Not
# part of CI (CI stays hermetic). Optional TMDB check: PA_TMDB_API_KEY in
# the environment or in a gitignored .env.local at the repo root; without
# it the recognition step is skipped with a note. Keys are never written
# anywhere by this script.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo="$(cd "$here/../../.." && pwd)"
home="${PA_E2E_HOME:-$HOME/pa-build/e2e-media}"
runner_image=pa-test-e2e-runner:2
net=pa-test-e2e-net

force=false soak=0 keep=false extra=()
while [ $# -gt 0 ]; do
    case "$1" in
        --force) force=true ;;
        --soak) soak="${2:?--soak needs a number}"; shift ;;
        --keep-data) keep=true ;;
        --) shift; extra=("$@"); break ;;
        *) echo "usage: run.sh [--force] [--soak N] [--keep-data] [-- flutter test args]" >&2; exit 64 ;;
    esac
    shift
done

# --- the owner's window -------------------------------------------------
in_window() {
    local dow hm
    dow="$(TZ=Europe/Kyiv date +%u)"   # 1..7, Monday = 1
    hm="$(TZ=Europe/Kyiv date +%H%M)"
    [ "$dow" -le 5 ] && [ "$((10#$hm))" -ge 1000 ] && [ "$((10#$hm))" -lt 1800 ]
}
if ! in_window; then
    if $force; then
        echo "e2e: outside weekdays 10:00–18:00 Europe/Kyiv, running because of --force" >&2
    else
        echo "e2e: live runs only on weekdays 10:00–18:00 Europe/Kyiv (now $(TZ=Europe/Kyiv date '+%a %H:%M')). Use --force for a run the owner approved." >&2
        exit 75
    fi
fi

# --- stand, credentials, optional TMDB key ------------------------------
"$here/stand.sh" status >/dev/null || "$here/stand.sh" up
set -a
# shellcheck disable=SC1091
. "$home/stand.env"
set +a
if [ -z "${PA_TMDB_API_KEY:-}" ] && [ -f "$repo/.env.local" ]; then
    PA_TMDB_API_KEY="$(sed -n 's/^PA_TMDB_API_KEY=//p' "$repo/.env.local" | tail -1)"
fi
export PA_TMDB_API_KEY="${PA_TMDB_API_KEY:-}"

# --- our libsmb2 (LGPL) from the newest app release ---------------------
native="$home/native"
if [ ! -f "$native/libsmb2.so.1" ]; then
    mkdir -p "$native"
    url="$(curl -fsSL https://api.github.com/repos/DenisHumen/kinolune/releases?per_page=20 |
        grep -o '"browser_download_url": *"[^"]*linux-x86_64\.tar\.gz"' | head -1 | sed 's/.*"\(https[^"]*\)"/\1/')"
    [ -n "$url" ] || { echo "e2e: no Linux release to take libsmb2 from" >&2; exit 1; }
    echo "e2e: libsmb2 from $url" >&2
    tmp="$(mktemp -d)"
    curl -fsSL -o "$tmp/app.tar.gz" "$url"
    tar -xzf "$tmp/app.tar.gz" -C "$tmp" --wildcards '*/lib/libsmb2.so*'
    cp -L "$(find "$tmp" -name 'libsmb2.so.1' | head -1)" "$native/libsmb2.so.1"
    rm -rf "$tmp"
fi

# --- the run --------------------------------------------------------------
stamp="$(date +%Y%m%d-%H%M%S)"
out="$home/runs/$stamp"
data="$home/runs/$stamp-data"
mkdir -p "$out/screenshots" "$data"
echo "e2e: results in $out" >&2

app="$repo/clients/flutter/app"
set +e
docker run --rm --name "pa-test-e2e-run-$$" --network "$net" \
    -u "$(id -u):$(id -g)" -e HOME="$HOME" -v "$HOME:$HOME" -w "$app" --cpus 8 \
    -e LANG=ru_RU.UTF-8 -e LC_ALL=ru_RU.UTF-8 \
    -e PA_E2E_SMB_HOST=kinolune-e2e -e PA_E2E_SMB_USER -e PA_E2E_SMB_PASS \
    -e PA_E2E_OUT="$out" -e PA_E2E_SOAK="$soak" -e PA_TMDB_API_KEY \
    -e PA_SMB2_LIB="$native/libsmb2.so.1" \
    -e XDG_DATA_HOME="$data/data" -e XDG_CONFIG_HOME="$data/config" -e XDG_CACHE_HOME="$data/cache" \
    -e CARGO_BUILD_JOBS=4 \
    "$runner_image" bash -c '
        export PATH=$HOME/opt/flutter/bin:$HOME/.cargo/bin:$PATH
        Xvfb :99 -screen 0 1600x1000x24 -nolisten tcp >/dev/null 2>&1 &
        for _ in $(seq 50); do [ -e /tmp/.X11-unix/X99 ] && break; sleep 0.2; done
        export DISPLAY=:99
        # No sound card: PulseAudio with its always-present null sink.
        export XDG_RUNTIME_DIR=/tmp/pa-e2e-run && mkdir -p -m 700 "$XDG_RUNTIME_DIR"
        pulseaudio --daemonize=yes --exit-idle-time=-1 >/dev/null 2>&1 || echo "e2e: no PulseAudio" >&2
        flutter pub get >/dev/null
        nice flutter test integration_test/live_test.dart -d linux --reporter expanded "$@"
    ' _ "${extra[@]}" 2>&1 | tee "$out/run.log"
rc=${PIPESTATUS[0]}
set -e

# --- report ---------------------------------------------------------------
{
    echo "# QA-016 live e2e — $stamp"
    echo
    echo "exit code: $rc; soak cycles requested: $soak"
    echo
    echo '## Steps'
    grep -E '^E2E (STEP|SKIP|FAIL|SOAK|MEM|NOTE)' "$out/run.log" | sed 's/^/- /' || true
    echo
    echo '## Errors in the log'
    grep -ciE 'exception|error' "$out/run.log" | sed 's/^/lines mentioning exception\/error: /' || true
} >"$out/report.md"
cat "$out/report.md"
$keep || rm -rf "$data"
exit "$rc"
