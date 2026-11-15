#!/bin/sh
# SPDX-License-Identifier: MPL-2.0
#
# Entry point of the UI-028 Samba container (alpine): installs Samba,
# creates the throwaway users and folders, starts smbd in the foreground.
set -eu

apk add --no-cache samba >/dev/null
mkdir -p /etc/samba
cp /etc/pa/smb.conf /etc/samba/smb.conf

for u in alex bob; do
    adduser -D -s /sbin/nologin "$u"
    printf '%s\n%s\n' "$PA_SMB_PASS" "$PA_SMB_PASS" | smbpasswd -a -s "$u" >/dev/null
done

mkdir -p /home/alex/smb_shared/torrents/downloads /home/alex/Documents /srv/public/Music /srv/private
echo film >/home/alex/smb_shared/torrents/downloads/film.mkv
echo song >/srv/public/Music/song.flac
echo secret >/srv/private/notes.txt
chown -R alex:alex /home/alex
chown -R bob:bob /srv/private
chmod 755 /home/alex /srv/public
chmod 700 /srv/private

exec smbd --foreground --no-process-group --debug-stdout
