#!/usr/bin/env bash
# SPDX-License-Identifier: MPL-2.0
#
# Prepare the native libraries of the local package media_kit_libs_windows_video
# (PKG-020, ADR-NNNN) before `flutter build windows`.
#
#   mpv-libs.sh <dest: .../media_kit_libs_windows_video/windows/libs> <libmpv-windows-x86_64>
#
# <libmpv-windows-x86_64> is the artifact of libmpv.yml (tools/mpv). Writes
#   <dest>/libmpv/  libmpv-2.dll + the FFmpeg DLLs it imports, libmpv.dll.a,
#                   the mpv client headers (flat: media_kit includes <client.h>),
#                   licenses/ and BUILDINFO.txt;
#   <dest>/ANGLE/   ANGLE (EGL/GLES on Direct3D 11, BSD-3-Clause) from a pinned
#                   archive checked by SHA-256: headers, import libraries,
#                   libEGL.dll, libGLESv2.dll and the DLLs they import from the
#                   archive (not d3dcompiler_47.dll: Windows 10+ has it), and
#                   ANGLE's LICENSE (pinned commit, SHA-256).
# Gates: the artifact is the windows-x86_64 build, libmpv-2.dll is -Dgpl=false,
# FFmpeg is "LGPL version 2.1 or later", every libav*/libsw* DLL libmpv imports
# is present. Runs in Git Bash on Windows and on Linux (needs curl, python3,
# cmake for the .7z).
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# ANGLE as media_kit_libs_windows_video 1.0.11 uses it (prebuilt from
# google/angle by the media_kit author); pinned by SHA-256 instead of MD5.
ANGLE_URL="https://github.com/alexmercerind/flutter-windows-ANGLE-OpenGL-ES/releases/download/v1.0.1/ANGLE.7z"
ANGLE_SHA256="cc5911bb15d596fd5a2b362613ad35b7093b427117269a7359054a65746a5f9a"
ANGLE_LICENSE_URL="https://chromium.googlesource.com/angle/angle/+/cce5cf7476ef25ddd2c15ede6e4e9d1ce1d3c51a/LICENSE?format=TEXT"
ANGLE_LICENSE_SHA256="bf4da21bd20bcfb5b60b7ecc67fa864a79be049e21d6178076887f178dd6c71a"

die() { printf 'mpv-libs.sh: %s\n' "$*" >&2; exit 1; }
log() { printf 'mpv-libs.sh: %s\n' "$*" >&2; }

[ $# -eq 2 ] || { sed -n '4,18p' "$0" >&2; exit 2; }
dest="$1"
art="$2"
# shellcheck source=common.sh
. "$here/common.sh"

[ -f "$art/BUILDINFO.txt" ] || die "$art/BUILDINFO.txt missing: not a tools/mpv artifact"
grep -qx 'target: windows-x86_64' "$art/BUILDINFO.txt" || die "$art is not the windows-x86_64 build"
grep -q '^mpv licence: LGPL-2.1-or-later (-Dgpl=false)' "$art/BUILDINFO.txt" || die "BUILDINFO: mpv is not LGPL"
grep -q '^ffmpeg licence: LGPL version 2.1 or later' "$art/BUILDINFO.txt" || die "BUILDINFO: FFmpeg is not LGPL-2.1+"
mpv="$art/bin/libmpv-2.dll"
[ -f "$mpv" ] || die "no bin/libmpv-2.dll in $art"
grep -aq -- '-Dgpl=false' "$mpv" || die "libmpv-2.dll is not an -Dgpl=false build"
if grep -aq -- '-Dgpl=true' "$mpv"; then die "libmpv-2.dll contains -Dgpl=true"; fi
[ -f "$art/lib/libmpv.dll.a" ] || die "no lib/libmpv.dll.a in $art"

rm -rf "$dest"
mkdir -p "$dest/libmpv/include" "$dest/libmpv/licenses" "$dest/ANGLE"

# libmpv and exactly the FFmpeg DLLs it imports (not avdevice: mpv is built
# without it), transitively.
cp "$mpv" "$art/lib/libmpv.dll.a" "$dest/libmpv/"
queue=(libmpv-2.dll)
while [ "${#queue[@]}" -gt 0 ]; do
    f="${queue[0]}"; queue=("${queue[@]:1}")
    while IFS=$'\t' read -r _ dep; do
        dep="${dep%$'\r'}"
        case "$dep" in
            av*.dll|sw*.dll|postproc*.dll)
                [ -f "$dest/libmpv/$dep" ] && continue
                [ -f "$art/bin/$dep" ] || die "$f imports $dep, which is not in $art/bin"
                cp "$art/bin/$dep" "$dest/libmpv/"
                queue+=("$dep")
                ;;
        esac
    done < <(pe_imports "$dest/libmpv/$f")
done
for h in client.h render.h render_gl.h stream_cb.h; do
    [ -f "$art/include/mpv/$h" ] || die "no include/mpv/$h in $art"
    cp "$art/include/mpv/$h" "$dest/libmpv/include/"
done
cp "$art"/share/licenses/* "$dest/libmpv/licenses/"
cp "$art/BUILDINFO.txt" "$dest/libmpv/licenses/BUILDINFO.txt"
log "libmpv: $(cd "$dest/libmpv" && ls -- *.dll | tr '\n' ' ')"

# ANGLE.
cache="${PA_ANGLE_CACHE:-${RUNNER_TEMP:-${TMPDIR:-/tmp}}}"
mkdir -p "$cache"
archive="$cache/ANGLE-v1.0.1.7z"
if [ ! -f "$archive" ] || [ "$(sha "$archive")" != "$ANGLE_SHA256" ]; then
    curl -fsSL --retry 3 -o "$archive.part" "$ANGLE_URL"
    [ "$(sha "$archive.part")" = "$ANGLE_SHA256" ] || die "ANGLE.7z: SHA-256 mismatch"
    mv "$archive.part" "$archive"
fi
angle_x="$dest/ANGLE.extract"
rm -rf "$angle_x"; mkdir -p "$angle_x"
(cd "$angle_x" && cmake -E tar xf "$archive")
for f in include/EGL/egl.h include/GLES2/gl2.h lib/libEGL.dll.lib lib/libGLESv2.dll.lib libEGL.dll libGLESv2.dll; do
    [ -f "$angle_x/$f" ] || die "ANGLE archive lacks $f"
done
cp -R "$angle_x/include" "$angle_x/lib" "$dest/ANGLE/"
# libEGL, libGLESv2 and what they import from the archive (libc++, zlib...).
queue=(libEGL.dll libGLESv2.dll)
cp "$angle_x/libEGL.dll" "$angle_x/libGLESv2.dll" "$dest/ANGLE/"
while [ "${#queue[@]}" -gt 0 ]; do
    f="${queue[0]}"; queue=("${queue[@]:1}")
    while IFS=$'\t' read -r _ dep; do
        [ -f "$angle_x/$dep" ] || continue
        [ -f "$dest/ANGLE/$dep" ] && continue
        cp "$angle_x/$dep" "$dest/ANGLE/"
        queue+=("$dep")
    done < <(pe_imports "$dest/ANGLE/$f")
done
rm -rf "$angle_x"
lic_b64="$cache/ANGLE-LICENSE.b64"
curl -fsSL --retry 3 -o "$lic_b64" "$ANGLE_LICENSE_URL"
"$py" -I -c 'import base64, sys; open(sys.argv[2], "wb").write(base64.b64decode(open(sys.argv[1], "rb").read()))' \
    "$lic_b64" "$dest/ANGLE/LICENSE"
[ "$(sha "$dest/ANGLE/LICENSE")" = "$ANGLE_LICENSE_SHA256" ] || die "ANGLE LICENSE: SHA-256 mismatch"
log "ANGLE: $(cd "$dest/ANGLE" && ls -- *.dll | tr '\n' ' ')"
