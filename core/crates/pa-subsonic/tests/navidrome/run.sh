#!/usr/bin/env bash
# SPDX-License-Identifier: MPL-2.0
#
# NET-016 done-when: integration tests against Navidrome in Docker (pinned
# image, see setup.sh) with a generated music library: the client
# (pa-subsonic), and the Subsonic source scanned into the library,
# streamed through the proxy and scrobbled (pa-catalog). Removes its
# container and files on exit; never prunes anything. PA_NAVIDROME_RMI=1
# also removes the image afterwards. Extra arguments go to every
# `cargo test`.
#
#   core/crates/pa-subsonic/tests/navidrome/run.sh
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
export PA_NAVIDROME_CONTAINER="${PA_NAVIDROME_CONTAINER:-pa-net016-navidrome}"
image="${PA_NAVIDROME_IMAGE:-deluan/navidrome:0.58.0}"
work="$(mktemp -d)"

cleanup() {
    docker rm -f "$PA_NAVIDROME_CONTAINER" >/dev/null 2>&1 || true
    rm -rf "$work"
    if [ "${PA_NAVIDROME_RMI:-0}" = 1 ]; then
        docker rmi "$image" >/dev/null 2>&1 || true
    fi
}
trap cleanup EXIT

"$here/setup.sh" "$work" >"$work/env"
# shellcheck disable=SC1091
. "$work/env"
cargo test -p pa-subsonic --features server-tests --test navidrome "$@" -- --test-threads 1
cargo test -p pa-catalog --features server-tests --test subsonic "$@" -- --test-threads 1
