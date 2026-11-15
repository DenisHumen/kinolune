#!/usr/bin/env bash
# SPDX-License-Identifier: MPL-2.0
#
# Put a libmpv artifact (tools/mpv/out/<target>, or the CI artifact
# libmpv-<target>) where media_kit loads it from (PLAY-002). Packaging scripts
# call this as a hook; it never rebuilds anything.
#
#   bundle.sh linux <libmpv-out> <flutter-linux-bundle>
#       Copy libmpv.so.2 and the FFmpeg libraries into <bundle>/lib, the licence
#       texts and BUILDINFO into <bundle>/licenses/libmpv, and make the
#       media_kit_video plugin find libmpv next to itself (RUNPATH=$ORIGIN,
#       needs patchelf). The app must then open <bundle>/lib/libmpv.so.2:
#       MediaKit.ensureInitialized(libmpv: ...) or LIBMPV_LIBRARY_PATH.
#
#   bundle.sh android <jniLibs-dir> <libmpv-out>...
#       Copy lib/*.so of each android-<abi> artifact into <jniLibs-dir>/<abi>/
#       (libmpv.so, libav*.so, libsw*.so, libmediakitandroidhelper.so), and the
#       licences into <jniLibs-dir>/../assets/licenses/libmpv/ is left to the caller.
#
#   bundle.sh check-android <libmpv-out>
#       Packaging gate for an Android artifact: unversioned SONAMEs equal to the
#       file names, every NEEDED library either in the set or an NDK system
#       library, 16 KB-aligned LOAD segments (Android 15+). Needs readelf.
#
#   bundle.sh macos <frameworks-dir> <libmpv-macos-arm64> [<libmpv-macos-x86_64>]
#       Wrap libmpv.2.dylib and the FFmpeg dylibs it links into macOS
#       frameworks (Mpv.framework, Avcodec.framework, ...) in <frameworks-dir>,
#       universal (lipo) when both artifacts are given (PKG-019). Install names
#       become @rpath/<Name>.framework/Versions/A/<Name>, each binary gets the
#       rpath @loader_path/../../.. (the app's Contents/Frameworks), licences and
#       BUILDINFO go into Mpv.framework/Resources/licenses, and
#       <frameworks-dir>/.symlinks/mpv/macos points back at <frameworks-dir>
#       (media_kit_video links -framework Mpv from there). Ad-hoc signed.
#       Needs macOS with Xcode (lipo, install_name_tool, otool, codesign).
set -euo pipefail

die() { printf 'bundle.sh: error: %s\n' "$*" >&2; exit 1; }
log() { printf 'bundle.sh: %s\n' "$*" >&2; }

target_of() { sed -n 's/^target: //p' "$1/BUILDINFO.txt" 2>/dev/null; }

# Libraries an Android app may link from the platform (NDK stable APIs).
ANDROID_SYSTEM_LIBS_RE='^lib(c|m|dl|log|android|EGL|GLESv[123]|mediandk|aaudio|OpenSLES|z|jnigraphics|vulkan|nativewindow|camera2ndk|sync)\.so$'

bundle_linux() {
    local out="$1" bundle="$2"
    [ "$(target_of "$out")" = linux-x86_64 ] || die "$out is not a linux-x86_64 libmpv artifact"
    [ -d "$bundle/lib" ] && [ -d "$bundle/data" ] || die "$bundle is not a Flutter Linux bundle"
    command -v patchelf >/dev/null || die "patchelf is required (apt install patchelf)"
    local f name soname
    # Real files, installed under their SONAME (what the dynamic linker looks for).
    for f in "$out"/lib/lib*.so.*; do
        [ -f "$f" ] && [ ! -L "$f" ] || continue
        soname="$(patchelf --print-soname "$f")"
        name="${soname:-$(basename "$f")}"
        install -m 0644 "$f" "$bundle/lib/$name"
        # FFmpeg libraries need each other; resolve them next to themselves too
        # (libmpv.so.2 already has RUNPATH=$ORIGIN from its build).
        case "$name" in libmpv.so*) ;; *) patchelf --set-rpath '$ORIGIN' "$bundle/lib/$name" ;; esac
        log "lib/$name"
    done
    [ -f "$bundle/lib/libmpv.so.2" ] || die "no libmpv.so.2 in $out/lib"
    # The plugin links libmpv.so.2; let it find our copy in the same directory
    # instead of a system libmpv.
    for f in "$bundle"/lib/libmedia_kit_video_plugin.so; do
        [ -f "$f" ] || continue
        patchelf --set-rpath '$ORIGIN' "$f"
        log "RUNPATH=\$ORIGIN on $(basename "$f")"
    done
    mkdir -p "$bundle/licenses/libmpv"
    cp "$out"/share/licenses/* "$bundle/licenses/libmpv/"
    cp "$out/BUILDINFO.txt" "$bundle/licenses/libmpv/BUILDINFO.txt"
    log "done; have the app load lib/libmpv.so.2 (MediaKit.ensureInitialized(libmpv: ...) or LIBMPV_LIBRARY_PATH)"
}

bundle_android() {
    local jni="$1"; shift
    local out t abi f
    for out in "$@"; do
        t="$(target_of "$out")"
        case "$t" in android-*) abi="${t#android-}" ;; *) die "$out is not an android libmpv artifact" ;; esac
        check_android "$out"
        mkdir -p "$jni/$abi"
        for f in "$out"/lib/*.so; do
            install -m 0644 "$f" "$jni/$abi/$(basename "$f")"
        done
        log "$abi: $(cd "$out/lib" && ls -- *.so | tr '\n' ' ')"
    done
}

check_android() {
    local out="$1" f name soname need fail=0 set=" "
    command -v readelf >/dev/null || die "readelf is required (binutils or llvm-readelf)"
    for f in "$out"/lib/*.so; do set="$set$(basename "$f") "; done
    [ "$set" != " " ] || die "no .so files in $out/lib"
    case "$set" in *" libmpv.so "*) ;; *) echo "FAIL: libmpv.so missing" >&2; fail=1 ;; esac
    case "$set" in *" libavcodec.so "*) ;; *) echo "FAIL: libavcodec.so missing" >&2; fail=1 ;; esac
    case "$set" in *" libmediakitandroidhelper.so "*) ;; *) echo "FAIL: libmediakitandroidhelper.so missing" >&2; fail=1 ;; esac
    for f in "$out"/lib/*.so; do
        name="$(basename "$f")"
        [ -L "$f" ] && { echo "FAIL: $name is a symlink (APKs do not keep symlinks)" >&2; fail=1; continue; }
        soname="$(readelf -d "$f" | sed -n 's/.*(SONAME).*\[\(.*\)\]/\1/p')"
        if [ "$soname" != "$name" ]; then echo "FAIL: $name has SONAME '$soname'" >&2; fail=1; fi
        while IFS= read -r need; do
            case "$set" in *" $need "*) continue ;; esac
            if ! printf '%s\n' "$need" | grep -Eq "$ANDROID_SYSTEM_LIBS_RE"; then
                echo "FAIL: $name needs $need, which is neither bundled nor an NDK system library" >&2; fail=1
            fi
        done < <(readelf -d "$f" | sed -n 's/.*(NEEDED).*\[\(.*\)\]/\1/p')
        # Every PT_LOAD alignment must be >= 16 KB (0x4000).
        local align
        for align in $(readelf -lW "$f" | awk '$1 == "LOAD" { print $NF }'); do
            if [ "$((align))" -lt 16384 ]; then
                echo "FAIL: $name has a LOAD segment aligned to $align (< 16 KB)" >&2; fail=1; break
            fi
        done
        echo "ok: $name (SONAME $soname)" >&2
    done
    [ "$fail" = 0 ] || die "android packaging check failed for $out"
}

# libavcodec.63.dylib -> Avcodec, libmpv.2.dylib -> Mpv.
macos_framework_name() {
    local stem="${1#lib}"
    stem="${stem%%.*}"
    printf '%s%s\n' "$(printf '%s' "${stem:0:1}" | tr '[:lower:]' '[:upper:]')" "${stem:1}"
}

# @rpath/<lib>.dylib dependencies of a Mach-O file, one file name per line.
macos_rpath_deps() {
    otool -L "$1" | awk 'NR > 1 && $1 ~ /^@rpath\// { sub("^@rpath/", "", $1); print $1 }'
}

macos_info_plist() {
    local name="$1" id
    id="$(printf '%s' "$name" | tr '[:upper:]' '[:lower:]')"
    cat <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>CFBundleDevelopmentRegion</key><string>en</string>
	<key>CFBundleExecutable</key><string>$name</string>
	<key>CFBundleIdentifier</key><string>app.kinolune.lib.$id</string>
	<key>CFBundleInfoDictionaryVersion</key><string>6.0</string>
	<key>CFBundleName</key><string>$name</string>
	<key>CFBundlePackageType</key><string>FMWK</string>
	<key>CFBundleShortVersionString</key><string>1.0</string>
	<key>CFBundleVersion</key><string>1</string>
</dict>
</plist>
PLIST
}

bundle_macos() {
    local dir="$1"; shift
    [ "$(uname -s)" = Darwin ] || die "bundle.sh macos needs macOS with Xcode"
    local t tool out arm="" x86=""
    for tool in lipo install_name_tool otool codesign; do
        command -v "$tool" >/dev/null || die "$tool is required (Xcode command line tools)"
    done
    for out in "$@"; do
        t="$(target_of "$out")"
        case "$t" in
            macos-arm64) arm="$out" ;;
            macos-x86_64) x86="$out" ;;
            *) die "$out is not a macos libmpv artifact (target: ${t:-none})" ;;
        esac
    done
    local first="${arm:-$x86}"
    [ -f "$first/lib/libmpv.2.dylib" ] || die "no lib/libmpv.2.dylib in $first"
    mkdir -p "$dir"
    rm -rf "$dir"/*.framework "$dir/.symlinks"

    # libmpv and everything it pulls in through @rpath, breadth first.
    local queue="libmpv.2.dylib" seen=" " lib name fw bin dep depname src
    while [ -n "$queue" ]; do
        lib="${queue%% *}"
        queue="${queue#"$lib"}"; queue="${queue# }"
        case "$seen" in *" $lib "*) continue ;; esac
        seen="$seen$lib "
        name="$(macos_framework_name "$lib")"
        fw="$dir/$name.framework"
        mkdir -p "$fw/Versions/A/Resources"
        bin="$fw/Versions/A/$name"
        local slices=()
        for src in "$arm" "$x86"; do
            [ -n "$src" ] || continue
            [ -f "$src/lib/$lib" ] || die "no lib/$lib in $src"
            slices+=("$src/lib/$lib")
        done
        if [ "${#slices[@]}" -gt 1 ]; then
            lipo -create "${slices[@]}" -output "$bin"
        else
            cp "${slices[0]}" "$bin"
        fi
        chmod 0755 "$bin"
        local args=(-id "@rpath/$name.framework/Versions/A/$name")
        while IFS= read -r dep; do
            [ "$dep" = "$lib" ] && continue
            depname="$(macos_framework_name "$dep")"
            args+=(-change "@rpath/$dep" "@rpath/$depname.framework/Versions/A/$depname")
            queue="$queue $dep"; queue="${queue# }"
        done < <(macos_rpath_deps "$bin")
        install_name_tool "${args[@]}" "$bin" 2>/dev/null
        if ! otool -l "$bin" | grep -qF '@loader_path/../../..'; then
            install_name_tool -add_rpath '@loader_path/../../..' "$bin" 2>/dev/null
        fi
        macos_info_plist "$name" > "$fw/Versions/A/Resources/Info.plist"
        ln -sfn A "$fw/Versions/Current"
        ln -sfn "Versions/Current/$name" "$fw/$name"
        ln -sfn Versions/Current/Resources "$fw/Resources"
        log "$name.framework <- $lib ($(lipo -archs "$bin"))"
    done
    if otool -L "$dir/Mpv.framework/Mpv" | grep -q '@rpath/lib'; then
        die "Mpv.framework still links a bare @rpath dylib"
    fi

    # Licences and build records of the shipped libmpv/FFmpeg.
    local lic="$dir/Mpv.framework/Versions/A/Resources/licenses"
    mkdir -p "$lic"
    cp "$first"/share/licenses/* "$lic/"
    for src in "$arm" "$x86"; do
        [ -n "$src" ] || continue
        cp "$src/BUILDINFO.txt" "$lic/BUILDINFO-$(target_of "$src").txt"
    done

    # install_name_tool voids signatures and arm64 macOS refuses unsigned
    # code: sign ad-hoc, like the app itself.
    for fw in "$dir"/*.framework; do
        codesign --force --sign - --timestamp=none "$fw" >/dev/null 2>&1 || die "codesign failed for $fw"
    done

    mkdir -p "$dir/.symlinks/mpv"
    ln -sfn ../.. "$dir/.symlinks/mpv/macos"
    [ -f "$dir/.symlinks/mpv/macos/Mpv.framework/Mpv" ] || die "symlink $dir/.symlinks/mpv/macos is broken"
    log "done: $(cd "$dir" && ls -d -- *.framework | tr '\n' ' ')"
}

case "${1:-}" in
    macos)
        [ $# -ge 3 ] && [ $# -le 4 ] \
            || die "usage: bundle.sh macos <frameworks-dir> <libmpv-macos-arm64> [<libmpv-macos-x86_64>]"
        shift; bundle_macos "$@" ;;
    linux) [ $# -eq 3 ] || die "usage: bundle.sh linux <libmpv-out> <flutter-linux-bundle>"; bundle_linux "$2" "$3" ;;
    android) [ $# -ge 3 ] || die "usage: bundle.sh android <jniLibs-dir> <libmpv-out>..."; shift; bundle_android "$@" ;;
    check-android) [ $# -eq 2 ] || die "usage: bundle.sh check-android <libmpv-out>"; check_android "$2" ;;
    *) sed -n '4,34p' "$0"; exit 2 ;;
esac
