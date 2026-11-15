#!/usr/bin/env bash
# SPDX-License-Identifier: MPL-2.0
#
# NET-014 done-when: integration tests against Jellyfin in Docker (pinned
# image, see setup.sh) with a generated test library: the client
# (pa-jellyfin), and the Jellyfin source scanned into the library and
# streamed through the proxy (pa-catalog). Removes its container and
# files on exit; never prunes anything. PA_JELLYFIN_RMI=1 also removes
# the image afterwards. Extra arguments go to every `cargo test`.
#
#   core/crates/pa-jellyfin/tests/server/run.sh
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
export PA_JELLYFIN_CONTAINER="${PA_JELLYFIN_CONTAINER:-pa-jellyfin-test}"
image="${PA_JELLYFIN_IMAGE:-jellyfin/jellyfin:10.10.7}"
work="$(mktemp -d)"

cleanup() {
    docker rm -f "$PA_JELLYFIN_CONTAINER" >/dev/null 2>&1 || true
    rm -rf "$work"
    if [ "${PA_JELLYFIN_RMI:-0}" = 1 ]; then
        docker rmi "$image" >/dev/null 2>&1 || true
    fi
}
trap cleanup EXIT

"$here/setup.sh" "$work" >"$work/env"
# shellcheck disable=SC1091
. "$work/env"
cargo test -p pa-jellyfin --features server-tests --test server "$@" -- --test-threads 1
cargo test -p pa-catalog --features server-tests --test jellyfin "$@" -- --test-threads 1
