#!/usr/bin/env bash
# SPDX-License-Identifier: MPL-2.0
# End-to-end check of alpha-admin (DIST-013) in a QEMU VM booted from our image.
#
#   distro/admin/tests/vm-admin-e2e.sh
#
# Builds the NAS OS image from this checkout, installs it to a raw disk
# (bootc install to-disk), boots it with a second empty disk and checks over
# SSH: units and sockets, then the REST API against the real systemd,
# NetworkManager, firewalld, UDisks2, logind, polkit and PAM
# (tests/vm_api_check.py): login with OS users and roles, CSRF, lockout,
# restart, journal, Wi-Fi radio, firewall openings, mount/unmount of a data
# disk, update proxy (a fake alpha-updated on its unix socket), LAN-only
# filter, reboot with confirmation, certificate kept across the reboot.
#
# Requirements as distro/tests/vm-switch-rollback.sh. QEMU_EXEC="docker exec <c>" runs
# QEMU in a helper container (see scripts/run-vm.sh). KEEP=1 leaves the VM running.
set -euo pipefail
source "$(dirname "$0")/../../scripts/lib.sh"

ADMIN_TESTS="${DISTRO_DIR}/admin/tests"
mkdir -p "${WORK_DIR}"
KEY="${WORK_DIR}/vm_key"
[[ -f "${KEY}" ]] || ssh-keygen -q -t ed25519 -N '' -C alpha-os-vm-test -f "${KEY}"
read -r -a qemu_exec <<< "${QEMU_EXEC:-}"

LOCAL_REPO="localhost:${REGISTRY_PORT}/alpha-os"
VM_REPO="10.0.2.2:${REGISTRY_PORT}/alpha-os"
TAG="admin-e2e"
VERSION="0.0.1-admin-e2e.1"

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
    "${ENGINE}" rm -f "${REGISTRY_NAME}" >/dev/null 2>&1 || true
}
trap cleanup EXIT

wait_ssh() {
    local i
    for i in $(seq 1 60); do
        if vm true 2>/dev/null; then return 0; fi
        sleep 5
    done
    die "VM did not come up over SSH (see ${WORK_DIR}/serial.log)"
}

wait_reboot() {
    local old="$1" i
    for i in $(seq 1 60); do
        sleep 5
        if new="$(vm cat /proc/sys/kernel/random/boot_id 2>/dev/null)" && [[ "${new}" != "${old}" ]]; then
            return 0
        fi
    done
    die "VM did not reboot"
}

# --- image, registry, disk ---------------------------------------------------------------
log "Starting local registry on 127.0.0.1:${REGISTRY_PORT}"
"${ENGINE}" rm -f "${REGISTRY_NAME}" >/dev/null 2>&1 || true
"${ENGINE}" run -d --name "${REGISTRY_NAME}" -p "127.0.0.1:${REGISTRY_PORT}:5000" \
    docker.io/library/registry:2 >/dev/null
export ALPHA_SIGNATURE_POLICY=off
"${DISTRO_DIR}/scripts/build-image.sh" "${LOCAL_REPO}:${TAG}" "${VERSION}"
"${ENGINE}" push -q "${LOCAL_REPO}:${TAG}" >/dev/null
"${DISTRO_DIR}/scripts/make-disk.sh" "docker://${LOCAL_REPO}:${TAG}" "${VM_REPO}:${TAG}" \
    "${WORK_DIR}/disk.raw" "${KEY}.pub"
rm -f "${WORK_DIR}/data.raw"
truncate -s 2G "${WORK_DIR}/data.raw"
VM_EXTRA_DISKS="${WORK_DIR}/data.raw" "${DISTRO_DIR}/scripts/run-vm.sh" "${WORK_DIR}/disk.raw"
wait_ssh
[[ "$(vm 'sed -n s/^ALPHA_OS_VERSION=//p /usr/share/alpha/version')" == "${VERSION}" ]] \
    || die "the VM did not boot our image"
log "OK booted ${VERSION}"

# --- units, sockets, users ---------------------------------------------------------------
for _ in $(seq 1 24); do
    vm systemctl is-active --quiet alpha-admin.service && break
    sleep 5
done
vm systemctl is-active --quiet alpha-admin.service || { vm journalctl -u alpha-admin -b --no-pager | tail -30; die "alpha-admin is not active"; }
vm systemctl is-active --quiet alpha-admin-auth.socket || die "alpha-admin-auth.socket is not active"
[[ "$(vm "stat -c '%a %U' /run/alpha-admin/auth.sock")" == "600 alpha-admin" ]] || die "auth socket permissions"
[[ "$(vm "stat -c '%U %a' /var/lib/alpha/admin")" == "alpha-admin 700" ]] || die "state dir permissions"
[[ "$(vm 'stat -c %U /proc/$(systemctl show -p MainPID --value alpha-admin)')" == "alpha-admin" ]] \
    || die "alpha-admin does not run as its own user"
vm "ss -ltnp | grep -q ':443 .*alpha-admin'" || die "alpha-admin is not listening on 443"
[[ "$(vm "curl -sS -o /dev/null -w '%{http_code}' http://127.0.0.1/")" == 301 ]] || die ":80 does not redirect"
log "OK alpha-admin runs as alpha-admin on :443, :80 redirects, auth socket 0600"

vm bash -s <<'EOF'
set -e
pass='E2e-pass-1'
useradd -m -G wheel anna
useradd -m boris
useradd -m -G alpha-admin carla
useradd -m locked
useradd -r svc
for u in anna boris carla locked svc; do echo "${u}:${pass}" | chpasswd; done
usermod -L locked
mkfs.btrfs -q -f -L media /dev/vdb
# A stand-in for alpha-updated's API socket, only when the image has no real agent (DIST-009).
mkdir -p /run/alpha-updated
cat > /run/alpha-updated/fake.py <<'PY'
import grp, http.server, json, os, socketserver
SOCK = "/run/alpha-updated/api.sock"
if os.path.exists(SOCK):
    os.unlink(SOCK)
class H(http.server.BaseHTTPRequestHandler):
    def handle_one(self):
        n = int(self.headers.get("Content-Length") or 0)
        body = self.rfile.read(n).decode()
        with open("/run/alpha-updated/requests.log", "a") as f:
            f.write(f"{self.command} {self.path} {body}\n")
        out = {"state": "up-to-date"} if self.path == "/v1/update" else {"settings": json.loads(body or "{}")}
        data = json.dumps(out).encode()
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)
    do_GET = do_PUT = do_POST = handle_one
    def address_string(self):
        return "unix"
class S(socketserver.ThreadingMixIn, socketserver.UnixStreamServer):
    pass
srv = S(SOCK, H)
os.chown(SOCK, 0, grp.getgrnam("alpha-admin").gr_gid)
os.chmod(SOCK, 0o660)
srv.serve_forever()
PY
if [ ! -S /run/alpha-updated/api.sock ]; then
    systemd-run --quiet --unit=fake-alpha-updated python3 /run/alpha-updated/fake.py
fi
for i in $(seq 1 20); do [ -S /run/alpha-updated/api.sock ] && break; sleep 0.5; done
EOF
log "OK test users, data disk and a fake alpha-updated socket in place"

# --- API ---------------------------------------------------------------------------------
vm python3 - main < "${ADMIN_TESTS}/vm_api_check.py"

# --- LAN only ----------------------------------------------------------------------------
vm "ip addr add 192.168.77.7/32 dev lo && ip addr add 203.0.113.7/32 dev lo"
vm "curl -fsSk --max-time 5 --interface 192.168.77.7 https://192.168.77.7/healthz" >/dev/null \
    || die "a LAN client (192.168.77.7) was refused"
if vm "curl -fsSk --max-time 5 --interface 203.0.113.7 https://203.0.113.7/healthz" >/dev/null 2>&1; then
    die "a public client (203.0.113.7) was served"
fi
vm "ip addr del 192.168.77.7/32 dev lo; ip addr del 203.0.113.7/32 dev lo"
log "OK LAN client served, public address refused"

log "systemd-analyze security:"
vm "systemd-analyze security alpha-admin.service | tail -1; systemd-analyze security alpha-admin-auth@x.service 2>/dev/null | tail -1" || true

# --- reboot with confirmation ------------------------------------------------------------
FP_BEFORE="$(vm python3 - fingerprint < "${ADMIN_TESTS}/vm_api_check.py")"
BOOT_ID="$(vm cat /proc/sys/kernel/random/boot_id)"
sleep 12   # the login rate limit (10 a minute per address) refills
# The VM may go down before SSH returns: judge by the output, not the exit code.
REBOOT_OUT="$(vm python3 - reboot < "${ADMIN_TESTS}/vm_api_check.py" 2>&1)" || true
echo "${REBOOT_OUT}" | grep -v '^Connection to'
grep -q '^OK reboot confirmed' <<< "${REBOOT_OUT}" || die "reboot through the API failed"
wait_reboot "${BOOT_ID}"
wait_ssh
for _ in $(seq 1 24); do
    vm systemctl is-active --quiet alpha-admin.service && break
    sleep 5
done
OUT="$(vm python3 - after < "${ADMIN_TESTS}/vm_api_check.py")"
echo "${OUT}" | grep '^OK'
FP_AFTER="$(echo "${OUT}" | sed -n 's/^FP //p')"
[[ "${FP_AFTER}" == "${FP_BEFORE}" ]] || die "certificate changed across the reboot: ${FP_BEFORE} -> ${FP_AFTER}"
log "OK rebooted through the API; same certificate after the reboot"

vm "journalctl -u alpha-admin -b -1 -o cat --no-pager | tail -n 12" > "${WORK_DIR}/admin-e2e-journal.txt" || true
log "ALL CHECKS PASSED"
