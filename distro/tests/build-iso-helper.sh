#!/usr/bin/env bash
# SPDX-License-Identifier: MPL-2.0
# DIST-007: build-iso.sh reuses a helper container kept by KEEP=1 only when its bind mounts
# match the current run; with another OUT_DIR it recreates the helper, so the ISO lands in
# the new OUT_DIR. Runs build-iso.sh against a fake container engine: no containers, no
# network, a few seconds.
#   distro/tests/build-iso-helper.sh
set -euo pipefail

SCRIPT="$(cd "$(dirname "$0")/../scripts" && pwd)/build-iso.sh"
T="$(mktemp -d)"
trap 'rm -rf "${T}"' EXIT
mkdir -p "${T}/state"

# Fake engine: one container at most; records its label and the host side of /out.
cat > "${T}/engine" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
S="${FAKE_STATE}"
echo "$*" >> "${S}/calls"
cmd="$1"; shift
case "${cmd}" in
    inspect)
        [[ -f "${S}/label" ]] || exit 1
        echo "true $(cat "${S}/label")" ;;
    rm) rm -f "${S}/label" "${S}/out" ;;
    run)
        while (($#)); do
            case "$1" in
                --label) echo "${2#*=}" > "${S}/label"; shift ;;
                -v) [[ "$2" == *:/out ]] && echo "${2%:/out}" > "${S}/out"; shift ;;
            esac
            shift
        done ;;
    exec)
        [[ -f "${S}/label" ]] || { echo "no container" >&2; exit 1; }
        shift 2 # -i NAME
        case "$1" in
            find) echo /out/build/image.iso ;;
            mv) out="$(cat "${S}/out")"; echo fake-iso > "${out}/${3#/out/}" ;;
        esac ;;
    *) echo "fake engine: unexpected ${cmd}" >&2; exit 1 ;;
esac
EOF
chmod +x "${T}/engine"

build() { # OUT_DIR [env...]
    local out="$1"; shift
    env "$@" ENGINE="${T}/engine" FAKE_STATE="${T}/state" WORK_DIR="${T}/work" \
        VERSION=1.0.0-test PAYLOAD_IMAGE=localhost/payload:test \
        bash "${SCRIPT}" "${out}" > "${T}/log" 2>&1 || { cat "${T}/log"; return 1; }
}
runs() { grep -c '^run ' "${T}/state/calls" || true; }
fail() { echo "FAIL: $*" >&2; cat "${T}/log" >&2; exit 1; }
iso=kinolune-os-1.0.0-test-x86_64.iso

build "${T}/a" KEEP=1
[[ -f "${T}/a/${iso}" && -f "${T}/a/${iso}.sha256" ]] || fail "first run: no ISO in OUT_DIR a"
[[ "$(runs)" == 1 ]] || fail "first run: helper not started once"

rm -f "${T}/a/${iso}"
build "${T}/a" KEEP=1
grep -q 'Reusing helper' "${T}/log" || fail "same mounts: helper not reused"
[[ "$(runs)" == 1 ]] || fail "same mounts: helper recreated"
[[ -f "${T}/a/${iso}" ]] || fail "same mounts: no ISO in OUT_DIR a"

rm -f "${T}/a/${iso}"
build "${T}/b"
grep -q 'other mounts or image: recreating' "${T}/log" || fail "other OUT_DIR: helper not recreated"
[[ "$(runs)" == 2 ]] || fail "other OUT_DIR: helper not started again"
[[ -f "${T}/b/${iso}" && -f "${T}/b/${iso}.sha256" ]] || fail "other OUT_DIR: no ISO in OUT_DIR b"
[[ ! -e "${T}/a/${iso}" ]] || fail "other OUT_DIR: ISO written into the old OUT_DIR a"
[[ ! -f "${T}/state/label" ]] || fail "without KEEP=1 the helper must be removed"

echo "OK: build-iso.sh helper reuse follows OUT_DIR"
