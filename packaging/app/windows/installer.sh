#!/usr/bin/env bash
# SPDX-License-Identifier: MPL-2.0
#
# Build the per-user Windows installer from the portable zip (PKG-022, ADR-NNNN).
#
#   installer.sh <kinolune-<version>-windows-x64.zip> <version> <out-dir>
#
# Writes <out-dir>/kinolune-<version>-windows-x64-setup.exe with Inno Setup 6
# (packaging/app/windows/kinolune.iss). The installer carries exactly the
# files of the zip (package.sh checked them: LGPL libmpv/FFmpeg, licences, no
# GPL). ISCC is $ISCC, else Inno Setup's default per-user location. Windows
# (Git Bash) only; not code-signed (PKG-005).
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd "$here/../../.." && pwd)"

die() { printf 'installer.sh: %s\n' "$*" >&2; exit 1; }

[ $# -eq 3 ] || { sed -n '4,12p' "$0" >&2; exit 2; }
zip="$1"
version="$2"
mkdir -p "$3"
out="$(cd "$3" && pwd)"
# shellcheck source=common.sh
. "$here/common.sh"

[[ "$version" =~ ^([0-9]+)\.([0-9]+)\.([0-9]+)(-([0-9A-Za-z.]+))?$ ]] || die "bad version: $version"
# VERSIONINFO takes four numbers: a.b.c plus the last numeric pre-release
# identifier (alpha.12 -> 12), else 0.
build=0
if [ -n "${BASH_REMATCH[5]}" ] && [[ "${BASH_REMATCH[5]##*.}" =~ ^[0-9]+$ ]]; then
    build="${BASH_REMATCH[5]##*.}"
fi
numbers="${BASH_REMATCH[1]}.${BASH_REMATCH[2]}.${BASH_REMATCH[3]}.$build"
[ -f "$zip" ] || die "no $zip"

iscc="${ISCC:-}"
if [ -z "$iscc" ] && [ -n "${LOCALAPPDATA:-}" ]; then
    iscc="$(cygpath -u "$LOCALAPPDATA")/Programs/Inno Setup 6/ISCC.exe"
fi
[ -n "$iscc" ] && [ -f "$iscc" ] || die "ISCC.exe not found (set ISCC)"

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
"$py" -I -c 'import sys, zipfile; zipfile.ZipFile(sys.argv[1]).extractall(sys.argv[2])' "$zip" "$work"
name="kinolune-$version-windows-x64"
src="$work/$name"
[ -f "$src/Kinolune.exe" ] || die "$name/Kinolune.exe is not in $zip"
[ -d "$src/licenses" ] || die "$name/licenses is not in $zip"

icon="$repo_root/clients/flutter/app/windows/runner/resources/app_icon.ico"
args=(/Qp "/DAppVersion=$version" "/DAppVersionNumbers=$numbers"
    "/DSourceDir=$(cygpath -w "$src")" "/DOutputDir=$(cygpath -w "$out")")
[ -f "$icon" ] && args+=("/DSetupIcon=$(cygpath -w "$icon")")
# Git Bash would turn "/DAppVersion=…" into a path: no conversion for ISCC.
MSYS_NO_PATHCONV=1 MSYS2_ARG_CONV_EXCL='*' "$iscc" "${args[@]}" "$(cygpath -w "$here/kinolune.iss")"

setup="$out/$name-setup.exe"
[ -f "$setup" ] || die "ISCC did not write $setup"
printf 'installer.sh: %s %s\n' "$(du -h "$setup" | cut -f1)" "$setup" >&2
