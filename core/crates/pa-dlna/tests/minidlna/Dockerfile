# SPDX-License-Identifier: MPL-2.0
# NET-018 test server: minidlna (and ffmpeg to make the test media) on a
# pinned Alpine. Only run in Docker by tests/minidlna/setup.sh; nothing of
# it ships with the app.
FROM alpine:3.22
RUN apk add --no-cache minidlna ffmpeg
COPY minidlna.conf /etc/minidlna.conf
ENTRYPOINT ["minidlnad", "-d", "-f", "/etc/minidlna.conf"]
