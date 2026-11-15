#!/usr/bin/env bash
# SPDX-License-Identifier: MPL-2.0
#
# QA-006: UI performance budgets. Runs the profile-mode scenarios of
# integration_test/perf_test.dart with `flutter drive --profile`, measures
# the cold start, writes build/perf/results.json and compares it with a
# baseline (regressions over 20 % are flagged). From clients/flutter/app:
#
#   tool/perf_bench.sh linux   [--core] [--no-startup] [--runs N] [--baseline F] [--github]
#   ANDROID_SERIAL=emulator-5600 tool/perf_bench.sh android [--core] ...
#
# linux:   needs a display; without $DISPLAY it runs under xvfb-run (the
#          numbers are then software rendering, llvmpipe). The cold start
#          is measured on a release bundle with a 10 000-title library
#          scanned by the core scenario (or by a quick seeding run).
# android: EMULATOR ONLY (ANDROID_SERIAL=emulator-*, ro.kernel.qemu=1),
#          like tool/chat_cpu_bench.sh. Emulator numbers are not phone
#          numbers (see docs/dev/performance.md).
#
# Extra dart-defines for the scenarios: PERF_DEFINES="--dart-define=K=V ..."
# (e.g. KINOLUNE_PERF_SLOWDOWN_MS=4 for a deliberately slowed run).
set -euo pipefail

cd "$(dirname "$0")/.."
target="${1:?usage: perf_bench.sh linux|android [options]}"
if [ "$target" = linux ] && [ -z "${DISPLAY:-}" ] && [ -z "${WAYLAND_DISPLAY:-}" ]; then
  # No display: the whole run on a virtual X server (software GL). Xvfb
  # directly: xvfb-run waits for a signal that never comes in containers.
  for n in $(seq 90 140); do [ -e "/tmp/.X11-unix/X$n" ] || break; done
  Xvfb ":$n" -screen 0 1920x1080x24 -nolisten tcp >/dev/null 2>&1 &
  xvfb_pid=$!
  trap 'kill "$xvfb_pid" 2>/dev/null || true' EXIT
  for _ in $(seq 1 50); do [ -e "/tmp/.X11-unix/X$n" ] && break; sleep 0.2; done
  export DISPLAY=":$n"
fi
shift
core=false startup=true runs=7 baseline="" github=false
while [ $# -gt 0 ]; do
  case "$1" in
    --core) core=true ;;
    --no-startup) startup=false ;;
    --runs) runs="$2"; shift ;;
    --baseline) baseline="$2"; shift ;;
    --github) github=true ;;
    *) echo "unknown option $1" >&2; exit 64 ;;
  esac
  shift
done

out=build/perf
mkdir -p "$out"
rm -f "$out/frames.json" "$out/results.json" "$out/compare.md"
work="${KINOLUNE_PERF_WORK:-$PWD/build/perf/work}"
mkdir -p "$work"
read -r -a extra <<<"${PERF_DEFINES:-}"
defines=(--dart-define=KINOLUNE_PERF_CORE="$core" "${extra[@]}")

startup_values=()

case "$target" in
  linux)
    [ "$(uname -s)" = Linux ] || { echo "linux target needs a Linux host" >&2; exit 1; }
    # The library of the cold start: $work/data/app.kinolune is the app's
    # support directory once XDG_DATA_HOME=$work/data.
    appdir="$work/data/app.kinolune"
    mkdir -p "$appdir"
    defines+=(--dart-define=KINOLUNE_PERF_CORE_DIR="$appdir")
    if $startup && ! $core && [ ! -f "$appdir/.scanned" ]; then
      echo "== seeding the 10 000-title core library"
      flutter drive --profile -d linux --driver test_driver/perf_driver.dart \
        --target integration_test/perf_test.dart \
        --dart-define=KINOLUNE_PERF_CORE=true --dart-define=KINOLUNE_PERF_ONLY=seed_only \
        --dart-define=KINOLUNE_PERF_CORE_DIR="$appdir" >"$out/seed.log" 2>&1 || {
        tail -40 "$out/seed.log"; exit 1; }
    fi
    echo "== frame timings (flutter drive --profile -d linux)"
    KINOLUNE_PERF_OUT="$out/frames.json" flutter drive --profile -d linux \
      --driver test_driver/perf_driver.dart --target integration_test/perf_test.dart \
      "${defines[@]}" 2>&1 | tee "$out/drive.log" | grep -E "perf:|All tests|Some tests|EXCEPTION|Error" || true
    if [ ! -f "$out/frames.json" ] || grep -q "Some tests failed" "$out/drive.log"; then
      echo "scenarios failed; see $out/drive.log" >&2; exit 1
    fi
    if $startup; then
      echo "== cold start (release bundle, $runs runs)"
      flutter build linux --release >"$out/build-linux.log" 2>&1 || {
        tail -40 "$out/build-linux.log"; exit 1; }
      bin=build/linux/x64/release/bundle/kinolune
      for i in $(seq 1 "$runs"); do
        line="$(XDG_DATA_HOME="$work/data" XDG_CACHE_HOME="$work/cache" \
          XDG_CONFIG_HOME="$work/config" KINOLUNE_STARTUP_PROBE=exit \
          timeout 60 "$bin" 2>/dev/null \
          | grep -m1 KINOLUNE_STARTUP || true)"
        echo "  run $i: ${line:-no probe line}"
        [ -n "$line" ] && startup_values+=("$(sed -E 's/.*first_frame_ms=([0-9]+) library_visible_ms=([0-9]+).*/\1,\2/' <<<"$line")")
      done
    fi
    ;;
  android)
    serial="${ANDROID_SERIAL:?set ANDROID_SERIAL to an emulator}"
    case "$serial" in
      emulator-*) ;;
      *) echo "refusing: $serial is not an emulator" >&2; exit 1 ;;
    esac
    A() { adb -s "$serial" "$@"; }
    qemu="$(A shell getprop ro.kernel.qemu | tr -d '\r')"
    [ "$qemu" = 1 ] || { echo "refusing: $serial does not report ro.kernel.qemu=1" >&2; exit 1; }
    # `app`: the app's own support directory (the emulator's app sandbox),
    # so the cold start below finds the scanned library.
    defines+=(--dart-define=KINOLUNE_PERF_CORE_DIR=app)
    echo "== frame timings (flutter drive --profile -d $serial)"
    KINOLUNE_PERF_OUT="$out/frames.json" flutter drive --profile -d "$serial" \
      --driver test_driver/perf_driver.dart --target integration_test/perf_test.dart \
      --no-android-gradle-daemon "${defines[@]}" 2>&1 | tee "$out/drive.log" \
      | grep -E "perf:|All tests|Some tests|EXCEPTION|Error" || true
    if [ ! -f "$out/frames.json" ] || grep -q "Some tests failed" "$out/drive.log"; then
      echo "scenarios failed; see $out/drive.log" >&2; exit 1
    fi
    if $startup; then
      echo "== cold start (profile APK with the probe, $runs runs)"
      flutter build apk --profile --target-platform android-x64 \
        --dart-define=KINOLUNE_STARTUP_PROBE=1 >"$out/build-apk.log" 2>&1 || {
        tail -40 "$out/build-apk.log"; exit 1; }
      A install -r build/app/outputs/flutter-apk/app-profile.apk >/dev/null
      pkg=app.kinolune
      activity="$(A shell cmd package resolve-activity --brief "$pkg" | tr -d '\r' | tail -1)"
      for i in $(seq 1 "$runs"); do
        A shell am force-stop "$pkg"
        A logcat -c
        A shell am start -W -n "$activity" >/dev/null
        line=""
        for _ in $(seq 1 60); do
          line="$(A logcat -d -s flutter:I | grep -m1 KINOLUNE_STARTUP || true)"
          [ -n "$line" ] && break
          sleep 0.5
        done
        echo "  run $i: ${line:-no probe line}"
        [ -n "$line" ] && startup_values+=("$(sed -E 's/.*first_frame_ms=([0-9]+) library_visible_ms=([0-9]+).*/\1,\2/' <<<"$line")")
      done
      A shell am force-stop "$pkg"
    fi
    ;;
  *) echo "unknown target $target" >&2; exit 64 ;;
esac

dart run tool/perf/merge.dart "$out/frames.json" "$out/results.json" "${startup_values[@]}"
echo "== results: $out/results.json"
if [ -n "$baseline" ]; then
  args=("$baseline" "$out/results.json")
  $github && args+=(--github)
  dart run tool/perf/compare.dart "${args[@]}" | tee "$out/compare.md"
fi
