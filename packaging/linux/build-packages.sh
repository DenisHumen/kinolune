#!/usr/bin/env bash
# SPDX-License-Identifier: MPL-2.0
#
# Build the Kinolune .deb and .rpm (PKG-002, PKG-023) from a Linux bundle.
#
#   build-packages.sh <bundle.tar.gz | bundle-dir> <version> <out-dir>
#
# <bundle> is the release tar.gz from packaging/app/linux/package.sh (bundle,
# licences, our LGPL libmpv/FFmpeg/libsmb2) or a directory with the same
# layout (the executable `kinolune`, lib/, data/). Writes to <out-dir> the
# release asset names the in-app updater looks for (update_platform.dart):
#   kinolune-<version>-linux-amd64.deb     kinolune-<version>-linux-arm64.deb
#   kinolune-<version>-linux-x86_64.rpm    kinolune-<version>-linux-aarch64.rpm
# (x86_64 or arm64 after the bundle's executable, PKG-026).
# The packages install to /opt/kinolune with /usr/bin/kinolune, the .desktop
# file and icon, /opt/kinolune/package-type (deb or rpm) and add the APT/DNF
# repository on the first install (scripts/).
#
# Env: PA_REPO_URL  repository base URL baked into the packages
#                   (default https://denishumen.github.io/kinolune; tests
#                   point it at a local server).
#      PA_NFPM      an nfpm binary to use instead of the pinned download.
#      PA_REPO_KEY  public key file (default kinolune-repo-public.asc here).
# Needs: tar, curl, sha256sum, gpg (to dearmor the public key).
set -euo pipefail

NFPM_VERSION="2.48.0"
NFPM_BASE="https://github.com/goreleaser/nfpm/releases/download/v${NFPM_VERSION}"
NFPM_SHA256_x86_64="6d7f9a28b2f397934403950fcbba83f598b08d02edbe79a0815b3b4897341750"
NFPM_SHA256_arm64="a9a75213763783b7669719e505474c748c5ad7dc66591a195619deb50f85c2f4"

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
app_linux="$(cd "$here/../app/linux" && pwd)"

die() { printf 'build-packages.sh: %s\n' "$*" >&2; exit 1; }

[ $# -eq 3 ] || { sed -n '4,25p' "$0"; exit 2; }
input="$1"
version="$2"
mkdir -p "$3"
out="$(cd "$3" && pwd)"
repo_url="${PA_REPO_URL:-https://denishumen.github.io/kinolune}"
repo_url="${repo_url%/}"
key="${PA_REPO_KEY:-$here/kinolune-repo-public.asc}"

[[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.]+)?$ ]] || die "bad version: $version"
[[ "$repo_url" =~ ^(https?|file):// ]] || die "bad PA_REPO_URL: $repo_url"
[ -f "$key" ] || die "no public key: $key"

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
stage="$work/stage"
mkdir -p "$stage/opt/kinolune" "$stage/scripts"

# The bundle (with its licences) becomes /opt/kinolune.
if [ -f "$input" ]; then
    tar -xzf "$input" -C "$work"
    src=$(find "$work" -mindepth 1 -maxdepth 1 -type d -name 'kinolune-*' | head -n1)
    [ -n "$src" ] || die "$input has no kinolune-* folder"
else
    src="$(cd "$input" && pwd)"
fi
{ [ -x "$src/kinolune" ] && [ -d "$src/lib" ] && [ -d "$src/data" ]; } \
    || die "$input is not a Kinolune bundle (kinolune, lib/, data/)"
cp -a "$src/." "$stage/opt/kinolune/"
rm -f "$stage/opt/kinolune/README.txt" # tar.gz instructions, not for a package

# Package architecture from the executable's ELF header (e_machine).
case "$(od -An -tx1 -j18 -N2 "$src/kinolune" | tr -d ' \n')" in
    3e00) deb_arch=amd64 rpm_arch=x86_64 ;;
    b700) deb_arch=arm64 rpm_arch=aarch64 ;;
    *) die "$src/kinolune is neither an x86_64 nor an aarch64 executable" ;;
esac

cp "$app_linux/app.kinolune.desktop" "$app_linux/app.kinolune.svg" "$stage/"
cp "$key" "$stage/kinolune.asc"
gpg --batch --yes --dearmor -o "$stage/kinolune.gpg" "$key"
printf 'deb\n' > "$stage/package-type.deb"
printf 'rpm\n' > "$stage/package-type.rpm"

# A pre-release package subscribes to the beta channel, a release to stable.
channel=stable
[[ "$version" == *-* ]] && channel=beta
for s in deb-postinst deb-postrm rpm-post rpm-postun; do
    sed -e "s|@BASE_URL@|$repo_url|g" -e "s|@CHANNEL@|$channel|g" "$here/scripts/$s" > "$stage/scripts/$s"
    chmod 755 "$stage/scripts/$s"
done

nfpm="${PA_NFPM:-}"
if [ -z "$nfpm" ]; then
    # nfpm for this host (it is a Go binary; the package arch is a setting).
    case "$(uname -m)" in
        x86_64) nfpm_arch=x86_64 ;;
        aarch64 | arm64) nfpm_arch=arm64 ;;
        *) die "no pinned nfpm for $(uname -m); set PA_NFPM" ;;
    esac
    nfpm_sha="NFPM_SHA256_$nfpm_arch"
    curl -fsSL --retry 3 -o "$work/nfpm.tar.gz" "$NFPM_BASE/nfpm_${NFPM_VERSION}_Linux_${nfpm_arch}.tar.gz"
    printf '%s  %s\n' "${!nfpm_sha}" "$work/nfpm.tar.gz" | sha256sum -c --quiet - || die "nfpm checksum mismatch"
    tar -xzf "$work/nfpm.tar.gz" -C "$work" nfpm
    nfpm="$work/nfpm"
fi

# nfpm expands variables only in some fields: render the paths ourselves.
sed -e "s|\${PA_PKG_STAGE}|$stage|g" -e "s|\${PA_PKG_VERSION}|$version|g" -e "s|\${PA_PKG_ARCH}|$deb_arch|g" "$here/nfpm.yaml" > "$work/nfpm.yaml"
# Reproducible timestamps inside the packages.
export SOURCE_DATE_EPOCH="${SOURCE_DATE_EPOCH:-$(date +%s)}"
"$nfpm" package --config "$work/nfpm.yaml" --packager deb --target "$out/kinolune-$version-linux-$deb_arch.deb"
"$nfpm" package --config "$work/nfpm.yaml" --packager rpm --target "$out/kinolune-$version-linux-$rpm_arch.rpm"
printf 'wrote %s\n' "$out/kinolune-$version-linux-$deb_arch.deb" "$out/kinolune-$version-linux-$rpm_arch.rpm"
