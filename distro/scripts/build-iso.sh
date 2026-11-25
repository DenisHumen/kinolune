#!/usr/bin/env bash
# SPDX-License-Identifier: MPL-2.0
# Build the Kinolune OS installer ISO (DIST-007):
#   distro/scripts/build-iso.sh [OUT_DIR]          # default OUT_DIR: distro/.work/iso
#
#   1. the OS image (distro/Containerfile) - the "payload" installed from the ISO offline;
#      skipped when PAYLOAD_IMAGE names an image already in the storage (CI: rechunked);
#   2. the live installer image (distro/installer/Containerfile);
#   3. image-builder generic-iso with --bootc-installer-payload-ref: the payload lands in
#      the ISO's containers-storage under PAYLOAD_TARGET, the ref alpha-installer installs
#      from (image.source = containers-storage:<image.target>) and that the installed
#      system then tracks in the registry;
#   4. size check: the ISO must stay below MAX_ISO_BYTES (GitHub Releases: < 2 GiB a file).
#
# Everything runs in one privileged Fedora helper container with podman and image-builder,
# so the host needs only docker or rootful podman (no image-builder, no sudo on the host).
# Its storage, caches and /var/tmp are host directories: overlayfs (podman, buildah) cannot
# use the helper's own overlay root as an upper layer.
# Output: kinolune-os-<VERSION>-x86_64.iso, .sha256 and iso-info.txt (sizes, squashfs).
#
# Env: VERSION          version written into the images (default 0.0.0-dev.<timestamp>);
#      PAYLOAD_TARGET   registry ref the installed system follows
#                       (default ghcr.io/denishumen/alpha-os:stable);
#      PAYLOAD_IMAGE    use this image from ISO_STORAGE instead of building one;
#      ISO_STORAGE      host dir used as the helper's /var/lib/containers/storage
#                       (default distro/.work/iso-storage; CI: /var/lib/containers/storage);
#      ISO_HELPER_NAME  name of the helper container (default alpha-os-iso);
#      MAX_ISO_BYTES    size limit (default 1.9 GiB);
#      ALPHA_SIGNATURE_POLICY, ALPHA_IMAGE_REPO  passed to the OS build (see Containerfile);
#      KEEP=1           leave the helper container running (faster rebuilds).
set -euo pipefail
source "$(dirname "$0")/lib.sh"

OUT_DIR="${1:-${WORK_DIR}/iso}"
VERSION="${VERSION:-0.0.0-dev.$(date -u +%Y%m%d%H%M%S)}"
PAYLOAD_TARGET="${PAYLOAD_TARGET:-ghcr.io/denishumen/alpha-os:stable}"
PAYLOAD_IMAGE="${PAYLOAD_IMAGE:-}"
ISO_STORAGE="${ISO_STORAGE:-${WORK_DIR}/iso-storage}"
ISO_HELPER_NAME="${ISO_HELPER_NAME:-alpha-os-iso}"
# Fedora 45 (same release as the OS base): podman + image-builder/osbuild from Fedora.
ISO_HELPER_IMAGE="${ISO_HELPER_IMAGE:-docker.io/library/fedora:45@sha256:a7d72ba2745b100a3351087472e5ee7b2450af7cf331e7673007e3817612839c}"
MAX_ISO_BYTES="${MAX_ISO_BYTES:-$((19 * 1024 * 1024 * 1024 / 10))}"
INSTALLER_TAG="localhost/kinolune-installer:${VERSION}"
ISO_NAME="kinolune-os-${VERSION}-x86_64"

[[ "${VERSION}" =~ ^[0-9A-Za-z][0-9A-Za-z._-]{0,100}$ ]] || die "bad VERSION '${VERSION}'"
mkdir -p "${OUT_DIR}" "${ISO_STORAGE}" "${WORK_DIR}/iso-cache" "${WORK_DIR}/iso-tmp"
OUT_DIR="$(cd "${OUT_DIR}" && pwd)"

helper() { "${ENGINE}" exec -i "${ISO_HELPER_NAME}" "$@"; }

cleanup() {
    if [[ "${KEEP:-0}" == 1 ]]; then
        log "KEEP=1: helper container ${ISO_HELPER_NAME} left running"
        return
    fi
    "${ENGINE}" rm -f "${ISO_HELPER_NAME}" >/dev/null 2>&1 || true
}
trap cleanup EXIT

if [[ "$("${ENGINE}" inspect -f '{{.State.Running}}' "${ISO_HELPER_NAME}" 2>/dev/null)" != true ]]; then
    log "Starting helper container ${ISO_HELPER_NAME} (${ISO_HELPER_IMAGE})"
    "${ENGINE}" rm -f "${ISO_HELPER_NAME}" >/dev/null 2>&1 || true
    "${ENGINE}" run -d --init --name "${ISO_HELPER_NAME}" --privileged --network host \
        --security-opt label=disable \
        -v /dev:/dev \
        -v "${ISO_STORAGE}:/var/lib/containers/storage" \
        -v "${WORK_DIR}/iso-cache:/var/cache/image-builder" \
        -v "${WORK_DIR}/iso-tmp:/var/tmp" \
        -v "${DISTRO_DIR}:/src/distro:ro" \
        -v "${OUT_DIR}:/out" \
        "${ISO_HELPER_IMAGE}" sleep infinity >/dev/null
    helper bash -euc '
        dnf -y -q install --setopt=install_weak_deps=False podman image-builder squashfs-tools >/dev/null
        # Nested podman: no systemd and no delegated cgroup controllers in the helper.
        mkdir -p /etc/containers/containers.conf.d
        printf "[containers]\ncgroups = \"disabled\"\npids_limit = 0\n[engine]\ncgroup_manager = \"cgroupfs\"\nevents_logger = \"file\"\n" \
            > /etc/containers/containers.conf.d/50-helper.conf
        # Ubuntu hosts (24.04+, also GitHub runners) confine /usr/bin/bwrap by path with the
        # AppArmor profile bwrap-userns-restrict, which drops every capability of its
        # children: osbuild stages then cannot mount. A copy under another path is not
        # matched; osbuild finds it first in PATH. The helper itself stays privileged.
        install -m 0755 /usr/bin/bwrap /usr/local/bin/bwrap
        image-builder version | head -n 3'
fi

if [[ -z "${PAYLOAD_IMAGE}" ]]; then
    log "Building the OS image (payload), version ${VERSION}"
    helper nice -n 10 podman build --isolation=chroot \
        --build-arg "ALPHA_OS_VERSION=${VERSION}" \
        --build-arg "ALPHA_SIGNATURE_POLICY=${ALPHA_SIGNATURE_POLICY:-auto}" \
        ${ALPHA_IMAGE_REPO:+--build-arg "ALPHA_IMAGE_REPO=${ALPHA_IMAGE_REPO}"} \
        -f /src/distro/Containerfile -t "${PAYLOAD_TARGET}" /src/distro
else
    log "Payload: ${PAYLOAD_IMAGE} -> ${PAYLOAD_TARGET}"
    helper podman tag "${PAYLOAD_IMAGE}" "${PAYLOAD_TARGET}"
fi

log "Building the live installer image ${INSTALLER_TAG}"
helper nice -n 10 podman build --isolation=chroot \
    --build-arg "BASE_IMAGE=${BASE_IMAGE}" \
    --build-arg "ALPHA_OS_VERSION=${VERSION}" \
    -f /src/distro/installer/Containerfile -t "${INSTALLER_TAG}" /src/distro

log "image-builder generic-iso (payload ${PAYLOAD_TARGET})"
helper rm -rf /out/build
helper nice -n 10 image-builder build generic-iso \
    --bootc-ref "${INSTALLER_TAG}" \
    --bootc-installer-payload-ref "${PAYLOAD_TARGET}" \
    --bootc-default-fs ext4 \
    --output-dir /out/build --output-name "${ISO_NAME}" --progress verbose

iso="$(helper find /out/build -name '*.iso' -print -quit)"
[[ -n "${iso}" ]] || die "image-builder produced no ISO"
helper mv "${iso}" "/out/${ISO_NAME}.iso"
helper rm -rf /out/build
helper chown "$(id -u):$(id -g)" "/out/${ISO_NAME}.iso"
ISO="${OUT_DIR}/${ISO_NAME}.iso"
(cd "${OUT_DIR}" && sha256sum "${ISO_NAME}.iso" > "${ISO_NAME}.iso.sha256")

size="$(stat -c %s "${ISO}")"
{
    echo "iso: ${ISO_NAME}.iso"
    echo "size_bytes: ${size}"
    echo "size_mib: $((size / 1024 / 1024))"
    echo "limit_bytes: ${MAX_ISO_BYTES}"
    echo "payload: ${PAYLOAD_TARGET}"
    echo "installer: ${INSTALLER_TAG}"
    helper podman image inspect --format 'payload_digest: {{.Digest}}' "${PAYLOAD_TARGET}"
    helper bash -c "podman image inspect --format '{{.Size}}' '${PAYLOAD_TARGET}' | sed 's/^/payload_unpacked_bytes: /'"
    helper bash -c "podman image inspect --format '{{.Size}}' '${INSTALLER_TAG}' | sed 's/^/installer_unpacked_bytes: /'"
    # Squashfs parameters (compression, block size, duplicates found) of the live root.
    helper bash -euc "
        d=\$(mktemp -d); mount -o loop,ro '/out/${ISO_NAME}.iso' \"\$d\"
        ls -l \"\$d/LiveOS\" | sed 's/^/LiveOS: /'
        unsquashfs -s \"\$d/LiveOS/squashfs.img\" | grep -Ei 'compression|block size|duplicates|filesystem size' | sed 's/^/squashfs: /'
        umount \"\$d\""
} | tee "${OUT_DIR}/iso-info.txt"

log "ISO: ${ISO} ($((size / 1024 / 1024)) MiB)"
if (( size >= MAX_ISO_BYTES )); then
    die "ISO is ${size} bytes, the limit is ${MAX_ISO_BYTES} (1.9 GiB, docs/research/05 §6.3)"
fi
log "Size OK: below $((MAX_ISO_BYTES / 1024 / 1024)) MiB"
