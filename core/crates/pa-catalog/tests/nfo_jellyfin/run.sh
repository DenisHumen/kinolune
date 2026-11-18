#!/usr/bin/env bash
# SPDX-License-Identifier: MPL-2.0
#
# LIB-014 done-when: an NFO exported by Kinolune is read by Jellyfin.
#
# 1. Makes three short videos with the ffmpeg of the Jellyfin image.
# 2. `cargo test --test nfo_jellyfin` scans them into a Kinolune library,
#    gives the works ids, titles and plots, writes `movie.nfo` and
#    `tvshow.nfo` and renames one movie folder with `[tmdbid-…]`.
# 3. Starts Jellyfin (pinned image) on a Docker network without internet,
#    so everything it knows comes from the files, and checks the names,
#    plots and provider ids it shows.
#
# Uses its own container and network (`pa-test-lib014-*`) and removes them
# and its files on exit; never prunes anything. Needs Docker, curl and
# python3. Extra arguments go to `cargo test`.
#
#   core/crates/pa-catalog/tests/nfo_jellyfin/run.sh
set -euo pipefail

image="${PA_JELLYFIN_IMAGE:-jellyfin/jellyfin:10.10.7}"
name="${PA_LIB014_CONTAINER:-pa-test-lib014-jellyfin}"
net="${name}-net"
work="$(mktemp -d)"
media="$work/media"

cleanup() {
    docker rm -f "$name" "$name-ffmpeg" >/dev/null 2>&1 || true
    docker network rm "$net" >/dev/null 2>&1 || true
    rm -rf "$work"
}
trap cleanup EXIT

mkdir -p "$work/config" "$work/cache" "$media"
video() { # <out>
    mkdir -p "$(dirname "$media/$1")"
    docker run --rm --name "$name-ffmpeg" --user "$(id -u):$(id -g)" -v "$media:/m" \
        --entrypoint /usr/lib/jellyfin-ffmpeg/ffmpeg "$image" -hide_banner -loglevel error -y \
        -f lavfi -i "testsrc2=size=320x240:rate=25:duration=4" \
        -f lavfi -i "sine=frequency=440:duration=4" \
        -c:v libx264 -preset ultrafast -pix_fmt yuv420p -c:a aac -b:a 64k "/m/$1"
}
video "Movies/Kinolune Export Film (2020)/Kinolune Export Film (2020).mkv"
video "Movies/Kinolune Tagged Film (2019)/Kinolune Tagged Film (2019).mkv"
video "Shows/Kinolune Export Show/Season 01/Kinolune Export Show S01E01.mkv"

PA_NFO_MEDIA="$media" cargo test -p pa-catalog --features server-tests --test nfo_jellyfin "$@"

# No internet: Jellyfin cannot ask TMDB, what it shows is from the files.
docker network create --internal "$net" >/dev/null
docker run -d --name "$name" --network "$net" --user "$(id -u):$(id -g)" \
    -v "$work/config:/config" -v "$work/cache:/cache" -v "$media:/media:ro" \
    "$image" >/dev/null
ip="$(docker inspect -f "{{(index .NetworkSettings.Networks \"$net\").IPAddress}}" "$name")"
url="http://$ip:8096"

for _ in $(seq 1 180); do
    curl -fs "$url/System/Info/Public" >/dev/null 2>&1 && break
    sleep 1
done
curl -fs "$url/System/Info/Public" >/dev/null || {
    docker logs --tail 50 "$name" >&2
    exit 1
}

client='MediaBrowser Client="pa-lib014-test", Device="test", DeviceId="pa-lib014-test", Version="1"'
post() { # <path> <json> [token]
    local auth="$client"
    [ -n "${3:-}" ] && auth="$client, Token=\"$3\""
    curl -fsS -X POST "$url$1" -H "Authorization: $auth" -H 'Content-Type: application/json' -d "$2"
}
get() { curl -fsS "$url$1" -H "Authorization: $client, Token=\"$2\""; }

# Throwaway credentials of the throwaway container.
user=kinolune
pass=jellyfin-test
for _ in $(seq 1 60); do
    post /Startup/Configuration \
        '{"UICulture":"en-US","MetadataCountryCode":"US","PreferredMetadataLanguage":"en"}' \
        >/dev/null 2>&1 && break
    sleep 1
done
curl -fsS "$url/Startup/User" -H "Authorization: $client" >/dev/null
post /Startup/User "{\"Name\":\"$user\",\"Password\":\"$pass\"}" >/dev/null
post /Startup/Complete '' >/dev/null
token="$(post /Users/AuthenticateByName "{\"Username\":\"$user\",\"Pw\":\"$pass\"}" |
    sed -n 's/.*"AccessToken":"\([^"]*\)".*/\1/p')"
[ -n "$token" ] || {
    echo "login failed" >&2
    exit 1
}

for lib in Movies:movies Shows:tvshows; do
    n="${lib%%:*}" t="${lib##*:}"
    post "/Library/VirtualFolders?name=$n&collectionType=$t&refreshLibrary=false" \
        "{\"LibraryOptions\":{\"EnableRealtimeMonitor\":false,\"PathInfos\":[{\"Path\":\"/media/$n\"}]}}" \
        "$token" >/dev/null
done
post /Library/Refresh '' "$token" >/dev/null

items() { get "/Items?Recursive=true&IncludeItemTypes=$1&Fields=ProviderIds,Overview,Path" "$token"; }
count() { items "$1" | python3 -c 'import json,sys; print(json.load(sys.stdin)["TotalRecordCount"])'; }
for _ in $(seq 1 240); do
    [ "$(count Movie)" = 2 ] && [ "$(count Series)" = 1 ] && break
    sleep 1
done
# The scan adds items first and reads their metadata right after.
sleep 10

movies="$(items Movie)"
series="$(items Series)"
MOVIES="$movies" SERIES="$series" python3 - <<'EOF'
import json, os, sys

movies = json.loads(os.environ["MOVIES"])["Items"]
series = json.loads(os.environ["SERIES"])["Items"]
fail = []

def check(what, item, field, want):
    got = item.get(field) if field != "ids" else item.get("ProviderIds", {})
    if field == "ids":
        for k, v in want.items():
            if got.get(k) != v:
                fail.append(f"{what}: ProviderIds[{k}] = {got.get(k)!r}, want {v!r}")
        return
    if got != want:
        fail.append(f"{what}: {field} = {got!r}, want {want!r}")

def by_path(items, part):
    for i in items:
        if part in i.get("Path", ""):
            return i
    fail.append(f"no item under {part!r}: {[i.get('Path') for i in items]}")
    return {}

film = by_path(movies, "Kinolune Export Film")
check("movie.nfo", film, "Name", "Экспортный фильм")
check("movie.nfo", film, "Overview", "Описание из Kinolune & NFO")
check("movie.nfo", film, "ids", {"Tmdb": "438631", "Imdb": "tt1160419"})
tagged = by_path(movies, "[tmdbid-550]")
check("folder tag", tagged, "ids", {"Tmdb": "550"})
show = by_path(series, "Kinolune Export Show")
check("tvshow.nfo", show, "Name", "Экспортный сериал")
check("tvshow.nfo", show, "Overview", "Сериал из Kinolune")
check("tvshow.nfo", show, "ids", {"Tmdb": "70523", "Tvdb": "334824"})

for line in fail:
    print("FAIL", line)
if fail:
    sys.exit(1)
print("Jellyfin read the Kinolune NFO files and the folder tag: OK")
EOF
