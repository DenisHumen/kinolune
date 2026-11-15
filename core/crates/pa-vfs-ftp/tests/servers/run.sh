#!/usr/bin/env bash
# SPDX-License-Identifier: MPL-2.0
#
# Runs the pa-vfs contract suite against vsftpd in Docker (NET-006): plain
# FTP, explicit FTPS and implicit FTPS. Needs Docker with the compose
# plugin, openssl and cargo. Extra arguments go to `cargo test`.
# PA_COMPOSE_PROJECT (default pa-ftp-contract) names the containers.
#
#   core/crates/pa-vfs-ftp/tests/servers/run.sh
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
work="$(mktemp -d)"
# Throwaway credentials of the throwaway containers.
export PA_FTP_USER=kinolune
export PA_FTP_PASS=contract-test
export PA_FTP_DATA="$work/data" PA_FTP_CERTS="$work/certs"
compose=(docker compose -p "${PA_COMPOSE_PROJECT:-pa-ftp-contract}" -f "$here/docker-compose.yml")

cleanup() {
    # Containers and the network only: there are no named volumes.
    "${compose[@]}" down --remove-orphans >/dev/null 2>&1 || true
    rm -rf "$work"
}
trap cleanup EXIT

# Fixture under /media/контракт next to unrelated data.
mkdir -p "$work/data/media" "$work/certs"
cargo run -q -p pa-vfs-ftp --example write_fixture -- "$work/data/media/контракт"
echo x >"$work/data/media/other.txt"

# A self-signed certificate, as on a NAS; the tests pin its fingerprint.
openssl req -x509 -newkey rsa:2048 -nodes -days 2 -subj "/CN=nas.test" \
    -keyout "$work/certs/key.pem" -out "$work/certs/cert.pem" 2>/dev/null
fp="$(openssl x509 -in "$work/certs/cert.pem" -outform DER | sha256sum | cut -d' ' -f1)"
export PA_FTP_PIN="sha256:$fp"
chmod -R a+rX "$work"

"${compose[@]}" up -d --quiet-pull

# vsftpd starts after `apk add`; the published port answers before that.
for svc in plain explicit implicit; do
    for i in $(seq 1 120); do
        if "${compose[@]}" exec -T "$svc" pgrep vsftpd >/dev/null 2>&1; then break; fi
        if [ "$i" = 120 ]; then
            echo "timeout waiting for vsftpd ($svc)" >&2
            "${compose[@]}" logs --tail 50 "$svc" >&2
            exit 1
        fi
        sleep 1
    done
done
sleep 1

export PA_FTP_PLAIN=127.0.0.1:18621
export PA_FTP_EXPLICIT=127.0.0.1:18631
export PA_FTP_IMPLICIT=127.0.0.1:18641
cargo test -p pa-vfs-ftp --features server-tests --test contract_servers "$@"
