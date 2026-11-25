#!/usr/bin/env bash
# SPDX-License-Identifier: MPL-2.0
# DIST-032: the app's Wi-Fi client against real NetworkManager and virtual
# Wi-Fi (mac80211_hwsim): a WPA2 access point and an open one, each with
# DHCP. Runs as root INSIDE a throwaway VM (never on a shared host: the
# module adds radios to the whole kernel); wifi-hwsim-vm.sh boots one.
#
#   distro/tests/wifi-hwsim.sh NM_PROBE [POLKIT_RULE]
#
# NM_PROBE is clients/flutter/app/tool/nm_probe.dart compiled with
# `dart compile exe`. With POLKIT_RULE (our 50-kinolune-network.rules) the
# probe runs as an unprivileged user of the "kinolune" group, so the rule
# itself is checked: no root, no password prompt.
#
# Checks: both networks are found with their security; the open one joins
# without a password; a wrong WPA2 password is refused and not saved; the
# right one joins and gets an address; a saved network joins again with no
# password; auto-connect switches; "Forget" removes it; with the access
# points gone the status is "no network".
#
# Needs: NetworkManager with the Wi-Fi plugin, hostapd, dnsmasq, iw,
# python3, the mac80211_hwsim module (Fedora: kernel-modules-internal).
set -euo pipefail

PROBE="$(readlink -f "${1:?nm_probe binary required}")"
RULE="${2:-}"
WPA_SSID="kinolune-wpa2"
WPA_PASS="kinolune-test-123"
OPEN_SSID="kinolune-open"
RUN="$(mktemp -d /run/kinolune-hwsim.XXXXXX)"
fail=0

log() { printf '\033[1;34m==>\033[0m %s\n' "$*" >&2; }
ok() { printf '  \033[1;32mok\033[0m %s\n' "$*" >&2; }
bad() { printf '  \033[1;31mFAIL\033[0m %s\n' "$*" >&2; fail=1; }

[[ $(id -u) == 0 ]] || { echo "run as root (in a VM)" >&2; exit 1; }

cleanup() {
    pkill -F "${RUN}/hostapd-wpa.pid" 2>/dev/null || true
    pkill -F "${RUN}/hostapd-open.pid" 2>/dev/null || true
    pkill -F "${RUN}/dnsmasq-wpa.pid" 2>/dev/null || true
    pkill -F "${RUN}/dnsmasq-open.pid" 2>/dev/null || true
    rm -f /etc/NetworkManager/conf.d/90-kinolune-hwsim.conf
    rm -rf "${RUN}"
}
trap cleanup EXIT

as_user=()
if [[ -n "${RULE}" ]]; then
    log "Polkit rule for the kinolune group, unprivileged probe user"
    install -D -m 0644 "${RULE}" /etc/polkit-1/rules.d/50-kinolune-network.rules
    getent group kinolune >/dev/null || groupadd kinolune
    id kinolune-test >/dev/null 2>&1 || useradd -m -G kinolune kinolune-test
    install -m 0755 "${PROBE}" /usr/local/bin/nm_probe
    PROBE=/usr/local/bin/nm_probe
    systemctl restart polkit
    # Not an active local session: only the rule can allow it.
    as_user=(runuser -u kinolune-test --)
fi

probe() { "${as_user[@]}" "${PROBE}" "$@"; }
field() { python3 -c 'import json,sys; d=json.load(sys.stdin); print(eval(sys.argv[1], {}, {"d": d}))' "$1"; }

log "Three virtual radios: wlan0 for NetworkManager, wlan1/wlan2 for the access points"
modprobe -r mac80211_hwsim 2>/dev/null || true
modprobe mac80211_hwsim radios=3
sleep 2
mapfile -t ifs < <(iw dev | awk '/Interface/ {print $2}' | sort)
((${#ifs[@]} >= 3)) || { echo "hwsim radios not found: ${ifs[*]}" >&2; exit 1; }
CLIENT="${ifs[0]}" AP_WPA="${ifs[1]}" AP_OPEN="${ifs[2]}"

cat > /etc/NetworkManager/conf.d/90-kinolune-hwsim.conf <<EOF
[keyfile]
unmanaged-devices=interface-name:${AP_WPA};interface-name:${AP_OPEN}
EOF
systemctl reload NetworkManager || systemctl restart NetworkManager

start_ap() { # IFACE SSID NET PASS(optional)
    local iface="$1" ssid="$2" net="$3" pass="${4:-}" name="$5"
    ip addr flush dev "${iface}"
    ip addr add "${net}.1/24" dev "${iface}"
    ip link set "${iface}" up
    {
        echo "interface=${iface}"
        echo "driver=nl80211"
        echo "ssid=${ssid}"
        echo "hw_mode=g"
        echo "channel=6"
        if [[ -n "${pass}" ]]; then
            echo "wpa=2"
            echo "wpa_key_mgmt=WPA-PSK"
            echo "rsn_pairwise=CCMP"
            echo "wpa_passphrase=${pass}"
        fi
    } > "${RUN}/hostapd-${name}.conf"
    hostapd -B -P "${RUN}/hostapd-${name}.pid" "${RUN}/hostapd-${name}.conf" >/dev/null
    dnsmasq --interface="${iface}" --bind-interfaces --except-interface=lo \
        --dhcp-range="${net}.10,${net}.50,1h" --port=0 \
        --pid-file="${RUN}/dnsmasq-${name}.pid" --dhcp-leasefile="${RUN}/leases-${name}"
}

log "Access points: ${WPA_SSID} (WPA2) on ${AP_WPA}, ${OPEN_SSID} (open) on ${AP_OPEN}"
start_ap "${AP_WPA}" "${WPA_SSID}" 192.168.77 "${WPA_PASS}" wpa
start_ap "${AP_OPEN}" "${OPEN_SSID}" 192.168.78 "" open
nmcli dev set "${CLIENT}" managed yes >/dev/null 2>&1 || true

log "Scan: both networks with their security"
found=0
for _ in $(seq 1 20); do
    probe scan >/dev/null || true
    sleep 2
    s="$(probe status)"
    if [[ "$(field '[n["ssid"] for n in d["networks"]]' <<<"$s")" == *"${WPA_SSID}"* &&
          "$(field '[n["ssid"] for n in d["networks"]]' <<<"$s")" == *"${OPEN_SSID}"* ]]; then
        found=1; break
    fi
done
if ((found)); then ok "found both networks"; else bad "networks not found: $s"; fi
[[ "$(field 'd["available"]' <<<"$s")" == True ]] && ok "available" || bad "not available"
sec="$(field '{n["ssid"]: n["security"] for n in d["networks"]}' <<<"$s")"
[[ "$sec" == *"'${WPA_SSID}': 'wpaPersonal'"* ]] && ok "WPA2 marked protected" || bad "security: $sec"
[[ "$sec" == *"'${OPEN_SSID}': 'open'"* ]] && ok "open marked open" || bad "security: $sec"

log "Open network, no password"
r="$(probe connect "${OPEN_SSID}" | field 'd["result"]')"
[[ "$r" == ok ]] && ok "joined ${OPEN_SSID}" || bad "open join: $r"
s="$(probe status)"
[[ "$(field 'd["active"]' <<<"$s")" == "${OPEN_SSID}" ]] && ok "active ${OPEN_SSID}" || bad "active: $s"
[[ "$(field 'd["address"]' <<<"$s")" == 192.168.78.* ]] && ok "address $(field 'd["address"]' <<<"$s")" || bad "no address: $s"

log "WPA2: a wrong password, then the right one"
j="$(probe connect "${WPA_SSID}" wrong-password-1)"
r="$(field 'd["result"]' <<<"$j")"
[[ "$r" == wrongPassword ]] && ok "wrong password refused" || bad "wrong password: $j"
nmcli -t -f NAME connection show | grep -qx "${WPA_SSID}" && bad "wrong password was saved" || ok "nothing saved"
r="$(probe connect "${WPA_SSID}" "${WPA_PASS}" | field 'd["result"]')"
[[ "$r" == ok ]] && ok "joined ${WPA_SSID}" || bad "WPA2 join: $r"
s="$(probe status)"
[[ "$(field 'd["address"]' <<<"$s")" == 192.168.77.* ]] && ok "address $(field 'd["address"]' <<<"$s")" || bad "no address: $s"

log "Saved: joins again without a password; auto-connect"
probe connect "${OPEN_SSID}" >/dev/null
r="$(probe connect "${WPA_SSID}" | field 'd["result"]')"
[[ "$r" == ok ]] && ok "rejoined without a password" || bad "rejoin: $r"
probe autoconnect "${WPA_SSID}" off >/dev/null
[[ "$(nmcli -g connection.autoconnect connection show "${WPA_SSID}")" == no ]] && ok "auto-connect off" || bad "auto-connect"
probe autoconnect "${WPA_SSID}" on >/dev/null
[[ -n "$(nmcli -s -g 802-11-wireless-security.psk connection show "${WPA_SSID}")" ]] && ok "password still in NetworkManager" || bad "password lost on update"

log "Forget"
probe forget "${WPA_SSID}" >/dev/null
nmcli -t -f NAME connection show | grep -qx "${WPA_SSID}" && bad "still saved" || ok "forgotten"

log "Access points gone: no network"
probe forget "${OPEN_SSID}" >/dev/null
pkill -F "${RUN}/hostapd-wpa.pid" || true
pkill -F "${RUN}/hostapd-open.pid" || true
sleep 3
s="$(probe status)"
[[ "$(field 'd["active"]' <<<"$s")" == None ]] && ok "no active network" || bad "still active: $s"
c="$(probe check | field 'd["connectivity"]')"
# The VM's own Ethernet (its SSH link) stays up, so the internet check may
# still pass through it; without it this is "none".
if [[ "$(field 'd["wired"]["connected"]' <<<"$s")" == True ]]; then
    ok "connectivity over the cable: $c"
else
    [[ "$c" == none ]] && ok "connectivity: none" || bad "connectivity: $c"
fi

if ((fail)); then log "FAILED"; exit 1; fi
log "All Wi-Fi checks passed"
