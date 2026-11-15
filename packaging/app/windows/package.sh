#!/usr/bin/env bash
# SPDX-License-Identifier: MPL-2.0
#
# Package a Flutter Windows release build as a portable zip (PKG-020, ADR-NNNN).
#
#   package.sh <build/windows/x64/runner/Release> <version> <out-dir> [<libsmb2-windows-x86_64>]
#
# Writes <out-dir>/kinolune-<version>-windows-x64.zip with one folder
# kinolune-<version>-windows-x64/: Kinolune.exe, the Flutter engine and plugin
# DLLs, data/, the Rust core (pa_frb.dll), our LGPL libmpv + FFmpeg and ANGLE
# (copied there by media_kit_libs_windows_video, packaging/app/windows/mpv-libs.sh),
# libsmb2.dll (tools/libsmb2, LIB-019) when given, the Visual C++ runtime
# DLLs the binaries import (app-local, from PA_VCREDIST_DIR), README-windows.txt
# and licenses/. Not signed (PKG-005).
# Gates: libmpv-2.dll is the one mpv-libs.sh prepared and -Dgpl=false; FFmpeg
# DLLs carry no --enable-gpl/--enable-nonfree; no GPL/nonfree library is
# imported or shipped; the core is there; on Windows, every DLL import
# resolves inside the folder or to a system DLL.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd "$here/../../.." && pwd)"
libs="${PA_WIN_LIBS:-$repo_root/clients/flutter/packages/media_kit_libs_windows_video/windows/libs}"

die() { printf 'package.sh: %s\n' "$*" >&2; exit 1; }
log() { printf 'package.sh: %s\n' "$*" >&2; }

[ $# -ge 3 ] && [ $# -le 4 ] || { sed -n '4,18p' "$0" >&2; exit 2; }
src="$(cd "$1" && pwd)"
version="$2"
mkdir -p "$3"
out="$(cd "$3" && pwd)"
smb2="${4:-}"
# shellcheck source=common.sh
. "$here/common.sh"

[[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.]+)?$ ]] || die "bad version: $version"
[ -f "$src/Kinolune.exe" ] || die "$src/Kinolune.exe missing: not a Flutter Windows release build"
[ -d "$src/data/flutter_assets" ] || die "$src/data/flutter_assets missing"

name="kinolune-$version-windows-x64"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
app="$work/$name"
cp -R "$src" "$app"
lic="$app/licenses"
mkdir -p "$lic"

# libmpv: copied next to the exe by media_kit_libs_windows_video; must be the
# file mpv-libs.sh checked, LGPL, with its FFmpeg.
mpv="$app/libmpv-2.dll"
[ -f "$mpv" ] || die "libmpv-2.dll missing: was packaging/app/windows/mpv-libs.sh run before the build?"
[ -f "$libs/libmpv/libmpv-2.dll" ] || die "no $libs/libmpv/libmpv-2.dll"
[ "$(sha "$mpv")" = "$(sha "$libs/libmpv/libmpv-2.dll")" ] || die "libmpv-2.dll in the build is not the one from $libs"
grep -aq -- '-Dgpl=false' "$mpv" || die "libmpv-2.dll is not an -Dgpl=false build"
if grep -aq -- '-Dgpl=true' "$mpv"; then die "libmpv-2.dll contains -Dgpl=true"; fi
for f in "$libs"/libmpv/*.dll; do
    [ -f "$app/$(basename "$f")" ] || die "$(basename "$f") (libmpv set) missing from the build"
done
for f in "$app"/av*.dll "$app"/sw*.dll; do
    [ -f "$f" ] || continue
    if grep -aq -e '--enable-gpl' -e '--enable-nonfree' -e '--enable-version3' "$f"; then
        die "$(basename "$f") was configured with --enable-gpl/nonfree/version3"
    fi
done
mkdir -p "$lic/libmpv"
cp "$libs"/libmpv/licenses/* "$lic/libmpv/"
for f in libEGL.dll libGLESv2.dll; do [ -f "$app/$f" ] || die "ANGLE $f missing from the build"; done
[ -f "$libs/ANGLE/LICENSE" ] || die "no $libs/ANGLE/LICENSE"
mkdir -p "$lic/ANGLE"
cp "$libs/ANGLE/LICENSE" "$lic/ANGLE/LICENSE"
cp "$here/ANGLE-NOTICE.md" "$lic/ANGLE/"

# libsmb2 (optional: without it SMB sources report the library as missing).
if [ -n "$smb2" ]; then
    [ -f "$smb2/bin/libsmb2.dll" ] || die "no bin/libsmb2.dll in $smb2"
    cp "$smb2/bin/libsmb2.dll" "$app/"
    mkdir -p "$lic/libsmb2"
    cp "$smb2"/share/licenses/* "$lic/libsmb2/"
    [ -f "$smb2/BUILDINFO.txt" ] && cp "$smb2/BUILDINFO.txt" "$lic/libsmb2/BUILDINFO.txt"
    log "libsmb2.dll"
else
    log "no libsmb2 given: SMB sources will not work in this build"
fi

# The library runs on the Rust core (UI-024), a Native Asset next to the exe.
[ -f "$app/pa_frb.dll" ] || die "the Rust core (pa_frb.dll) is not next to Kinolune.exe"

cp "$repo_root/LICENSE" "$repo_root/packaging/app/THIRD-PARTY-NOTICES.md" "$lic/"
cp "$here/README-windows.txt" "$app/"

# Imports of every PE file in the folder.
pes=()
while IFS= read -r -d '' f; do pes+=("$f"); done < <(find "$app" -type f \( -iname '*.dll' -o -iname '*.exe' \) -print0)
pe_imports "${pes[@]}" > "$work/imports.tsv"

# Visual C++ runtime, app-local: Windows without the VC++ Redistributable has
# no msvcp140/vcruntime140 (Flutter engine, plugins and the core need them).
vcrt="$(cut -f2 "$work/imports.tsv" | tr 'A-Z' 'a-z' | grep -E '^(msvcp140[a-z0-9_]*|vcruntime140[a-z0-9_]*|concrt140)\.dll$' | sort -u || true)"
for dll in $vcrt; do
    [ -n "$(find "$app" -maxdepth 1 -iname "$dll")" ] && continue
    [ -n "${PA_VCREDIST_DIR:-}" ] || die "$dll is imported; set PA_VCREDIST_DIR to the VC++ redist CRT directory"
    f="$(find "$PA_VCREDIST_DIR" -maxdepth 1 -iname "$dll" | head -n1)"
    [ -n "$f" ] || die "$dll not in PA_VCREDIST_DIR=$PA_VCREDIST_DIR"
    cp "$f" "$app/"
    log "VC++ runtime: $dll"
done

# No GPL or nonfree library may be imported or shipped.
forbidden='(rubberband|cdio|dvdnav|dvdread|jack|caca|x264|x265|xvidcore|fdk-aac|vidstab|postproc)'
if cut -f2 "$work/imports.tsv" | grep -Eiq "$forbidden"; then
    die "a binary imports a GPL/nonfree library: $(cut -f2 "$work/imports.tsv" | grep -Ei "$forbidden" | sort -u | tr '\n' ' ')"
fi
if find "$app" -type f | grep -Eiq "/[^/]*$forbidden[^/]*\.dll$"; then
    die "a GPL/nonfree library is in the folder"
fi

# On Windows: every (non-delay-load) import resolves to the folder or to a
# system DLL. Exception: dartjni.dll (package:jni, pulled in by Android
# implementations of plugins) imports jvm.dll; its /DELAYLOAD flag is set on
# the wrong CMake target upstream (jni 1.1.2). Nothing loads dartjni.dll on
# Windows, so its jvm.dll import is not required.
case "$(uname -s)" in
    MINGW* | MSYS* | CYGWIN*)
        sys32="$(cygpath -u "${SYSTEMROOT:-C:\\Windows}")/System32"
        missing=""
        while IFS= read -r dll; do
            case "$(printf '%s' "$dll" | tr 'A-Z' 'a-z')" in api-ms-win-* | ext-ms-*) continue ;; esac
            [ -n "$(find "$app" -maxdepth 1 -iname "$dll")" ] && continue
            [ -f "$sys32/$dll" ] && continue
            missing="$missing $dll"
        done < <(awk -F'\t' '$3 == "import" && !(tolower($2) == "jvm.dll" && tolower($1) ~ /dartjni\.dll$/) { print $2 }' \
            "$work/imports.tsv" | sort -uf)
        [ -z "$missing" ] || die "unresolved DLL imports:$missing"
        ;;
esac

zip="$out/$name.zip"
rm -f "$zip"
"$py" -I - "$work" "$name" "$zip" <<'PY'
import os, sys, zipfile
root, name, dest = sys.argv[1:4]
with zipfile.ZipFile(dest, "w", zipfile.ZIP_DEFLATED, compresslevel=9) as z:
    for d, dirs, files in os.walk(os.path.join(root, name)):
        dirs.sort()
        for f in sorted(files):
            p = os.path.join(d, f)
            z.write(p, os.path.relpath(p, root).replace(os.sep, "/"))
PY
log "$(du -h "$zip" | cut -f1) $zip"
