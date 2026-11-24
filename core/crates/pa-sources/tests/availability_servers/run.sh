#!/usr/bin/env bash
# SPDX-License-Identifier: MPL-2.0
#
# Runs the NET-024 availability contract (tests/availability_servers.rs):
# the Samba, WebDAV (rclone), SFTP and FTP servers of the NET-023 write
# contract under the compose project pa-test-net024, plus a throwaway
# Jellyfin (pa-test-net024-jellyfin). The test stops and starts these
# containers itself. Removes only its own containers.
#
#   PA_AV_NO_JELLYFIN=1   skip Jellyfin
#   PA_AV_SLOW=1          also the 5-minute backoff case
#
#   core/crates/pa-sources/tests/availability_servers/run.sh
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
export PA_W_PROJECT="${PA_W_PROJECT:-pa-test-net024}"
export PA_W_PORT_BASE="${PA_W_PORT_BASE:-19540}"
export PA_W_TEST=availability_servers

jellyfin="$PA_W_PROJECT-jellyfin"
cleanup() { docker rm -f "$jellyfin" >/dev/null 2>&1 || true; }
trap cleanup EXIT
cleanup

if [ -z "${PA_AV_NO_JELLYFIN:-}" ]; then
    port="$((PA_W_PORT_BASE + 50))"
    docker run -d --name "$jellyfin" -p "127.0.0.1:$port:8096" \
        "${PA_JELLYFIN_IMAGE:-jellyfin/jellyfin:10.10.7}" >/dev/null
    for _ in $(seq 1 180); do
        if curl -fsS "http://127.0.0.1:$port/System/Ping" >/dev/null 2>&1; then break; fi
        sleep 1
    done
    export PA_AV_JELLYFIN="127.0.0.1:$port" PA_AV_JELLYFIN_CONTAINER="$jellyfin"
fi

"$here/../write_servers/run.sh" "$@"
