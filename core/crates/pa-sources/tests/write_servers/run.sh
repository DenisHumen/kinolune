#!/usr/bin/env bash
# SPDX-License-Identifier: MPL-2.0
#
# Runs the NET-023 write contract against real servers in Docker: Samba,
# Apache mod_dav, rclone serve webdav, OpenSSH and vsftpd, each writable
# plus a read-only variant, and moves between them and a local folder.
# Needs Docker with the compose plugin and cargo. libsmb2 comes from
# $PA_SMB2_LIB or tools/libsmb2/out/linux-x86_64 (built in Docker if
# missing). Extra arguments are test name filters (e.g. `smb`).
#
#   PA_W_BIG        size of the resumed upload, bytes (default 1 GiB)
#   PA_W_WORK       scratch folder (default: mktemp -d; it holds the big
#                   uploads, so put it on a disk, not on a tmpfs)
#   PA_W_PROJECT    compose project and container name prefix
#                   (default pa-test-net023)
#   PA_W_PORT_BASE  first of the host ports used (default 19440)
#
#   core/crates/pa-sources/tests/write_servers/run.sh
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
repo="$(cd "$here/../../../../.." && pwd)"
work="${PA_W_WORK:-$(mktemp -d)}"
mkdir -p "$work"
work="$(cd "$work" && pwd)"
project="${PA_W_PROJECT:-pa-test-net023}"
base="${PA_W_PORT_BASE:-19440}"

# Throwaway credentials of the throwaway containers.
export PA_W_USER=kinolune PA_W_PASS=contract-test
export PA_W_DATA="$work/data" PA_W_RO="$work/ro" PA_W_AUTH="$work/auth" PA_W_KEYS="$work/keys"
export PA_W_UID="$(id -u)" PA_W_GID="$(id -g)"
export PA_W_SMB_PORT=$((base)) PA_W_APACHE_PORT=$((base + 1)) PA_W_RCLONE_PORT=$((base + 2))
export PA_W_RCLONE_RO_PORT=$((base + 3)) PA_W_SFTP_PORT=$((base + 4)) PA_W_FTP_PORT=$((base + 5))
export PA_W_FTP_PASV="$((base + 20))-$((base + 39))"
export PA_W_BIG="${PA_W_BIG:-1073741824}"
compose=(docker compose -p "$project" -f "$here/docker-compose.yml")

cleanup() {
    # Files written by the containers belong to their users: remove them
    # from a container of our own, then our containers and network only.
    docker run --rm --name "$project-cleanup" -v "$work:/w" alpine:3.22 \
        sh -c 'rm -rf /w/data /w/ro /w/auth /w/keys' >/dev/null 2>&1 || true
    "${compose[@]}" down --remove-orphans >/dev/null 2>&1 || true
    rm -rf "$work" 2>/dev/null || true
}
trap cleanup EXIT

if [ -z "${PA_SMB2_LIB:-}" ]; then
    lib="$repo/tools/libsmb2/out/linux-x86_64/lib/libsmb2.so.1"
    [ -f "$lib" ] || "$repo/tools/libsmb2/docker-build.sh" linux-x86_64
    export PA_SMB2_LIB="$lib"
fi

# Every server writes into its own folder of the shared scratch tree; the
# read-only folder holds one file and is mounted read-only everywhere.
mkdir -p "$PA_W_DATA"/{smb,dav,rclone,sftp,ftp} "$PA_W_RO" "$PA_W_AUTH" "$PA_W_KEYS"
echo "read-only" >"$PA_W_RO/film.mkv"
ha1="$(printf '%s:kinolune:%s' "$PA_W_USER" "$PA_W_PASS" | md5sum | cut -d' ' -f1)"
printf '%s:kinolune:%s\n' "$PA_W_USER" "$ha1" >"$PA_W_AUTH/digest"
chmod -R a+rwX "$PA_W_DATA" "$PA_W_KEYS"
chmod -R a+rX,go-w "$PA_W_RO" "$PA_W_AUTH"

"${compose[@]}" up -d --quiet-pull

wait_tcp() { # port
    for _ in $(seq 1 180); do
        if (exec 3<>"/dev/tcp/127.0.0.1/$1") 2>/dev/null; then return 0; fi
        sleep 1
    done
    echo "timeout waiting for port $1" >&2
    "${compose[@]}" logs --tail 50 >&2
    return 1
}
# Docker's proxy accepts before the daemons run (they install packages
# first): wait inside the containers too.
wait_in() { # service port
    for _ in $(seq 1 180); do
        if "${compose[@]}" exec -T "$1" nc -z 127.0.0.1 "$2" >/dev/null 2>&1; then return 0; fi
        sleep 1
    done
    echo "timeout waiting for $1" >&2
    "${compose[@]}" logs --tail 50 "$1" >&2
    return 1
}
for p in "$PA_W_APACHE_PORT" "$PA_W_RCLONE_PORT" "$PA_W_RCLONE_RO_PORT"; do wait_tcp "$p"; done
wait_in samba 445
wait_in openssh 2222
wait_in vsftpd 2121

export PA_W_SMB="127.0.0.1:$PA_W_SMB_PORT"
export PA_W_APACHE="http://127.0.0.1:$PA_W_APACHE_PORT/dav/"
export PA_W_RCLONE="http://127.0.0.1:$PA_W_RCLONE_PORT/"
export PA_W_RCLONE_RO="http://127.0.0.1:$PA_W_RCLONE_RO_PORT/"
export PA_W_SFTP="127.0.0.1:$PA_W_SFTP_PORT"
export PA_W_FTP="127.0.0.1:$PA_W_FTP_PORT"
export PA_W_LOCAL="$PA_W_DATA/local"
mkdir -p "$PA_W_LOCAL"
cargo test -p pa-sources --features server-tests --test write_servers -- --test-threads=4 --nocapture "$@"
