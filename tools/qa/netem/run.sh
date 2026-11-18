#!/usr/bin/env bash
# SPDX-License-Identifier: MPL-2.0
#
# QA-007: tests on a bad network. Starts a throwaway HTTP server container
# whose outgoing traffic the test shapes with tc netem (delay, loss,
# bandwidth limit, a dropout) inside the container's own network
# namespace, runs pa_player/test/netem_libmpv_test.dart on real libmpv
# against it, and removes the container. Host interfaces are never
# touched.
#
# Needs Docker, Flutter, libmpv (system, or LIBMPV_LIBRARY_PATH) and
# ffmpeg (PA_TEST_FFMPEG, default: ffmpeg on PATH). The host kernel needs
# the sch_netem module (stock Ubuntu kernels load it on demand).
#
#   tools/qa/netem/run.sh [extra flutter test args]
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo="$(cd "$here/../../.." && pwd)"
image="pa-test-qa007-srv:local"
name="pa-test-qa007-srv-$$"
media="$(mktemp -d "${TMPDIR:-/tmp}/pa-test-qa007-media.XXXXXX")"

cleanup() {
  docker rm -f "$name" >/dev/null 2>&1 || true
  rm -rf "$media"
  if [ "${PA_NETEM_KEEP_IMAGE:-0}" != 1 ]; then
    docker rmi "$image" >/dev/null 2>&1 || true
  fi
}
trap cleanup EXIT

ffmpeg_bin="${PA_TEST_FFMPEG:-$(command -v ffmpeg || true)}"
if [ -z "$ffmpeg_bin" ]; then
  echo "QA-007: ffmpeg not found (set PA_TEST_FFMPEG)" >&2
  exit 2
fi

docker build -q -t "$image" "$here" >/dev/null
# nginx reads the films as its own user.
chmod 755 "$media"
docker run -d --name "$name" --cap-add NET_ADMIN \
  -v "$media:/media:ro" "$image" >/dev/null
ip="$(docker inspect -f '{{range .NetworkSettings.Networks}}{{.IPAddress}}{{end}}' "$name")"
for _ in $(seq 50); do
  if (exec 3<>"/dev/tcp/$ip/8080") 2>/dev/null; then break; fi
  sleep 0.1
done
# Fails early when the kernel has no netem.
docker exec "$name" tc qdisc replace dev eth0 root netem delay 0ms

cd "$repo/clients/flutter/packages/pa_player"
PA_LIBMPV_TESTS=1 \
  PA_TEST_FFMPEG="$ffmpeg_bin" \
  PA_NETEM_CONTAINER="$name" \
  PA_NETEM_URL="http://$ip:8080/" \
  PA_NETEM_MEDIA="$media" \
  flutter test test/netem_libmpv_test.dart "$@"
