# SPDX-License-Identifier: MPL-2.0
#
# NET-010 test-only image: a "Windows share" — Samba (smbd + nmbd for
# NetBIOS) with wsdd answering WS-Discovery like Windows does. Samba (GPL)
# and wsdd (MIT) run only in this throwaway container; none of their code
# is linked into Kinolune.
FROM alpine:3.22
RUN apk add --no-cache samba-server samba-common-tools wsdd
COPY smb.conf /etc/samba/smb.conf
RUN mkdir -p /srv/media/Movies && echo test > /srv/media/Movies/readme.txt
COPY entrypoint.sh /entrypoint.sh
ENTRYPOINT ["/bin/sh", "/entrypoint.sh"]
