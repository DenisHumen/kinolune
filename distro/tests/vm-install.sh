#!/usr/bin/env bash
# SPDX-License-Identifier: MPL-2.0
# QEMU check of the installer (DIST-005): non-interactive install from an answer file
# onto a virtual NVMe with UEFI (OVMF), then boot the installed disk alone.
#
#   distro/tests/vm-install.sh
#
# Scenario:
#   1. build the OS image (unsigned, local registry) and the installer binary;
#   2. "installer VM": our OS on a virtio disk (stands in for the live ISO of DIST-007),
#      plus an empty 40 GB NVMe; the payload is pulled into its containers-storage;
#   3. negative checks: the disk the installer boots from is refused, --yes is required;
#   4. `alpha-installer install --answers … --yes` onto the NVMe (data partition on);
#   5. the NVMe boots alone with the same OVMF NVRAM; over SSH: booted image tracks the
#      registry (not containers-storage), layout, @var, hostname, user (yescrypt, wheel),
#      NetworkManager, install.json, SELinux labels;
#   6. a new image is pushed under the same tag and `bootc upgrade --check` sees it;
#   7. DIST-010: alpha-updated applies it (read-only snapshot of @var first), the new
#      version changes /var, the OS rolls back, and /var is back to the snapshot before
#      it is mounted; the replaced /var is kept;
#   8. BUG-031: the image has no signature policy (no project key yet), so the installer
#      chose the plain registry transport; a device of ISO 0.0.0-ci.7 (ostree-image-signed
#      origin under the stock policy) cannot check for updates; `bootc switch` to the same
#      tag fixes it; then alpha-updated's check, `bootc upgrade --check`, `bootc upgrade`
#      + reboot -> new version, `bootc rollback` + reboot -> old version.
#
# Requirements: docker (or rootful podman) with --privileged, qemu-system-x86_64, OVMF,
# /dev/kvm, ssh, python3, ~10 GB free in distro/.work. Takes ~20 minutes.
# Env: QEMU_EXEC  prefix for qemu and kill, e.g. "docker exec my-qemu" when QEMU lives
#                 in a container sharing the host network and distro/.work;
#      OVMF_CODE, OVMF_VARS  firmware paths as seen by QEMU;
#      CONTAINER_PREFIX      names of the helper containers (default alpha-os-test);
#      KEEP=1                leave the VM and the registry running;
#      VM_SMB_PORT, SMB_EXEC also log in to the installed system's Samba share
#                            with the installer's password from outside the VM
#                            (smbclient, e.g. SMB_EXEC="docker exec <c> smbclient").
set -euo pipefail
source "$(dirname "$0")/../scripts/lib.sh"

CONTAINER_PREFIX="${CONTAINER_PREFIX:-alpha-os-test}"
REGISTRY_NAME="${CONTAINER_PREFIX}-registry"
W="${WORK_DIR}/install"
mkdir -p "${W}"
KEY="${W}/vm_key"
[[ -f "${KEY}" ]] || ssh-keygen -q -t ed25519 -N '' -C alpha-os-install-test -f "${KEY}"

LOCAL_REPO="localhost:${REGISTRY_PORT}/alpha-os"
VM_REPO="10.0.2.2:${REGISTRY_PORT}/alpha-os"
NVME_SERIAL="alphatarget"
TARGET_SIZE="${TARGET_SIZE:-40G}"
read -r -a QEMU_EXEC_ARR <<< "${QEMU_EXEC:-}"

if [[ -z "${OVMF_CODE:-}" ]]; then
    for c in /usr/share/OVMF/OVMF_CODE_4M.fd /usr/share/edk2/ovmf/OVMF_CODE.fd /usr/share/OVMF/OVMF_CODE.fd; do
        if "${QEMU_EXEC_ARR[@]}" test -f "${c}"; then OVMF_CODE="${c}"; break; fi
    done
fi
[[ -n "${OVMF_CODE:-}" ]] || die "OVMF not found, set OVMF_CODE and OVMF_VARS"
OVMF_VARS="${OVMF_VARS:-${OVMF_CODE/CODE/VARS}}"

SSH_OPTS=(-o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o LogLevel=ERROR
          -o ConnectTimeout=5 -o BatchMode=yes -i "${KEY}")
vm() { ssh "${SSH_OPTS[@]}" -p "${VM_SSH_PORT}" root@127.0.0.1 "$@"; }
vm_as() { local u="$1"; shift; ssh "${SSH_OPTS[@]}" -p "${VM_SSH_PORT}" "${u}@127.0.0.1" "$@"; }
vm_put() { scp -q "${SSH_OPTS[@]}" -P "${VM_SSH_PORT}" "$1" "root@127.0.0.1:$2"; }

vm_stop() {
    local pid _
    # The pid file belongs to whoever runs QEMU (root in a helper container).
    pid="$("${QEMU_EXEC_ARR[@]}" cat "${W}/qemu.pid" 2>/dev/null)" || return 0
    "${QEMU_EXEC_ARR[@]}" kill "${pid}" 2>/dev/null || true
    for _ in $(seq 1 30); do
        "${QEMU_EXEC_ARR[@]}" kill -0 "${pid}" 2>/dev/null || break
        sleep 1
    done
    "${QEMU_EXEC_ARR[@]}" rm -f "${W}/qemu.pid"
}

cleanup() {
    if [[ "${KEEP:-0}" == 1 ]]; then
        log "KEEP=1: VM and registry left running"
        return
    fi
    vm_stop
    "${ENGINE}" rm -f "${REGISTRY_NAME}" >/dev/null 2>&1 || true
}
trap cleanup EXIT

# vm_start NAME DRIVE_ARGS... : UEFI VM with one NVRAM shared by both boots, as on real hardware.
vm_start() {
    local name="$1"; shift
    log "Starting ${name} VM (${VM_CPUS} vCPU, ${VM_MEM_MB} MiB), ssh on 127.0.0.1:${VM_SSH_PORT}"
    nice -n 10 "${QEMU_EXEC_ARR[@]}" qemu-system-x86_64 \
        -enable-kvm -machine q35 -cpu host -smp "${VM_CPUS}" -m "${VM_MEM_MB}" \
        -drive "if=pflash,format=raw,readonly=on,file=${OVMF_CODE}" \
        -drive "if=pflash,format=raw,file=${W}/OVMF_VARS.fd" \
        "$@" \
        -netdev "user,id=n0,hostfwd=tcp:127.0.0.1:${VM_SSH_PORT}-:22${VM_SMB_PORT:+,hostfwd=tcp:127.0.0.1:${VM_SMB_PORT}-:445}" \
        -device virtio-net-pci,netdev=n0 \
        -display none -serial "file:${W}/serial-${name}.log" \
        -daemonize -pidfile "${W}/qemu.pid"
}

wait_ssh() {
    local _
    for _ in $(seq 1 90); do
        if vm true 2>/dev/null; then return 0; fi
        sleep 5
    done
    die "VM did not come up over SSH (see ${W}/serial-*.log)"
}

check() { # check DESCRIPTION COMMAND... (runs on the host)
    local what="$1"; shift
    if "$@"; then log "OK ${what}"; else die "FAILED: ${what}"; fi
}

# --- 1. images and the installer binary ---------------------------------------------------
log "Building the installer binary"
if command -v go >/dev/null 2>&1; then
    (cd "${DISTRO_DIR}/installer" && CGO_ENABLED=0 go build -trimpath -o "${W}/alpha-installer" ./cmd/alpha-installer)
else
    "${ENGINE}" run --rm --name "${CONTAINER_PREFIX}-go" -v "${DISTRO_DIR}/installer:/src:ro" -v "${W}:/out" \
        -w /src -e CGO_ENABLED=0 -e GOFLAGS=-buildvcs=false -e GOCACHE=/tmp/gocache \
        "${GO_IMAGE:-docker.io/library/golang:1.24}" go build -trimpath -o /out/alpha-installer ./cmd/alpha-installer
fi

log "Starting local registry on 127.0.0.1:${REGISTRY_PORT}"
"${ENGINE}" rm -f "${REGISTRY_NAME}" >/dev/null 2>&1 || true
"${ENGINE}" run -d --name "${REGISTRY_NAME}" -p "127.0.0.1:${REGISTRY_PORT}:5000" \
    docker.io/library/registry:2 >/dev/null

# Unsigned test images: the stock policy (tests/signature-policy.sh covers the real one).
export ALPHA_SIGNATURE_POLICY=off
"${DISTRO_DIR}/scripts/build-image.sh" "${LOCAL_REPO}:v1" "0.0.1-install.1"
"${ENGINE}" tag "${LOCAL_REPO}:v1" "${LOCAL_REPO}:dev"
"${ENGINE}" push -q "${LOCAL_REPO}:dev" >/dev/null

# --- 2. installer VM -----------------------------------------------------------------------
VM_DISK_SIZE="${INSTALLER_DISK_SIZE:-20G}" MAKE_DISK_CONTAINER="${CONTAINER_PREFIX}-make-disk" \
    "${DISTRO_DIR}/scripts/make-disk.sh" \
        "docker://${LOCAL_REPO}:dev" "${VM_REPO}:dev" "${W}/installer.raw" "${KEY}.pub"
rm -f "${W}/target.raw"
truncate -s "${TARGET_SIZE}" "${W}/target.raw"
"${QEMU_EXEC_ARR[@]}" cp "${OVMF_VARS}" "${W}/OVMF_VARS.fd"

VM_MEM_MB="${VM_MEM_MB_INSTALL:-4096}"
vm_start installer \
    -drive "file=${W}/installer.raw,format=raw,if=virtio,cache=unsafe" \
    -drive "file=${W}/target.raw,format=raw,if=none,id=nvm,cache=unsafe" \
    -device "nvme,serial=${NVME_SERIAL},drive=nvm"
wait_ssh

vm "printf '[[registry]]\nlocation = \"10.0.2.2:${REGISTRY_PORT}\"\ninsecure = true\n' \
    > /etc/containers/registries.conf.d/50-alpha-local.conf"
log "Pulling the payload into the installer's containers-storage"
vm podman pull -q "${VM_REPO}:dev" >/dev/null
# /usr is read-only on a bootc system; the live ISO (DIST-007) will ship the binary in the image.
vm_put "${W}/alpha-installer" /root/alpha-installer
vm chmod 0755 /root/alpha-installer

TARGET_BY_ID="/dev/disk/by-id/nvme-QEMU_NVMe_Ctrl_${NVME_SERIAL}"
vm test -e "${TARGET_BY_ID}" || die "${TARGET_BY_ID} not found in the installer VM"
PUB="$(cat "${KEY}.pub")"
python3 - "${W}/answers.json" "${TARGET_BY_ID}" "${VM_REPO}:dev" "${PUB}" <<'EOF'
import json, sys
path, disk, ref, pub = sys.argv[1:]
json.dump({
    "version": 1,
    "disk": {"path": disk, "data_partition": True, "root_size_gib": 20, "data_pool": "media"},
    "hostname": "kinolune-test",
    "user": {"name": "admin", "full_name": "Test admin", "password": "kinolune-test-pw",
             "ssh_authorized_keys": [pub]},
    # QEMU user networking, set statically: SSH works only if the connection is applied.
    "network": {"mode": "static", "address": "10.0.2.15/24", "gateway": "10.0.2.2",
                "dns": ["10.0.2.3"]},
    # enforce_signature_policy unset: the installer follows the payload (BUG-031).
    "image": {"source": "containers-storage:" + ref, "target": ref},
    "root_ssh_authorized_keys": [pub],
    "kernel_args": ["console=ttyS0,115200"],
    "language": "uk",
}, open(path, "w"), indent=2)
EOF
vm_put "${W}/answers.json" /root/answers.json
sed "s|${TARGET_BY_ID}|/dev/vda|" "${W}/answers.json" > "${W}/answers-boot-disk.json"
vm_put "${W}/answers-boot-disk.json" /root/answers-boot-disk.json

# --- 3. safety checks ----------------------------------------------------------------------
out="$(vm /root/alpha-installer install --answers /root/answers-boot-disk.json --yes 2>&1 || true)"
check "the installer's own disk is refused" grep -q "boots from this disk" <<< "${out}"
out="$(vm /root/alpha-installer install --answers /root/answers.json < /dev/null 2>&1 || true)"
check "--yes is required without a terminal" grep -q "add --yes" <<< "${out}"
vm /root/alpha-installer install --answers /root/answers.json --dry-run | tee "${W}/plan.txt"
check "dry run prints the plan" grep -q "Dry run: nothing was written" "${W}/plan.txt"
check "dry run left the NVMe empty" test -z "$(vm "lsblk -no PTTYPE,FSTYPE /dev/nvme0n1 | tr -d ' \n'")"

# --- 4. install ----------------------------------------------------------------------------
log "Installing onto ${TARGET_BY_ID}"
vm /root/alpha-installer install --answers /root/answers.json --yes 2>&1 | tee "${W}/install.log"
vm "lsblk -o NAME,SIZE,FSTYPE,LABEL,PARTLABEL /dev/nvme0n1 /dev/vda" | tee "${W}/lsblk-installer.txt"
check "the installer's disk is untouched" grep -q "vda.*xfs\|vda[0-9].*xfs" "${W}/lsblk-installer.txt"
vm systemctl poweroff || true
vm_stop

# --- 5. boot the installed NVMe alone ------------------------------------------------------
VM_MEM_MB=3072
vm_start installed \
    -drive "file=${W}/target.raw,format=raw,if=none,id=nvm,cache=unsafe" \
    -device "nvme,serial=${NVME_SERIAL},drive=nvm"
wait_ssh
vm systemctl is-system-running --wait >/dev/null || true

status="$(vm bootc status --format=json | tee "${W}/status-installed.json")"
img="$(python3 -c 'import json,sys; i=json.load(sys.stdin)["status"]["booted"]["image"]["image"]; print(i["transport"], i["image"])' <<< "${status}")"
check "booted image tracks the registry (${img})" test "${img}" = "registry ${VM_REPO}:dev"
# shellcheck disable=SC2016 # expanded in the VM
origin() { vm 'cat "$(ostree admin --print-current-dir).origin"' | sed -n 's/^container-image-reference=//p'; }
check "unsigned payload: plain registry transport, not ostree-image-signed (BUG-031)" \
    test "$(origin)" = "ostree-unverified-registry:${VM_REPO}:dev"
check "hostname" test "$(vm hostname)" = kinolune-test
check "/ is btrfs" test "$(vm findmnt -no FSTYPE /sysroot)" = btrfs
check "/var is the @var subvolume" grep -q "subvol=/@var" <<< "$(vm findmnt -no OPTIONS /var)"
check "data partition mounted" test "$(vm findmnt -no FSTYPE /var/srv/data/media)" = btrfs
check "/boot is ext4" test "$(vm findmnt -no FSTYPE /boot)" = ext4
check "ESP is vfat" test "$(vm findmnt -no FSTYPE /boot/efi 2>/dev/null || vm "blkid -s TYPE -o value /dev/nvme0n1p1")" = vfat
check "partition labels" test "$(vm "lsblk -no PARTLABEL /dev/nvme0n1 | grep -v '^$' | tr '\n' ' '")" = "EFI-SYSTEM boot root kinolune-data "
check "admin is in wheel" grep -qw wheel <<< "$(vm id -nG admin)"
# shellcheck disable=SC2016 # a literal "$y$" prefix
check "admin password is yescrypt" grep -q '^admin:\$y\$' <<< "$(vm getent shadow admin)"
check "admin logs in with the SSH key" test "$(vm_as admin whoami)" = admin
check "admin home is owned by admin" test "$(vm stat -c %U /var/home/admin)" = admin
check "NetworkManager connection active" grep -q '^kinolune-lan:' <<< "$(vm nmcli -t -f NAME,DEVICE connection show --active)"
check "install.json" python3 -c 'import json,sys; d=json.load(sys.stdin); assert d["hostname"]=="kinolune-test" and d["data_pool"]["name"]=="media" and d["first_boot"], d' \
    <<< "$(vm cat /var/lib/alpha/install.json)"
check "SELinux enforcing" test "$(vm getenforce)" = Enforcing
check "/etc/hostname labeled" grep -q hostname_etc_t <<< "$(vm ls -Z /etc/hostname)"
check "no failed units" test -z "$(vm systemctl --failed --no-legend --plain | grep -v '^$' || true)"
# DIST-018: the installer's clear-text password also opens the shares; keys were given, so SSH is on.
check "installer's Samba password imported at first boot" grep -q '^admin:' <<< "$(vm pdbedit -L)"
check "Samba import file removed" test "$(vm 'test -e /var/lib/alpha/samba/import.smbpasswd && echo left || echo gone')" = gone
check "admin's share folders" test "$(vm stat -c '%U:%a' /var/srv/users/admin)" = admin:700
check "SSH on (keys were given), keys only" test "$(vm systemctl is-enabled sshd.service)" = enabled
if [[ -n "${VM_SMB_PORT:-}" ]]; then
    read -r -a smb_exec <<< "${SMB_EXEC:-smbclient}"
    check "smbclient with the installer's password (Media, Users/admin)" \
        "${smb_exec[@]}" -p "${VM_SMB_PORT}" -m SMB3 -U 'admin%kinolune-test-pw' //127.0.0.1/Users -c 'put /etc/hostname admin/from-install-test.txt'
    if "${smb_exec[@]}" -p "${VM_SMB_PORT}" -m SMB3 -N //127.0.0.1/Media -c ls >/dev/null 2>&1; then
        die "FAILED: anonymous access to Media"
    fi
    log "OK anonymous access to Media refused"
fi

# --- 6. the installed system follows the registry -----------------------------------------
vm "printf '[[registry]]\nlocation = \"10.0.2.2:${REGISTRY_PORT}\"\ninsecure = true\n' \
    > /etc/containers/registries.conf.d/50-alpha-local.conf"
"${DISTRO_DIR}/scripts/build-image.sh" "${LOCAL_REPO}:v2" "0.0.1-install.2"
"${ENGINE}" tag "${LOCAL_REPO}:v2" "${LOCAL_REPO}:dev"
"${ENGINE}" push -q "${LOCAL_REPO}:dev" >/dev/null
vm bootc upgrade --check 2>&1 | tee "${W}/upgrade-check.txt"
vm bootc status --format=json > "${W}/status-after-check.json"
# cachedUpdate is what `upgrade --check` found under the tracked tag; it must differ from the booted image.
read -r booted_digest cached < <(python3 -c 'import json,sys; s=json.load(sys.stdin)["status"]["booted"]; print(s["image"]["imageDigest"], (s.get("cachedUpdate") or {}).get("imageDigest", "-"))' \
    < "${W}/status-after-check.json")
check "bootc upgrade --check found a new image in the registry (${cached})" test "${cached}" != - -a "${cached}" != "${booted_digest}"


# --- 7. /var snapshot before the update and restore after an OS rollback (DIST-010) --------
# The media server is a stub in this image, so its "database" is a file the test writes:
# schema 1 under v1, "migrated" to schema 2 while v2 runs. The rollback is what greenboot
# does after failed health checks (bootc rollback + reboot, before boot-complete is
# confirmed); greenboot itself is DIST-011.
TOP_DEV="$(vm findmnt -no SOURCE /sysroot | sed 's/\[.*//')"
top() { vm "mkdir -p /run/t && mount -o subvolid=5 ${TOP_DEV} /run/t && { $*; }; r=\$?; umount /run/t; exit \$r"; }
api() { vm curl -fsS --unix-socket /run/alpha-updated/api.sock "$@"; }
boot_id() { vm cat /proc/sys/kernel/random/boot_id 2>/dev/null || true; }
# wait_reboot OLD_BOOT_ID: the agent pulls and stages before it reboots, so wait for a new boot.
wait_reboot() {
    local _ id
    for _ in $(seq 1 120); do
        id="$(boot_id)"
        if [[ -n "${id}" && "${id}" != "$1" ]]; then
            vm systemctl is-system-running --wait >/dev/null || true
            return 0
        fi
        sleep 5
    done
    api http://localhost/v1/status > "${W}/agent-status.json" 2>&1 || true
    vm journalctl -b -u alpha-updated --no-pager -o cat > "${W}/agent-journal.txt" 2>&1 || true
    die "no reboot within 10 minutes (see ${W}/agent-status.json, agent-journal.txt)"
}
version() { vm sed -n 's/^ALPHA_OS_VERSION=//p' /usr/share/alpha/version; }
DB=/var/lib/alpha-media-test/library.db
vm "mkdir -p ${DB%/*} && echo schema=1 > ${DB}"
check "alpha-var-restore ran on the installed system" \
    grep -q "action=nothing" <<< "$(vm journalctl -b -u alpha-var-restore.service --no-pager)"
log "Step 7: apply v2 through alpha-updated (snapshot of @var, then reboot)"
BOOT="$(boot_id)"
# ssh joins its arguments into one remote shell line: quote the JSON body for that shell.
applied=
for _ in $(seq 1 30); do
    if api -X POST -H "'Content-Type: application/json'" -d "'{\"when\":\"now\"}'" \
        http://localhost/v1/update/apply > "${W}/apply.json" 2>&1; then applied=1; break; fi
    sleep 10   # 409 while the scheduler's own check or download runs
done
[[ -n "${applied}" ]] || die "alpha-updated refused the apply: $(cat "${W}/apply.json")"
wait_reboot "${BOOT}"
check "v2 booted after the agent's apply" test "$(version)" = 0.0.1-install.2
# The agent confirms a healthy update at its first scheduler step (after ~2 min):
# stop it now, as if greenboot's required checks had not passed.
vm systemctl stop alpha-updated.service
top "cat /run/t/@var-snapshots/pending.json" | tee "${W}/var-pending.json"
check "pending marker: v2 booted, snapshot recorded" python3 -c \
    'import json,sys; p=json.load(sys.stdin); assert p["to_booted"] and p["to_version"]=="0.0.1-install.2" and p["snapshot"], p' \
    < "${W}/var-pending.json"
SNAP="$(python3 -c 'import json,sys; print(json.load(sys.stdin)["snapshot"])' < "${W}/var-pending.json")"
check "snapshot ${SNAP} is read-only" grep -q true <<< "$(top "btrfs property get -ts /run/t/@var-snapshots/${SNAP} ro")"
check "snapshot holds schema 1" test "$(top "cat /run/t/@var-snapshots/${SNAP}${DB#/var}")" = schema=1
vm "echo schema=2 > ${DB}"   # the new media server migrated its database
log "Step 7: the update fails its checks, the OS goes back to v1"
vm bootc rollback
BOOT="$(boot_id)"
vm systemctl reboot || true
wait_reboot "${BOOT}"
check "v1 booted after the rollback" test "$(version)" = 0.0.1-install.1
vm journalctl -b -u alpha-var-restore.service --no-pager | tee "${W}/var-restore.log"
check "alpha-var-restore restored /var" grep -q "action=restored" "${W}/var-restore.log"
check "database is back to schema 1" test "$(vm cat "${DB}")" = schema=1
check "/var is still the @var subvolume" grep -q "subvol=/@var" <<< "$(vm findmnt -no OPTIONS /var)"
check "/var is writable" vm "touch /var/.rw-test && rm /var/.rw-test"
top "cat /run/t/@var-snapshots/last-restore.json" | tee "${W}/var-last-restore.json"
REPL="$(python3 -c 'import json,sys; d=json.load(sys.stdin); assert d["snapshot"]==sys.argv[1], d; print(d["replaced"])' "${SNAP}" \
    < "${W}/var-last-restore.json")"
check "replaced /var kept with schema 2" test "$(top "cat /run/t/@var-snapshots/${REPL}${DB#/var}")" = schema=2
check "no pending marker left" test -z "$(top "ls /run/t/@var-snapshots/pending.json 2>/dev/null || true")"
check "SELinux labels survived the restore" vm "ls -Zd /var/lib | grep -q var_lib_t"
check "media server (old version) answers" vm "curl -fsS http://127.0.0.1:8487/health"
check "agent status shows the restore" python3 -c \
    'import json,sys; v=json.load(sys.stdin)["var_snapshots"]; assert v["last_restore"]["version"]=="0.0.1-install.2", v' \
    <<< "$(api http://localhost/v1/status)"
check "no failed units after the restore" test -z "$(vm systemctl --failed --no-legend --plain | grep -v '^$' || true)"

# --- 8. BUG-031: updates of a device installed from ISO 0.0.0-ci.7 ----------------------
# That ISO always passed --enforce-container-sigpolicy, so the origin says
# ostree-image-signed while the image keeps the stock policy (no project key yet).
log "Step 8: a device tracking :stable with the ostree-image-signed transport (ISO ci.7)"
"${ENGINE}" tag "${LOCAL_REPO}:v1" "${LOCAL_REPO}:stable"
"${ENGINE}" push -q "${LOCAL_REPO}:stable" >/dev/null
vm systemctl stop alpha-updated.service
# shellcheck disable=SC2016 # $o and $(…) are expanded in the VM
vm 'mount -o remount,rw /sysroot && o="$(ostree admin --print-current-dir).origin" &&
    sed -i "s|^container-image-reference=.*|container-image-reference=ostree-image-signed:docker://'"${VM_REPO}"':stable|" "$o"; r=$?;
    mount -o remount,ro /sysroot 2>/dev/null; exit $r'
check "origin as on ISO ci.7" test "$(origin)" = "ostree-image-signed:docker://${VM_REPO}:stable"
out="$(vm bootc upgrade --check 2>&1 || true)"
check "bug reproduced: bootc refuses the signed transport under the stock policy" \
    grep -q insecureAcceptAnything <<< "${out}"
log "Step 8: the fix for such devices: bootc switch to the same tag"
vm bootc switch "${VM_REPO}:stable"
BOOT="$(boot_id)"; vm systemctl reboot || true; wait_reboot "${BOOT}"
check "after the switch: v1, plain registry transport, :stable" \
    test "$(version) $(origin)" = "0.0.1-install.1 ostree-unverified-registry:${VM_REPO}:stable"

log "Step 8: publish v2 as :stable; alpha-updated (admin API) sees it"
"${ENGINE}" tag "${LOCAL_REPO}:v2" "${LOCAL_REPO}:stable"
"${ENGINE}" push -q "${LOCAL_REPO}:stable" >/dev/null
checked=
for _ in $(seq 1 30); do
    if api -X POST http://localhost/v1/update/check > "${W}/api-check.json" 2>&1; then checked=1; break; fi
    sleep 10   # 409 while the scheduler's own check runs
done
[[ -n "${checked}" ]] || die "alpha-updated refused the check: $(cat "${W}/api-check.json")"
for _ in $(seq 1 60); do
    api http://localhost/v1/update > "${W}/api-update.json"
    python3 -c 'import json,sys; u=json.load(open(sys.argv[1])); sys.exit(0 if u.get("available") and not u.get("operation") else 1)' \
        "${W}/api-update.json" && break
    sleep 5
done
check "alpha-updated reports an available update" python3 -c \
    'import json,sys; u=json.load(open(sys.argv[1])); assert u.get("available") and not u.get("last_error"), u' "${W}/api-update.json"
vm systemctl stop alpha-updated.service   # bootc is driven by hand below

log "Step 8: bootc upgrade --check, upgrade, reboot"
vm bootc upgrade --check 2>&1 | tee "${W}/upgrade-check-stable.txt"
vm bootc upgrade 2>&1 | tail -5
BOOT="$(boot_id)"; vm systemctl reboot || true; wait_reboot "${BOOT}"
check "bootc upgrade: v2 booted, still tracking :stable" \
    test "$(version) $(origin)" = "0.0.1-install.2 ostree-unverified-registry:${VM_REPO}:stable"
log "Step 8: bootc rollback, reboot"
vm systemctl stop alpha-updated.service
vm bootc rollback
BOOT="$(boot_id)"; vm systemctl reboot || true; wait_reboot "${BOOT}"
check "bootc rollback: v1 booted again" test "$(version)" = 0.0.1-install.1
log "ALL CHECKS PASSED"
