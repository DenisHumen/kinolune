#!/usr/bin/env bash
# SPDX-License-Identifier: MPL-2.0
#
# NET-016: starts a throwaway Navidrome (pinned image) on 127.0.0.1 with a
# small generated music library and prints `export` lines for the tests:
#
#   eval "$(core/crates/pa-subsonic/tests/navidrome/setup.sh <work dir>)"
#
# The songs are made with the ffmpeg inside the Navidrome image (no ffmpeg
# on the host needed). The container runs as the calling user and keeps
# its data and music in <work dir> (bind mounts, no named volumes). Needs
# Docker, curl and python3. run.sh calls this, runs the tests and removes
# the container.
set -euo pipefail

image="${PA_NAVIDROME_IMAGE:-deluan/navidrome:0.58.0}"
name="${PA_NAVIDROME_CONTAINER:-pa-net016-navidrome}"
port="${PA_NAVIDROME_PORT:-14533}"
work="$1"
# Throwaway credentials of the throwaway container.
user=kinolune
pass=navidrome-test
url="http://127.0.0.1:$port"

mkdir -p "$work/data" "$work/music"
music="$work/music"

ff() {
    docker run --rm --user "$(id -u):$(id -g)" -v "$music:/m" \
        --entrypoint /usr/bin/ffmpeg "$image" \
        -hide_banner -loglevel error -y "$@"
}
track() { # <n> <disc> <title> <artist> <album> <year> <codec> <out>
    mkdir -p "$(dirname "$music/$8")"
    ff -f lavfi -i "sine=frequency=$((300 + $1 * 100 + $2 * 50)):duration=6" -c:a "$7" \
        -metadata title="$3" -metadata artist="$4" -metadata album_artist="$4" \
        -metadata album="$5" -metadata track="$1" -metadata disc="$2" \
        -metadata date="$6" -metadata genre="Test" "/m/$8"
}

if [ ! -f "$music/.done" ]; then
    a="Kinolune Test Artist/Kinolune Test Album"
    track 1 1 "First Song" "Kinolune Test Artist" "Kinolune Test Album" 2021 \
        libmp3lame "$a/01 - First Song.mp3"
    track 2 1 "Second Song" "Kinolune Test Artist" "Kinolune Test Album" 2021 \
        libmp3lame "$a/02 - Second Song.mp3"
    ff -f lavfi -i "color=c=red:s=64x64" -frames:v 1 "/m/$a/cover.jpg"
    b="Kinolune Other Artist/Kinolune Double Album"
    track 1 1 "Opening" "Kinolune Other Artist" "Kinolune Double Album" 2019 \
        flac "$b/1-01 - Opening.flac"
    track 1 2 "Closing" "Kinolune Other Artist" "Kinolune Double Album" 2019 \
        flac "$b/2-01 - Closing.flac"
    touch "$music/.done"
fi

docker rm -f "$name" >/dev/null 2>&1 || true
docker run -d --name "$name" --user "$(id -u):$(id -g)" \
    -p "127.0.0.1:$port:4533" \
    -v "$work/data:/data" -v "$music:/music:ro" \
    -e ND_LOGLEVEL=warn -e ND_SCANNER_SCHEDULE=0 -e ND_SCANONSTARTUP=true \
    -e ND_ENABLEINSIGHTSCOLLECTOR=false -e ND_LASTFM_ENABLED=false \
    -e ND_LISTENBRAINZ_ENABLED=false \
    "$image" >/dev/null

for _ in $(seq 1 120); do
    curl -fs "$url/ping" >/dev/null 2>&1 && break
    sleep 1
done
curl -fs "$url/ping" >/dev/null || {
    docker logs --tail 50 "$name" >&2
    exit 1
}

# The first user of a new Navidrome is created through its own API.
curl -fsS -X POST "$url/auth/createAdmin" -H 'Content-Type: application/json' \
    -d "{\"username\":\"$user\",\"password\":\"$pass\"}" >/dev/null 2>&1 || true

rest() { # <method> [query]
    curl -fsS "$url/rest/$1.view?u=$user&p=$pass&v=1.16.1&c=pa-setup&f=json${2:+&$2}"
}
rest ping | grep -q '"status":"ok"' || {
    echo "admin login failed" >&2
    exit 1
}
rest startScan >/dev/null 2>&1 || true

songs() {
    rest search3 'query=&songCount=100&artistCount=0&albumCount=0' |
        python3 -c 'import json, sys
r = json.load(sys.stdin)["subsonic-response"]
print(len(r.get("searchResult3", {}).get("song", [])))' 2>/dev/null || echo 0
}
for _ in $(seq 1 120); do
    scanning="$(rest getScanStatus | grep -o '"scanning":[a-z]*' | cut -d: -f2 || true)"
    [ "$scanning" = false ] && [ "$(songs)" = 4 ] && break
    sleep 1
done
[ "$(songs)" = 4 ] || {
    echo "library scan did not finish" >&2
    docker logs --tail 50 "$name" >&2
    exit 1
}

# MUS-012: a playlist of the user, its songs not in album order.
playlist="Kinolune Road"
ids="$(rest search3 'query=&songCount=100&artistCount=0&albumCount=0' |
    python3 -c 'import json, sys
songs = json.load(sys.stdin)["subsonic-response"]["searchResult3"]["song"]
by = {s["title"]: s["id"] for s in songs}
print("&".join("songId=" + by[t] for t in ("Second Song", "Opening", "First Song")))')"
rest getPlaylists | grep -q "\"name\":\"$playlist\"" ||
    rest createPlaylist "name=Kinolune%20Road&$ids" >/dev/null

echo "export PA_SUBSONIC_URL='$url' PA_SUBSONIC_USER='$user' PA_SUBSONIC_PASS='$pass'"
echo "export PA_SUBSONIC_MUSIC='$music' PA_SUBSONIC_PLAYLIST='$playlist'"
