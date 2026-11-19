#!/bin/sh
# SPDX-License-Identifier: MPL-2.0
#
# Runs inside a clean distro container for repo-test.sh (PKG-023).
#
#   client.sh <phase> <base-url>
#
# phase setup    test tools only (curl, Xvfb, Mesa)
# phase install  add the repository the README way, install, check the
#                package (repo files, package-type, links, a 20 s start)
# phase upgrade  after the newer release is published: `apt upgrade` /
#                `dnf upgrade` must bring it; the beta channel offers the
#                pre-release; a tampered repository must be refused
set -eu

phase="$1"
url="$2"
fail() { echo "FAIL: $*" >&2; exit 1; }
ok() { echo "ok: $*"; }

if command -v apt-get >/dev/null 2>&1; then fam=apt; else fam=dnf; fi
export DEBIAN_FRONTEND=noninteractive

version() {
    # Upstream version without the package revision (-1).
    if [ "$fam" = apt ]; then dpkg-query -W -f '${Version}' kinolune | sed 's/-[^-]*$//'; else rpm -q --qf '%{VERSION}' kinolune; fi
}

case "$phase" in
setup)
    if [ "$fam" = apt ]; then
        apt-get update -qq
        apt-get install -y -qq --no-install-recommends curl ca-certificates xvfb xauth \
            libgl1-mesa-dri libegl-mesa0 >/dev/null
    else
        dnf -y -q install curl xorg-x11-server-Xvfb xorg-x11-xauth mesa-dri-drivers mesa-libEGL which >/dev/null
    fi
    ok "test tools"
    ;;

install)
    # README: one command adds the repository, then the package manager installs.
    if [ "$fam" = apt ]; then
        curl -fsSL "$url/apt/kinolune.sources" -o /etc/apt/sources.list.d/kinolune.sources
        apt-get update -qq
        apt-get install -y -qq kinolune >/dev/null
        [ "$(version)" = 0.0.1 ] || fail "installed $(version), expected 0.0.1"
        # postinst moved the key to the keyring file and kept the channel.
        [ -s /etc/apt/keyrings/kinolune.gpg ] || fail "no /etc/apt/keyrings/kinolune.gpg"
        grep -qx 'Signed-By: /etc/apt/keyrings/kinolune.gpg' /etc/apt/sources.list.d/kinolune.sources \
            || fail "kinolune.sources does not use the keyring: $(cat /etc/apt/sources.list.d/kinolune.sources)"
        grep -qx 'Suites: stable' /etc/apt/sources.list.d/kinolune.sources || fail "channel is not stable"
    else
        curl -fsSL "$url/rpm/kinolune.repo" -o /etc/yum.repos.d/kinolune.repo
        dnf -y -q install kinolune >/dev/null
        [ "$(version)" = 0.0.1 ] || fail "installed $(version), expected 0.0.1"
        grep -qx 'gpgkey=file:///usr/share/kinolune/kinolune.asc' /etc/yum.repos.d/kinolune.repo \
            || fail "kinolune.repo was not rewritten: $(cat /etc/yum.repos.d/kinolune.repo)"
        grep -q '/rpm/stable/\$basearch$' /etc/yum.repos.d/kinolune.repo || fail "channel is not stable"
        rpm -q --qf '%{SIGPGP:pgpsig}%{RSAHEADER:pgpsig}\n' kinolune | grep -qi 'rsa' \
            || fail "installed rpm is not signed"
    fi
    ok "installed $(version) from the repository"
    [ "$(cat /opt/kinolune/package-type)" = "$( [ "$fam" = apt ] && echo deb || echo rpm )" ] \
        || fail "package-type: $(cat /opt/kinolune/package-type)"
    [ "$(readlink -f /usr/bin/kinolune)" = /opt/kinolune/kinolune ] || fail "/usr/bin/kinolune"
    [ -f /usr/share/applications/app.kinolune.desktop ] || fail ".desktop file"
    [ -f /usr/share/icons/hicolor/scalable/apps/app.kinolune.svg ] || fail "icon"
    [ -f /opt/kinolune/licenses/libmpv/BUILDINFO.txt ] || echo "note: no libmpv licences (bundle without them)"
    ok "files, package-type, symlink, desktop entry"
    # Every library resolves from the bundle or a declared dependency
    # (libdartjni's libjvm is only used on Android).
    missing=$(for f in /opt/kinolune/kinolune /opt/kinolune/lib/*.so*; do
        case "$f" in */libdartjni.so) continue ;; esac
        # Plugins find the bundle libraries through the already loaded runner.
        LD_LIBRARY_PATH=/opt/kinolune/lib ldd "$f" 2>/dev/null | grep 'not found' || true
    done | sort -u)
    [ -z "$missing" ] || fail "unresolved libraries: $missing"
    ok "all libraries resolve"
    # Headless start: the app must still run after 20 s under Xvfb.
    rc=0
    HOME=/root timeout 20 xvfb-run -a -s '-screen 0 1280x800x24' kinolune > /tmp/kinolune.log 2>&1 || rc=$?
    if [ "$rc" != 124 ]; then
        tail -n 40 /tmp/kinolune.log >&2
        fail "kinolune exited with $rc within 20 s"
    fi
    ok "kinolune ran 20 s under Xvfb"
    ;;

upgrade)
    if [ "$fam" = apt ]; then
        apt-get update -qq
        apt-get upgrade -y -qq >/dev/null
        [ "$(version)" = 0.0.2 ] || fail "after apt upgrade: $(version), expected 0.0.2"
        ok "apt upgrade -> $(version) (the pre-release is not in stable)"
        sed -i 's/^Suites: stable/Suites: beta/' /etc/apt/sources.list.d/kinolune.sources
        apt-get update -qq
        cand=$(apt-cache policy kinolune | sed -n 's/^ *Candidate: //p')
        [ "$cand" = '0.0.3~rc.1-1' ] || [ "$cand" = '0.0.3~rc.1' ] || fail "beta candidate: $cand"
        ok "beta channel offers $cand"
        sed -i 's/^Suites: beta/Suites: stable/' /etc/apt/sources.list.d/kinolune.sources
        for bad in tampered unsigned; do
            sed -e "s|$url/apt|$url/_$bad/apt|" -e '/^#/d' /etc/apt/sources.list.d/kinolune.sources \
                > /etc/apt/sources.list.d/kinolune-$bad.sources
            if apt-get update > /tmp/bad.log 2>&1 && ! grep -qiE 'not signed|signature|NO_PUBKEY|BADSIG' /tmp/bad.log; then
                cat /tmp/bad.log >&2; fail "apt accepted the $bad repository"
            fi
            grep -iE 'not signed|signature|BADSIG|NO_PUBKEY' /tmp/bad.log | head -n 2
            rm -f /etc/apt/sources.list.d/kinolune-$bad.sources
            ok "apt refuses the $bad InRelease"
        done
    else
        dnf -y -q upgrade --refresh >/dev/null
        [ "$(version)" = 0.0.2 ] || fail "after dnf upgrade: $(version), expected 0.0.2"
        ok "dnf upgrade -> $(version) (the pre-release is not in stable)"
        sed 's|/rpm/stable/|/rpm/beta/|; s|^\[kinolune\]|[kinolune-beta]|' /etc/yum.repos.d/kinolune.repo \
            > /etc/yum.repos.d/kinolune-beta.repo
        # -y: dnf asks to import the key for the new repository's metadata.
        avail=$(dnf -y -q repoquery --repo=kinolune-beta --qf '%{version}\n' kinolune | sort -u | tr '\n' ' ')
        case " $avail " in *" 0.0.3~rc.1 "*) ;; *) fail "beta channel: $avail" ;; esac
        rm -f /etc/yum.repos.d/kinolune-beta.repo
        ok "beta channel offers 0.0.3~rc.1"
        for bad in tampered unsigned; do
            sed -e "s|$url/rpm|$url/_$bad/rpm|" -e "s|^\[kinolune\]|[kinolune-$bad]|" \
                /etc/yum.repos.d/kinolune.repo > /etc/yum.repos.d/kinolune-$bad.repo
            if dnf -y makecache --repo="kinolune-$bad" > /tmp/bad.log 2>&1; then
                cat /tmp/bad.log >&2; fail "dnf accepted the $bad repomd.xml"
            fi
            grep -iE 'signature|gpg|pgp|verif' /tmp/bad.log | head -n 2
            rm -f /etc/yum.repos.d/kinolune-$bad.repo
            ok "dnf refuses the $bad repomd.xml"
        done
    fi
    ;;
*) fail "unknown phase $phase" ;;
esac
