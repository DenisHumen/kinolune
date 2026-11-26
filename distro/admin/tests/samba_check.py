#!/usr/bin/python3
# SPDX-License-Identifier: MPL-2.0
"""Samba checks of Kinolune OS (DIST-018) against the real alpha-admin-priv.

Runs as root in the container of samba-container.sh (python3 stdlib only):
imports the installer's Samba password, starts smbd and the helper socket,
then drives the helper like the web daemon does and checks every effect with
smbclient, pdbedit and the file system.
"""

import glob
import json
import os
import socket
import struct
import subprocess
import sys
import time

SOCK = "/run/alpha-admin/priv.sock"
HOST = "//127.0.0.1"
ANNA = ("anna", "Anna-pass-1")


def ok(msg):
    print(f"OK {msg}", flush=True)


def fail(msg):
    print(f"FAIL {msg}", flush=True)
    sys.exit(1)


def sh(cmd, stdin=""):
    r = subprocess.run(cmd, shell=True, input=stdin, capture_output=True, text=True)
    return r.returncode, (r.stdout + r.stderr).strip()


def md4(data):
    """RFC 1320 MD4 (hashlib's is gone with OpenSSL 3), for the NT hash."""
    def f(x, y, z): return x & y | ~x & z
    def g(x, y, z): return x & y | x & z | y & z
    def h(x, y, z): return x ^ y ^ z
    def rol(v, s): return (v << s | v >> (32 - s)) & 0xFFFFFFFF
    msg = data + b"\x80" + b"\0" * ((55 - len(data)) % 64) + struct.pack("<Q", len(data) * 8)
    a0, b0, c0, d0 = 0x67452301, 0xEFCDAB89, 0x98BADCFE, 0x10325476
    for off in range(0, len(msg), 64):
        x = struct.unpack("<16I", msg[off:off + 64])
        a, b, c, d = a0, b0, c0, d0
        for i in (0, 4, 8, 12):
            a = rol((a + f(b, c, d) + x[i]) & 0xFFFFFFFF, 3)
            d = rol((d + f(a, b, c) + x[i + 1]) & 0xFFFFFFFF, 7)
            c = rol((c + f(d, a, b) + x[i + 2]) & 0xFFFFFFFF, 11)
            b = rol((b + f(c, d, a) + x[i + 3]) & 0xFFFFFFFF, 19)
        for i in (0, 1, 2, 3):
            a = rol((a + g(b, c, d) + x[i] + 0x5A827999) & 0xFFFFFFFF, 3)
            d = rol((d + g(a, b, c) + x[i + 4] + 0x5A827999) & 0xFFFFFFFF, 5)
            c = rol((c + g(d, a, b) + x[i + 8] + 0x5A827999) & 0xFFFFFFFF, 9)
            b = rol((b + g(c, d, a) + x[i + 12] + 0x5A827999) & 0xFFFFFFFF, 13)
        for i in (0, 2, 1, 3):
            a = rol((a + h(b, c, d) + x[i] + 0x6ED9EBA1) & 0xFFFFFFFF, 3)
            d = rol((d + h(a, b, c) + x[i + 8] + 0x6ED9EBA1) & 0xFFFFFFFF, 9)
            c = rol((c + h(d, a, b) + x[i + 4] + 0x6ED9EBA1) & 0xFFFFFFFF, 11)
            b = rol((b + h(c, d, a) + x[i + 12] + 0x6ED9EBA1) & 0xFFFFFFFF, 15)
        a0, b0, c0, d0 = [(v + w) & 0xFFFFFFFF for v, w in zip((a0, b0, c0, d0), (a, b, c, d))]
    return struct.pack("<4I", a0, b0, c0, d0)


def helper(req):
    s = socket.socket(socket.AF_UNIX)
    s.settimeout(60)
    s.connect(SOCK)
    s.sendall(json.dumps(req).encode() + b"\n")
    data = b""
    while not data.endswith(b"\n"):
        chunk = s.recv(65536)
        if not chunk:
            break
        data += chunk
    s.close()
    return json.loads(data)


def smb(share, user=None, cmd="ls", extra=""):
    auth = f"-U '{user[0]}%{user[1]}'" if user else "-N"
    return sh(f"smbclient {HOST}/{share} {auth} -m SMB3 {extra} -c '{cmd}'")


def expect_ok(r, what):
    if not r.get("ok"):
        fail(f"{what}: {r}")


def main():
    assert md4(b"abc").hex() == "a448017aaf21d8525fc10ae87aa6729d"

    # --- first boot: render, import the installer's Samba password -------------------
    uid = sh("id -u anna")[1]
    nt = md4(ANNA[1].encode("utf-16-le")).hex().upper()
    with open("/var/lib/alpha/samba/import.smbpasswd", "w") as f:
        f.write(f"anna:{uid}:{'X' * 32}:{nt}:[U          ]:LCT-{int(time.time()):08X}:\n")
    os.chmod("/var/lib/alpha/samba/import.smbpasswd", 0o600)
    code, out = sh("/usr/libexec/alpha/alpha-admin-priv render-samba")
    if code != 0:
        fail(f"render-samba: {out}")
    if os.path.exists("/var/lib/alpha/samba/import.smbpasswd"):
        fail("import file left behind")
    if "anna:" not in sh("pdbedit -L")[1]:
        fail("installer's Samba password not imported")
    for d in ("/var/srv/users/anna", "/var/srv/timemachine/anna"):
        st = os.stat(d)
        if st.st_uid != int(uid) or st.st_mode & 0o777 != 0o700:
            fail(f"{d}: owner {st.st_uid} mode {oct(st.st_mode)}")
    code, out = sh("testparm -s 2>/dev/null")
    guest = sh("testparm -s --parameter-name='map to guest' 2>/dev/null")[1]
    if "server string = Kinolune OS (%h)" not in out or guest != "Never" or "restrict anonymous = 2" not in out:
        fail(f"testparm (map to guest = {guest}):\n{out}")
    ok("render-samba: config rendered, installer's password imported, folders 0700, testparm OK")

    sh("smbd -D")
    os.makedirs(os.path.dirname(SOCK), exist_ok=True)
    subprocess.Popen(["systemd-socket-activate", "-l", SOCK, "--accept", "--inetd",
                      "/usr/libexec/alpha/alpha-admin-priv"], stdout=subprocess.DEVNULL, stderr=open("/tmp/priv.log", "w"))
    for _ in range(40):
        if os.path.exists(SOCK) and sh("ss -ltn")[1].count(":445 "):
            break
        time.sleep(0.25)
    else:
        fail("smbd or the helper socket did not come up")
    if ":139 " in sh("ss -ltn")[1]:
        fail("smbd listens on 139 (NetBIOS)")
    ok("smbd on 445 only; helper on its socket")

    # --- anonymous: nothing -----------------------------------------------------------
    code, out = sh(f"smbclient -N -L {HOST} -m SMB3")
    if code == 0:
        fail(f"anonymous share list works:\n{out}")
    for share in ("Media", "Users", "TimeMachine"):
        code, out = smb(share)
        if code == 0:
            fail(f"anonymous access to {share}:\n{out}")
    ok(f"anonymous: share list and Media/Users/TimeMachine refused ({out.splitlines()[-1]})")

    # --- the installer's admin with its password ----------------------------------------
    code, out = sh(f"smbclient -L {HOST} -U 'anna%{ANNA[1]}' -m SMB3")
    if code != 0 or "Media" not in out or "Kinolune OS" not in out:
        fail(f"anna share list:\n{out}")
    with open("/tmp/film.mkv", "w") as f:
        f.write("film")
    code, out = smb("Media", ANNA, "put /tmp/film.mkv film.mkv")
    st = os.stat("/var/srv/media/film.mkv")
    if code != 0 or st.st_mode & 0o777 != 0o664 or st.st_gid != int(sh("getent group alpha-users | cut -d: -f3")[1]):
        fail(f"anna write Media: {code} {out} {oct(st.st_mode)}")
    code, out = smb("Users", ANNA, "put /tmp/film.mkv anna/private.txt")
    if code != 0 or not os.path.exists("/var/srv/users/anna/private.txt"):
        fail(f"anna write Users/anna: {out}")
    ok("anna (installer's admin): share list shows 'Kinolune OS', writes Media (0664 alpha-users) and Users/anna")

    # --- users through the helper -------------------------------------------------------
    r = helper({"op": "user_create", "actor": "anna", "actor_password": "wrong", "user": "boris", "password": "Boris-pass-1"})
    if r.get("error") != "denied":
        fail(f"create with a wrong admin password: {r}")
    r = helper({"op": "user_create", "actor": "anna", "actor_password": ANNA[1], "user": "boris", "full_name": "Boris", "password": "Boris-pass-1", "admin": False})
    expect_ok(r, "create boris")
    users = {u["name"]: u for u in helper({"op": "users"})["users"]}
    if not users["boris"]["samba"] or users["boris"]["admin"] or not users["boris"]["nas"] or not users["anna"]["admin"]:
        fail(f"users: {users}")
    if "alpha-users" not in sh("id -nG boris")[1]:
        fail("boris not in alpha-users")
    ok("helper: wrong admin password refused; boris created (alpha-users, Samba, not admin)")

    BORIS = ("boris", "Boris-pass-1")
    code, out = smb("Users", BORIS, "ls")
    if code != 0 or "boris" not in out or "anna" in out:
        fail(f"boris sees in Users:\n{out}")
    code, _ = smb("Users", BORIS, "cd anna")
    if code == 0 and smb("Users", BORIS, "get anna/private.txt /tmp/x")[0] == 0:
        fail("boris reads anna's folder")
    code, out = smb("Users", BORIS, "put /tmp/film.mkv boris/mine.txt")
    if code != 0:
        fail(f"boris write Users/boris: {out}")
    code, out = smb("Media", BORIS, "put /tmp/film.mkv film2.mkv")
    if code != 0:
        fail(f"boris write Media: {out}")
    code, out = smb("TimeMachine", BORIS, "put /tmp/film.mkv backup.sparsebundle")
    if code != 0 or not os.path.exists("/var/srv/timemachine/boris/backup.sparsebundle"):
        fail(f"boris TimeMachine: {out}")
    ok("boris: sees only his folder in Users, cannot read anna's, writes Users/boris, Media, TimeMachine/boris")

    # Password changed in the console: the shares follow at the next sync (log-in).
    sh("chpasswd", "boris:Boris-pass-2\n")
    r = helper({"op": "samba_sync", "user": "boris", "password": "Boris-pass-1"})
    if r.get("error") != "denied":
        fail(f"sync with the old password: {r}")
    expect_ok(helper({"op": "samba_sync", "user": "boris", "password": "Boris-pass-2"}), "sync")
    if smb("Media", ("boris", "Boris-pass-2"))[0] != 0 or smb("Media", BORIS)[0] == 0:
        fail("Samba password not synced")
    ok("samba_sync: checks the password with PAM, then the new password works and the old one not")

    expect_ok(helper({"op": "user_password", "actor": "anna", "actor_password": ANNA[1], "user": "boris", "password": "Boris-pass-3"}), "password")
    BORIS = ("boris", "Boris-pass-3")
    if smb("Media", BORIS)[0] != 0:
        fail("password set by the admin does not open the shares")
    ok("user_password: OS and Samba password changed together")

    # --- guest read on Media ------------------------------------------------------------
    expect_ok(helper({"op": "sharing_set", "settings": {"media_guest_read": True}}), "guest on")
    time.sleep(1)
    code, out = smb("Media")
    if code != 0 or "film.mkv" not in out:
        fail(f"guest cannot read Media:\n{out}")
    code, out = smb("Media", None, "put /tmp/film.mkv guest.mkv")
    if code == 0 or os.path.exists("/var/srv/media/guest.mkv"):
        fail(f"guest wrote to Media:\n{out}")
    for share in ("Users", "TimeMachine"):
        if smb(share)[0] == 0:
            fail(f"guest reaches {share}")
    if smb("Media", BORIS, "put /tmp/film.mkv film3.mkv")[0] != 0:
        fail("boris cannot write Media with guest read on")
    ok("guest read on: anonymous reads Media, cannot write; Users/TimeMachine still refused; users still write")

    expect_ok(helper({"op": "sharing_set", "settings": {"media_guest_read": False, "time_machine_off": True}}), "guest off")
    time.sleep(1)
    if smb("Media")[0] == 0:
        fail("guest still reads Media")
    code, out = sh(f"smbclient -L {HOST} -U 'anna%{ANNA[1]}' -m SMB3")
    if "TimeMachine" in out:
        fail("TimeMachine still listed")
    expect_ok(helper({"op": "sharing_set", "settings": {}}), "defaults")
    ok("guest read off again, TimeMachine share off and on")

    # --- administrators and deletion ----------------------------------------------------
    r = helper({"op": "user_admin", "actor": "anna", "actor_password": ANNA[1], "user": "anna", "admin": False})
    if r.get("error") != "conflict":
        fail(f"anna demoted herself: {r}")
    r = helper({"op": "user_delete", "actor": "boris", "actor_password": BORIS[1], "user": "anna"})
    if r.get("error") != "denied":
        fail(f"non-admin deleted: {r}")
    expect_ok(helper({"op": "user_delete", "actor": "anna", "actor_password": ANNA[1], "user": "boris"}), "delete boris")
    if "boris" in sh("pdbedit -L")[1] or sh("id boris")[0] == 0:
        fail("boris still exists")
    kept = glob.glob("/var/srv/users/deleted-boris-*")
    if len(kept) != 1 or os.stat(kept[0]).st_uid != 0 or not os.path.exists(kept[0] + "/mine.txt"):
        fail(f"boris's files: {kept}")
    if smb("Media", BORIS)[0] == 0:
        fail("deleted boris still logs in")
    ok("last admin protected, non-admin refused; boris deleted: no account, files kept as root:wheel " + os.path.basename(kept[0]))


main()
