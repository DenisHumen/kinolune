#!/usr/bin/env bash
# SPDX-License-Identifier: MPL-2.0
#
# NET-018: builds the minidlna test image (Alpine 3.22, see Dockerfile),
# makes a small media library with its ffmpeg, starts a throwaway
# container on the default Docker bridge and prints `export` lines for the
# tests:
#
#   eval "$(core/crates/pa-dlna/tests/minidlna/setup.sh <work dir>)"
#
# The container is named $PA_DLNA_CONTAINER (default
# pa-test-net018-minidlna-<pid>); run.sh removes it. The media stays in
# <work dir> (bind mount, no volumes). Needs Docker and curl.
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
image="${PA_DLNA_IMAGE:-pa-test-net018-minidlna:1}"
name="${PA_DLNA_CONTAINER:-pa-test-net018-minidlna-$$}"
work="$1"
media="$work/media"
mkdir -p "$media/Movies/Test Movie (2020)" "$media/Music/Test Artist"

docker build -q -t "$image" "$here" >/dev/null

ff() {
    docker run --rm --user "$(id -u):$(id -g)" -v "$media:/m" --entrypoint ffmpeg "$image" \
        -hide_banner -loglevel error -y "$@"
}
if [ ! -f "$media/.done" ]; then
    ff -f lavfi -i testsrc=duration=4:size=320x240:rate=25 -f lavfi -i sine=duration=4 \
        -c:v mpeg4 -c:a aac -shortest "/m/Movies/Test Movie (2020)/Test Movie (2020).mp4"
    ff -f lavfi -i sine=frequency=440:duration=3 -metadata title="Test Song" \
        -metadata artist="Test Artist" -c:a libmp3lame "/m/Music/Test Artist/01 - Test Song.mp3"
    touch "$media/.done"
fi

docker rm -f "$name" >/dev/null 2>&1 || true
docker run -d --name "$name" -v "$media:/media:ro" "$image" >/dev/null
ip="$(docker inspect -f '{{range .NetworkSettings.Networks}}{{.IPAddress}}{{end}}' "$name")"
gw="$(docker inspect -f '{{range .NetworkSettings.Networks}}{{.Gateway}}{{end}}' "$name")"
location="http://$ip:8200/rootDesc.xml"

browse() { # <object id>
    curl -fsS "http://$ip:8200/ctl/ContentDir" \
        -H 'Content-Type: text/xml; charset="utf-8"' \
        -H 'SOAPACTION: "urn:schemas-upnp-org:service:ContentDirectory:1#Browse"' \
        --data "<?xml version=\"1.0\"?><s:Envelope xmlns:s=\"http://schemas.xmlsoap.org/soap/envelope/\"><s:Body><u:Browse xmlns:u=\"urn:schemas-upnp-org:service:ContentDirectory:1\"><ObjectID>$1</ObjectID><BrowseFlag>BrowseDirectChildren</BrowseFlag><Filter>*</Filter><StartingIndex>0</StartingIndex><RequestedCount>50</RequestedCount><SortCriteria></SortCriteria></u:Browse></s:Body></s:Envelope>" \
        2>/dev/null || true
}
for _ in $(seq 1 60); do
    browse '64$0$0' | grep -q 'Test Movie' && browse '64$1$0' | grep -q 'Test Song' && break
    sleep 1
done
browse '64$1$0' | grep -q 'Test Song' || {
    echo "minidlna did not index the media" >&2
    docker logs --tail 30 "$name" >&2
    exit 1
}

echo "export PA_DLNA_CONTAINER='$name' PA_DLNA_LOCATION='$location' PA_DLNA_IFACE='$gw'"
echo "export PA_DLNA_MEDIA='$media'"
