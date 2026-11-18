#!/usr/bin/env bash
# SPDX-License-Identifier: MPL-2.0
#
# NET-010 done-when: a "Windows share" (Samba + wsdd, see Dockerfile) and a
# Jellyfin server in Docker are found by pa-lan-probe on the Docker
# bridge. Starts two throwaway containers named pa-test-net010-*-<pid>,
# runs the tests, removes the containers on exit; never prunes anything.
# PA_LAN_RMI=1 also removes the Samba test image afterwards (the Jellyfin
# image is pinned and shared with the NET-014 tests: kept). Extra
# arguments go to `cargo test`.
#
#   core/crates/pa-lan-probe/tests/lan/run.sh
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
smb_image="${PA_LAN_SMB_IMAGE:-pa-test-net010-samba:1}"
jf_image="${PA_JELLYFIN_IMAGE:-jellyfin/jellyfin:10.10.7}"
smb="pa-test-net010-smb-$$"
jf="pa-test-net010-jellyfin-$$"

cleanup() {
    docker rm -f "$smb" "$jf" >/dev/null 2>&1 || true
    if [ "${PA_LAN_RMI:-0}" = 1 ]; then
        docker rmi "$smb_image" >/dev/null 2>&1 || true
    fi
}
trap cleanup EXIT

docker build -q -t "$smb_image" "$here" >/dev/null
docker run -d --name "$smb" "$smb_image" >/dev/null
# No config volume: a fresh Jellyfin answers auto-discovery before its
# setup wizard; the container keeps nothing.
docker run -d --name "$jf" "$jf_image" >/dev/null

ip_of() { docker inspect -f '{{range .NetworkSettings.Networks}}{{.IPAddress}}{{end}}' "$1"; }
smb_ip="$(ip_of "$smb")"
jf_ip="$(ip_of "$jf")"
gw="$(docker inspect -f '{{range .NetworkSettings.Networks}}{{.Gateway}}{{end}}' "$smb")"

# Jellyfin starts its discovery listener a few seconds after the web
# server; wait for the web server, then a little.
for _ in $(seq 1 90); do
    curl -fsS "http://$jf_ip:8096/System/Info/Public" >/dev/null 2>&1 && break
    sleep 1
done
sleep 3

export PA_LAN_IFACE="$gw" PA_LAN_SMB_IP="$smb_ip" PA_LAN_JELLYFIN_IP="$jf_ip"
cargo test -p pa-lan-probe --features server-tests --test lan_servers "$@" -- --test-threads 1
