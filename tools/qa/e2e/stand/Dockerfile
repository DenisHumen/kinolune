# SPDX-License-Identifier: MPL-2.0
#
# QA-016 test-only image: the e2e media stand. Samba (smbd + nmbd for
# NetBIOS), wsdd for WS-Discovery and Avahi announcing _smb._tcp, like a
# home NAS. Samba (GPL), wsdd and Avahi run only in this container; none of
# their code is linked into Kinolune.
FROM alpine:3.22
RUN apk add --no-cache avahi samba-client samba-server samba-common-tools wsdd
COPY smb.conf /etc/samba/smb.conf
COPY smb.service /etc/avahi/services/smb.service
COPY entrypoint.sh /entrypoint.sh
ENTRYPOINT ["/bin/sh", "/entrypoint.sh"]
