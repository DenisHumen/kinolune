#!/usr/bin/env bash
# SPDX-License-Identifier: MPL-2.0
#
# Content-addressed prebuilt FFmpeg and libmpv for CI (CI-008, ADR-0119).
#
# A build is named by what goes into it: the recipe files (scripts, pinned
# versions, patches, Docker images) and, for targets built on the runner, the
# toolchain (NDK, Xcode, meson, ninja). The same name means the same inputs,
# so CI reuses the build instead of rebuilding it:
#
#   1. actions/cache (fast, but scoped to a branch and evicted at 10 GB);
#   2. the release `prebuilt` of this repository (public, never evicted,
#      written only by prebuilt.yml on main);
#   3. a fresh build.
#
# The licence gate runs on the result in every case.
#
# Usage:
#   prebuilt.sh recipe <lib>              16-hex hash of the recipe files
#   prebuilt.sh key <lib> <target>        <lib>-<target>-<recipe>[-<toolchain>]
#   prebuilt.sh fetch <key> <dir>         download <key>.tar.xz into <dir>;
#                                         exit 0 found, 3 not published
#   prebuilt.sh pack <key> <dir> <outdir> write <outdir>/<key>.tar.xz from <dir>
#
# <lib> is ffmpeg or libmpv. Environment:
#   PA_PREBUILT_URL      where `fetch` looks (default: the release `prebuilt`
#                        of $GITHUB_REPOSITORY)
#   ANDROID_NDK_PACKAGE  MESON_VERSION  NINJA_VERSION   toolchain of android-*
#                        and Apple targets (the workflows' env)
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
export LC_ALL=C

die() { echo "prebuilt: $*" >&2; exit 1; }

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

sha256_stdin() {
    if command -v sha256sum >/dev/null 2>&1; then sha256sum | cut -d' ' -f1
    else shasum -a 256 | cut -d' ' -f1; fi   # macOS
}

# Every file that defines what gets built. Keep in sync with the build scripts.
recipe_paths() {
    case "$1" in
        ffmpeg) echo "tools/ffmpeg/*.sh tools/ffmpeg/*.c tools/ffmpeg/lib tools/ffmpeg/patches tools/ffmpeg/docker" ;;
        libmpv) echo "tools/ffmpeg/versions.sh tools/ffmpeg/lib tools/ffmpeg/patches tools/ffmpeg/check-license.sh" \
                     "tools/mpv/*.sh tools/mpv/*.c tools/mpv/lib tools/mpv/patches tools/mpv/docker" ;;
        *) die "unknown lib '$1' (ffmpeg, libmpv)" ;;
    esac
}

recipe_hash() {
    local paths file
    paths="$(recipe_paths "$1")"
    # Paths relative to the repository root, sorted bytewise: the same hash on
    # Linux and macOS runners and on any checkout location.
    # shellcheck disable=SC2086 # the globs expand here on purpose
    (cd "$ROOT" && find $paths -type f | sort | while IFS= read -r file; do
        printf '%s  %s\n' "$(sha256_stdin < "$file")" "$file"
    done) | sha256_stdin | cut -c1-16
}

# The toolchain outside the recipe; Docker targets pin theirs in the recipe.
toolchain_id() {
    case "$1" in
        linux-*|windows-*) ;;
        android-*)
            : "${ANDROID_NDK_PACKAGE:?}" "${MESON_VERSION:?}" "${NINJA_VERSION:?}"
            echo "$ANDROID_NDK_PACKAGE meson=$MESON_VERSION ninja=$NINJA_VERSION" ;;
        macos-*|ios-*|tvos-*)
            : "${MESON_VERSION:?}" "${NINJA_VERSION:?}"
            echo "$(xcodebuild -version | tr -s ' \n' ' ') meson=$MESON_VERSION ninja=$NINJA_VERSION" ;;
        *) die "unknown target '$1'" ;;
    esac
}

key() {
    local lib="$1" target="$2" tc k
    [[ "$target" =~ ^[a-z0-9_-]+$ ]] || die "bad target '$target'"
    k="$lib-$target-$(recipe_hash "$lib")"
    tc="$(toolchain_id "$target")"
    [ -z "$tc" ] || k="$k-$(printf '%s' "$tc" | sha256_stdin | cut -c1-8)"
    echo "$k"
}

check_key() { [[ "$1" =~ ^(ffmpeg|libmpv)-[a-z0-9_-]+$ ]] || die "bad key '$1'"; }

base_url() {
    if [ -n "${PA_PREBUILT_URL:-}" ]; then echo "$PA_PREBUILT_URL"
    else echo "https://github.com/${GITHUB_REPOSITORY:?}/releases/download/prebuilt"; fi
}

fetch() {
    local k="$1" dir="$2" tmp="$TMP" code
    check_key "$k"
    # Anonymous download of a public asset: no token, works for fork PRs.
    code="$(curl -sSL --retry 3 --retry-all-errors -o "$tmp/$k.tar.xz" -w '%{http_code}' "$(base_url)/$k.tar.xz")" \
        || die "download of $k failed"
    case "$code" in
        200) ;;
        404) echo "prebuilt: $k is not published"; return 3 ;;
        *) die "download of $k: HTTP $code" ;;
    esac
    # The tarball holds exactly the directory <key>/ (see pack).
    mkdir "$tmp/x"
    tar -C "$tmp/x" -xJf "$tmp/$k.tar.xz"
    [ "$(ls -A "$tmp/x")" = "$k" ] && [ -d "$tmp/x/$k" ] || die "$k.tar.xz does not hold $k/"
    rm -rf "$dir"; mkdir -p "$(dirname "$dir")"
    mv "$tmp/x/$k" "$dir"
    echo "prebuilt: $k from $(base_url)"
}

pack() {
    local k="$1" dir="$2" out="$3" stage="$TMP"
    check_key "$k"
    [ -d "$dir" ] || die "no directory $dir"
    mkdir -p "$out"; out="$(cd "$out" && pwd)"
    cp -R "$dir" "$stage/$k"
    XZ_OPT="${XZ_OPT:--T0}" tar -C "$stage" -cJf "$out/$k.tar.xz" "$k"
    echo "prebuilt: wrote $out/$k.tar.xz"
}

case "${1:-}" in
    recipe) [ $# = 2 ] || die "usage: recipe <lib>"; recipe_hash "$2" ;;
    key) [ $# = 3 ] || die "usage: key <lib> <target>"; recipe_paths "$2" >/dev/null; key "$2" "$3" ;;
    fetch) [ $# = 3 ] || die "usage: fetch <key> <dir>"; fetch "$2" "$3" ;;
    pack) [ $# = 4 ] || die "usage: pack <key> <dir> <outdir>"; pack "$2" "$3" "$4" ;;
    *) sed -n '4,29p' "$0"; exit 2 ;;
esac
