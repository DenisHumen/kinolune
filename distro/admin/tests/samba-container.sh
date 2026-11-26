#!/usr/bin/env bash
# SPDX-License-Identifier: MPL-2.0
# Samba of Kinolune OS (DIST-018) with the real alpha-admin-priv, real PAM,
# useradd/chpasswd/smbpasswd/pdbedit and smbd in a throwaway Fedora
# container (no VM needed):
#
#   distro/admin/tests/samba-container.sh
#
# Builds alpha-admin-priv with -tags pam, installs our smb.conf, PAM file,
# sysusers and tmpfiles, serves the helper on a unix socket
# (systemd-socket-activate --accept, like alpha-admin-priv.socket) and runs
# tests/samba_check.py: the installer's Samba password import, users created
# through the helper, Samba password sync, guest policy, shares and their
# permissions with smbclient. SSH and the firewall need systemd and firewalld:
# they are checked in the VM (vm-admin-e2e.sh).
# ENGINE=docker|podman, NAME=container name.
set -euo pipefail
source "$(dirname "$0")/../../scripts/lib.sh"

NAME="${NAME:-alpha-admin-samba-test}"
ADMIN_DIR="${DISTRO_DIR}/admin"
"${ENGINE}" rm -f "${NAME}" >/dev/null 2>&1 || true
trap '"${ENGINE}" rm -f "${NAME}" >/dev/null 2>&1 || true' EXIT

log "Starting ${BASE_IMAGE%%@*} container ${NAME}"
"${ENGINE}" run -d --name "${NAME}" -v "${DISTRO_DIR}:/src/distro:ro" "${BASE_IMAGE}" sleep infinity >/dev/null
"${ENGINE}" exec -i "${NAME}" bash -s <<'EOF'
set -euo pipefail
dnf -y -q install --setopt=install_weak_deps=False golang gcc pam-devel samba samba-client python3 >/dev/null
export GOTOOLCHAIN=local GOFLAGS="-trimpath -mod=readonly" GOCACHE=/tmp/gocache GOPATH=/tmp/gopath
cp -a /src/distro/admin /tmp/admin
cd /tmp/admin
mkdir -p /usr/libexec/alpha
CGO_ENABLED=1 go build -tags pam -o /usr/libexec/alpha/alpha-admin-priv ./cmd/alpha-admin-priv
install -m 0644 /src/distro/rootfs/etc/pam.d/alpha-admin /etc/pam.d/alpha-admin
install -m 0644 /src/distro/rootfs/etc/samba/smb.conf /etc/samba/smb.conf
install -d /etc/samba/smb.conf.d
systemd-sysusers /src/distro/rootfs/usr/lib/sysusers.d/alpha.conf /src/distro/rootfs/usr/lib/sysusers.d/alpha-admin.conf
systemd-tmpfiles --create /src/distro/rootfs/usr/lib/tmpfiles.d/alpha.conf || true
# The installer's first user: an administrator.
useradd -m -G wheel anna
echo 'anna:Anna-pass-1' | chpasswd
EOF
log "Helper built, Samba installed; running checks"
"${ENGINE}" exec -i "${NAME}" python3 - < "${ADMIN_DIR}/tests/samba_check.py"
log "ALL CHECKS PASSED"
