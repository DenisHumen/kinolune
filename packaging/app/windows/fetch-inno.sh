#!/usr/bin/env bash
# SPDX-License-Identifier: MPL-2.0
#
# Fetch and install Inno Setup for the installer build (PKG-022, ADR-NNNN).
#
#   fetch-inno.sh <dir>     # prints the path of ISCC.exe
#
# Downloads the pinned Inno Setup release from its GitHub releases, checks
# the pinned SHA-256 and installs it silently, per user, into <dir>.
# Inno Setup is free software under its own licence
# (https://jrsoftware.org/files/is/license.txt); only its compiler runs in
# CI, the installers it writes carry its setup runtime. Windows (Git Bash)
# only.
set -euo pipefail

INNO_VERSION=6.7.4
INNO_SHA256=a549929319f9120716677c685535ca4671e4b937b5fdc87e2585f32d8eb30e8c
INNO_URL="https://github.com/jrsoftware/issrc/releases/download/is-${INNO_VERSION//./_}/innosetup-$INNO_VERSION.exe"

die() { printf 'fetch-inno.sh: %s\n' "$*" >&2; exit 1; }

[ $# -eq 1 ] || { sed -n '4,13p' "$0" >&2; exit 2; }
mkdir -p "$1"
dir="$(cd "$1" && pwd)"
setup="$dir/innosetup-$INNO_VERSION.exe"

curl -fsSL --retry 3 -o "$setup" "$INNO_URL"
printf '%s  %s\n' "$INNO_SHA256" "$setup" | sha256sum -c - >&2 || die "SHA-256 of $INNO_URL does not match"
# Git Bash would turn "/DIR=…" into a path: no conversion for the setup.
MSYS_NO_PATHCONV=1 MSYS2_ARG_CONV_EXCL='*' "$setup" /VERYSILENT /SUPPRESSMSGBOXES /NORESTART /SP- \
    /CURRENTUSER "/DIR=$(cygpath -w "$dir/inno")" "/LOG=$(cygpath -w "$dir/innosetup-install.log")"
iscc="$dir/inno/ISCC.exe"
for _ in $(seq 30); do [ -f "$iscc" ] && break; sleep 2; done
[ -f "$iscc" ] || die "ISCC.exe not installed into $dir/inno"
[ -f "$dir/inno/Languages/Russian.isl" ] || die "Russian.isl missing from Inno Setup"
printf '%s\n' "$iscc"
