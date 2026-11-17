#!/usr/bin/env bash
# SPDX-License-Identifier: MPL-2.0
#
# Package a Flutter macOS release build (PKG-019, ADR-NNNN).
#
#   package.sh <Kinolune.app> <version> <out-dir> [<libsmb2-macos-arm64> [<libsmb2-macos-x86_64>]]
#
# Writes <out-dir>/kinolune-<version>-macos-{arm64,x86_64}.dmg: the app bundle
# (thinned to one architecture, drag-to-Applications layout, install notes
# with the xattr command), ad-hoc signed
# (no Developer ID, not notarized: PKG-008/PKG-012), with
#   - our LGPL libmpv + FFmpeg frameworks (already embedded by the
#     media_kit_libs_macos_video pod, packaging/app/macos/mpv-frameworks.sh);
#   - libsmb2 (tools/libsmb2, LIB-019) as Contents/Frameworks/libsmb2.1.dylib,
#     universal when both artifacts are given (pa-smb2 opens it through
#     @executable_path/../Frameworks);
#   - LICENSE, THIRD-PARTY-NOTICES.md, the macOS README and the libmpv and
#     libsmb2 licences in Contents/Resources/licenses.
# Gates: libmpv built with -Dgpl=false, no GPL/nonfree library linked, the
# Rust core (pa_frb) embedded, every Mach-O signed. Needs macOS with Xcode.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd "$here/../../.." && pwd)"

die() { printf 'package.sh: %s\n' "$*" >&2; exit 1; }
log() { printf 'package.sh: %s\n' "$*" >&2; }

# thin_app <Kinolune.app> <arch>: keep only <arch> in every Mach-O file, then
# re-sign inside out (the app seal hashes the nested code) and verify.
thin_app() {
    local a="$1" arch="$2" f archs
    while IFS= read -r -d '' f; do
        archs="$(lipo -archs "$f" 2>/dev/null)" || continue
        case " $archs " in
            *" $arch "*) [ "$archs" = "$arch" ] || lipo -thin "$arch" "$f" -output "$f" ;;
            *) die "${f#"$a/"} has no $arch slice ($archs)" ;;
        esac
    done < <(find "$a" -type f -print0)
    while IFS= read -r -d '' f; do
        codesign --force --sign - --timestamp=none "$f" >/dev/null 2>&1
    done < <(find "$a/Contents/Frameworks" -maxdepth 1 \( -name '*.framework' -o -name '*.dylib' \) -print0)
    codesign --force --sign - --timestamp=none --preserve-metadata=entitlements "$a" 2>/dev/null
    codesign --verify --deep --strict "$a" || die "signature check failed ($arch)"
    [ "$(lipo -archs "$a/Contents/MacOS/Kinolune")" = "$arch" ] || die "thinning to $arch failed"
}

[ $# -ge 3 ] && [ $# -le 5 ] || { sed -n '4,18p' "$0"; exit 2; }
[ "$(uname -s)" = Darwin ] || die "needs macOS (codesign, lipo, ditto, hdiutil)"
src_app="$(cd "$1" && pwd)"
version="$2"
mkdir -p "$3"
out="$(cd "$3" && pwd)"
shift 3
smb2_dirs=("$@")

[[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.]+)?$ ]] || die "bad version: $version"
[ -f "$src_app/Contents/Info.plist" ] || die "$src_app is not an app bundle"

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
app="$work/Kinolune.app"
ditto "$src_app" "$app"
fw="$app/Contents/Frameworks"
lic="$app/Contents/Resources/licenses"
mkdir -p "$lic"

# libmpv: embedded by the pod; check it is ours and LGPL.
mpv="$fw/Mpv.framework/Mpv"
[ -f "$mpv" ] || die "Mpv.framework missing: was packaging/app/macos/mpv-frameworks.sh run before the build?"
grep -aq -- '-Dgpl=false' "$mpv" || die "Mpv.framework is not an -Dgpl=false build"
if grep -aq -- '-Dgpl=true' "$mpv"; then die "Mpv.framework contains -Dgpl=true"; fi
[ -d "$fw/Mpv.framework/Resources/licenses" ] || die "Mpv.framework has no licences"
mkdir -p "$lic/libmpv"
cp "$fw/Mpv.framework/Resources/licenses/"* "$lic/libmpv/"

# libsmb2 (optional: without it SMB sources report the library as missing).
if [ "${#smb2_dirs[@]}" -gt 0 ]; then
    slices=()
    for d in "${smb2_dirs[@]}"; do
        [ -f "$d/lib/libsmb2.1.dylib" ] || die "no lib/libsmb2.1.dylib in $d"
        slices+=("$d/lib/libsmb2.1.dylib")
    done
    if [ "${#slices[@]}" -gt 1 ]; then
        lipo -create "${slices[@]}" -output "$fw/libsmb2.1.dylib"
    else
        cp "${slices[0]}" "$fw/libsmb2.1.dylib"
    fi
    chmod 0755 "$fw/libsmb2.1.dylib"
    otool -D "$fw/libsmb2.1.dylib" | grep -q '@rpath/libsmb2.1.dylib' || die "libsmb2 install name is not @rpath/libsmb2.1.dylib"
    mkdir -p "$lic/libsmb2"
    cp "${smb2_dirs[0]}"/share/licenses/* "$lic/libsmb2/"
    [ -f "${smb2_dirs[0]}/BUILDINFO.txt" ] && cp "${smb2_dirs[0]}/BUILDINFO.txt" "$lic/libsmb2/BUILDINFO.txt"
    log "libsmb2.1.dylib ($(lipo -archs "$fw/libsmb2.1.dylib"))"
else
    log "no libsmb2 given: SMB sources will not work in this build"
fi

cp "$repo_root/LICENSE" "$repo_root/packaging/app/THIRD-PARTY-NOTICES.md" "$here/README-macos.txt" "$lic/"

# The library runs on the Rust core (UI-024).
core="$(find "$fw" -path '*pa_frb*' -type f -perm -u+x | head -n1)"
[ -n "$core" ] || die "the Rust core (pa_frb) is not in Contents/Frameworks"
log "core: ${core#"$app/"} ($(lipo -archs "$core"))"

# No GPL or nonfree library may be linked by anything in the bundle.
forbidden='lib(rubberband|cdio|dvdnav|dvdread|jack|caca|x264|x265|xvidcore|fdk-aac|vidstab|postproc)'
while IFS= read -r -d '' f; do
    if file -b "$f" | grep -q 'Mach-O'; then
        if otool -L "$f" | grep -Eq "$forbidden"; then die "${f#"$app/"} links a GPL/nonfree library"; fi
    fi
done < <(find "$app" -type f -print0)

# Re-sign inside out (we added files to Contents), ad-hoc, keeping the
# app's entitlements.
while IFS= read -r -d '' f; do
    codesign --force --sign - --timestamp=none "$f" >/dev/null
done < <(find "$fw" -maxdepth 1 \( -name '*.framework' -o -name '*.dylib' \) -print0)
codesign --force --sign - --timestamp=none --preserve-metadata=entitlements "$app"
codesign --verify --deep --strict "$app" || die "signature check failed"
codesign -d --entitlements - "$app" 2>/dev/null | grep -q 'network.client' || die "entitlements lost in re-signing"

# One disk image per architecture (smaller downloads): the universal app is
# thinned with lipo, re-signed and put next to an Applications link and the
# install notes with the xattr command (the app is not notarized).
for arch in arm64 x86_64; do
    stage="$work/dmg-$arch"
    mkdir -p "$stage"
    ditto "$app" "$stage/Kinolune.app"
    thin_app "$stage/Kinolune.app" "$arch"
    ln -s /Applications "$stage/Applications"
    sed '1,2{/^SPDX-License-Identifier/d;/^$/d;}' "$here/INSTALL-dmg.txt" > "$stage/Установка — Install.txt"
    dmg="$out/kinolune-$version-macos-$arch.dmg"
    rm -f "$dmg"
    hdiutil create -quiet -volname "Kinolune $version" -srcfolder "$stage" \
        -fs HFS+ -format UDZO -imagekey zlib-level=9 -ov "$dmg" || die "hdiutil failed for $arch"
    log "$(du -h "$dmg" | cut -f1) $dmg"
done
