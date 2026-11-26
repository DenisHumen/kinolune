#!/usr/bin/env bash
# SPDX-License-Identifier: MPL-2.0
#
# Wayland smoke test of the installed Kinolune (PKG-026): starts
# /usr/bin/kinolune on a headless sway (no X server, GDK_BACKEND=wayland),
# lets it open the player on a local file (KINOLUNE_SMOKE_PLAY, PKG-021)
# and checks that it still runs 30 s later. Screenshots and logs go to
# <out-dir>. Runs on x86_64 and aarch64 alike (the release workflow runs
# it on the ARM runner too).
#
#   wayland-smoke.sh <out-dir>
#
# Needs: the kinolune package installed, sway, grim, python3. No ffmpeg:
# the sample is an uncompressed AVI from tools/mpv/tests/audio_devices.
set -euo pipefail

out="$1"
here="$(cd "$(dirname "$0")" && pwd)"
repo="$(cd "$here/../../.." && pwd)"
app="${KINOLUNE_BIN:-/usr/bin/kinolune}"
mkdir -p "$out"
out="$(cd "$out" && pwd)"
fail() { echo "FAIL: $*" >&2; exit 1; }

[ -x "$app" ] || fail "$app is not installed"
# Wayland only: no X server to fall back on.
unset DISPLAY

export XDG_RUNTIME_DIR="$out/xdg-runtime"
mkdir -p "$XDG_RUNTIME_DIR" && chmod 700 "$XDG_RUNTIME_DIR"
cat >"$out/sway.conf" <<'EOF'
output HEADLESS-1 resolution 1280x720 position 0 0
default_border none
for_window [app_id=".*"] floating enable, move position 0 0, resize set 1280 720
EOF
# --unsupported-gpu: only skips the check for a loaded proprietary driver on
# the host (build machines); the headless backend uses no GPU anyway.
WLR_BACKENDS=headless WLR_RENDERER=pixman WLR_LIBINPUT_NO_DEVICES=1 \
    sway --unsupported-gpu -c "$out/sway.conf" >"$out/sway.log" 2>&1 &
sway_pid=$!
app_pid=
cleanup() {
    if [ -n "$app_pid" ]; then kill "$app_pid" 2>/dev/null || true; fi
    kill "$sway_pid" 2>/dev/null || true
}
trap cleanup EXIT
sock=
for _ in $(seq 100); do
    sock="$(find "$XDG_RUNTIME_DIR" -maxdepth 1 -name 'wayland-*' -type s | head -n 1)"
    [ -n "$sock" ] && break
    sleep 0.2
done
[ -n "$sock" ] || { cat "$out/sway.log"; fail "headless sway did not start"; }
export WAYLAND_DISPLAY="${sock##*/}" GDK_BACKEND=wayland

python3 -I "$repo/tools/mpv/tests/audio_devices/make_sample.py" "$out/sample.avi" --seconds 60

# The runner has no sound card: libmpv falls back to no audio, video plays.
KINOLUNE_SMOKE_PLAY="$out/sample.avi" "$app" >"$out/app.log" 2>&1 &
app_pid=$!
sleep 15
grim "$out/wayland-play.png" || echo "grim failed"
sleep 15
alive=true
kill -0 "$app_pid" 2>/dev/null || alive=false
grim "$out/wayland-play-30s.png" || echo "grim failed"
echo "--- app.log"
cat "$out/app.log"
[ "$alive" = true ] || fail "kinolune exited within 30 s on Wayland (playing a file)"
grep -q 'KINOLUNE_SMOKE_PLAY open ' "$out/app.log" || fail "the app did not open the player"
# The player got the sample's 160x90 video (its output resized to it).
grep -q 'width: 160, height: 90' "$out/app.log" || fail "no 160x90 video from the sample: nothing decoded"
# GTK must not have needed an X display (it would print this and exit).
if grep -qiE 'cannot open display|Gdk-WARNING.*wayland' "$out/app.log"; then fail "GTK did not run on Wayland"; fi
# The window is a native Wayland one (app_id), not an Xwayland client.
SWAYSOCK="$(find "$XDG_RUNTIME_DIR" -maxdepth 1 -name 'sway-ipc.*.sock' -type s | head -n 1)"
export SWAYSOCK
swaymsg -t get_tree >"$out/sway-tree.json"
grep -q '"app_id": "app.kinolune"' "$out/sway-tree.json" \
    || fail "no native Wayland window with app_id app.kinolune"
echo "ok: kinolune ran 30 s on Wayland ($(uname -m)) and played a local file"
