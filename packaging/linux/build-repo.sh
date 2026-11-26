#!/usr/bin/env bash
# SPDX-License-Identifier: MPL-2.0
#
# Build the signed static APT and DNF repositories of Kinolune (PKG-023).
#
#   build-repo.sh <site-dir> <manifest>
#
# <manifest>: one release per line, newest first:
#   <version> <package>...
# the .deb and .rpm of that version, in any order, for amd64/x86_64 and/or
# arm64/aarch64 (PKG-026; releases before it have only amd64). The
# architecture is read from each package; every architecture of a version
# needs both its .deb and .rpm.
# A version with a suffix (0.1.0-alpha.7) is a pre-release. Channels:
#   stable  the newest $PA_REPO_KEEP releases without a suffix;
#   beta    the newest $PA_REPO_KEEP releases of any kind.
# Older versions are dropped so the site stays small (GitHub Pages, ~50 MB
# per package).
#
# Writes to <site-dir> (the GitHub Pages root; / stays free for the site,
# SITE-001):
#   kinolune.asc                              public key
#   apt/dists/{stable,beta}/{InRelease,Release,Release.gpg}
#   apt/dists/{stable,beta}/main/binary-{amd64,arm64}/Packages(.gz)
#   apt/pool/main/k/kinolune/kinolune_<debver>_{amd64,arm64}.deb
#   apt/kinolune.sources                      deb822, key inline (README)
#   rpm/{stable,beta}/{x86_64,aarch64}/{*.rpm,repodata/repomd.xml(.asc)}
# Both architectures always exist (an empty index when a channel has no
# arm64 build yet), so apt and dnf on arm64 never fail on a missing index.
#   rpm/kinolune.repo                         for /etc/yum.repos.d (README)
#
# Env: PA_REPO_SIGN_KEY  fingerprint of the signing key in $GNUPGHOME (required);
#      PA_REPO_URL       public base URL (default https://denishumen.github.io/kinolune);
#      PA_REPO_KEY       public key file (default kinolune-repo-public.asc here);
#      PA_REPO_KEEP      versions per channel (default 3).
# Needs: gpg, apt-ftparchive (apt-utils), dpkg-deb, rpm, rpmsign, createrepo_c, gzip.
# The rpms are signed here (rpmsign --addsign) whether or not they already are.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
die() { printf 'build-repo.sh: %s\n' "$*" >&2; exit 1; }

[ $# -eq 2 ] || { sed -n '4,35p' "$0"; exit 2; }
mkdir -p "$1"
site="$(cd "$1" && pwd)"
manifest="$2"
fpr="${PA_REPO_SIGN_KEY:?PA_REPO_SIGN_KEY: fingerprint of the signing key}"
url="${PA_REPO_URL:-https://denishumen.github.io/kinolune}"
url="${url%/}"
pubkey="${PA_REPO_KEY:-$here/kinolune-repo-public.asc}"
keep="${PA_REPO_KEEP:-3}"
[ -f "$manifest" ] || die "no manifest: $manifest"
[ -f "$pubkey" ] || die "no public key: $pubkey"
gpg --batch --list-secret-keys "$fpr" >/dev/null 2>&1 || die "signing key $fpr is not in GNUPGHOME"

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

# --- choose the versions of each channel --------------------------------------
declare -a beta=() stable=()
# deb_of/rpm_of are keyed by "<version> <deb-arch>" (amd64, arm64).
declare -A deb_of=() rpm_of=()
ARCHES=(amd64 arm64)
declare -A rpm_arch=([amd64]=x86_64 [arm64]=aarch64) deb_arch=([x86_64]=amd64 [aarch64]=arm64)
while read -r v files; do
    if [ -z "$v" ] || [ "${v:0:1}" = "#" ]; then continue; fi
    [[ "$v" =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.]+)?$ ]] || die "bad version in manifest: $v"
    for f in $files; do
        [ -f "$f" ] || die "missing package file for $v: $f"
        case "$f" in
            *.deb) a=$(dpkg-deb -f "$f" Architecture); deb_of["$v $a"]="$f" ;;
            *.rpm) a=${deb_arch[$(rpm -qp --nosignature --qf '%{ARCH}' "$f" 2>/dev/null)]:-?}; rpm_of["$v $a"]="$f" ;;
            *) die "not a .deb or .rpm: $f" ;;
        esac
        [[ " ${ARCHES[*]} " == *" $a "* ]] || die "unsupported architecture of $f"
    done
    n=0
    for a in "${ARCHES[@]}"; do
        [ -n "${deb_of["$v $a"]:-}" ] || [ -n "${rpm_of["$v $a"]:-}" ] || continue
        [ -n "${deb_of["$v $a"]:-}" ] && [ -n "${rpm_of["$v $a"]:-}" ] || die "$v $a needs both the .deb and the .rpm"
        n=$((n + 1))
    done
    [ "$n" -gt 0 ] || die "missing package files for $v"
    if [ "${#beta[@]}" -lt "$keep" ]; then beta+=("$v"); fi
    if [[ "$v" != *-* ]] && [ "${#stable[@]}" -lt "$keep" ]; then stable+=("$v"); fi
done < "$manifest"
[ "${#beta[@]}" -gt 0 ] || die "the manifest lists no packages"
printf 'stable: %s\nbeta:   %s\n' "${stable[*]:-(none)}" "${beta[*]}"

rm -rf "$site/apt" "$site/rpm"
mkdir -p "$site/apt/pool/main/k/kinolune" "$site/rpm"
cp "$pubkey" "$site/kinolune.asc"

# --- APT ----------------------------------------------------------------------
declare -A pool_name=()
for v in "${beta[@]}" "${stable[@]}"; do
    for a in "${ARCHES[@]}"; do
        k="$v $a"
        [ -n "${deb_of[$k]:-}" ] && [ -z "${pool_name[$k]:-}" ] || continue
        debver=$(dpkg-deb -f "${deb_of[$k]}" Version)
        name="kinolune_${debver}_$a.deb"
        cp "${deb_of[$k]}" "$site/apt/pool/main/k/kinolune/$name"
        pool_name[$k]="pool/main/k/kinolune/$name"
    done
done

cd "$site/apt"
apt-ftparchive packages pool > "$work/Packages.all"
for suite in stable beta; do
    if [ "$suite" = stable ]; then list=("${stable[@]}"); else list=("${beta[@]}"); fi
    for a in "${ARCHES[@]}"; do
        dir="dists/$suite/main/binary-$a"
        mkdir -p "$dir"
        files=""
        for v in "${list[@]}"; do files="$files ${pool_name["$v $a"]:-}"; done
        # Keep the stanzas whose Filename is in this channel and architecture.
        awk -v keep="$files" 'BEGIN { RS=""; ORS="\n\n"; n=split(keep, a, " "); for (i=1;i<=n;i++) want[a[i]]=1 }
            { if (match($0, /\nFilename: [^\n]*/)) { f=substr($0, RSTART+11, RLENGTH-11); if (f in want) print } }' \
            "$work/Packages.all" > "$dir/Packages"
        gzip -9nkf "$dir/Packages"
    done
    apt-ftparchive \
        -o APT::FTPArchive::Release::Origin=Kinolune \
        -o APT::FTPArchive::Release::Label=Kinolune \
        -o APT::FTPArchive::Release::Suite="$suite" \
        -o APT::FTPArchive::Release::Codename="$suite" \
        -o APT::FTPArchive::Release::Architectures="${ARCHES[*]}" \
        -o APT::FTPArchive::Release::Components=main \
        -o APT::FTPArchive::Release::Description="Kinolune $suite" \
        release "dists/$suite" > "$work/Release"
    mv "$work/Release" "dists/$suite/Release"
    gpg --batch --yes --local-user "$fpr" --digest-algo SHA512 --clearsign \
        -o "dists/$suite/InRelease" "dists/$suite/Release"
    gpg --batch --yes --local-user "$fpr" --digest-algo SHA512 --armor --detach-sign \
        -o "dists/$suite/Release.gpg" "dists/$suite/Release"
done
cd - >/dev/null

# --- DNF ----------------------------------------------------------------------
declare -A signed=()
for v in "${beta[@]}" "${stable[@]}"; do
    for a in "${ARCHES[@]}"; do
        k="$v $a"
        [ -n "${rpm_of[$k]:-}" ] && [ -z "${signed[$k]:-}" ] || continue
        mkdir -p "$work/$a"
        f="$work/$a/$(basename "${rpm_of[$k]}")"
        cp "${rpm_of[$k]}" "$f"
        # Ubuntu's rpm expects /usr/bin/gpg2; use the gpg on PATH.
        rpmsign --define "__gpg $(command -v gpg)" --define "_gpg_name $fpr" --define "_gpg_path ${GNUPGHOME:-$HOME/.gnupg}" \
            --define "_gpg_digest_algo sha512" --addsign "$f" >/dev/null
        signed[$k]="$f"
    done
done
for channel in stable beta; do
    if [ "$channel" = stable ]; then list=("${stable[@]}"); else list=("${beta[@]}"); fi
    for a in "${ARCHES[@]}"; do
        dir="$site/rpm/$channel/${rpm_arch[$a]}"
        mkdir -p "$dir"
        for v in "${list[@]}"; do
            [ -n "${signed["$v $a"]:-}" ] || continue
            name=$(rpm -qp --nosignature --qf '%{NAME}-%{VERSION}-%{RELEASE}.%{ARCH}.rpm' "${signed["$v $a"]}")
            cp "${signed["$v $a"]}" "$dir/$name"
        done
        createrepo_c --quiet --general-compress-type=gz --no-database "$dir"
        gpg --batch --yes --local-user "$fpr" --digest-algo SHA512 --armor --detach-sign \
            -o "$dir/repodata/repomd.xml.asc" "$dir/repodata/repomd.xml"
    done
done

# --- files for the one-line setup in the README ----------------------------------
# Until the first release without a suffix, the default channel is beta.
default=stable
[ "${#stable[@]}" -gt 0 ] || default=beta
mark="# Kinolune repository (written by the kinolune package)"
{
    echo "$mark"
    echo "# Channel: Suites: stable or beta. The kinolune package moves the key to"
    echo "# /etc/apt/keyrings/kinolune.gpg on its first install."
    echo "Types: deb"
    echo "URIs: $url/apt"
    echo "Suites: $default"
    echo "Components: main"
    echo "Architectures: ${ARCHES[*]}"
    echo "Signed-By:"
    sed -e 's/^$/./' -e 's/^/ /' "$pubkey"
} > "$site/apt/kinolune.sources"
{
    echo "$mark"
    echo "# Channel: stable or beta in baseurl."
    echo "[kinolune]"
    echo "name=Kinolune ($default)"
    echo "baseurl=$url/rpm/$default/\$basearch"
    echo "enabled=1"
    echo "gpgcheck=1"
    echo "repo_gpgcheck=1"
    echo "gpgkey=$url/kinolune.asc"
} > "$site/rpm/kinolune.repo"

printf 'wrote %s (default channel: %s)\n' "$site" "$default"
