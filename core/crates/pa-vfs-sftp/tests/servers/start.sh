#!/bin/sh
# SPDX-License-Identifier: MPL-2.0
#
# Entry point of the OpenSSH test container: installs sshd, creates the
# test user (home = the read-only fixture mount), generates host keys and
# the client keys the tests log in with, then runs sshd in the foreground.
set -eu

apk add --no-cache openssh-server openssh-keygen >/dev/null
ssh-keygen -A >/dev/null
adduser -D -h /data -s /bin/sh "$PA_SFTP_USER"
echo "$PA_SFTP_USER:$PA_SFTP_PASS" | chpasswd >/dev/null 2>&1

# Client keys: ed25519 without a passphrase, RSA with one.
if [ ! -f /keys/id_ed25519 ]; then
    ssh-keygen -q -t ed25519 -N '' -C contract -f /keys/id_ed25519
    ssh-keygen -q -t rsa -b 3072 -N "$PA_SFTP_KEY_PASS" -C contract -f /keys/id_rsa
fi
cat /keys/id_ed25519.pub /keys/id_rsa.pub >/keys/authorized_keys
# Host key fingerprints, for the tests to compare with what they see.
for t in ed25519 ecdsa rsa; do
    ssh-keygen -l -f "/etc/ssh/ssh_host_${t}_key.pub" | cut -d' ' -f2 >"/keys/host_${t}.fp"
done
chmod -R a+r /keys

exec /usr/sbin/sshd -D -e -f /etc/pa/sshd_config
