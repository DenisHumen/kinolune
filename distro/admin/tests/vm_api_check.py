#!/usr/bin/python3
# SPDX-License-Identifier: MPL-2.0
"""API checks of alpha-admin inside a NAS OS VM (DIST-013).

Runs as root inside the VM (python3 stdlib only), talks to the real daemon
over https://127.0.0.1 and checks the effects with the real tools
(systemctl, firewall-cmd, findmnt). Used by vm-admin-e2e.sh:

    python3 - main  < vm_api_check.py    # everything except the reboot
    python3 - reboot < vm_api_check.py   # confirmation flow, starts a reboot
    python3 - after  < vm_api_check.py   # after the reboot: cert kept, login works
"""

import hashlib
import http.cookiejar
import json
import os
import ssl
import subprocess
import sys
import time
import urllib.error
import urllib.request

BASE = "https://127.0.0.1"
PASSWORD = os.environ.get("E2E_PASSWORD", "E2e-pass-1")
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


def cert_fingerprint(path="/var/lib/alpha/admin/tls-cert.pem"):
    with open(path) as f:
        der = ssl.PEM_cert_to_DER_cert(f.read())
    h = hashlib.sha256(der).hexdigest()
    return ":".join(h[i:i + 2] for i in range(0, len(h), 2))


class Client:
    def __init__(self):
        self.jar = http.cookiejar.CookieJar()
        self.op = urllib.request.build_opener(
            urllib.request.HTTPSHandler(context=CTX), urllib.request.HTTPCookieProcessor(self.jar)
        )
        self.csrf = ""

    def call(self, method, path, body=None, csrf=True, headers=None):
        data = None if body is None else json.dumps(body).encode()
        req = urllib.request.Request(BASE + path, data=data, method=method)
        if data is not None:
            req.add_header("Content-Type", "application/json")
        if csrf and self.csrf:
            req.add_header("X-CSRF-Token", self.csrf)
        for k, v in (headers or {}).items():
            req.add_header(k, v)
        try:
            with self.op.open(req, timeout=60) as r:
                raw = r.read()
                return r.status, (json.loads(raw) if raw and r.headers.get_content_type() == "application/json" else raw), r.headers
        except urllib.error.HTTPError as e:
            raw = e.read()
            try:
                return e.code, json.loads(raw), e.headers
            except ValueError:
                return e.code, raw, e.headers

    def login(self, user, password=PASSWORD):
        code, body, hdr = self.call("POST", "/api/v1/session", {"username": user, "password": password}, csrf=False)
        if code == 200:
            self.csrf = body["csrf_token"]
        return code, body, hdr


def expect(code, want, what, body=None):
    if code != want:
        fail(f"{what}: status {code}, want {want}: {body}")


def main_checks():
    anon = Client()
    code, body, _ = anon.call("GET", "/healthz")
    expect(code, 200, "healthz", body)
    code, info, _ = anon.call("GET", "/api/v1/info")
    expect(code, 200, "info", info)
    cert_fp = cert_fingerprint()
    if info["tls_fingerprint"] != cert_fp:
        fail(f"fingerprint {info['tls_fingerprint']} != {cert_fp}")
    ok(f"https up, self-signed cert in /var/lib/alpha/admin, fingerprint {cert_fp[:23]}…")
    expect(anon.call("GET", "/api/v1/services")[0], 401, "anonymous services")
    ok("anonymous requests refused (401)")

    admin = Client()
    code, body, hdr = admin.login("anna")
    expect(code, 200, "admin login", body)
    if body["role"] != "admin":
        fail(f"anna role {body}")
    cookie = hdr.get("Set-Cookie", "")
    for attr in ("__Host-alpha_admin=", "Secure", "HttpOnly", "SameSite=Strict"):
        if attr not in cookie:
            fail(f"cookie lacks {attr}: {cookie}")
    ok("PAM login of anna (wheel) -> admin, cookie Secure/HttpOnly/SameSite=Strict")

    code, body, _ = admin.call("GET", "/api/v1/system")
    expect(code, 200, "system", body)
    if body["memory_total"] <= 0 or body["cpus"] < 1:
        fail(f"system {body}")
    ok(f"system: {body['hostname']} os {body['os_version']} kernel {body.get('kernel')}")

    code, body, _ = admin.call("GET", "/api/v1/services")
    expect(code, 200, "services", body)
    states = {u["name"]: u["active_state"] for u in body["services"]}
    if states.get("alpha-admin.service") != "active" or states.get("firewalld.service") != "active":
        fail(f"service states {states}")
    ok(f"services over systemd D-Bus: {sum(1 for s in states.values() if s == 'active')}/{len(states)} active")

    before = sh("systemctl show -p InvocationID --value smb.service")
    code, body, _ = admin.call("POST", "/api/v1/services/smb.service/restart")
    expect(code, 202, "restart smb", body)
    for _ in range(30):
        after = sh("systemctl show -p InvocationID --value smb.service")
        if after and after != before:
            break
        time.sleep(1)
    else:
        fail("smb.service was not restarted")
    ok("restart smb.service via systemd D-Bus + polkit (new InvocationID)")
    code, body, _ = admin.call("POST", "/api/v1/services/sshd.service/restart")
    expect(code, 403, "restart sshd", body)
    code, body, _ = admin.call("POST", "/api/v1/services/smb.service/restart", csrf=False)
    expect(code, 403, "restart without CSRF", body)
    ok("non-restartable unit and missing CSRF token refused (403)")

    code, body, _ = admin.call("GET", "/api/v1/services/alpha-admin.service/journal?lines=20")
    expect(code, 200, "journal", body)
    if not any("login" in e["message"] for e in body["entries"]):
        fail(f"journal lacks the login line: {body['entries'][-3:]}")
    ok(f"journal of alpha-admin.service: {len(body['entries'])} lines")

    code, body, _ = admin.call("GET", "/api/v1/network")
    expect(code, 200, "network", body)
    if not any(d.get("ipv4") for d in body["devices"]):
        fail(f"network {body}")
    ok(f"network over NetworkManager D-Bus: state {body['state']}, {len(body['devices'])} device(s)")
    code, body, _ = admin.call("GET", "/api/v1/network/connections")
    expect(code, 200, "connections", body)
    code, body, _ = admin.call("PUT", "/api/v1/network/wireless", {"enabled": False})
    expect(code, 200, "wireless off", body)
    if sh("nmcli radio wifi") != "disabled":
        fail("wifi radio still enabled")
    admin.call("PUT", "/api/v1/network/wireless", {"enabled": True})
    ok("Wi-Fi radio off/on via NetworkManager D-Bus + polkit")

    code, body, _ = admin.call("GET", "/api/v1/firewall")
    expect(code, 200, "firewall", body)
    opens = {o["name"]: o["open"] for o in body["openings"]}
    if body["default_zone"] != "home" or not opens.get("media") or opens.get("cockpit"):
        fail(f"firewall {body}")
    code, body, _ = admin.call("PUT", "/api/v1/firewall/openings/cockpit", {"open": True})
    expect(code, 200, "open cockpit", body)
    if sh("firewall-cmd --zone=home --query-service=cockpit") != "yes" or \
            sh("firewall-cmd --permanent --zone=home --query-service=cockpit") != "yes":
        fail("cockpit not open in runtime+permanent")
    code, body, _ = admin.call("PUT", "/api/v1/firewall/openings/cockpit", {"open": False})
    expect(code, 200, "close cockpit", body)
    if sh("firewall-cmd --zone=home --query-service=cockpit") != "no":
        fail("cockpit still open")
    ok("firewalld: open/close cockpit in zone home, runtime + permanent")

    code, body, _ = admin.call("GET", "/api/v1/disks")
    expect(code, 200, "disks", body)
    blocks = {b["device"]: b for d in body["drives"] for b in d["blocks"]}
    blocks.update({b["device"]: b for b in body["other"]})
    system = [d["model"] or d["id"] for d in body["drives"] if d["system"]]
    if "/dev/vdb" not in blocks or not blocks["/dev/vdb"]["mountable"]:
        fail(f"disks {json.dumps(body)[:1500]}")
    for dev, b in blocks.items():
        if any(mp in ("/sysroot", "/boot", "/var") for mp in b.get("mount_points") or []) and b["mountable"] or b["unmountable"] and "/sysroot" in (b.get("mount_points") or []):
            fail(f"system block offered: {b}")
    ok(f"UDisks2: {len(body['drives'])} drive(s) (system: {system}), {len(blocks)} block(s), data disk /dev/vdb mountable")
    code, body, _ = admin.call("POST", "/api/v1/disks/vdb/mount")
    expect(code, 200, "mount vdb", body)
    where = body["mount_point"]
    if sh(f"findmnt -n -o SOURCE --target {where}") != "/dev/vdb":
        fail(f"vdb not mounted at {where}")
    code, body, _ = admin.call("POST", "/api/v1/disks/vdb/unmount")
    expect(code, 200, "unmount vdb", body)
    if sh("findmnt -n -S /dev/vdb"):
        fail("vdb still mounted")
    root_dev = sh("findmnt -n -o SOURCE /sysroot || findmnt -n -o SOURCE /").split("[")[0].replace("/dev/", "")
    code, body, _ = admin.call("POST", f"/api/v1/disks/{root_dev}/unmount")
    if code not in (403, 409):
        fail(f"unmount of the system fs {root_dev}: {code} {body}")
    ok(f"UDisks2 mount/unmount of the data disk at {where}; system fs {root_dev} refused ({code})")

    code, body, _ = admin.call("GET", "/api/v1/system/update")
    expect(code, 200, "update proxy", body)
    code, body, _ = admin.call("PUT", "/api/v1/system/update/settings", {"window": "02:00-04:00"})
    expect(code, 200, "update settings", body)
    # With the fake agent (no DIST-009 in the image) also check what it received.
    seen = sh("cat /run/alpha-updated/requests.log 2>/dev/null")
    if seen and "GET /v1/update" not in seen or 'PUT /v1/settings {"window": "02:00-04:00"}' not in seen.replace('":"', '": "'):
        fail(f"update agent saw: {seen}")
    ok("update API proxied to the unix socket /run/alpha-updated/api.sock")

    plain = Client()
    code, body, _ = plain.login("boris")
    expect(code, 403, "non-admin login", body)
    if body.get("error") != "not_admin" or body.get("message") != "Нет прав администратора":
        fail(f"boris: {body}")
    expect(plain.call("GET", "/api/v1/disks")[0], 401, "non-admin without session")
    ok("boris (no wheel) cannot log in: 403 «Нет прав администратора»")

    for user in ("svc", "root", "locked"):
        code, body, _ = Client().login(user)
        expect(code, 401, f"login {user}", body)
    ok("system account, root and locked account cannot log in")

    attacker = Client()
    codes = [attacker.login("anna", "guess")[0] for _ in range(5)]
    code, body, hdr = attacker.login("anna")
    if codes != [401] * 5 or code != 429 or not hdr.get("Retry-After"):
        fail(f"lockout: {codes} then {code} {body}")
    ok(f"5 wrong passwords -> locked (429, Retry-After {hdr.get('Retry-After')} s), even with the right one")


def reboot():
    # anna is locked out from this address by the lockout check above (per
    # account and address): carla, the second admin (group alpha-admin),
    # reboots.
    admin = Client()
    code, body, _ = admin.login("carla")
    expect(code, 200, "carla login", body)
    code, body, _ = admin.call("POST", "/api/v1/power/reboot")
    expect(code, 409, "reboot step 1", body)
    if body.get("error") != "confirmation_required" or not body.get("confirm_token"):
        fail(f"no confirmation token: {body}")
    ok(f"reboot asks for confirmation ({len(body.get('inhibitors', []))} inhibitor(s) reported)")
    code, b2, _ = admin.call("POST", "/api/v1/power/reboot", {"confirm": "wrong"})
    expect(code, 409, "reboot wrong token", b2)
    code, b3, _ = admin.call("POST", "/api/v1/power/reboot")
    code, b4, _ = admin.call("POST", "/api/v1/power/reboot", {"confirm": b3["confirm_token"]})
    expect(code, 202, "reboot confirmed", b4)
    ok("reboot confirmed via logind D-Bus + polkit")


def after():
    anon = Client()
    code, info, _ = anon.call("GET", "/api/v1/info")
    expect(code, 200, "info after reboot", info)
    if info["tls_fingerprint"] != cert_fingerprint():
        fail("fingerprint does not match the certificate on disk")
    print(f"FP {info['tls_fingerprint']}")
    c = Client()
    expect(c.login("carla")[0], 200, "login after reboot")
    ok("alpha-admin up after reboot, login works")


if __name__ == "__main__":
    if sys.argv[1] == "fingerprint":
        print(cert_fingerprint())
    else:
        {"main": main_checks, "reboot": reboot, "after": after}[sys.argv[1]]()
