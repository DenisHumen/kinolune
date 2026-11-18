#!/usr/bin/env bash
# SPDX-License-Identifier: MPL-2.0
#
# NET-021: SMB streaming speed per network profile. Starts one throwaway
# Samba container (no published ports, reached by its bridge address),
# shapes its egress — the data direction of reads — with tc netem inside
# the container's own network namespace, and runs the `stream_bench` test
# (sequential 256 KiB reads of a random file, SHA-256 checked) once per
# profile and run. Host interfaces are never touched; the container and
# the data are removed on exit.
#
# Needs Docker, cargo and the sch_netem kernel module. libsmb2 comes from
# $PA_SMB2_LIB or tools/libsmb2/out/linux-x86_64 (built in Docker if
# missing).
#
#   core/crates/pa-vfs-smb/tests/servers/bench.sh [profile...]
#
# Profiles (default: lan gige wifi lossy):
#   lan     no shaping (container bridge: the client's CPU is the limit)
#   gige    wired gigabit LAN: 1 Gbit/s, 0.5 ms RTT
#   wifi    30 ms ± 5 ms, 0.1 % loss, 300 Mbit/s: Wi-Fi where the link
#           layer retransmits most losses (802.11 ARQ) and TCP sees the rest
#   lossy   30 ms, 1 % random loss: the NET-003 profile, every loss
#           reaches TCP
#   custom  netem arguments from $PA_SMB_BENCH_NETEM
#
# Environment: PA_SMB_BENCH_RUNS (runs per profile, default 3),
# PA_SMB_BENCH_MIB (file size, default 192), PA_SMB_BENCH_MIN_MBIT_<PROFILE>
# (fail below this, e.g. PA_SMB_BENCH_MIN_MBIT_LOSSY=80), PA_SMB_BENCH_NAME
# (container name prefix), PA_SMB_BENCH_LOG (directory for the output of
# every run), PA_SMB_BENCH_CARGO (cargo build flags, default
# --release), PA_SMB_PREFETCH (fixed number of parallel blocks instead of
# the adaptive default).
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo="$(cd "$here/../../../../.." && pwd)"
name="${PA_SMB_BENCH_NAME:-pa-test-smb-bench}-$$"
runs="${PA_SMB_BENCH_RUNS:-3}"
mib="${PA_SMB_BENCH_MIB:-192}"
read -r -a cargo_args <<<"${PA_SMB_BENCH_CARGO---release}"
profiles=("$@")
[ ${#profiles[@]} -gt 0 ] || profiles=(lan gige wifi lossy)
work="$(mktemp -d "${TMPDIR:-/tmp}/pa-smb-bench.XXXXXX")"
# Throwaway credentials of the throwaway container.
export PA_SMB_USER=kinolune
export PA_SMB_PASS=bench-test

cleanup() {
    docker rm -f "$name" >/dev/null 2>&1 || true
    rm -rf "$work"
}
trap cleanup EXIT

netem_args() { # profile
    case "$1" in
        lan) echo "" ;;
        gige) echo "delay 500us rate 1gbit" ;;
        wifi) echo "delay 30ms 5ms distribution normal loss 0.1% rate 300mbit" ;;
        lossy) echo "delay 30ms loss 1%" ;;
        custom) echo "${PA_SMB_BENCH_NETEM:?PA_SMB_BENCH_NETEM is not set}" ;;
        *)
            echo "unknown profile $1" >&2
            return 1
            ;;
    esac
}
for p in "${profiles[@]}"; do netem_args "$p" >/dev/null; done

if [ -z "${PA_SMB2_LIB:-}" ]; then
    lib="$repo/tools/libsmb2/out/linux-x86_64/lib/libsmb2.so.1"
    [ -f "$lib" ] || "$repo/tools/libsmb2/docker-build.sh" linux-x86_64
    export PA_SMB2_LIB="$lib"
fi

mkdir -p "$work/data/stream"
head -c $((mib * 1024 * 1024)) /dev/urandom >"$work/data/stream/stream.bin"
PA_SMB_STREAM_SHA256="$(sha256sum "$work/data/stream/stream.bin" | cut -d' ' -f1)"
export PA_SMB_STREAM_SHA256
chmod -R a+rX "$work"

test_cmd=(cargo test -q "${cargo_args[@]}" -p pa-vfs-smb --features server-tests
    --test contract_servers)
# Build the test before the clock matters.
(cd "$repo" && "${test_cmd[@]}" --no-run)

docker run -d --name "$name" --cap-add NET_ADMIN \
    -e PA_SMB_USER -e PA_SMB_PASS \
    -v "$here/smb.conf:/etc/pa/smb.conf:ro" \
    -v "$here/start.sh:/etc/pa/start.sh:ro" \
    -v "$work/data:/data:ro" \
    alpine:3.22 sh /etc/pa/start.sh >/dev/null
for _ in $(seq 1 180); do
    if docker exec "$name" nc -z 127.0.0.1 445 >/dev/null 2>&1; then break; fi
    sleep 1
done
docker exec "$name" nc -z 127.0.0.1 445
ip="$(docker inspect -f '{{range .NetworkSettings.Networks}}{{.IPAddress}}{{end}}' "$name")"
echo "server congestion control: $(docker exec "$name" cat /proc/sys/net/ipv4/tcp_congestion_control)"

summary=()
for p in "${profiles[@]}"; do
    args="$(netem_args "$p")"
    if [ -n "$args" ]; then
        # shellcheck disable=SC2086 # netem arguments are a word list
        docker exec "$name" tc qdisc replace dev eth0 root netem $args
    else
        docker exec "$name" tc qdisc del dev eth0 root >/dev/null 2>&1 || true
    fi
    upper="$(echo "$p" | tr '[:lower:]' '[:upper:]')"
    min_var="PA_SMB_BENCH_MIN_MBIT_$upper"
    results=()
    for r in $(seq 1 "$runs"); do
        if ! out="$(cd "$repo" && PA_SMB_BENCH_SERVER="$ip" \
            PA_SMB_BENCH_MIN_MBIT="${!min_var:-}" \
            "${test_cmd[@]}" stream_bench -- --exact --nocapture 2>&1)"; then
            echo "$out" >&2
            exit 1
        fi
        if [ -n "${PA_SMB_BENCH_LOG:-}" ]; then
            mkdir -p "$PA_SMB_BENCH_LOG"
            echo "$out" >"$PA_SMB_BENCH_LOG/$p-$r.log"
        fi
        mbit="$(echo "$out" | sed -n 's/.*= \([0-9.]*\) Mbit\/s.*/\1/p' | tail -1)"
        echo "$p run $r: $mbit Mbit/s $(echo "$out" | grep -o 'adaptive:.*' | tail -1)"
        results+=("$mbit")
    done
    median="$(printf '%s\n' "${results[@]}" | sort -n | awk '{a[NR]=$1} END {print a[int((NR+1)/2)]}')"
    summary+=("$p [${args:-no shaping}]: median $median Mbit/s (${results[*]})")
done
echo
printf '%s\n' "${summary[@]}"
