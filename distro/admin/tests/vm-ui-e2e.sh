#!/usr/bin/env bash
# SPDX-License-Identifier: MPL-2.0
# End-to-end check of the alpha-admin web UI (DIST-014) in a QEMU VM: an OS update
# and a rollback done by clicking through the UI in a real browser (Playwright).
#
#   distro/admin/tests/vm-ui-e2e.sh
#
# Builds two NAS OS images from this checkout (V1 and V2, same code, different
# version), installs V1 to a raw disk tracking a tag in a local registry, boots it,
# pushes V2 under the same tag and then, in Chromium (tests/ui_update_rollback.py):
#   log in as an OS administrator -> Overview -> Updates -> "Check" -> V2 available
#   -> "Install now" + confirm -> the page waits for the reboot and comes back
#   -> log in -> V2 installed, V1 previous -> "Roll back" + confirm -> reboot
#   -> log in -> V1 installed again.
# The real alpha-updated drives bootc; after each reboot the script also checks
# over SSH which version booted. Screenshots go to ${WORK_DIR}/ui-e2e/.
#
# Requirements as tests/vm-admin-e2e.sh, plus a Playwright for Python: by default
# the container image ${PLAYWRIGHT_IMAGE} on the host network; PLAYWRIGHT_RUN="python3"
# runs the scenario with a local Playwright instead. KEEP=1 leaves the VM running.
set -euo pipefail
source "$(dirname "$0")/../../scripts/lib.sh"

ADMIN_TESTS="${DISTRO_DIR}/admin/tests"
mkdir -p "${WORK_DIR}"
KEY="${WORK_DIR}/vm_key"
[[ -f "${KEY}" ]] || ssh-keygen -q -t ed25519 -N '' -C alpha-os-vm-test -f "${KEY}"
read -r -a qemu_exec <<< "${QEMU_EXEC:-}"

export VM_ADMIN_PORT="${VM_ADMIN_PORT:-$((VM_SSH_PORT + 2))}"
# The image carries the browsers; the Python package (same version) is installed per run.
PLAYWRIGHT_VERSION="${PLAYWRIGHT_VERSION:-1.49.1}"
PLAYWRIGHT_IMAGE="${PLAYWRIGHT_IMAGE:-mcr.microsoft.com/playwright/python:v${PLAYWRIGHT_VERSION}-noble}"
OUT="${WORK_DIR}/ui-e2e"

LOCAL_REPO="localhost:${REGISTRY_PORT}/alpha-os"
VM_REPO="10.0.2.2:${REGISTRY_PORT}/alpha-os"
TAG="ui-e2e"
V1="0.0.1-ui-e2e.1"
V2="0.0.1-ui-e2e.2"

SSH=(ssh -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o LogLevel=ERROR
     -o ConnectTimeout=5 -o BatchMode=yes -i "${KEY}" -p "${VM_SSH_PORT}" root@127.0.0.1)
vm() { "${SSH[@]}" "$@"; }

cleanup() {
    if [[ "${KEEP:-0}" == 1 ]]; then
        log "KEEP=1: VM and registry left running"
        return
    fi
    if [[ -f "${WORK_DIR}/qemu.pid" ]]; then
        "${qemu_exec[@]}" sh -c "kill \$(cat ${WORK_DIR}/qemu.pid)" 2>/dev/null || true
        rm -f "${WORK_DIR}/qemu.pid"
    fi
    "${ENGINE}" rm -f "${CONTAINER_PREFIX:-pa-test}-ui-e2e-browser" >/dev/null 2>&1 || true
    "${ENGINE}" rm -f "${REGISTRY_NAME}" >/dev/null 2>&1 || true
}
trap cleanup EXIT

wait_ssh() {
    local i
    for i in $(seq 1 90); do
        if vm true 2>/dev/null; then return 0; fi
        sleep 5
    done
    die "VM did not come up over SSH (see ${WORK_DIR}/serial.log)"
}

booted_version() { vm 'sed -n s/^ALPHA_OS_VERSION=//p /usr/share/alpha/version'; }

wait_admin() {
    for _ in $(seq 1 36); do
        vm systemctl is-active --quiet alpha-admin.service alpha-updated.socket && return 0
        sleep 5
    done
    die "alpha-admin or alpha-updated.socket is not active"
}

# --- images, registry, disk --------------------------------------------------------------
log "Starting local registry on 127.0.0.1:${REGISTRY_PORT}"
"${ENGINE}" rm -f "${REGISTRY_NAME}" >/dev/null 2>&1 || true
"${ENGINE}" run -d --name "${REGISTRY_NAME}" -p "127.0.0.1:${REGISTRY_PORT}:5000" \
    docker.io/library/registry:2 >/dev/null
# Test images are unsigned and come from a local registry (the policy has its own test).
export ALPHA_SIGNATURE_POLICY=off
"${DISTRO_DIR}/scripts/build-image.sh" "${LOCAL_REPO}:${TAG}-v1" "${V1}"
"${DISTRO_DIR}/scripts/build-image.sh" "${LOCAL_REPO}:${TAG}-v2" "${V2}"
"${ENGINE}" tag "${LOCAL_REPO}:${TAG}-v1" "${LOCAL_REPO}:${TAG}"
"${ENGINE}" push -q "${LOCAL_REPO}:${TAG}" >/dev/null
"${DISTRO_DIR}/scripts/make-disk.sh" "docker://${LOCAL_REPO}:${TAG}" "${VM_REPO}:${TAG}" \
    "${WORK_DIR}/disk.raw" "${KEY}.pub"
"${DISTRO_DIR}/scripts/run-vm.sh" "${WORK_DIR}/disk.raw"
wait_ssh
[[ "$(booted_version)" == "${V1}" ]] || die "the VM did not boot ${V1}"
wait_admin
log "OK booted ${V1}, admin UI on https://127.0.0.1:${VM_ADMIN_PORT}/"

# The VM pulls updates from the plain-HTTP registry on the host.
vm "printf '[[registry]]\nlocation = \"10.0.2.2:${REGISTRY_PORT}\"\ninsecure = true\n' \
    > /etc/containers/registries.conf.d/50-alpha-local.conf"
# An administrator account of the OS; a fresh password every run.
UI_USER="olena"
UI_PASS="Ui-$(od -An -N9 -tx1 /dev/urandom | tr -d ' \n')"
vm "useradd -m -G wheel ${UI_USER} && echo '${UI_USER}:${UI_PASS}' | chpasswd"

# Publish V2 under the tracked tag: this is "the update".
"${ENGINE}" tag "${LOCAL_REPO}:${TAG}-v2" "${LOCAL_REPO}:${TAG}"
"${ENGINE}" push -q "${LOCAL_REPO}:${TAG}" >/dev/null
log "OK ${V2} published as ${VM_REPO}:${TAG}"

# --- the browser does the rest ---------------------------------------------------------
rm -rf "${OUT}"
mkdir -p "${OUT}"
chmod 0777 "${OUT}"
export UI_URL="https://127.0.0.1:${VM_ADMIN_PORT}/" UI_USER UI_PASS UI_V1="${V1}" UI_V2="${V2}"
if [[ -n "${PLAYWRIGHT_RUN:-}" ]]; then
    UI_OUT="${OUT}" ${PLAYWRIGHT_RUN} "${ADMIN_TESTS}/ui_update_rollback.py" &
else
    "${ENGINE}" run --rm --network host --init --ipc host --user "$(id -u):$(id -g)" \
        --name "${CONTAINER_PREFIX:-pa-test}-ui-e2e-browser" \
        -e UI_URL -e UI_USER -e UI_PASS -e UI_V1 -e UI_V2 -e UI_OUT=/out -e HOME=/tmp \
        -v "${ADMIN_TESTS}:/t:ro" -v "${OUT}:/out" \
        "${PLAYWRIGHT_IMAGE}" sh -c "pip install -q --disable-pip-version-check --target /tmp/pw \
            'playwright==${PLAYWRIGHT_VERSION}' && PYTHONPATH=/tmp/pw python3 /t/ui_update_rollback.py" &
fi
BROWSER=$!

# Follow the VM from outside: each reboot must land on the expected version.
expect_after_reboot() {
    local want="$1" old="$2" i new
    for i in $(seq 1 180); do
        sleep 5
        kill -0 "${BROWSER}" 2>/dev/null || break
        if new="$(vm cat /proc/sys/kernel/random/boot_id 2>/dev/null)" && [[ "${new}" != "${old}" ]]; then
            wait_admin
            [[ "$(booted_version)" == "${want}" ]] || die "after the reboot ${want} expected, booted $(booted_version)"
            log "OK rebooted into ${want}"
            return 0
        fi
    done
    wait "${BROWSER}" || die "the browser scenario failed (screenshots in ${OUT})"
    die "the VM did not reboot into ${want}"
}
expect_after_reboot "${V2}" "$(vm cat /proc/sys/kernel/random/boot_id)"
expect_after_reboot "${V1}" "$(vm cat /proc/sys/kernel/random/boot_id)"
wait "${BROWSER}" || die "the browser scenario failed (screenshots in ${OUT})"

vm "journalctl -u alpha-updated -b -1 -o cat --no-pager | tail -n 20" > "${OUT}/updated-journal.txt" || true
log "ALL CHECKS PASSED (screenshots in ${OUT})"
