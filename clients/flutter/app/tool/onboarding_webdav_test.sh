#!/usr/bin/env bash
# SPDX-License-Identifier: MPL-2.0
#
# UI-004 end to end: the first-run setup adds a WebDAV share from Docker
# (the nginx WebDAV of core/crates/pa-catalog/tests/servers) through the
# real Rust core and scans it. Needs Docker with the compose plugin, curl
# and Flutter; fonts for screenshots come from
# `node design/scripts/fonts-fetch.mjs`. Only the throwaway container and
# its network are created and removed; there are no named volumes. Extra
# arguments go to `flutter test`, e.g. with screenshots:
#
#   KINOLUNE_SCREENSHOTS_DIR=/tmp/shots clients/flutter/app/tool/onboarding_webdav_test.sh
set -euo pipefail

app="$(cd "$(dirname "$0")/.." && pwd)"
servers="$(cd "$app/../../../core/crates/pa-catalog/tests/servers" && pwd)"
work="$(mktemp -d)"
# Throwaway credentials of the throwaway container.
export PA_SERVERS_USER=kinolune
export PA_SERVERS_PASS=onboarding-test
export PA_SERVERS_DATA="$work/data" PA_SERVERS_AUTH="$work/auth"
compose=(docker compose -p pa-ui004-onboarding -f "$servers/docker-compose.yml")

cleanup() {
    "${compose[@]}" down --remove-orphans >/dev/null 2>&1 || true
    rm -rf "$work"
}
trap cleanup EXIT

# Three movies and two episodes of a show, as in the LIB-018 suite.
mkdir -p "$work/data" "$work/auth"
mk() { mkdir -p "$(dirname "$work/data/$1")"; head -c "$2" /dev/urandom >"$work/data/$1"; }
mk "Movies/Dune (2021)/Dune.2021.2160p.WEB-DL.mkv" 300000
mk "Movies/The Matrix 1999.mkv" 200000
mk "Фильмы/Брат (1997)/Брат.1997.BDRip.mkv" 200000
mk "Shows/Dark/Season 1/Dark.S01E01.Secrets.1080p.mkv" 200000
mk "Shows/Dark/Season 1/Dark.S01E02.Lies.1080p.mkv" 200000
printf '%s:{PLAIN}%s\n' "$PA_SERVERS_USER" "$PA_SERVERS_PASS" >"$work/auth/htpasswd"
chmod -R a+rX "$work"

"${compose[@]}" up -d --quiet-pull webdav
url=http://127.0.0.1:18581/dav/
for _ in $(seq 1 180); do
    curl -s -o /dev/null "$url" && break
    sleep 1
done
curl -s -o /dev/null "$url" || { "${compose[@]}" logs --tail 50 webdav >&2; exit 1; }

cd "$app"
# The test keeps the password in an in-memory platform store of the core,
# never in the user's keychain.
PA_ONBOARDING_WEBDAV="$url" flutter test test/onboarding_webdav_rust_test.dart "$@"
