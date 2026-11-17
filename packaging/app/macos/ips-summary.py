#!/usr/bin/env python3
# SPDX-License-Identifier: MPL-2.0
"""Prints the exception and the crashed thread of macOS crash reports (.ips).

    ips-summary.py REPORT.ips [...]

Used by the release smoke tests (PKG-021) so a crash on the runner shows up
in the job log, not only in the uploaded report.
"""

import json
import sys


def summary(path: str) -> None:
    with open(path, encoding="utf-8") as f:
        head, _, rest = f.read().partition("\n")
    try:
        body = json.loads(rest)
    except ValueError:
        print(f"{path}: not a JSON crash report")
        return
    print(f"== {path}: {json.loads(head).get('name', '?')}")
    print("exception:", body.get("exception"))
    term = body.get("termination") or {}
    print("termination:", term.get("indicator"), "|", " ".join(term.get("reasons") or []))
    info = body.get("asi") or {}
    for k, v in info.items():
        print(f"asi {k}: {' '.join(v) if isinstance(v, list) else v}")
    images = body.get("usedImages") or []
    for t in body.get("threads") or []:
        if not t.get("triggered"):
            continue
        print("crashed thread:", t.get("name") or t.get("queue") or t.get("id"))
        for i, fr in enumerate(t.get("frames") or []):
            img = images[fr["imageIndex"]] if fr.get("imageIndex", -1) < len(images) else {}
            print(f"  {i:2} {img.get('name', '???'):28} {fr.get('symbol', '???')} + {fr.get('symbolLocation', fr.get('imageOffset'))}")
            if i >= 30:
                break


for p in sys.argv[1:]:
    summary(p)
