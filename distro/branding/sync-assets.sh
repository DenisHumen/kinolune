#!/usr/bin/env bash
# SPDX-License-Identifier: MPL-2.0
# Copy the Kinolune brand assets into the OS rootfs (DIST-003).
#   distro/branding/sync-assets.sh          # copy / regenerate
#   distro/branding/sync-assets.sh --check  # fail if the copies are stale (CI)
#
# The image build context is distro/ only, so the brand exports from design/brand/export
# (DSN-006) are committed as copies under distro/rootfs. Icon name: kinolune-logo-icon
# (os-release LOGO=). The fastfetch logo is rendered from the mark geometry.
set -euo pipefail
cd "$(dirname "$0")/../.."

SRC=design/brand/export
DST=distro/rootfs/usr/share
CHECK=0
[[ "${1:-}" == --check ]] && CHECK=1
stale=0

put() { # put SRC DST
    if [[ "${CHECK}" == 1 ]]; then
        cmp -s "$1" "$2" || { echo "stale: $2 (from $1)" >&2; stale=1; }
    else
        mkdir -p "$(dirname "$2")"
        cp "$1" "$2"
    fi
}

for s in 16 22 24 32 48 64 128 256 512; do
    put "${SRC}/linux/icons/hicolor/${s}x${s}/apps/app.kinolune.png" \
        "${DST}/icons/hicolor/${s}x${s}/apps/kinolune-logo-icon.png"
done
put "${SRC}/linux/icons/hicolor/scalable/apps/app.kinolune.svg" \
    "${DST}/icons/hicolor/scalable/apps/kinolune-logo-icon.svg"
put "${SRC}/linux/icons/hicolor/symbolic/apps/app.kinolune-symbolic.svg" \
    "${DST}/icons/hicolor/symbolic/apps/kinolune-logo-icon-symbolic.svg"
put "${SRC}/logo/lockup-dark.png" "${DST}/kinolune-os/logo-dark.png"

tmp="$(mktemp)"
trap 'rm -f "${tmp}"' EXIT
python3 distro/branding/gen-fastfetch-logo.py 32 > "${tmp}"
put "${tmp}" "${DST}/kinolune-os/fastfetch-logo.txt"

if [[ "${stale}" == 1 ]]; then
    echo "run distro/branding/sync-assets.sh and commit the result" >&2
    exit 1
fi
