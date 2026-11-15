#!/usr/bin/env bash
# SPDX-License-Identifier: MPL-2.0
#
# NET-019 end to end on the Samba and WebDAV stand of LIB-018
# (pa-catalog/tests/servers/docker-compose.yml): a 4 GiB movie is copied
# from SMB while the server drops out and the copying process is killed,
# then played from the copy with the servers stopped; an episode with its
# subtitles is copied from WebDAV and played offline.
# Needs Docker with the compose plugin and cargo. libsmb2 comes from
# $PA_SMB2_LIB or tools/libsmb2/out/linux-x86_64 (built in Docker if
# missing). $PA_OFFLINE_BIG_MIB sets the size of the big file (default
# 4096). Extra arguments go to `cargo test`.
#
#   core/crates/pa-offline/tests/servers/run.sh
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
repo="$(cd "$here/../../../../.." && pwd)"
stand="$repo/core/crates/pa-catalog/tests/servers/docker-compose.yml"
work="$(mktemp -d)"
# Throwaway credentials of the throwaway containers.
export PA_SERVERS_USER=kinolune
export PA_SERVERS_PASS=offline-test
export PA_SERVERS_DATA="$work/data" PA_SERVERS_AUTH="$work/auth"
export PA_SERVERS_COMPOSE="docker compose -p pa-offline-servers -f $stand"
compose=(docker compose -p pa-offline-servers -f "$stand")

cleanup() {
    # Containers and the network of this project only: no named volumes.
    "${compose[@]}" down --remove-orphans >/dev/null 2>&1 || true
    rm -rf "$work"
}
trap cleanup EXIT

if [ -z "${PA_SMB2_LIB:-}" ]; then
    lib="$repo/tools/libsmb2/out/linux-x86_64/lib/libsmb2.so.1"
    [ -f "$lib" ] || "$repo/tools/libsmb2/docker-build.sh" linux-x86_64
    export PA_SMB2_LIB="$lib"
fi

mkdir -p "$work/data" "$work/auth"
big_mib="${PA_OFFLINE_BIG_MIB:-4096}"
export PA_OFFLINE_BIG="Movies/Big Buck Bunny (2008)/Big.Buck.Bunny.2008.1080p.mkv"
mkdir -p "$(dirname "$work/data/$PA_OFFLINE_BIG")"
head -c "$((big_mib * 1024 * 1024))" /dev/urandom >"$work/data/$PA_OFFLINE_BIG"
mkdir -p "$work/data/Shows/Dark/Season 1"
head -c 3000000 /dev/urandom >"$work/data/Shows/Dark/Season 1/Dark.S01E01.Secrets.1080p.mkv"
head -c 2000000 /dev/urandom >"$work/data/Shows/Dark/Season 1/Dark.S01E02.Lies.1080p.mkv"
printf '1\n00:00:01,000 --> 00:00:03,000\nHallo\n' \
    >"$work/data/Shows/Dark/Season 1/Dark.S01E01.Secrets.1080p.de.srt"
printf '%s:{PLAIN}%s\n' "$PA_SERVERS_USER" "$PA_SERVERS_PASS" >"$work/auth/htpasswd"
chmod -R a+rX "$work"

"${compose[@]}" up -d --quiet-pull

wait_smb() {
    for _ in $(seq 1 180); do
        if "${compose[@]}" exec -T samba nc -z 127.0.0.1 445 >/dev/null 2>&1; then return 0; fi
        sleep 1
    done
    echo "timeout waiting for samba" >&2
    "${compose[@]}" logs --tail 50 samba >&2
    return 1
}
wait_http() {
    for _ in $(seq 1 180); do
        if curl -s -o /dev/null "$1"; then return 0; fi
        sleep 1
    done
    echo "timeout waiting for $1" >&2
    return 1
}
wait_smb
wait_http http://127.0.0.1:18581/dav/

export PA_SERVERS_SMB="smb://127.0.0.1:18545/media"
export PA_SERVERS_WEBDAV="http://127.0.0.1:18581/dav/"
cargo test -p pa-offline --features server-tests --test servers "$@" -- --nocapture --test-threads=1
