#!/usr/bin/env bash
# SPDX-License-Identifier: MPL-2.0
#
# Real PAM check of alpha-admin-auth (DIST-013) in a throw-away Fedora
# container: creates test accounts (an admin in wheel, a non-admin, a locked
# account, a system account), installs our PAM service file and runs the
# helper's tests built with -tags pam against them, then the whole module's
# tests with the race detector and a private dbus-daemon.
#
#   distro/admin/tests/pam-container.sh            # docker, image fedora:45
#   ENGINE=podman IMAGE=registry.fedoraproject.org/fedora:45 distro/admin/tests/pam-container.sh
#
# Inside the container (called by the script itself): pam-container.sh --inside
set -euo pipefail

ADMIN_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DISTRO_DIR="$(cd "${ADMIN_DIR}/.." && pwd)"

if [[ "${1:-}" != "--inside" ]]; then
    ENGINE="${ENGINE:-docker}"
    IMAGE="${IMAGE:-fedora:45}"
    NAME="${CONTAINER_NAME:-alpha-admin-pam-test}"
    exec "${ENGINE}" run --rm --name "${NAME}" \
        -v "${DISTRO_DIR}:/src/distro:ro" \
        -e GOFLAGS="${GOFLAGS:--p=4}" \
        "${IMAGE}" bash /src/distro/admin/tests/pam-container.sh --inside
fi

dnf -y -q install --setopt=install_weak_deps=False golang gcc pam-devel dbus-daemon passwd >/dev/null
# Work on a copy: the source is mounted read-only.
cp -a /src/distro/admin /work
cd /work

install -m 0644 /src/distro/rootfs/etc/pam.d/alpha-admin /etc/pam.d/alpha-admin
pass='Test-pass-1'
useradd -m -u 1500 -G wheel anna
useradd -m -u 1501 boris
useradd -m -u 1502 locked
useradd -r -u 950 svc
for u in anna boris locked svc; do echo "${u}:${pass}" | chpasswd; done
usermod -L locked

export ALPHA_ADMIN_PAM_PASSWORD="${pass}"
echo "==> PAM: admin in wheel"
ALPHA_ADMIN_PAM_USER=anna ALPHA_ADMIN_PAM_GROUP=wheel ALPHA_ADMIN_PAM_LOCKED_USER=locked \
    ALPHA_ADMIN_PAM_SYSTEM_USER=svc \
    go test -tags pam -count=1 -v -run 'TestRealPAM' ./cmd/alpha-admin-auth
echo "==> PAM: non-admin account (PAM says yes; the daemon refuses it later)"
ALPHA_ADMIN_PAM_USER=boris go test -tags pam -count=1 -run 'TestRealPAM' ./cmd/alpha-admin-auth

echo "==> build (as in the image)"
CGO_ENABLED=0 go build -trimpath -ldflags=-s -o /tmp/alpha-admin ./cmd/alpha-admin
CGO_ENABLED=1 go build -tags pam -trimpath -ldflags=-s -o /tmp/alpha-admin-auth ./cmd/alpha-admin-auth

echo "==> gofmt, vet, all tests (race, private D-Bus)"
test -z "$(gofmt -l .)" || { gofmt -l .; exit 1; }
go vet ./... && go vet -tags pam ./...
ALPHA_ADMIN_REQUIRE_DBUS=1 go test -race -count=1 ./...
echo "==> ALL OK"
