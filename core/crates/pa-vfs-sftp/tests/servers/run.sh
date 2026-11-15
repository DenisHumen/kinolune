#!/usr/bin/env bash
# SPDX-License-Identifier: MPL-2.0
#
# Runs the pa-vfs contract suite against OpenSSH in Docker (NET-006).
# Needs Docker with the compose plugin and cargo. Extra arguments go to
# `cargo test`. PA_SFTP_PORT (default 18622) and PA_COMPOSE_PROJECT
# (default pa-sftp-contract) let several runs share a machine.
#
#   core/crates/pa-vfs-sftp/tests/servers/run.sh
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
work="$(mktemp -d)"
# Throwaway credentials of the throwaway container.
export PA_SFTP_USER=kinolune
export PA_SFTP_PASS=contract-test
export PA_SFTP_KEY_PASS=key-passphrase
export PA_SFTP_DATA="$work/data" PA_SFTP_KEYS="$work/keys"
export PA_SFTP_PORT="${PA_SFTP_PORT:-18622}"
compose=(docker compose -p "${PA_COMPOSE_PROJECT:-pa-sftp-contract}" -f "$here/docker-compose.yml")

cleanup() {
    # Containers and the network only: there are no named volumes.
    "${compose[@]}" down --remove-orphans >/dev/null 2>&1 || true
    rm -rf "$work"
}
trap cleanup EXIT

# Fixture under /media/контракт next to unrelated data.
mkdir -p "$work/data/media" "$work/keys"
cargo run -q -p pa-vfs-sftp --example write_fixture -- "$work/data/media/контракт"
echo x >"$work/data/media/other.txt"
chmod -R a+rX "$work/data"

"${compose[@]}" up -d --quiet-pull

for _ in $(seq 1 120); do
    if [ -f "$work/keys/host_rsa.fp" ] && (exec 3<>"/dev/tcp/127.0.0.1/$PA_SFTP_PORT") 2>/dev/null; then
        break
    fi
    sleep 1
done
if [ ! -f "$work/keys/host_rsa.fp" ]; then
    echo "timeout waiting for sshd" >&2
    "${compose[@]}" logs --tail 50 >&2
    exit 1
fi
sleep 1

export PA_SFTP_ADDR="127.0.0.1:$PA_SFTP_PORT"
export PA_SFTP_KEY_ED25519="$work/keys/id_ed25519" PA_SFTP_KEY_RSA="$work/keys/id_rsa"
export PA_SFTP_HOST_FPS="$(cat "$work"/keys/host_*.fp | tr '\n' ' ')"
cargo test -p pa-vfs-sftp --features server-tests --test contract_servers "$@"
