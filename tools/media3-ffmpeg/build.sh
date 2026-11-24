#!/usr/bin/env bash
# SPDX-License-Identifier: MPL-2.0
#
# Media3 FFmpeg audio extension (PLAY-AND-004) on top of our LGPL FFmpeg.
#
#   build.sh fetch
#       Download the pinned Media3 files (versions.sh) into the cache and
#       verify each SHA-256.
#
#   build.sh <android-target> <ffmpeg-out>
#       Build libffmpegJNI.so for android-arm64-v8a | android-armeabi-v7a |
#       android-x86_64 against the FFmpeg of a tools/ffmpeg or tools/mpv
#       artifact (include/ and lib/libavcodec.so, libswresample.so,
#       libavutil.so) — linked dynamically, the FFmpeg .so files are the ones
#       the APK already ships for libmpv. Writes out/<target>/:
#         jniLibs/<abi>/libffmpegJNI.so, java/ (the extension's audio classes),
#         proguard-rules.txt, share/licenses/, BUILDINFO.txt
#       and runs check-license.sh on it. That directory is what Gradle takes as
#       -PpaFfmpegExtDir (clients/android/player, pa_player_android).
#
#   build.sh merge <dest> <out-target>...
#       One extension directory with every ABI (for an APK with several).
#
# Environment: ANDROID_NDK_HOME (NDK r28c, versions.sh), PA_M3FF_CACHE,
# PA_M3FF_OUT. The build is reproducible: same NDK, same FFmpeg headers →
# the same bytes (no build id, source paths mapped, BUILDINFO has hashes).
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=versions.sh
. "$ROOT/versions.sh"
CACHE="${PA_M3FF_CACHE:-$ROOT/.cache}/media3-$MEDIA3_VERSION"
OUT="${PA_M3FF_OUT:-$ROOT/out}"

log() { printf '\033[1;34m[media3-ffmpeg]\033[0m %s\n' "$*" >&2; }
die() { printf '\033[1;31m[media3-ffmpeg] error:\033[0m %s\n' "$*" >&2; exit 1; }

sha256_of() { sha256sum "$1" | cut -d' ' -f1; }

fetch_all() {
    local sha path file got
    while read -r sha path; do
        [ -n "$sha" ] || continue
        file="$CACHE/$path"
        mkdir -p "$(dirname "$file")"
        if [ ! -f "$file" ]; then
            log "downloading $path"
            curl -fsSL --retry 3 --proto '=https' -o "$file.part" "$MEDIA3_RAW/$path" \
                || die "download failed: $MEDIA3_RAW/$path"
            mv "$file.part" "$file"
        fi
        got="$(sha256_of "$file")"
        if [ "$got" != "$sha" ]; then
            rm -f "$file"
            die "SHA-256 mismatch for $path: expected $sha, got $got"
        fi
    done <<< "$MEDIA3_FILES"
    log "Media3 $MEDIA3_VERSION sources verified in $CACHE"
}

ndk_bin() {
    local ndk="${ANDROID_NDK_HOME:-}"
    [ -d "$ndk" ] || die "ANDROID_NDK_HOME is not set (NDK $ANDROID_NDK_VERSION)"
    local have
    have="$(sed -n 's/^Pkg.Revision *= *//p' "$ndk/source.properties" 2>/dev/null || true)"
    [ "$have" = "$ANDROID_NDK_VERSION" ] || die "NDK $have at $ndk, need $ANDROID_NDK_VERSION (reproducible builds)"
    printf '%s' "$ndk/toolchains/llvm/prebuilt/linux-x86_64/bin"
}

build_target() {
    local target="$1" ff="$2" abi triple extra=()
    case "$target" in
        android-arm64-v8a)   abi=arm64-v8a   triple=aarch64-linux-android; extra=(-Wl,-Bsymbolic) ;;
        android-armeabi-v7a) abi=armeabi-v7a triple=armv7a-linux-androideabi ;;
        android-x86_64)      abi=x86_64      triple=x86_64-linux-android ;;
        *) die "unknown target $target (android-arm64-v8a, android-armeabi-v7a, android-x86_64)" ;;
    esac
    ff="$(cd "$ff" && pwd)"
    local lib
    for lib in avcodec swresample avutil; do
        [ -f "$ff/lib/lib$lib.so" ] || die "$ff/lib/lib$lib.so missing: FFmpeg must be shared (LGPL, tools/ffmpeg)"
        [ -d "$ff/include/lib$lib" ] || die "$ff/include/lib$lib missing"
    done
    # FFmpeg itself must pass the project's licence gate before we link to it.
    "$ROOT/../ffmpeg/check-license.sh" --artifact "$ff" || die "FFmpeg at $ff failed the licence check"

    fetch_all
    local bin dest src
    bin="$(ndk_bin)"
    dest="$OUT/$target"
    src="$CACHE/libraries/decoder_ffmpeg"
    rm -rf "$dest"
    mkdir -p "$dest/jniLibs/$abi" "$dest/share/licenses"

    log "compiling libffmpegJNI.so for $abi against $ff"
    # Flags as upstream's CMakeLists (hidden visibility, -Bsymbolic on arm64), plus: dynamic FFmpeg, static
    # libc++ (no libc++_shared.so in the APK), 16 KB pages (Android 15+), no build id and mapped paths
    # (reproducible), no undefined symbols left.
    "$bin/clang++" --target="${triple}${ANDROID_API}" -std=c++17 -O2 -fPIC -shared \
        -fvisibility=hidden -fvisibility-inlines-hidden \
        -D__STDC_CONSTANT_MACROS \
        -ffile-prefix-map="$src"=media3/decoder_ffmpeg -ffile-prefix-map="$ff"=ffmpeg \
        -I"$ff/include" \
        "$src/src/main/jni/ffmpeg_jni.cc" \
        -L"$ff/lib" -lavcodec -lswresample -lavutil -landroid -llog \
        -static-libstdc++ \
        -Wl,-soname,libffmpegJNI.so -Wl,--build-id=none -Wl,--no-undefined \
        -Wl,-z,max-page-size=16384 -Wl,-z,separate-loadable-segments \
        "${extra[@]}" \
        -o "$dest/jniLibs/$abi/libffmpegJNI.so"
    "$bin/llvm-strip" --strip-unneeded "$dest/jniLibs/$abi/libffmpegJNI.so"

    # The audio classes only (see versions.sh), and the extension's ProGuard rules.
    mkdir -p "$dest/java/androidx/media3/decoder/ffmpeg"
    cp "$src"/src/main/java/androidx/media3/decoder/ffmpeg/*.java "$dest/java/androidx/media3/decoder/ffmpeg/"
    cp "$src/proguard-rules.txt" "$dest/proguard-rules.txt"
    cp "$CACHE/LICENSE" "$dest/share/licenses/media3-LICENSE.txt"
    [ -d "$ff/share/licenses" ] && cp "$ff"/share/licenses/* "$dest/share/licenses/" 2>/dev/null || true

    {
        echo "Kinolune Media3 FFmpeg audio extension (tools/media3-ffmpeg, PLAY-AND-004)"
        echo "target: $target"
        echo "media3: $MEDIA3_VERSION ($MEDIA3_LICENSE) commit $MEDIA3_COMMIT"
        echo "ndk: $ANDROID_NDK_VERSION, api $ANDROID_API"
        echo "ffmpeg: linked dynamically (libavcodec.so, libswresample.so, libavutil.so)"
        grep -E '^(ffmpeg licence:| *ffmpeg [0-9])' "$ff/BUILDINFO.txt" 2>/dev/null || true
        echo "sources:"
        printf '%s\n' "$MEDIA3_FILES" | sed '/^$/d; s/^/  /'
        echo "output:"
        echo "  $(sha256_of "$dest/jniLibs/$abi/libffmpegJNI.so")  jniLibs/$abi/libffmpegJNI.so"
    } > "$dest/BUILDINFO.txt"

    "$ROOT/check-license.sh" "$dest" "$ff" || die "licence check failed for $dest"
    log "done: $dest"
}

merge() {
    local dest="$1"; shift
    [ $# -gt 0 ] || die "merge <dest> <out-target>..."
    rm -rf "$dest"
    mkdir -p "$dest"
    local t first=1
    for t in "$@"; do
        [ -f "$t/BUILDINFO.txt" ] || die "$t is not a built extension (no BUILDINFO.txt)"
        if [ "$first" = 1 ]; then
            cp -R "$t/java" "$t/proguard-rules.txt" "$t/share" "$dest/"
            first=0
        fi
        mkdir -p "$dest/jniLibs"
        cp -R "$t"/jniLibs/* "$dest/jniLibs/"
        cat "$t/BUILDINFO.txt" >> "$dest/BUILDINFO.txt"
        echo >> "$dest/BUILDINFO.txt"
    done
    log "merged: $dest ($(ls "$dest/jniLibs" | tr '\n' ' '))"
}

case "${1:-}" in
    fetch) fetch_all ;;
    merge) shift; merge "$@" ;;
    android-*) [ $# -eq 2 ] || die "usage: build.sh <android-target> <ffmpeg-out>"; build_target "$1" "$2" ;;
    *) sed -n '4,27p' "$0"; exit 2 ;;
esac
