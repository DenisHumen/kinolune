#!/bin/sh
# SPDX-License-Identifier: MPL-2.0
#
# Entry point of a vsftpd test container. $PA_FTP_MODE is plain, explicit
# or implicit; $PA_FTP_PASV is the passive port range `min-max`, mapped 1:1
# on the host.
set -eu

apk add --no-cache vsftpd >/dev/null
mkdir -p /var/empty
adduser -D -H -h /data -s /bin/sh "$PA_FTP_USER"
echo "$PA_FTP_USER:$PA_FTP_PASS" | chpasswd >/dev/null 2>&1

conf=/etc/vsftpd-pa.conf
cp /etc/pa/vsftpd.conf "$conf"
{
    echo "listen_port=2121"
    echo "pasv_min_port=${PA_FTP_PASV%-*}"
    echo "pasv_max_port=${PA_FTP_PASV#*-}"
    if [ "$PA_FTP_MODE" != plain ]; then
        echo "ssl_enable=YES"
        echo "rsa_cert_file=/certs/cert.pem"
        echo "rsa_private_key_file=/certs/key.pem"
        echo "force_local_logins_ssl=YES"
        echo "force_local_data_ssl=YES"
        # The data connections resume the control connection's TLS session
        # (the default, and what a careful server demands).
        echo "require_ssl_reuse=YES"
    fi
    if [ "$PA_FTP_MODE" = implicit ]; then
        echo "implicit_ssl=YES"
    fi
} >>"$conf"
exec vsftpd "$conf"
