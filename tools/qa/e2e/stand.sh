#!/usr/bin/env bash
# SPDX-License-Identifier: MPL-2.0
#
# QA-016: the long-lived e2e media stand on the build server — a Samba
# container with a real test library (Blender open movies + a synthetic
# library with realistic names), announced on the network like a NAS.
#
#   tools/qa/e2e/stand.sh up [--rebuild-library] [--no-real]
#   tools/qa/e2e/stand.sh status
#   tools/qa/e2e/stand.sh down [--purge]
#
# Everything it owns is named pa-test-e2e-*: the container pa-test-e2e-smb,
# the network pa-test-e2e-net, the volume pa-test-e2e-media (a bind of
# $PA_E2E_HOME/library), the images pa-test-e2e-smb:1 and
# pa-test-e2e-runner:2. It never touches other containers and never
# prunes. `up` is idempotent: it only builds or starts what is missing.
#
# Data lives in $PA_E2E_HOME (default ~/pa-build/e2e-media):
#   stand.env  throwaway SMB credentials, generated on the first `up`
#              (mode 600; never commit, never copy into the repo)
#   cache/     downloads of the real films (checked by sha256)
#   library/   the read-only share «media»
#   scratch/   the read-write share «scratch»
#   runs/      results of tools/qa/e2e/run.sh
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo="$(cd "$here/../../.." && pwd)"
home="${PA_E2E_HOME:-$HOME/pa-build/e2e-media}"
name=pa-test-e2e-smb
net=pa-test-e2e-net
vol=pa-test-e2e-media
smb_image=pa-test-e2e-smb:1
runner_image=pa-test-e2e-runner:2
# Host port for SMB clients on the LAN (the owner's Mac); 445 itself may
# be taken on the host. Inside $net the stand answers on 445.
port="${PA_E2E_SMB_PORT:-4450}"

log() { printf 'stand: %s\n' "$*" >&2; }

ensure_creds() {
    mkdir -p "$home"
    if [ ! -f "$home/stand.env" ]; then
        umask 077
        pass="$(head -c 18 /dev/urandom | base64 | tr -d '/+=' | cut -c1-20)"
        printf 'PA_E2E_SMB_USER=e2e-viewer\nPA_E2E_SMB_PASS=%s\n' "$pass" >"$home/stand.env"
        log "new throwaway credentials in $home/stand.env"
    fi
    set -a
    # shellcheck disable=SC1091
    . "$home/stand.env"
    set +a
}

ensure_images() {
    docker image inspect "$smb_image" >/dev/null 2>&1 ||
        docker build -q -t "$smb_image" "$here/stand" >/dev/null
    docker image inspect "$runner_image" >/dev/null 2>&1 ||
        docker build -q -t "$runner_image" -f "$here/runner.Dockerfile" "$here" >/dev/null
}

build_library() {
    local extra=()
    [ "${1:-}" = --no-real ] && extra=(--no-real)
    mkdir -p "$home/cache" "$home/library" "$home/scratch"
    log "building the library in $home/library (downloads go to $home/cache)"
    docker run --rm --name pa-test-e2e-library-$$ -u "$(id -u):$(id -g)" \
        -v "$repo:$repo:ro" -v "$home:$home" -w "$repo" "$runner_image" \
        python3 tools/qa/e2e/library.py --out "$home/library" --cache "$home/cache" "${extra[@]}"
    touch "$home/library/.built"
}

up() {
    local rebuild=false noreal=""
    while [ $# -gt 0 ]; do
        case "$1" in
            --rebuild-library) rebuild=true ;;
            --no-real) noreal=--no-real ;;
            *) log "unknown option $1"; exit 64 ;;
        esac
        shift
    done
    ensure_creds
    ensure_images
    if $rebuild || [ ! -f "$home/library/.built" ]; then build_library $noreal; fi
    mkdir -p "$home/scratch"
    docker network inspect "$net" >/dev/null 2>&1 || docker network create "$net" >/dev/null
    docker volume inspect "$vol" >/dev/null 2>&1 ||
        docker volume create --driver local --opt type=none --opt o=bind \
            --opt device="$home/library" "$vol" >/dev/null
    if [ "$(docker inspect -f '{{.State.Status}}' "$name" 2>/dev/null || true)" != running ]; then
        docker rm -f "$name" >/dev/null 2>&1 || true
        docker run -d --name "$name" --hostname kinolune-e2e --restart unless-stopped \
            --network "$net" -p "$port:445" \
            -v "$vol:/srv/media:ro" -v "$home/scratch:/srv/scratch" \
            -e PA_E2E_SMB_USER -e PA_E2E_SMB_PASS \
            -e PA_E2E_UID="$(id -u)" -e PA_E2E_GID="$(id -g)" \
            "$smb_image" >/dev/null
        log "started $name"
    fi
    for _ in $(seq 50); do
        if docker exec "$name" sh -c 'nc -z 127.0.0.1 445' 2>/dev/null; then break; fi
        sleep 0.2
    done
    status
}

status() {
    if [ "$(docker inspect -f '{{.State.Status}}' "$name" 2>/dev/null || true)" != running ]; then
        echo "stand: $name is not running (tools/qa/e2e/stand.sh up)"
        return 1
    fi
    local ip files size
    ip="$(docker inspect -f "{{(index .NetworkSettings.Networks \"$net\").IPAddress}}" "$name")"
    files="$(find "$home/library" -type f ! -name .built | wc -l)"
    size="$(du -sh --apparent-size "$home/library" | cut -f1)"
    echo "stand: $name running; in $net at $ip:445, on the host at port $port"
    echo "stand: shares //kinolune-e2e/media (read-only, $files files, $size), //kinolune-e2e/scratch (read-write)"
    ensure_creds
    # A real sign-in to the share, as the app will do it.
    # (smbclient exits 1 after the listing: no SMB1 workgroup browsing.)
    local shares
    shares="$(docker exec -e U="$PA_E2E_SMB_USER%$PA_E2E_SMB_PASS" "$name" \
        sh -c 'smbclient -L //127.0.0.1 -U "$U" -m SMB3 -g 2>/dev/null' || true)"
    if grep -q '^Disk|media|' <<<"$shares"; then
        echo "stand: sign-in as $PA_E2E_SMB_USER works (SMB3); credentials in $home/stand.env"
    else
        echo "stand: sign-in to the share FAILED; see docker logs $name"
        return 1
    fi
}

down() {
    docker rm -f "$name" >/dev/null 2>&1 || true
    if [ "${1:-}" = --purge ]; then
        docker volume rm "$vol" >/dev/null 2>&1 || true
        docker network rm "$net" >/dev/null 2>&1 || true
        log "removed $name, $vol and $net (files in $home are kept)"
    else
        log "removed $name (network, volume and files are kept)"
    fi
}

cmd="${1:-status}"
shift || true
case "$cmd" in
    up) up "$@" ;;
    status) status ;;
    down) down "$@" ;;
    *) echo "usage: stand.sh up [--rebuild-library] [--no-real] | status | down [--purge]" >&2; exit 64 ;;
esac
