#!/usr/bin/env bash
# SPDX-License-Identifier: MPL-2.0
#
# Runs the UI-028 SMB browse tests (tests/smb_browse.rs) against Samba in
# Docker: share listing, finding a typed path (`[homes]`, a share deep in a
# home folder), guest logon and every error the add flow tells apart.
# Needs Docker and cargo; libsmb2 from $PA_SMB2_LIB or
# tools/libsmb2/out/linux-x86_64. Removes only its own container.
#
#   core/crates/pa-sources/tests/smb_server/run.sh
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
repo="$(cd "$here/../../../../.." && pwd)"
name="${PA_SMB_CONTAINER:-pa-ui028-samba}"
port="${PA_SMB_PORT:-18545}"
# Throwaway password of the throwaway container.
export PA_SMB_PASS=browse-test

cleanup() { docker rm -f "$name" >/dev/null 2>&1 || true; }
trap cleanup EXIT
cleanup

if [ -z "${PA_SMB2_LIB:-}" ]; then
    lib="$repo/tools/libsmb2/out/linux-x86_64/lib/libsmb2.so.1"
    [ -f "$lib" ] || "$repo/tools/libsmb2/docker-build.sh" linux-x86_64
    export PA_SMB2_LIB="$lib"
fi

docker run -d --name "$name" -p "127.0.0.1:$port:445" -e PA_SMB_PASS \
    -v "$here/smb.conf:/etc/pa/smb.conf:ro" -v "$here/start.sh:/etc/pa/start.sh:ro" \
    alpine:3.22 sh /etc/pa/start.sh >/dev/null

for _ in $(seq 1 180); do
    if docker exec "$name" nc -z 127.0.0.1 445 >/dev/null 2>&1; then break; fi
    sleep 1
done

export PA_SMB_SERVER="127.0.0.1:$port"
cargo test -p pa-sources --features server-tests --test smb_browse "$@"
