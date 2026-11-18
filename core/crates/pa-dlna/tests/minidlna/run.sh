#!/usr/bin/env bash
# SPDX-License-Identifier: MPL-2.0
#
# NET-018 done-when: tests against minidlna in Docker (setup.sh) — the
# client (pa-dlna: SSDP search, description, browse, streaming) and the
# DLNA server scanned into the library and played through the proxy
# (pa-catalog). Removes its container and files on exit; never prunes
# anything. PA_DLNA_RMI=1 also removes the image afterwards. Extra
# arguments go to every `cargo test`.
#
#   core/crates/pa-dlna/tests/minidlna/run.sh
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
export PA_DLNA_CONTAINER="${PA_DLNA_CONTAINER:-pa-test-net018-minidlna-$$}"
image="${PA_DLNA_IMAGE:-pa-test-net018-minidlna:1}"
work="$(mktemp -d)"

cleanup() {
    docker rm -f "$PA_DLNA_CONTAINER" >/dev/null 2>&1 || true
    rm -rf "$work"
    if [ "${PA_DLNA_RMI:-0}" = 1 ]; then
        docker rmi "$image" >/dev/null 2>&1 || true
    fi
}
trap cleanup EXIT

"$here/setup.sh" "$work" >"$work/env"
# shellcheck disable=SC1091
. "$work/env"
cargo test -p pa-dlna --features server-tests --test minidlna "$@" -- --test-threads 1
cargo test -p pa-catalog --features server-tests --test dlna "$@" -- --test-threads 1
