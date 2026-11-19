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
#   tools/qa/e2e/run.sh [--force] [--soak N] [--profile] [--env K=V]...
#                       [--keep-data] [-- extra flutter test args]
#
# --soak N     after the scenarios, repeat browse → play → seek → back N
#              times and report memory, stalls and errors. BUG-001: the
#              memory slope of the last PA_E2E_SOAK_WINDOW cycles (50; the
#              first PA_E2E_SOAK_WARMUP=10 cycles left out) is projected
#              to 50 cycles; above PA_E2E_SOAK_LIMIT_MB (50) the run fails
# --profile    a profile (AOT) build of the scenarios, run as the app
#              itself instead of `flutter test` (debug, JIT): memory as
#              users have it
# --env K=V    an extra variable for the app (e.g. MALLOC_ARENA_MAX=2 for
#              a bisect); PA_E2E_SOAK_MODE in the environment picks what a
#              soak cycle opens (live_test.dart: app, no-trickplay,
#              player-smb, player-local)
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

force=false soak=0 keep=false profile=false extra=() appenv=()
while [ $# -gt 0 ]; do
    case "$1" in
        --force) force=true ;;
        --soak) soak="${2:?--soak needs a number}"; shift ;;
        --keep-data) keep=true ;;
        --profile) profile=true ;;
        --env) appenv+=(-e "${2:?--env needs K=V}"); shift ;;
        --) shift; extra=("$@"); break ;;
        *) echo "usage: run.sh [--force] [--soak N] [--profile] [--env K=V]... [--keep-data] [-- flutter test args]" >&2; exit 64 ;;
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
    -e CARGO_BUILD_JOBS=4 -e PA_E2E_PROFILE="$profile" -e PA_E2E_DATA="$data" \
    -e PA_E2E_LIBRARY="$home/library" -e PA_E2E_SOAK_MODE "${appenv[@]}" \
    "$runner_image" bash -c '
        export PATH=$HOME/opt/flutter/bin:$HOME/.cargo/bin:$PATH
        Xvfb :99 -screen 0 1600x1000x24 -nolisten tcp >/dev/null 2>&1 &
        for _ in $(seq 50); do [ -e /tmp/.X11-unix/X99 ] && break; sleep 0.2; done
        export DISPLAY=:99
        # No sound card: PulseAudio with its always-present null sink.
        export XDG_RUNTIME_DIR=/tmp/pa-e2e-run && mkdir -p -m 700 "$XDG_RUNTIME_DIR"
        pulseaudio --daemonize=yes --exit-idle-time=-1 >/dev/null 2>&1 || echo "e2e: no PulseAudio" >&2
        flutter pub get >/dev/null
        if [ "$PA_E2E_PROFILE" != true ]; then
            exec nice flutter test integration_test/live_test.dart -d linux --reporter expanded "$@"
        fi
        # The scenarios as an AOT app: the test framework runs them from
        # main() and prints the summary; then the app is closed. It runs
        # from a copy, so another run may rebuild meanwhile.
        nice flutter build linux --profile -t integration_test/live_test.dart "$@" >&2 || exit 1
        cp -a build/linux/x64/profile/bundle "$PA_E2E_DATA/bundle"
        rc=1
        while IFS= read -r line; do
            printf "%s\n" "$line"
            case "$line" in
                *"All tests passed"*) rc=0; break ;;
                *"Some tests failed"*|*"tests failed"*) break ;;
            esac
        done < <(nice timeout 7h "$PA_E2E_DATA/bundle/kinolune" 2>&1)
        pkill -f "$PA_E2E_DATA/bundle/kinolune" || true
        exit $rc
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

# --- BUG-001: memory slope of the soak ------------------------------------
if [ "$soak" -gt 0 ]; then
    set +e
    python3 "$here/soak_memory.py" "$out/run.log" >>"$out/report.md"
    mem_rc=$?
    set -e
    [ "$mem_rc" -ne 0 ] && [ "$rc" -eq 0 ] && rc=$mem_rc
fi
cat "$out/report.md"
$keep || rm -rf "$data"
exit "$rc"
