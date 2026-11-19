#!/bin/sh
# SPDX-License-Identifier: MPL-2.0
#
# QA-016 stand entry point: the throwaway user (credentials come from the
# environment, never baked into the image) with the uid of the host user
# that owns the volume, then Avahi, NetBIOS, WS-Discovery and smbd in the
# foreground.
set -eu

: "${PA_E2E_SMB_USER:?}" "${PA_E2E_SMB_PASS:?}"
uid="${PA_E2E_UID:-1000}"
gid="${PA_E2E_GID:-1000}"

getent group e2e >/dev/null || addgroup -g "$gid" e2e
id "$PA_E2E_SMB_USER" >/dev/null 2>&1 ||
    adduser -D -H -s /sbin/nologin -u "$uid" -G e2e "$PA_E2E_SMB_USER"
printf '%s\n%s\n' "$PA_E2E_SMB_PASS" "$PA_E2E_SMB_PASS" | smbpasswd -a -s "$PA_E2E_SMB_USER" >/dev/null

mkdir -p /run/avahi-daemon
sed -i 's/^#\{0,1\}enable-dbus=.*/enable-dbus=no/' /etc/avahi/avahi-daemon.conf
avahi-daemon --daemonize --no-drop-root --no-chroot || echo "stand: avahi-daemon did not start" >&2
nmbd --daemon || echo "stand: nmbd did not start" >&2
wsdd --shortlog --hostname KINOLUNE-E2E --workgroup WORKGROUP >/dev/null 2>&1 &

exec smbd --foreground --no-process-group --debug-stdout
