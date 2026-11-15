#!/usr/bin/env bash
# SPDX-License-Identifier: MPL-2.0
#
# NET-015: starts a throwaway Emby Server (pinned image) on 127.0.0.1 with a
# small generated library and prints `export` lines for the tests:
#
#   eval "$(core/crates/pa-jellyfin/tests/emby/setup.sh <work dir>)"
#
# Emby Server is proprietary (Emby's terms of use: a personal,
# non-commercial licence): this runs only on a developer's own machine and
# is not part of CI (ADR-NNNN). Media are made with the ffmpeg inside the
# Emby image (no ffmpeg on the host needed). The server runs as the
# calling user and keeps its config and media in <work dir> (bind mounts,
# no named volumes). Needs Docker, curl and python3. run.sh calls this,
# runs the tests and removes the container.
set -euo pipefail

image="${PA_EMBY_IMAGE:-emby/embyserver:4.10.2.0}"
name="${PA_EMBY_CONTAINER:-pa-emby-test}"
port="${PA_EMBY_PORT:-18097}"
work="$1"
# Throwaway credentials of the throwaway container.
user=kinolune
pass=emby-test
url="http://127.0.0.1:$port"

mkdir -p "$work/config" "$work/media"
media="$work/media"

ff() {
    docker run --rm --user "$(id -u):$(id -g)" -v "$media:/m" \
        --entrypoint /bin/ffmpeg "$image" -hide_banner -loglevel error -y "$@"
}
video() { # <secs> <out> [extra args]
    local secs="$1" out="$2"
    shift 2
    mkdir -p "$(dirname "$media/$out")"
    ff -f lavfi -i "testsrc2=size=320x240:rate=25:duration=$secs" \
        -f lavfi -i "sine=frequency=440:duration=$secs" \
        -c:v libx264 -preset ultrafast -g 25 -pix_fmt yuv420p -c:a aac -b:a 64k \
        "$@" "/m/$out"
}
track() { # <n> <title> <out>
    mkdir -p "$(dirname "$media/$3")"
    ff -f lavfi -i "sine=frequency=$((300 + $1 * 100)):duration=8" -c:a libmp3lame -b:a 96k \
        -metadata title="$2" -metadata artist="Kinolune Test Artist" \
        -metadata album_artist="Kinolune Test Artist" -metadata album="Kinolune Test Album" \
        -metadata track="$1" -metadata date=2021 "/m/$3"
}

if [ ! -f "$media/.done" ]; then
    video 40 "Movies/Kinolune Test Movie (2020)/Kinolune Test Movie (2020).mkv"
    # Artwork next to the movie: Emby takes it as the poster.
    ff -f lavfi -i "testsrc2=size=300x450" -frames:v 1 \
        "/m/Movies/Kinolune Test Movie (2020)/poster.jpg"
    video 12 "Movies/Kinolune Second Film (2019)/Kinolune Second Film (2019).mp4" -movflags +faststart
    video 15 "Shows/Kinolune Test Show/Season 01/Kinolune Test Show S01E01.mkv"
    video 15 "Shows/Kinolune Test Show/Season 01/Kinolune Test Show S01E02.mkv"
    track 1 "First Song" "Music/Kinolune Test Artist/Kinolune Test Album/01 - First Song.mp3"
    track 2 "Second Song" "Music/Kinolune Test Artist/Kinolune Test Album/02 - Second Song.mp3"
    touch "$media/.done"
fi

docker rm -f "$name" >/dev/null 2>&1 || true
# The image's init drops to UID/GID itself.
docker run -d --name "$name" -e UID="$(id -u)" -e GID="$(id -g)" -e GIDLIST="$(id -g)" \
    -p "127.0.0.1:$port:8096" \
    -v "$work/config:/config" -v "$media:/media:ro" \
    "$image" >/dev/null

for _ in $(seq 1 180); do
    curl -fs "$url/System/Info/Public" >/dev/null 2>&1 && break
    sleep 1
done
curl -fs "$url/System/Info/Public" >/dev/null || {
    docker logs --tail 50 "$name" >&2
    exit 1
}

client='MediaBrowser Client="pa-emby-setup", Device="setup", DeviceId="pa-emby-setup", Version="1"'
post() { # <path> <json> [token]
    local auth="$client"
    [ -n "${3:-}" ] && auth="$client, Token=\"$3\""
    curl -fsS -X POST "$url$1" -H "X-Emby-Authorization: $auth" -H 'Content-Type: application/json' -d "$2"
}
get() { curl -fsS "$url$1" -H "X-Emby-Authorization: $client, Token=\"$2\""; }

login() {
    post /Users/AuthenticateByName "{\"Username\":\"$user\",\"Pw\":\"$pass\"}" 2>/dev/null |
        sed -n 's/.*"AccessToken":"\([^"]*\)".*/\1/p'
}
token="$(login || true)"
if [ -z "$token" ]; then
    # A fresh server: the startup wizard (it answers errors for a few
    # seconds after start).
    for _ in $(seq 1 60); do
        post /Startup/Configuration \
            '{"UICulture":"en-US","MetadataCountryCode":"US","PreferredMetadataLanguage":"en"}' \
            >/dev/null 2>&1 && break
        sleep 1
    done
    curl -fsS "$url/Startup/User" >/dev/null
    post /Startup/User "{\"Name\":\"$user\",\"Password\":\"$pass\"}" >/dev/null
    post /Startup/Complete '' >/dev/null
    token="$(login)"
fi
[ -n "$token" ] || {
    echo "admin login failed" >&2
    exit 1
}

if ! get /Library/VirtualFolders/Query "$token" | grep -q '"Name":"Movies"'; then
    # The test videos are seconds long: keep resume positions of short
    # items (the default needs minutes); thumbnails (BIF) every 10 s.
    opts='"MinResumeDurationSeconds":0,"MinResumePct":1,"MaxResumePct":95,"EnableRealtimeMonitor":false,"ThumbnailImagesIntervalSeconds":10,"EnableMarkerDetection":true,"EnableMarkerDetectionDuringLibraryScan":true'
    for lib in Movies:movies Shows:tvshows Music:music; do
        n="${lib%%:*}" t="${lib##*:}"
        post "/Library/VirtualFolders?Name=$n&CollectionType=$t&RefreshLibrary=false" \
            "{\"LibraryOptions\":{$opts,\"PathInfos\":[{\"Path\":\"/media/$n\"}]}}" "$token" >/dev/null
    done
    viewer="$(post /Users/New '{"Name":"viewer"}' "$token" | sed -n 's/^{"Name":"viewer".*"Id":"\([0-9a-f]*\)".*/\1/p')"
    [ -n "$viewer" ] && post "/Users/$viewer/Password" '{"NewPw":"viewer-test"}' "$token" >/dev/null
    post /Library/Refresh '' "$token" >/dev/null
fi

count() {
    get "/Items?Recursive=true&IncludeItemTypes=$1&Limit=0" "$token" |
        sed -n 's/.*"TotalRecordCount":\([0-9]*\).*/\1/p'
}
for _ in $(seq 1 240); do
    [ "$(count Movie)" = 2 ] && [ "$(count Episode)" = 2 ] && [ "$(count Audio)" = 2 ] && break
    sleep 1
done
[ "$(count Movie)" = 2 ] || {
    echo "library scan did not finish" >&2
    exit 1
}

echo "export PA_EMBY_URL=$url PA_EMBY_USER=$user PA_EMBY_PASS=$pass PA_EMBY_MEDIA=$media"
