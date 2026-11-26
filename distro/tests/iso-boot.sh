#!/usr/bin/env bash
# SPDX-License-Identifier: MPL-2.0
# Smoke test of the installer ISO (DIST-007): boot it in QEMU with UEFI (OVMF) and drive
# the installer on the serial console.
#
#   distro/tests/iso-boot.sh ISO [LOG]        # LOG: serial transcript (default next to ISO)
#
# Checks: the GRUB menu shows "Install Kinolune OS"; the live system boots to the
# full-screen installer (TUI, DIST-006; it owns tty1, not kmscon), which offers the empty
# NVMe and explains why the ISO drive is not offered; Ctrl+C leads to the text menu; the
# menu lists the disks; in its shell the live system is Kinolune OS and the offline OS
# image (payload) is in containers-storage; the installer passes a dry run on the NVMe,
# from an answer file and through every TUI screen (wrong disk name refused).
#
# Requirements: qemu-system-x86_64, OVMF, /dev/kvm, python3. ~3-5 minutes with KVM.
# Env: QEMU_EXEC  prefix for qemu and kill, e.g. "docker exec my-qemu" (container on the
#                 host network, sharing the ISO directory under the same path);
#      OVMF_CODE, OVMF_VARS  firmware paths as seen by QEMU;
#      PAYLOAD_TARGET  ref expected in the ISO (default ghcr.io/denishumen/alpha-os:stable);
#      SERIAL_PORT     TCP port of the serial console on 127.0.0.1 (default 22541);
#      BOOT_TIMEOUT    seconds until the installer prompt (default 600);
#      INSTALL=tui     the same through the TUI screens (DIST-006), plus the saved
#                      repeat-install answer file (yescrypt hash, valid for --dry-run);
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
mode = sys.argv[5]
install = mode in ("1", "tui")
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

DOWN, ENTER, CTRL_C = "\x1b[B", "\r", "\x03"

wait(r"Install Kinolune OS", 120, "GRUB menu shows 'Install Kinolune OS'")
# The full-screen installer (DIST-006) owns the console: language first, Ukrainian on top.
wait(r"Мова / Language / Язык", boot_timeout, "live system booted to the full-screen installer")
wait(r"Українська", 30, "language list")
send(DOWN)
send(ENTER)
wait(r"System disk", 60, "TUI disk screen (English)")
wait(r"/dev/nvme0n1", 30, "TUI offers the empty NVMe")
wait(r"/dev/sr0: (optical drive|the installer boots from it)", 30, "TUI explains why the ISO drive is not offered")
# Leave the TUI for the text menu (its fallback), as a user would with Ctrl+C.
send(CTRL_C)
wait(r"Leave the installer\?", 30, "TUI exit dialog")
send(DOWN)
send(ENTER)
wait(r"Choose an action \[1-9\]", 60, "text menu (English, language taken from the TUI)")
send("1\r")
m = wait(r"Press Enter to go back to the menu", 120, "disk list printed")
send("\r")
wait(r"Choose an action \[1-9\]", 60, "back in the menu")
send("6\r")
time.sleep(2)
# Markers are typed as O""SR= etc.: the echoed command line never matches the pattern.
send("echo O\"\"SR=$(. /etc/os-release; echo \"$NAME|$VARIANT\"); "
     "echo PAY\"\"LOAD=$(podman images --format '{{.Repository}}:{{.Tag}}' | tr '\\n' ' '); "
     "echo DI\"\"SKS=$(alpha-installer disks --all | tr -s ' ' | tr '\\n' ' '); "
     "echo TT\"\"Y1=$(systemctl is-active alpha-live-installer@tty1.service)/$(systemctl is-active kmsconvt@tty1.service); "
     "printf 'SMOKE-%s\\n' done\r")
m = wait(r"OSR=(.*)\r?\n", 60, "os-release read in the live shell")
osr = m.group(1)
m = wait(r"PAYLOAD=(.*)\r?\n", 120, "podman images listed")
images = m.group(1)
m = wait(r"DISKS=(.*)\r?\n", 120, "disks listed")
disks = m.group(1)
m = wait(r"TTY1=(\S*)\r?\n", 30, "tty1 owner read")
tty1 = m.group(1)
wait(r"SMOKE-done", 30, "shell commands finished")
# The monitor console: our installer owns tty1, kmscon (Fedora's VT login) must not.
if tty1 != "active/inactive":
    errors_tty1 = [f"tty1: installer/kmscon state is {tty1!r}, expected 'active/inactive'"]
else:
    errors_tty1 = []
errors = errors_tty1
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
m = wait(r"DRYRUN=(\d+)\r?\n", 120, "installer dry run finished")
if m.group(1) != "0":
    errors.append("alpha-installer --dry-run failed in the live system")
# The same dry run through every screen of the TUI, on this serial console (80x24).
send("alpha-installer tui --dry-run --lang en --action-file /tmp/tui-action; "
     "echo TUI\"\"EXIT=$?:$(cat /tmp/tui-action)\r")
wait(r"Мова / Language / Язык", 60, "TUI started from the shell")
send(ENTER)
wait(r"System disk", 60, "TUI: disk")
send(ENTER)
wait(r"Disk layout", 30, "TUI: layout")
send(ENTER)
wait(r"Computer name", 30, "TUI: host name")
send(ENTER)
wait(r"Administrator", 30, "TUI: user")
send(DOWN); send(DOWN); send("smoke-test-pw"); send(DOWN); send("smoke-test-pw"); send(ENTER)
wait(r"Network", 30, "TUI: network")
send(ENTER); send(ENTER)
wait(r"To start, type the disk name nvme0n1", 30, "TUI: summary asks for the disk name")
send("sda" + ENTER)
wait(r"The name does not match", 30, "TUI: a wrong disk name is refused")
send("\x15nvme0n1" + ENTER)
wait(r"Dry run finished", 120, "TUI: dry run through the installer finished")
send(DOWN); send(ENTER)
m = wait(r"TUIEXIT=(\d+):(\w*)\r?\n", 60, "TUI exited")
if m.group(1) != "0" or m.group(2) != "menu":
    errors.append(f"TUI dry run: exit {m.group(1)}, action {m.group(2)!r}")
if errors:
    sys.exit("FAILED:\n  " + "\n  ".join(errors))
print(f"==> live system: {osr}; payload: {images.strip()}", flush=True)
if mode == "tui":
    # Real offline installation through the TUI screens (unsigned test payload: the
    # answer file only pre-fills what the screens do not ask).
    send("printf '%s' '{\"version\":1,\"hostname\":\"kinolune-iso\",\"image\":{\"enforce_signature_policy\":false},"
         "\"kernel_args\":[\"console=ttyS0,115200\"]}' > /root/base.json; "
         "alpha-installer tui --answers /root/base.json --lang en --action-file /tmp/tui-action; "
         "echo TUI\"\"INSTALL=$?:$(cat /tmp/tui-action)\r")
    wait(r"Мова / Language / Язык", 60, "TUI install: started")
    send(ENTER)
    wait(r"System disk", 60, "TUI install: disk")
    send(ENTER)
    wait(r"Disk layout", 30, "TUI install: layout")
    send(" "); send(DOWN); send("\x15" + "20"); send(ENTER); send(ENTER)
    wait(r"Computer name", 30, "TUI install: host name (data partition, 20 GiB system)")
    send(ENTER)
    wait(r"Administrator", 30, "TUI install: user")
    send(DOWN); send(DOWN); send("smoke-test-pw"); send(DOWN); send("smoke-test-pw"); send(ENTER)
    wait(r"Network", 30, "TUI install: network")
    send(ENTER); send(ENTER)
    wait(r"To start, type the disk name nvme0n1", 30, "TUI install: summary")
    send("nvme0n1" + ENTER)
    wait(r"Installing Kinolune OS", 60, "TUI install: progress screen")
    # DIST-034: a line per task with its bar; the copy shows amounts and speed.
    wait(r"Copying the system image", 120, "TUI install: task list")
    wait(r"MB of [0-9.]+ [MG]B, [0-9.]+ [kMG]?B/s", 600, "TUI install: copy amounts and speed")
    wait(r"Kinolune OS is installed", 1800, "TUI install: success screen")
    send(DOWN); send(ENTER)
    wait(r"Save an answer file", 30, "TUI install: save screen")
    send(ENTER)
    wait(r"Saved: /root/kinolune-answers.json", 30, "TUI install: answer file saved")
    send("\x1b"); time.sleep(1); send(DOWN); send(DOWN); send(ENTER)
    m = wait(r"TUIINSTALL=(\d+):(\w*)\r?\n", 60, "TUI install: exited")
    if m.group(1) != "0" or m.group(2) != "menu":
        sys.exit(f"FAILED: TUI install: exit {m.group(1)}, action {m.group(2)!r}")
    # DIST-034: every task moved and finished, the ETA halfway was close.
    send("cat /run/alpha-installer/progress.jsonl; alpha-installer progress; echo PRO\"\"GRESS=$?\r")
    m = wait(r"PROGRESS=(\d+)\r?\n", 60, "progress stream checked")
    if m.group(1) != "0":
        sys.exit("FAILED: progress stream (see alpha-installer progress above)")
    # The saved file carries the yescrypt hash, no clear-text password, and is valid.
    send("echo HA\"\"SH=$(grep -c 'password_hash\": \"\\$y\\$' /root/kinolune-answers.json)"
         ":$(grep -c smoke-test-pw /root/kinolune-answers.json)"
         ":$(alpha-installer install --answers /root/kinolune-answers.json --dry-run >/dev/null 2>&1; echo $?)\r")
    m = wait(r"HASH=(\d+):(\d+):(\d+)\r?\n", 60, "saved answer file checked")
    if m.groups() != ("1", "0", "0"):
        sys.exit(f"FAILED: saved answer file: hash/clear-text/dry-run = {m.groups()}")
    send("systemctl poweroff\r")
elif install:
    # Real offline installation from the ISO onto the NVMe (unsigned test payload).
    send("printf '%s' '{\"version\":1,\"disk\":{\"path\":\"/dev/nvme0n1\",\"data_partition\":true,\"root_size_gib\":20},"
         "\"hostname\":\"kinolune-iso\",\"user\":{\"name\":\"admin\",\"password\":\"smoke-test-pw\"},"
         "\"image\":{\"enforce_signature_policy\":false},\"kernel_args\":[\"console=ttyS0,115200\"]}' > /root/answers.json; "
         "alpha-installer install --answers /root/answers.json --yes 2>&1 | tail -n 40; echo INST\"\"ALL=${PIPESTATUS[0]}\r")
    m = wait(r"INSTALL=(\d+)\r?\n", 1800, "offline installation from the ISO finished")
    if m.group(1) != "0":
        sys.exit("FAILED: alpha-installer install from the ISO")
    send("systemctl poweroff\r")
else:
    send("exit\r")
    wait(r"Choose an action \[1-9\]", 60, "back in the menu after the shell")
EOF

if [[ "${INSTALL}" == 1 || "${INSTALL}" == tui ]]; then
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
