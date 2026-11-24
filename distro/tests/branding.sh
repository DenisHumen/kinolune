#!/usr/bin/env bash
# SPDX-License-Identifier: MPL-2.0
# DIST-003: Kinolune OS branding of a built image.
#   distro/tests/branding.sh IMAGE [CAPTURE_DIR]
#
# 1. the brand copies in distro/rootfs match design/brand (sync-assets.sh --check);
# 2. inside IMAGE: os-release fields, no "fedora" (case-insensitive) in the branding places
#    except the attribution "based on Fedora Linux" and technical identifiers (ID_LIKE),
#    no fedora-logos, our icon, dnf still sees the Fedora release, and `fastfetch --pipe`
#    (as root and as an unprivileged user) prints "Kinolune OS" and our logo.
# With CAPTURE_DIR the plain and the coloured fastfetch output are saved there.
# Needs: ENGINE (docker or podman), python3.
set -euo pipefail
source "$(dirname "$0")/../scripts/lib.sh"

IMAGE="${1:?OS image required, e.g. localhost/alpha-os:dev}"
CAPTURE="${2:-}"

log "Brand assets in distro/rootfs are in sync with design/brand"
"${DISTRO_DIR}/branding/sync-assets.sh" --check

log "Branding checks inside ${IMAGE}"
out="$("${ENGINE}" run --rm -i --network none "${IMAGE}" bash -s <<'CHECK'
set -uo pipefail
fail=0
bad() { echo "FAIL: $*"; fail=1; }
ok() { echo "ok: $*"; }

# --- os-release ---------------------------------------------------------------
[ "$(readlink -f /etc/os-release)" = /usr/lib/os-release ] || bad "/etc/os-release is not the image file"
. /usr/lib/os-release
[ "${NAME:-}" = "Kinolune OS" ] && ok "NAME=${NAME}" || bad "NAME='${NAME:-}'"
[ "${ID:-}" = kinolune-os ] && ok "ID=${ID}" || bad "ID='${ID:-}'"
case " ${ID_LIKE:-} " in *" fedora "*) ok "ID_LIKE=${ID_LIKE}" ;; *) bad "ID_LIKE='${ID_LIKE:-}' lacks fedora" ;; esac
case "${VERSION_ID:-}" in ''|*[!0-9]*) bad "VERSION_ID='${VERSION_ID:-}' is not the Fedora release" ;; *) ok "VERSION_ID=${VERSION_ID}" ;; esac
case "${PRETTY_NAME:-}" in "Kinolune OS "?*) ok "PRETTY_NAME=${PRETTY_NAME}" ;; *) bad "PRETTY_NAME='${PRETTY_NAME:-}'" ;; esac
[ -n "${VERSION:-}" ] && [ "${IMAGE_VERSION:-}" = "${VERSION}" ] || bad "VERSION/IMAGE_VERSION mismatch"
[ "$(sed -n 's/^ALPHA_OS_VERSION=//p' /usr/share/alpha/version)" = "${VERSION:-}" ] || bad "VERSION differs from /usr/share/alpha/version"
[ "${VARIANT_ID:-}" = nas ] || bad "VARIANT_ID='${VARIANT_ID:-}'"
[ "${LOGO:-}" = kinolune-logo-icon ] || bad "LOGO='${LOGO:-}'"
[ "${DEFAULT_HOSTNAME:-}" = kinolune ] || bad "DEFAULT_HOSTNAME='${DEFAULT_HOSTNAME:-}'"
[ -n "${ANSI_COLOR:-}" ] || bad "ANSI_COLOR is empty"
for v in HOME_URL SUPPORT_URL BUG_REPORT_URL DOCUMENTATION_URL; do
    case "${!v:-}" in https://github.com/DenisHumen/kinolune*) ;; *) bad "${v}='${!v:-}'" ;; esac
done

# --- no Fedora brand in the branding places -----------------------------------
# Allowed: the attribution and technical identifiers.
allow='based on Fedora Linux|^[0-9]+:ID_LIKE=|/EFI/fedora|\bfedora-release\b'
places="/usr/lib/os-release /etc/system-release /usr/lib/kinolune-release
/etc/issue /etc/issue.net /usr/lib/issue /usr/lib/issue.net /etc/motd /etc/hostname
/etc/default/grub /etc/plymouth/plymouthd.conf /etc/xdg/fastfetch/config.jsonc
/etc/fastfetch/config.jsonc /usr/share/kinolune-os/fastfetch-logo.txt"
for d in /etc/issue.d /etc/motd.d /usr/lib/motd.d /run/motd.d /usr/lib/bootupd/grub2-static \
        /usr/share/plymouth/themes/default.plymouth; do
    [ -e "$d" ] && places="${places} $(find -L "$d" -type f 2>/dev/null | tr '\n' ' ')"
done
checked=0
for f in ${places}; do
    [ -f "$f" ] || continue
    checked=$((checked + 1))
    hits="$(grep -i -n fedora "$f" | grep -v -i -E "${allow}")"
    [ -z "${hits}" ] || bad "Fedora brand in ${f}: ${hits}"
done
ok "${checked} branding files checked for the Fedora brand"
grep -q -i "based on Fedora Linux" /usr/lib/motd.d/10-kinolune-os || bad "motd lacks the attribution"
case "$(cat /etc/system-release)" in "Kinolune OS release "*) ok "system-release: $(cat /etc/system-release)" ;; *) bad "system-release: $(cat /etc/system-release)" ;; esac
# Tools that probe the family by file keep working.
[ -e /etc/fedora-release ] && [ -e /etc/redhat-release ] || bad "fedora-release/redhat-release links are gone"

# --- logos --------------------------------------------------------------------
if rpm -q fedora-logos >/dev/null 2>&1; then bad "fedora-logos is installed"; else ok "no fedora-logos"; fi
for f in /usr/share/icons/hicolor/scalable/apps/kinolune-logo-icon.svg \
        /usr/share/icons/hicolor/256x256/apps/kinolune-logo-icon.png; do
    [ -s "$f" ] || bad "missing ${f}"
done
if rpm -q plymouth >/dev/null 2>&1; then
    theme="$(plymouth-set-default-theme 2>/dev/null)"
    case "${theme}" in *fedora*|*Fedora*) bad "plymouth theme ${theme}" ;; esac
fi

# --- dnf / rpm still see the Fedora release -----------------------------------
rel="$(rpm -E %fedora)"
[ "${rel}" = "${VERSION_ID:-}" ] && ok "rpm %fedora=${rel}" || bad "rpm %fedora='${rel}' VERSION_ID='${VERSION_ID:-}'"
dnfrel="$(dnf --dump-variables 2>/dev/null | sed -n 's/^releasever = //p')"
[ "${dnfrel}" = "${VERSION_ID:-}" ] && ok "dnf releasever=${dnfrel}" || bad "dnf releasever='${dnfrel}'"

# --- fastfetch ----------------------------------------------------------------
logo_line="$(sed -n '3p' /usr/share/kinolune-os/fastfetch-logo.txt | sed 's/\$[0-9]//g; s/ *$//')"
check_ff() { # check_ff WHO OUTPUT
    printf '%s\n' "$2" | grep -q "OS: Kinolune OS " || bad "fastfetch ($1) does not show 'OS: Kinolune OS'"
    printf '%s\n' "$2" | grep -q -F "${logo_line}" || bad "fastfetch ($1) does not show the Kinolune logo"
    printf '%s\n' "$2" | grep -q -i fedora && bad "fastfetch ($1) mentions Fedora"
    return 0
}
ff_root="$(HOME=/root fastfetch --pipe 2>&1)"
check_ff root "${ff_root}"
ff_user="$(cd /tmp && runuser -u nobody -- env HOME=/tmp/ffhome fastfetch --pipe 2>&1)"
check_ff nobody "${ff_user}"
[ "${fail}" = 0 ] && ok "fastfetch shows Kinolune OS and the logo"
echo "----- fastfetch --pipe -----"
printf '%s\n' "${ff_user}"
echo "----- fastfetch (colour) -----"
HOME=/root fastfetch --pipe false 2>&1
echo "----- end -----"
exit "${fail}"
CHECK
)" && rc=0 || rc=$?

printf '%s\n' "${out}" | sed '/^----- fastfetch (colour) -----$/,/^----- end -----$/d'
if [[ -n "${CAPTURE}" ]]; then
    mkdir -p "${CAPTURE}"
    printf '%s\n' "${out}" | sed -n '/^----- fastfetch --pipe -----$/,/^----- fastfetch (colour) -----$/p' \
        | sed '1d;$d' > "${CAPTURE}/fastfetch.txt"
    printf '%s\n' "${out}" | sed -n '/^----- fastfetch (colour) -----$/,/^----- end -----$/p' \
        | sed '1d;$d' > "${CAPTURE}/fastfetch.ansi"
    log "fastfetch output saved to ${CAPTURE}"
fi
[[ "${rc}" == 0 ]] || die "branding checks failed"
log "Branding OK"
