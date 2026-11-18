#!/usr/bin/env bash
# SPDX-License-Identifier: MPL-2.0
#
# NET-014: starts a throwaway Jellyfin (pinned image) on 127.0.0.1 with a
# small generated library and prints `export` lines for the tests:
#
#   eval "$(core/crates/pa-jellyfin/tests/server/setup.sh <work dir>)"
#
# Media are made with the ffmpeg inside the Jellyfin image (no ffmpeg on
# the host needed). The container runs as the calling user and keeps its
# config, cache and media in <work dir> (bind mounts, no named volumes).
# Needs Docker, curl and python3. run.sh calls this, runs the tests and
# removes the container.
set -euo pipefail

image="${PA_JELLYFIN_IMAGE:-jellyfin/jellyfin:10.10.7}"
name="${PA_JELLYFIN_CONTAINER:-pa-jellyfin-test}"
port="${PA_JELLYFIN_PORT:-18096}"
work="$1"
# Throwaway credentials of the throwaway container.
user=kinolune
pass=jellyfin-test
url="http://127.0.0.1:$port"

mkdir -p "$work/config" "$work/cache" "$work/media"
media="$work/media"

ff() {
    docker run --rm --user "$(id -u):$(id -g)" -v "$media:/m" \
        --entrypoint /usr/lib/jellyfin-ffmpeg/ffmpeg "$image" \
        -hide_banner -loglevel error -y "$@"
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
    video 12 "Movies/Kinolune Second Film (2019)/Kinolune Second Film (2019).mp4" -movflags +faststart
    video 15 "Shows/Kinolune Test Show/Season 01/Kinolune Test Show S01E01.mkv"
    video 15 "Shows/Kinolune Test Show/Season 01/Kinolune Test Show S01E02.mkv"
    track 1 "First Song" "Music/Kinolune Test Artist/Kinolune Test Album/01 - First Song.mp3"
    track 2 "Second Song" "Music/Kinolune Test Artist/Kinolune Test Album/02 - Second Song.mp3"
    touch "$media/.done"
fi

docker rm -f "$name" >/dev/null 2>&1 || true
docker run -d --name "$name" --user "$(id -u):$(id -g)" \
    -p "127.0.0.1:$port:8096" \
    -v "$work/config:/config" -v "$work/cache:/cache" -v "$media:/media:ro" \
    "$image" >/dev/null

for _ in $(seq 1 180); do
    curl -fs "$url/System/Info/Public" >/dev/null 2>&1 && break
    sleep 1
done
curl -fs "$url/System/Info/Public" >/dev/null || {
    docker logs --tail 50 "$name" >&2
    exit 1
}

client='MediaBrowser Client="pa-jellyfin-setup", Device="setup", DeviceId="pa-jellyfin-setup", Version="1"'
post() { # <path> <json> [token]
    local auth="$client"
    [ -n "${3:-}" ] && auth="$client, Token=\"$3\""
    curl -fsS -X POST "$url$1" -H "Authorization: $auth" -H 'Content-Type: application/json' -d "$2"
}
get() { curl -fsS "$url$1" -H "Authorization: $client, Token=\"$2\""; }

wizard_done="$(curl -fs "$url/System/Info/Public" | grep -o '"StartupWizardCompleted":[a-z]*' | cut -d: -f2)"
if [ "$wizard_done" != "true" ]; then
    # The startup wizard answers errors for a few seconds after start.
    for _ in $(seq 1 60); do
        post /Startup/Configuration \
            '{"UICulture":"en-US","MetadataCountryCode":"US","PreferredMetadataLanguage":"en"}' \
            >/dev/null 2>&1 && break
        sleep 1
    done
    curl -fsS "$url/Startup/User" -H "Authorization: $client" >/dev/null
    post /Startup/User "{\"Name\":\"$user\",\"Password\":\"$pass\"}" >/dev/null
    post /Startup/RemoteAccess '{"EnableRemoteAccess":true,"EnableAutomaticPortMapping":false}' >/dev/null
    post /Startup/Complete '' >/dev/null
fi

token="$(post /Users/AuthenticateByName "{\"Username\":\"$user\",\"Pw\":\"$pass\"}" |
    sed -n 's/.*"AccessToken":"\([^"]*\)".*/\1/p')"
[ -n "$token" ] || {
    echo "admin login failed" >&2
    exit 1
}

# The test videos are seconds long: let the server keep resume positions
# of short items (the default needs 5 min) and keep Quick Connect on.
get /System/Configuration "$token" |
    python3 -c 'import json, sys
c = json.load(sys.stdin)
c.update(MinResumeDurationSeconds=0, MinResumePct=1, MaxResumePct=95, QuickConnectAvailable=True)
print(json.dumps(c))' >"$work/system.json"
post /System/Configuration "$(cat "$work/system.json")" "$token" >/dev/null

if ! get /Library/VirtualFolders "$token" | grep -q '"Name":"Movies"'; then
    opts='"EnableTrickplayImageExtraction":true,"ExtractTrickplayImagesDuringLibraryScan":true,"EnableRealtimeMonitor":false'
    for lib in Movies:movies Shows:tvshows Music:music; do
        n="${lib%%:*}" t="${lib##*:}"
        post "/Library/VirtualFolders?name=$n&collectionType=$t&refreshLibrary=false" \
            "{\"LibraryOptions\":{$opts,\"PathInfos\":[{\"Path\":\"/media/$n\"}]}}" "$token" >/dev/null
    done
    post /Users/New '{"Name":"viewer","Password":"viewer-test"}' "$token" >/dev/null
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

# Trickplay of the long movie: the scan makes it; the task is the fallback.
has_trickplay() {
    get "/Items?Recursive=true&IncludeItemTypes=Movie&SearchTerm=Test%20Movie&Fields=Trickplay" "$token" |
        grep -q '"Trickplay":{"'
}
if ! has_trickplay; then
    task="$(get /ScheduledTasks "$token" | grep -o '{[^{}]*"Key":"RefreshTrickplayImages"[^{}]*}' |
        sed -n 's/.*"Id":"\([^"]*\)".*/\1/p')"
    [ -n "$task" ] && post "/ScheduledTasks/Running/$task" '' "$token" >/dev/null
    for _ in $(seq 1 180); do
        has_trickplay && break
        sleep 1
    done
fi

# MUS-012: a music playlist of the admin, its songs not in album order.
uid="$(get /Users/Me "$token" | python3 -c 'import json, sys; print(json.load(sys.stdin)["Id"])')"
if ! get "/Items?Recursive=true&IncludeItemTypes=Playlist&userId=$uid" "$token" |
    grep -q '"Name":"Kinolune Road"'; then
    ids="$(get "/Items?Recursive=true&IncludeItemTypes=Audio&userId=$uid" "$token" |
        python3 -c 'import json, sys
by = {i["Name"]: i["Id"] for i in json.load(sys.stdin)["Items"]}
print(json.dumps([by["Second Song"], by["First Song"]]))')"
    post /Playlists "{\"Name\":\"Kinolune Road\",\"Ids\":$ids,\"UserId\":\"$uid\",\"MediaType\":\"Audio\"}" \
        "$token" >/dev/null
fi

echo "export PA_JELLYFIN_URL=$url PA_JELLYFIN_USER=$user PA_JELLYFIN_PASS=$pass PA_JELLYFIN_MEDIA=$media"
echo "export PA_JELLYFIN_PLAYLIST='Kinolune Road'"
