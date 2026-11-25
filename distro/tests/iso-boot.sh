#!/usr/bin/env bash
# SPDX-License-Identifier: MPL-2.0
# Smoke test of the installer ISO (DIST-007): boot it in QEMU with UEFI (OVMF) and drive
# the installer on the serial console.
#
#   distro/tests/iso-boot.sh ISO [LOG]        # LOG: serial transcript (default next to ISO)
#
# Checks: the GRUB menu shows "Install Kinolune OS"; the live system boots to the
# installer prompt; the menu lists the disks (the empty NVMe is offered, the ISO drive is
# excluded); in its shell the live system is Kinolune OS and the offline OS image
# (payload) is in containers-storage; the installer passes a dry run on the NVMe.
#
# Requirements: qemu-system-x86_64, OVMF, /dev/kvm, python3. ~3-5 minutes with KVM.
# Env: QEMU_EXEC  prefix for qemu and kill, e.g. "docker exec my-qemu" (container on the
#                 host network, sharing the ISO directory under the same path);
#      OVMF_CODE, OVMF_VARS  firmware paths as seen by QEMU;
#      PAYLOAD_TARGET  ref expected in the ISO (default ghcr.io/denishumen/alpha-os:stable);
#      SERIAL_PORT     TCP port of the serial console on 127.0.0.1 (default 22541);
#      BOOT_TIMEOUT    seconds until the installer prompt (default 600);
#      INSTALL=1       then install offline from the ISO onto the NVMe (unsigned payload,
#                      ~10 min more) and boot the installed disk up to its login prompt.
set -euo pipefail
source "$(dirname "$0")/../scripts/lib.sh"

ISO="$(realpath "${1:?ISO path required}")"
LOG="${2:-${ISO%.iso}-serial.log}"
PAYLOAD_TARGET="${PAYLOAD_TARGET:-ghcr.io/denishumen/alpha-os:stable}"
SERIAL_PORT="${SERIAL_PORT:-22541}"
BOOT_TIMEOUT="${BOOT_TIMEOUT:-600}"
INSTALL="${INSTALL:-0}"
W="$(dirname "${ISO}")/iso-boot"
mkdir -p "${W}"
read -r -a QEMU_EXEC_ARR <<< "${QEMU_EXEC:-}"

if [[ -z "${OVMF_CODE:-}" ]]; then
    for c in /usr/share/OVMF/OVMF_CODE_4M.fd /usr/share/edk2/ovmf/OVMF_CODE.fd /usr/share/OVMF/OVMF_CODE.fd; do
        if "${QEMU_EXEC_ARR[@]}" test -f "${c}"; then OVMF_CODE="${c}"; break; fi
    done
fi
[[ -n "${OVMF_CODE:-}" ]] || die "OVMF not found, set OVMF_CODE and OVMF_VARS"
OVMF_VARS="${OVMF_VARS:-${OVMF_CODE/CODE/VARS}}"

vm_stop() {
    local pid _
    pid="$("${QEMU_EXEC_ARR[@]}" cat "${W}/qemu.pid" 2>/dev/null)" || return 0
    "${QEMU_EXEC_ARR[@]}" kill "${pid}" 2>/dev/null || true
    for _ in $(seq 1 30); do
        "${QEMU_EXEC_ARR[@]}" kill -0 "${pid}" 2>/dev/null || break
        sleep 1
    done
    "${QEMU_EXEC_ARR[@]}" rm -f "${W}/qemu.pid"
}
trap vm_stop EXIT

rm -f "${W}/target.raw"
truncate -s 32G "${W}/target.raw"
"${QEMU_EXEC_ARR[@]}" cp "${OVMF_VARS}" "${W}/OVMF_VARS.fd"

log "Booting ${ISO} (UEFI, ${VM_CPUS} vCPU, ${VM_MEM_MB} MiB), serial on 127.0.0.1:${SERIAL_PORT}"
nice -n 10 "${QEMU_EXEC_ARR[@]}" qemu-system-x86_64 \
    -enable-kvm -machine q35 -cpu host -smp "${VM_CPUS}" -m "${VM_MEM_MB}" \
    -drive "if=pflash,format=raw,readonly=on,file=${OVMF_CODE}" \
    -drive "if=pflash,format=raw,file=${W}/OVMF_VARS.fd" \
    -drive "file=${ISO},media=cdrom,readonly=on,if=none,id=cd" \
    -device ide-cd,drive=cd,bootindex=0 \
    -drive "file=${W}/target.raw,format=raw,if=none,id=nvm,cache=unsafe" \
    -device nvme,serial=kinolunetarget,drive=nvm \
    -nic none -display none \
    -chardev "socket,id=s0,host=127.0.0.1,port=${SERIAL_PORT},server=on,wait=off" \
    -serial chardev:s0 \
    -daemonize -pidfile "${W}/qemu.pid"

# Drive the serial console: wait for a pattern, then type. The transcript goes to LOG.
python3 - "${SERIAL_PORT}" "${LOG}" "${BOOT_TIMEOUT}" "${PAYLOAD_TARGET}" "${INSTALL}" <<'EOF'
import codecs, re, socket, sys, time

port, log_path, boot_timeout, payload = int(sys.argv[1]), sys.argv[2], int(sys.argv[3]), sys.argv[4]
install = sys.argv[5] == "1"
log = open(log_path, "wb")
dec = codecs.getincrementaldecoder("utf-8")("replace")
buf = ""

for _ in range(60):
    try:
        s = socket.create_connection(("127.0.0.1", port), timeout=5)
        break
    except OSError:
        time.sleep(1)
else:
    sys.exit("cannot connect to the serial console")
s.settimeout(1)

def wait(pattern, timeout, what):
    """Read until `pattern` (regex over the decoded text since the last match)."""
    global buf
    rx = re.compile(pattern)
    end = time.time() + timeout
    while time.time() < end:
        m = rx.search(buf)
        if m:
            buf = buf[m.end():]
            print(f"==> OK {what}", flush=True)
            return m
        try:
            data = s.recv(65536)
        except socket.timeout:
            continue
        if not data:
            break
        log.write(data); log.flush()
        buf += dec.decode(data)
    tail = buf[-2000:]
    sys.exit(f"FAILED: {what}: no /{pattern}/ within {timeout}s; last output:\n{tail}")

def send(text):
    time.sleep(0.5)
    s.sendall(text.encode())

wait(r"Install Kinolune OS", 120, "GRUB menu shows 'Install Kinolune OS'")
wait(r"Language / Мова / Язык", boot_timeout, "live system booted to the installer prompt")
send("1\r")
wait(r"Choose an action \[1-8\]", 60, "installer menu (English)")
send("1\r")
m = wait(r"Press Enter to go back to the menu", 120, "disk list printed")
send("\r")
wait(r"Choose an action \[1-8\]", 60, "back in the menu")
send("6\r")
time.sleep(2)
# Markers are typed as O""SR= etc.: the echoed command line never matches the pattern.
send("echo O\"\"SR=$(. /etc/os-release; echo \"$NAME|$VARIANT\"); "
     "echo PAY\"\"LOAD=$(podman images --format '{{.Repository}}:{{.Tag}}' | tr '\\n' ' '); "
     "echo DI\"\"SKS=$(alpha-installer disks --all | tr -s ' ' | tr '\\n' ' '); "
     "printf 'SMOKE-%s\\n' done\r")
m = wait(r"OSR=(.*)\r?\n", 60, "os-release read in the live shell")
osr = m.group(1)
m = wait(r"PAYLOAD=(.*)\r?\n", 120, "podman images listed")
images = m.group(1)
m = wait(r"DISKS=(.*)\r?\n", 120, "disks listed")
disks = m.group(1)
wait(r"SMOKE-done", 30, "shell commands finished")
errors = []
if not osr.startswith("Kinolune OS|Installer"):
    errors.append(f"live system is not Kinolune OS Installer: {osr!r}")
if payload not in images.split():
    errors.append(f"payload {payload} not in containers-storage: {images!r}")
if "/dev/nvme0n1" not in disks:
    errors.append(f"NVMe target not offered: {disks!r}")
if not re.search(r"excluded: (optical|boot-media)", disks):
    errors.append(f"the ISO drive is not excluded: {disks!r}")
# A dry run against the empty NVMe: answer file validation and the install plan.
send("printf '%s' '{\"version\":1,\"disk\":{\"path\":\"/dev/nvme0n1\"},\"user\":{\"name\":\"admin\",\"password\":\"smoke-test-pw\"}}' > /root/answers.json; "
     "alpha-installer install --answers /root/answers.json --dry-run; echo DRY\"\"RUN=$?\r")
m = wait(r"DRYRUN=(\d+)", 120, "installer dry run finished")
if m.group(1) != "0":
    errors.append("alpha-installer --dry-run failed in the live system")
if errors:
    sys.exit("FAILED:\n  " + "\n  ".join(errors))
print(f"==> live system: {osr}; payload: {images.strip()}", flush=True)
if install:
    # Real offline installation from the ISO onto the NVMe (unsigned test payload).
    send("printf '%s' '{\"version\":1,\"disk\":{\"path\":\"/dev/nvme0n1\",\"data_partition\":true,\"root_size_gib\":20},"
         "\"hostname\":\"kinolune-iso\",\"user\":{\"name\":\"admin\",\"password\":\"smoke-test-pw\"},"
         "\"image\":{\"enforce_signature_policy\":false},\"kernel_args\":[\"console=ttyS0,115200\"]}' > /root/answers.json; "
         "alpha-installer install --answers /root/answers.json --yes 2>&1 | tail -n 40; echo INST\"\"ALL=${PIPESTATUS[0]}\r")
    m = wait(r"INSTALL=(\d+)", 1800, "offline installation from the ISO finished")
    if m.group(1) != "0":
        sys.exit("FAILED: alpha-installer install from the ISO")
    send("systemctl poweroff\r")
else:
    send("exit\r")
    wait(r"Choose an action \[1-8\]", 60, "back in the menu after the shell")
EOF

if [[ "${INSTALL}" == 1 ]]; then
    for _ in $(seq 1 60); do
        pid="$("${QEMU_EXEC_ARR[@]}" cat "${W}/qemu.pid" 2>/dev/null || echo 0)"
        "${QEMU_EXEC_ARR[@]}" kill -0 "${pid}" 2>/dev/null || break
        sleep 2
    done
    vm_stop
    log "Booting the installed NVMe alone (same OVMF NVRAM)"
    nice -n 10 "${QEMU_EXEC_ARR[@]}" qemu-system-x86_64 \
        -enable-kvm -machine q35 -cpu host -smp "${VM_CPUS}" -m "${VM_MEM_MB}" \
        -drive "if=pflash,format=raw,readonly=on,file=${OVMF_CODE}" \
        -drive "if=pflash,format=raw,file=${W}/OVMF_VARS.fd" \
        -drive "file=${W}/target.raw,format=raw,if=none,id=nvm,cache=unsafe" \
        -device nvme,serial=kinolunetarget,drive=nvm \
        -nic none -display none \
        -chardev "socket,id=s0,host=127.0.0.1,port=${SERIAL_PORT},server=on,wait=off" \
        -serial chardev:s0 \
        -daemonize -pidfile "${W}/qemu.pid"
    python3 - "${SERIAL_PORT}" "${LOG%.log}-installed.log" <<'EOF'
import re, socket, sys, time
port, log_path = int(sys.argv[1]), sys.argv[2]
log = open(log_path, "wb")
for _ in range(60):
    try:
        s = socket.create_connection(("127.0.0.1", port), timeout=5)
        break
    except OSError:
        time.sleep(1)
else:
    sys.exit("cannot connect to the serial console")
s.settimeout(1)
buf, end = b"", time.time() + 600
while time.time() < end:
    try:
        data = s.recv(65536)
    except socket.timeout:
        continue
    if not data:
        break
    log.write(data)
    log.flush()
    buf += data
    text = buf.decode("utf-8", "replace")
    if re.search(r"kinolune-iso login:", text):
        if "Kinolune OS" not in text:
            sys.exit("FAILED: the installed system boots, but its console does not show Kinolune OS")
        print("==> OK installed system booted to 'kinolune-iso login:' (Kinolune OS)", flush=True)
        sys.exit(0)
sys.exit("FAILED: the installed system did not reach the login prompt")
EOF
    log "ISO INSTALL TEST PASSED (serial: ${LOG%.log}-installed.log)"
fi
log "ISO BOOT SMOKE TEST PASSED (serial transcript: ${LOG})"
