#!/usr/bin/env bash
# SPDX-License-Identifier: MPL-2.0
# DIST-032: boots a throwaway Fedora Cloud VM and runs wifi-hwsim.sh in it
# (the app's NetworkManager client against virtual Wi-Fi). QEMU runs in a
# container, so the host needs only the engine and /dev/kvm; the host
# kernel is never touched.
#
#   distro/tests/wifi-hwsim-vm.sh FEDORA_CLOUD_QCOW2 NM_PROBE
#
# FEDORA_CLOUD_QCOW2: a Fedora Cloud Base Generic image (read-only; the VM
#   writes to an overlay). The VM installs NetworkManager-wifi, hostapd,
#   dnsmasq and kernel-modules-internal (mac80211_hwsim) from Fedora repos.
# NM_PROBE: `dart compile exe clients/flutter/app/tool/nm_probe.dart`.
# Env: CONTAINER_NAME (default kinolune-wifi-hwsim), ENGINE, WORK_DIR,
#   VM_CPUS, VM_MEM_MB, CONTAINER_IMAGE (default fedora:latest).
set -euo pipefail
source "$(dirname "$0")/../scripts/lib.sh"

IMAGE="$(readlink -f "${1:?Fedora Cloud qcow2 required}")"
PROBE="$(readlink -f "${2:?nm_probe binary required}")"
NAME="${CONTAINER_NAME:-kinolune-wifi-hwsim}"
CIMAGE="${CONTAINER_IMAGE:-fedora:latest}"
W="${WORK_DIR}/wifi-hwsim"
[[ -r /dev/kvm ]] || die "/dev/kvm is not available"

rm -rf "${W}"
mkdir -p "${W}"
cp "${PROBE}" "${W}/nm_probe"
cp "${DISTRO_DIR}/tests/wifi-hwsim.sh" "${W}/"
cp "${DISTRO_DIR}/rootfs/usr/share/polkit-1/rules.d/50-kinolune-network.rules" "${W}/"
ssh-keygen -q -t ed25519 -N '' -f "${W}/key"
cat > "${W}/user-data" <<EOF
#cloud-config
users:
  - name: kl
    sudo: ALL=(ALL) NOPASSWD:ALL
    ssh_authorized_keys:
      - $(cat "${W}/key.pub")
EOF
printf 'instance-id: kinolune-wifi-hwsim\nlocal-hostname: kinolune-wifi\n' > "${W}/meta-data"

cleanup() { "${ENGINE}" rm -f "${NAME}" >/dev/null 2>&1 || true; }
trap cleanup EXIT
cleanup

log "Container ${NAME} with QEMU"
"${ENGINE}" run -d --name "${NAME}" --device /dev/kvm \
    -v "${W}:/work" -v "${IMAGE}:/base.qcow2:ro" "${CIMAGE}" sleep infinity >/dev/null
in_c() { "${ENGINE}" exec "${NAME}" bash -c "$*"; }
in_c "dnf -y -q install --setopt=install_weak_deps=False qemu-system-x86-core qemu-img xorriso openssh-clients >/dev/null"
in_c "cd /work && xorriso -as genisoimage -quiet -output seed.iso -volid cidata -joliet -rock user-data meta-data"
in_c "qemu-img create -q -f qcow2 -F qcow2 -b /base.qcow2 /work/disk.qcow2 10G"

log "Booting the VM (${VM_CPUS} vCPU, ${VM_MEM_MB} MiB)"
in_c "nice -n 10 qemu-system-x86_64 -enable-kvm -machine q35 -cpu host \
    -smp ${VM_CPUS} -m ${VM_MEM_MB} \
    -drive file=/work/disk.qcow2,if=virtio,cache=unsafe \
    -drive file=/work/seed.iso,media=cdrom \
    -netdev user,id=n0,hostfwd=tcp:127.0.0.1:2222-:22 -device virtio-net-pci,netdev=n0 \
    -display none -serial file:/work/serial.log -daemonize -pidfile /work/qemu.pid"
SSH="ssh -q -i /work/key -p 2222 -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o ConnectTimeout=5 kl@127.0.0.1"
for _ in $(seq 1 60); do
    in_c "${SSH} true" 2>/dev/null && break
    sleep 5
done
in_c "${SSH} true" || die "VM did not come up (see ${W}/serial.log)"

log "Installing NetworkManager Wi-Fi, hostapd, dnsmasq and mac80211_hwsim in the VM"
in_c "${SSH} 'sudo dnf -y -q install --setopt=install_weak_deps=False NetworkManager-wifi wpa_supplicant hostapd dnsmasq iw python3 polkit kernel-modules-internal-\$(uname -r) && sudo systemctl restart NetworkManager'"
in_c "scp -q -i /work/key -P 2222 -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null \
    /work/nm_probe /work/wifi-hwsim.sh /work/50-kinolune-network.rules kl@127.0.0.1:"

log "Running the Wi-Fi scenario"
in_c "${SSH} 'chmod +x nm_probe wifi-hwsim.sh && sudo ./wifi-hwsim.sh ./nm_probe ./50-kinolune-network.rules'"
