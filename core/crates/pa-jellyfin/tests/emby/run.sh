#!/usr/bin/env bash
# SPDX-License-Identifier: MPL-2.0
#
# NET-015 done-when: integration tests against Emby Server in Docker
# (pinned image, see setup.sh) with a generated test library: the client
# (pa-jellyfin with ServerKind::Emby), and the Emby source scanned into the
# library and streamed through the proxy (pa-catalog). Local only, not in
# CI: Emby Server is proprietary software under Emby's terms of use
# (ADR-NNNN). Removes its container and files on exit; never prunes
# anything. PA_EMBY_RMI=1 also removes the image afterwards. Extra
# arguments go to every `cargo test`.
#
#   core/crates/pa-jellyfin/tests/emby/run.sh
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
export PA_EMBY_CONTAINER="${PA_EMBY_CONTAINER:-pa-emby-test}"
image="${PA_EMBY_IMAGE:-emby/embyserver:4.10.2.0}"
work="$(mktemp -d)"

cleanup() {
    docker rm -f "$PA_EMBY_CONTAINER" >/dev/null 2>&1 || true
    # The server writes as the calling user, so plain rm works.
    rm -rf "$work"
    if [ "${PA_EMBY_RMI:-0}" = 1 ]; then
        docker rmi "$image" >/dev/null 2>&1 || true
    fi
}
trap cleanup EXIT

"$here/setup.sh" "$work" >"$work/env"
# shellcheck disable=SC1091
. "$work/env"
cargo test -p pa-jellyfin --features server-tests --test emby "$@" -- --test-threads 1
cargo test -p pa-catalog --features server-tests --test emby "$@" -- --test-threads 1
