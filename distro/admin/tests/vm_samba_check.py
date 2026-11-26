#!/usr/bin/python3
# SPDX-License-Identifier: MPL-2.0
"""Samba, users, firewall and SSH checks inside a Kinolune OS VM (DIST-018).

Runs as root inside the VM like vm_api_check.py (stdlib only) and talks to
the real alpha-admin API; vm-admin-e2e.sh calls it between smbclient checks
made from outside the VM (through QEMU's port forward and the VM firewall):

    python3 - setup       < vm_samba_check.py  # state after boot, create dina
    python3 - guest on|off < vm_samba_check.py
    python3 - opening NAME open|close < vm_samba_check.py
    python3 - ssh-toggle  < vm_samba_check.py  # detached: SSH off, then on
    python3 - after       < vm_samba_check.py  # after the reboot: all kept
"""

import json
import os
import socket
import struct
import subprocess
import sys

sys.argv[0] = "vm_samba_check"
BASE = "https://127.0.0.1"
PASSWORD = os.environ.get("E2E_PASSWORD", "E2e-pass-1")
DINA_PASSWORD = "Dina-pass-1"

# vm_api_check.py's client, without importing a file we get on stdin.
import http.cookiejar  # noqa: E402
import ssl  # noqa: E402
import urllib.error  # noqa: E402
import urllib.request  # noqa: E402

CTX = ssl.create_default_context()
CTX.check_hostname = False
CTX.verify_mode = ssl.CERT_NONE


def ok(msg):
    print(f"OK {msg}", flush=True)


def fail(msg):
    print(f"FAIL {msg}", flush=True)
    sys.exit(1)


def sh(cmd):
    return subprocess.run(cmd, shell=True, check=False, capture_output=True, text=True).stdout.strip()


class Client:
    def __init__(self):
        self.op = urllib.request.build_opener(
            urllib.request.HTTPSHandler(context=CTX),
            urllib.request.HTTPCookieProcessor(http.cookiejar.CookieJar()))
        self.csrf = ""

    def call(self, method, path, body=None):
        data = None if body is None else json.dumps(body).encode()
        req = urllib.request.Request(BASE + path, data=data, method=method)
        if data is not None:
            req.add_header("Content-Type", "application/json")
        if self.csrf:
            req.add_header("X-CSRF-Token", self.csrf)
        try:
            with self.op.open(req, timeout=120) as r:
                raw = r.read()
                return r.status, (json.loads(raw) if raw else None)
        except urllib.error.HTTPError as e:
            raw = e.read()
            try:
                return e.code, json.loads(raw)
            except ValueError:
                return e.code, raw

    def login(self, user="carla", password=PASSWORD):
        code, body = self.call("POST", "/api/v1/session", {"username": user, "password": password})
        if code != 200:
            fail(f"login {user}: {code} {body}")
        self.csrf = body["csrf_token"]
        return self


def expect(code, want, what, body=None):
    if code != want:
        fail(f"{what}: status {code}, want {want}: {body}")


def zone_query(kind, value, permanent=False):
    p = "--permanent " if permanent else ""
    return sh(f"firewall-cmd {p}--zone=home --query-{kind}={value}") == "yes"


def mdns_services(name):
    """Legacy-unicast DNS-SD query (PTR name) to the local Avahi."""
    q = struct.pack(">HHHHHH", 0x4b4c, 0, 1, 0, 0, 0)
    for label in name.split("."):
        q += bytes([len(label)]) + label.encode()
    q += b"\0" + struct.pack(">HH", 12, 1)
    ip = sh("ip -4 -o addr show scope global | awk '{print $4}' | cut -d/ -f1 | head -1")
    s = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    s.setsockopt(socket.IPPROTO_IP, socket.IP_MULTICAST_IF, socket.inet_aton(ip))
    s.settimeout(3)
    s.bind((ip, 0))
    s.sendto(q, ("224.0.0.251", 5353))
    try:
        data, _ = s.recvfrom(9000)
    except socket.timeout:
        return b""
    return data


def setup():
    c = Client().login()

    # SSH: keys were installed (bootc --root-ssh-authorized-keys), so the
    # first boot turned it on; passwords are refused.
    code, body = c.call("GET", "/api/v1/ssh")
    expect(code, 200, "ssh", body)
    if not (body["enabled"] and body["active"] and body["port_open"] and body["authorized_keys"] >= 1):
        fail(f"ssh after first boot with keys: {body}")
    if "passwordauthentication no" not in sh("sshd -T 2>/dev/null"):
        fail("sshd accepts passwords")
    ok(f"SSH on after the first boot because keys were installed ({body['authorized_keys']} key), keys only")

    # Firewall: only what we need.
    if not (zone_query("port", "445/tcp") and zone_query("port", "8487/tcp") and zone_query("service", "https")):
        fail("445/8487/https not open")
    for svc in ("samba", "samba-client"):
        if zone_query("service", svc):
            fail(f"{svc} (NetBIOS 137-139) open")
    if ":139 " in sh("ss -ltn"):
        fail("smbd listens on 139")
    ok("firewall zone home: " + sh("firewall-cmd --zone=home --list-services") + " | " + sh("firewall-cmd --zone=home --list-ports"))

    # Samba config, SELinux labels, server string.
    if sh("testparm -s --parameter-name='server string' 2>/dev/null") != "Kinolune OS (%h)":
        fail("server string")
    if sh("testparm -s --parameter-name='map to guest' 2>/dev/null") != "Never":
        fail("map to guest")
    for d in ("/var/srv/media", "/var/srv/users", "/var/srv/timemachine"):
        if "samba_share_t" not in sh(f"stat -c %C {d}"):
            fail(f"{d} label {sh(f'stat -c %C {d}')}")
    if sh("getenforce") != "Enforcing":
        fail("SELinux not enforcing")
    ok("smb.conf.d rendered at boot (server string 'Kinolune OS (%h)', no guest), shares samba_share_t, SELinux enforcing")

    # Users: create dina through the API (re-authenticated), check Samba.
    code, body = c.call("POST", "/api/v1/users", {"username": "dina", "full_name": "Dina", "password": DINA_PASSWORD,
                                                 "admin_password": "wrong"})
    expect(code, 403, "create with wrong admin password", body)
    code, body = c.call("POST", "/api/v1/users", {"username": "dina", "full_name": "Dina", "password": DINA_PASSWORD,
                                                 "admin_password": PASSWORD})
    expect(code, 201, "create dina", body)
    if not body["samba"] or body["admin"] or "alpha-users" not in sh("id -nG dina"):
        fail(f"dina {body}")
    code, body = c.call("GET", "/api/v1/users")
    users = {u["name"]: u for u in body["users"]}
    if not users.get("carla", {}).get("samba") or not users["carla"]["admin"]:
        fail(f"carla not synced at login: {users.get('carla')}")
    if sh("stat -c '%U %a' /var/srv/users/dina") != "dina 700":
        fail("dina's folder")
    ok("API: dina created (wrong admin password refused first), Samba password set; carla's synced at log-in")

    # Discovery: <host>.local, _smb._tcp and _adisk._tcp (Time Machine) over Avahi.
    host = sh("hostname -s")
    if not sh(f"getent hosts {host}.local"):
        fail(f"{host}.local does not resolve")
    smb, adisk = mdns_services("_smb._tcp.local"), mdns_services("_adisk._tcp.local")
    if b"_smb" not in smb or b"_adisk" not in adisk:
        fail(f"mDNS: smb {smb!r} adisk {adisk!r}")
    ok(f"{host}.local resolves; Avahi answers _smb._tcp and _adisk._tcp (Time Machine)")


def guest(state):
    c = Client().login()
    code, body = c.call("PUT", "/api/v1/sharing", {"media_guest_read": state == "on"})
    expect(code, 200, "sharing", body)
    if body["media_guest_read"] != (state == "on"):
        fail(f"sharing {body}")
    ok(f"API: Media guest read {state}")


def opening(name, action):
    c = Client().login()
    code, body = c.call("PUT", f"/api/v1/firewall/openings/{name}", {"open": action == "open"})
    expect(code, 200, f"{action} {name}", body)
    ok(f"API: firewall opening {name} {action}")


def ssh_toggle():
    c = Client().login()
    out = []
    code, body = c.call("PUT", "/api/v1/ssh", {"enabled": False})
    off = code == 200 and sh("systemctl is-active sshd") != "active" and sh("systemctl is-enabled sshd") == "disabled" \
        and not zone_query("service", "ssh") and not zone_query("service", "ssh", True)
    out.append(f"off={off} {code}")
    code, body = c.call("PUT", "/api/v1/ssh", {"enabled": True})
    on = code == 200 and sh("systemctl is-active sshd") == "active" and sh("systemctl is-enabled sshd") == "enabled" \
        and zone_query("service", "ssh") and zone_query("service", "ssh", True)
    out.append(f"on={on} {code}")
    with open("/run/ssh-toggle.result", "w") as f:
        f.write(" ".join(out) + "\n")


def after():
    c = Client().login()
    code, body = c.call("GET", "/api/v1/sharing")
    if not body["media_guest_read"]:
        fail(f"guest read lost on reboot: {body}")
    code, body = c.call("GET", "/api/v1/firewall")
    opens = {o["name"]: o["open"] for o in body["openings"]}
    if opens.get("media") or not opens.get("smb") or not opens.get("ssh"):
        fail(f"openings after reboot: {opens}")
    if zone_query("port", "8487/tcp"):
        fail("media port open again after reboot")
    code, body = c.call("GET", "/api/v1/ssh")
    if not (body["enabled"] and body["active"]):
        fail(f"ssh after reboot {body}")
    if os.path.exists("/var/lib/alpha/ssh-firstboot.done") is False:
        fail("first-boot marker missing")
    ok("after reboot: guest read kept, media port still closed, smb and ssh open, SSH on (first boot not repeated)")
    opening("media", "open")
    guest("off")


if __name__ == "__main__":
    mode = sys.argv[1]
    if mode == "setup":
        setup()
    elif mode == "guest":
        guest(sys.argv[2])
    elif mode == "opening":
        opening(sys.argv[2], sys.argv[3])
    elif mode == "ssh-toggle":
        ssh_toggle()
    elif mode == "after":
        after()
    else:
        fail(f"unknown mode {mode}")
