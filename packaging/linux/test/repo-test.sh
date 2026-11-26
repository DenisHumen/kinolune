#!/usr/bin/env bash
# SPDX-License-Identifier: MPL-2.0
#
# End-to-end test of the .deb/.rpm and the APT/DNF repositories (PKG-023),
# in Docker containers only.
#
#   repo-test.sh <bundle.tar.gz | bundle-dir>
#
# 1. A builder container (ubuntu:24.04 + apt-utils, createrepo-c, rpm, gpg)
#    makes a throwaway signing key, packages the bundle as 0.0.1, 0.0.2 and
#    0.0.3-rc.1, and builds two sites: "first release" (0.0.1) and "next
#    release" (0.0.2 stable, 0.0.3-rc.1 beta), plus a tampered and an
#    unsigned copy.
# 2. A web container serves the first site; clean ubuntu:24.04, debian:13 and
#    fedora:latest containers add the repository the README way, install
#    kinolune and start it under Xvfb (client.sh install).
# 3. The next site is published; `apt upgrade` / `dnf upgrade` must bring
#    0.0.2, the beta channel must offer 0.0.3-rc.1 and the tampered or
#    unsigned repositories must be refused (client.sh upgrade).
#
# Env: PA_TEST_PREFIX  container/network name prefix (default pa-test-pkg023);
#      PA_TEST_DISTROS images to test (default "ubuntu:24.04 debian:13 fedora:latest");
#      PA_TEST_WORK    work directory (default: a temporary one, removed).
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
pkg="$(cd "$here/.." && pwd)"
repo_root="$(cd "$pkg/../.." && pwd)"
die() { printf 'repo-test.sh: %s\n' "$*" >&2; exit 1; }

[ $# -eq 1 ] || { sed -n '4,24p' "$0"; exit 2; }
input="$(realpath "$1")"
[ -e "$input" ] || die "no bundle: $1"
prefix="${PA_TEST_PREFIX:-pa-test-pkg023}"
distros="${PA_TEST_DISTROS:-ubuntu:24.04 debian:13 fedora:latest}"
net="$prefix-net"
web="$prefix-web"
url="http://$web:8000/repo"

if [ -n "${PA_TEST_WORK:-}" ]; then
    mkdir -p "$PA_TEST_WORK"; work="$(cd "$PA_TEST_WORK" && pwd)"; keep_work=true
else
    work="$(mktemp -d)"; keep_work=false
fi
containers=()
cleanup() {
    for c in "${containers[@]}"; do docker rm -f "$c" >/dev/null 2>&1 || true; done
    docker network rm "$net" >/dev/null 2>&1 || true
    docker rmi "$prefix-tools:latest" >/dev/null 2>&1 || true
    # Files the containers wrote as root.
    docker run --rm -v "$work:/w" ubuntu:24.04 sh -c 'rm -rf /w/* /w/.[!.]*' >/dev/null 2>&1 || true
    [ "$keep_work" = true ] || rm -rf "$work"
}
trap cleanup EXIT

docker network create "$net" >/dev/null
uidgid="$(id -u):$(id -g)"

# --- builder --------------------------------------------------------------------
builder="$prefix-builder"
containers+=("$builder")
mkdir -p "$work/in"
if [ -d "$input" ]; then cp -a "$input" "$work/in/bundle"; else cp "$input" "$work/in/bundle.tar.gz"; fi
docker run -d --name "$builder" --network "$net" \
    -v "$work:/w" -v "$repo_root/packaging:/src/packaging:ro" ubuntu:24.04 sleep infinity >/dev/null
docker exec "$builder" sh -c 'apt-get update -qq && DEBIAN_FRONTEND=noninteractive apt-get install -y -qq --no-install-recommends \
    apt-utils createrepo-c rpm gnupg curl ca-certificates python3 >/dev/null'
docker exec -u "$uidgid" -e HOME=/w -e PA_REPO_URL="$url" -e URL="$url" "$builder" bash -euo pipefail -c '
    export GNUPGHOME=/w/gnupg; mkdir -m 700 -p "$GNUPGHOME"
    # Throwaway test key, only for this run.
    gpg --batch --quiet --passphrase "" --quick-gen-key "Kinolune repo test key (throwaway)" rsa3072 sign 1d
    fpr=$(gpg --batch --with-colons --list-secret-keys | awk -F: "/^fpr:/ {print \$10; exit}")
    gpg --batch --armor --export "$fpr" > /w/test-key.asc
    in=/w/in/bundle; [ -e "$in" ] || in=/w/in/bundle.tar.gz
    for v in 0.0.1 0.0.2 0.0.3-rc.1; do
        PA_REPO_KEY=/w/test-key.asc /src/packaging/linux/build-packages.sh "$in" "$v" /w/pkgs
    done
    # The architecture of the bundle (x86_64 or aarch64, PKG-026) names the files.
    p() { echo "$1" /w/pkgs/kinolune-"$1"-linux-*.deb /w/pkgs/kinolune-"$1"-linux-*.rpm; }
    rpmarch=$(uname -m) debarch=$(dpkg --print-architecture)
    p 0.0.1 > /w/first.txt
    { p 0.0.3-rc.1; p 0.0.2; p 0.0.1; } > /w/next.txt
    export PA_REPO_SIGN_KEY="$fpr" PA_REPO_KEY=/w/test-key.asc
    /src/packaging/linux/build-repo.sh /w/site-first /w/first.txt
    /src/packaging/linux/build-repo.sh /w/site-next /w/next.txt
    # Checks of the repositories themselves.
    grep -q "^Suites: stable" /w/site-next/apt/kinolune.sources
    gpg --batch --verify /w/site-next/apt/dists/stable/InRelease 2>/dev/null
    gpg --batch --verify /w/site-next/rpm/stable/$rpmarch/repodata/repomd.xml.asc /w/site-next/rpm/stable/$rpmarch/repodata/repomd.xml 2>/dev/null
    if grep -q "0.0.3~rc.1" /w/site-next/apt/dists/stable/main/binary-$debarch/Packages; then exit 1; fi
    grep -q "0.0.3~rc.1" /w/site-next/apt/dists/beta/main/binary-$debarch/Packages
    grep -q "^Suites: beta" /w/site-first/apt/kinolune.sources && exit 1 || true
    # Tampered: one changed byte in the signed index. Unsigned: no signature at all.
    for bad in tampered unsigned; do
        mkdir -p /w/site-next/_$bad
        cp -a /w/site-next/apt /w/site-next/rpm /w/site-next/_$bad/
    done
    sed -i "s/^Origin: Kinolune/Origin: Kinolunf/" /w/site-next/_tampered/apt/dists/stable/InRelease
    cmp -s /w/site-next/apt/dists/stable/InRelease /w/site-next/_tampered/apt/dists/stable/InRelease && { echo "tamper failed"; exit 1; }
    sed -i "s|<revision>|<revision>1|" /w/site-next/_tampered/rpm/stable/$rpmarch/repodata/repomd.xml
    rm /w/site-next/_unsigned/apt/dists/stable/InRelease /w/site-next/_unsigned/apt/dists/stable/Release.gpg
    rm /w/site-next/_unsigned/rpm/stable/$rpmarch/repodata/repomd.xml.asc
    mkdir -p /w/www && cp -a /w/site-first /w/www/repo
    ls -la /w/pkgs
'
echo "== packages and repositories built"

# --- web server -----------------------------------------------------------------
containers+=("$web")
# ubuntu:24.04 has no python3: serve from the builder image.
docker commit "$builder" "$prefix-tools:latest" >/dev/null
docker run -d --name "$web" --network "$net" -u "$uidgid" -v "$work:/w:ro" "$prefix-tools:latest" \
    python3 -m http.server 8000 -d /w/www >/dev/null

# --- clients --------------------------------------------------------------------
clients=()
for img in $distros; do
    c="$prefix-$(echo "$img" | tr -c 'a-z0-9\n' '-')"
    containers+=("$c")
    clients+=("$c")
    docker run -d --name "$c" --network "$net" -v "$here/client.sh:/client.sh:ro" "$img" sleep infinity >/dev/null
    echo "== $img: setup"
    docker exec "$c" sh /client.sh setup "$url"
done
for c in "${clients[@]}"; do
    echo "== $c: install"
    docker exec "$c" sh /client.sh install "$url"
done

# Publish the next release.
docker exec -u "$uidgid" "$builder" sh -c 'rm -rf /w/www/repo.new && cp -a /w/site-next /w/www/repo.new && mv /w/www/repo /w/www/repo.old && mv /w/www/repo.new /w/www/repo'
for c in "${clients[@]}"; do
    echo "== $c: upgrade"
    docker exec "$c" sh /client.sh upgrade "$url"
done
echo "== all repository tests passed: $distros"
