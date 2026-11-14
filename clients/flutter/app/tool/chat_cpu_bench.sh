#!/usr/bin/env bash
# SPDX-License-Identifier: MPL-2.0
#
# TWI-012: CPU of the Twitch chat on an Android EMULATOR. Installs the chat
# benchmark APK (lib/features/twitch/chat/chat_bench_main.dart: the chat
# panel fed by the core's made-up chat, local images, no network), lets it
# warm up, then reads the process CPU time from /proc for SECONDS.
#
#   flutter build apk --profile --target-platform android-x64 \
#     -t lib/features/twitch/chat/chat_bench_main.dart \
#     --dart-define=KINOLUNE_CHAT_BENCH_RATE=100
#   ANDROID_SERIAL=emulator-5554 tool/chat_cpu_bench.sh build/app/outputs/flutter-apk/app-profile.apk 60
#
# Refuses to run on anything but an emulator: ANDROID_SERIAL must be set,
# start with `emulator-` and the device must report ro.kernel.qemu=1.
set -euo pipefail

serial="${ANDROID_SERIAL:?set ANDROID_SERIAL to an emulator}"
case "$serial" in
  emulator-*) ;;
  *) echo "refusing: $serial is not an emulator" >&2; exit 1 ;;
esac
A() { adb -s "$serial" "$@"; }
qemu="$(A shell getprop ro.kernel.qemu | tr -d '\r')"
[ "$qemu" = 1 ] || { echo "refusing: $serial does not report ro.kernel.qemu=1" >&2; exit 1; }

apk="${1:?usage: chat_cpu_bench.sh <apk> [seconds] [warm-up seconds]}"
secs="${2:-60}"
warm="${3:-10}"
pkg=app.kinolune

A install -r "$apk" >/dev/null
A shell am force-stop "$pkg"
activity="$(A shell cmd package resolve-activity --brief "$pkg" | tr -d '\r' | tail -1)"
A shell am start -n "$activity" >/dev/null
sleep "$warm"
pid="$(A shell pidof "$pkg" | tr -d '\r')"
[ -n "$pid" ] || { echo "app is not running" >&2; exit 1; }

hz="$(A shell getconf CLK_TCK 2>/dev/null | tr -d '\r')"
hz="${hz:-100}"
cores="$(A shell nproc | tr -d '\r')"
ticks() { A shell cat "/proc/$pid/stat" | tr -d '\r' | awk '{print $14 + $15}'; }

t0="$(ticks)"
s0="$(date +%s.%N)"
sleep "$secs"
t1="$(ticks)"
s1="$(date +%s.%N)"

echo "emulator: $(A shell getprop ro.product.model | tr -d '\r'), Android $(A shell getprop ro.build.version.release | tr -d '\r'), $cores vCPU"
awk -v a="$t0" -v b="$t1" -v hz="$hz" -v s0="$s0" -v s1="$s1" -v n="$cores" 'BEGIN {
  wall = s1 - s0; cpu = (b - a) / hz;
  printf "window: %.1f s, process CPU time: %.2f s\n", wall, cpu;
  printf "CPU: %.1f %% of one core, %.1f %% of all %d cores\n", 100 * cpu / wall, 100 * cpu / wall / n, n;
}'
echo "--- busiest threads (top -H, one sample)"
A shell top -H -b -n 1 -p "$pid" | tr -d '\r' | sed -n '1,/^$/d;p' | head -14
echo "--- frames"
A shell dumpsys gfxinfo "$pkg" | tr -d '\r' | grep -E 'Total frames|Janky frames|50th|90th|99th' || true
