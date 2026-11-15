#!/usr/bin/env bash
# SPDX-License-Identifier: MPL-2.0
#
# Our LGPL libmpv (tools/mpv) as macOS frameworks for media_kit (PKG-019,
# ADR-NNNN). Never rebuilds anything; the inputs are tools/mpv outputs or the
# CI artifacts libmpv-macos-arm64 / libmpv-macos-x86_64. Lives here and not in
# tools/mpv/bundle.sh because every file there is part of the libmpv recipe
# hash: touching it would rebuild libmpv for every platform.
#
#   mpv-frameworks.sh <frameworks-dir> <libmpv-macos-arm64> [<libmpv-macos-x86_64>]
#
# Wraps libmpv.2.dylib and the FFmpeg dylibs it links into frameworks
# (Mpv.framework, Avcodec.framework, ...) in <frameworks-dir>, universal
# (lipo) when both artifacts are given. Install names become
# @rpath/<Name>.framework/<Name>, Mpv gets the rpath @loader_path/../../..
# (the app's Contents/Frameworks), licences and BUILDINFO go into
# Mpv.framework/Resources/licenses, and <frameworks-dir>/.symlinks/mpv/macos
# points back at <frameworks-dir> (media_kit_video links -framework Mpv from
# there). Ad-hoc signed. Needs macOS with Xcode (lipo, install_name_tool,
# otool, codesign). The app build uses
#   mpv-frameworks.sh clients/flutter/packages/media_kit_libs_macos_video/macos/Frameworks ...
set -euo pipefail

die() { printf 'mpv-frameworks.sh: error: %s\n' "$*" >&2; exit 1; }
log() { printf 'mpv-frameworks.sh: %s\n' "$*" >&2; }

target_of() { sed -n 's/^target: //p' "$1/BUILDINFO.txt" 2>/dev/null; }

# libavcodec.63.dylib -> Avcodec, libmpv.2.dylib -> Mpv.
macos_framework_name() {
    local stem="${1#lib}"
    stem="${stem%%.*}"
    printf '%s%s\n' "$(printf '%s' "${stem:0:1}" | tr '[:lower:]' '[:upper:]')" "${stem:1}"
}

# @rpath/<lib>.dylib dependencies of a Mach-O file (every slice of a fat one),
# one file name per line, each once.
macos_rpath_deps() {
    otool -L "$1" | awk 'NR > 1 && $1 ~ /^@rpath\// { sub("^@rpath/", "", $1); print $1 }' | sort -u
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
    [ "$(uname -s)" = Darwin ] || die "mpv-frameworks.sh needs macOS with Xcode"
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
        # Short install names (@rpath/X.framework/X, through the framework's
        # top-level symlink): FFmpeg is linked without
        # -headerpad_max_install_names and has only 64-96 bytes of header
        # space, too little for .../Versions/A/X names or an extra rpath.
        local args=(-id "@rpath/$name.framework/$name")
        while IFS= read -r dep; do
            [ "$dep" = "$lib" ] && continue
            depname="$(macos_framework_name "$dep")"
            args+=(-change "@rpath/$dep" "@rpath/$depname.framework/$depname")
            queue="$queue $dep"; queue="${queue# }"
        done < <(macos_rpath_deps "$bin")
        install_name_tool "${args[@]}" "$bin"
        # libmpv (its long build-path install name left room) also finds the
        # FFmpeg frameworks next to its own when opened by path; FFmpeg
        # resolves @rpath through the app and the loading framework.
        if [ "$name" = Mpv ] && ! otool -l "$bin" | grep -F '@loader_path/../../..' >/dev/null; then
            install_name_tool -add_rpath '@loader_path/../../..' "$bin"
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

[ $# -ge 2 ] && [ $# -le 3 ] || { sed -n '4,21p' "$0"; exit 2; }
bundle_macos "$@"
