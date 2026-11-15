#!/usr/bin/env bash
# SPDX-License-Identifier: MPL-2.0
#
# LIB-023 benchmark: scan, recognition, provider metadata and posters of a
# ~200-group library on a Samba share (Docker, netem LAN latency). The
# fixture is sparse (a few MB on disk for terabytes of apparent size) and is
# kept in $PA_BENCH_DATA_DIR between runs (default: a temporary directory).
# Needs Docker with the compose plugin and cargo; libsmb2 from $PA_SMB2_LIB
# or tools/libsmb2/out/linux-x86_64; FFmpeg (probe) from $PA_FFMPEG_LIB_DIR
# if set. Extra arguments go to `cargo test`.
#
#   core/crates/pa-catalog/tests/large_library/run.sh
#
# Knobs: PA_BENCH_NETEM (default "delay 1ms rate 940mbit"),
# PA_BENCH_STALL_SECS (watchdog, 120), PA_BENCH_MAX_SECS (time budget,
# none), PA_BENCH_TMDB_MS / _ANILIST_MS / _SHIKIMORI_MS / _IMAGE_MS
# (provider latencies).
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
repo="$(cd "$here/../../../../.." && pwd)"
work="$(mktemp -d)"
# Throwaway credentials of the throwaway container.
export PA_BENCH_USER=kinolune
export PA_BENCH_PASS=bench-test
export PA_BENCH_DATA_DIR="${PA_BENCH_DATA_DIR:-$work/data}"
export PA_BENCH_PORT="${PA_BENCH_PORT:-18645}"
compose=(docker compose -p "${PA_COMPOSE_PROJECT:-pa-lib023-bench}" -f "$here/docker-compose.yml")

cleanup() {
    # The container and its network only: there are no named volumes.
    "${compose[@]}" down --remove-orphans >/dev/null 2>&1 || true
    rm -rf "$work"
}
trap cleanup EXIT

if [ -z "${PA_SMB2_LIB:-}" ]; then
    lib="$repo/tools/libsmb2/out/linux-x86_64/lib/libsmb2.so.1"
    [ -f "$lib" ] || "$repo/tools/libsmb2/docker-build.sh" linux-x86_64
    export PA_SMB2_LIB="$lib"
fi

mkdir -p "$PA_BENCH_DATA_DIR"
chmod a+rx "$PA_BENCH_DATA_DIR"
"${compose[@]}" up -d --quiet-pull
for _ in $(seq 1 180); do
    if "${compose[@]}" exec -T samba nc -z 127.0.0.1 445 >/dev/null 2>&1; then break; fi
    sleep 1
done
sleep 1

export PA_BENCH_DATA="$PA_BENCH_DATA_DIR"
export PA_BENCH_SMB="smb://127.0.0.1:$PA_BENCH_PORT/media"
cargo test -p pa-catalog --features server-tests --test large_library "$@" -- --nocapture
