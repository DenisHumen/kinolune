#!/usr/bin/env python3
# SPDX-License-Identifier: MPL-2.0
"""BUG-001: the memory verdict of a soak run (tools/qa/e2e/run.sh --soak).

Reads the `E2E SOAK cycle N ok|FAIL <film> <secs>s rss=<MB>MB ...` lines of
a run log and prints a Markdown section: RSS per cycle, the least-squares
slope over the last cycles and that slope projected to 50 cycles. Exits 1
when the projection is above the limit — the memory keeps growing with
every open and close of the player instead of levelling off.

    soak_memory.py RUN_LOG

Environment (all optional):
    PA_E2E_SOAK_WINDOW    cycles the slope is taken over (50)
    PA_E2E_SOAK_WARMUP    first cycles never in the window (10): the first
                          plays fill caches, pools and the JIT once
    PA_E2E_SOAK_LIMIT_MB  allowed growth per 50 cycles (50)

Too few cycles after the warm-up (fewer than 5) give a note, not a verdict.

When the lines carry glibc's `inuse=<MB>` (live malloc allocations), the
same slope is checked for it: RSS can grow from fragmentation, `inuse` only
from memory somebody still holds — a leak fails the run even when the RSS
window looks fine.
"""

from __future__ import annotations

import os
import re
import sys

CYCLE = re.compile(r"^E2E SOAK cycle (\d+) \S+ .*?\brss=(\d+)MB(.*)$")
FIELD = re.compile(r"\b(threads|heap|inuse|free)=(\d+)")


def slope(points: list[tuple[int, int]]) -> float:
    """Least-squares MB per cycle."""
    n = len(points)
    mx = sum(x for x, _ in points) / n
    my = sum(y for _, y in points) / n
    den = sum((x - mx) ** 2 for x, _ in points)
    if den == 0:
        return 0.0
    return sum((x - mx) * (y - my) for x, y in points) / den


def verdict(
    cycles: list[tuple[int, int]],
    window: int,
    warmup: int,
    limit: float,
    what: str = "RSS",
):
    """(lines, failed) for the `what` samples `cycles` [(cycle, MB)]."""
    tail = [c for c in cycles if c[0] > warmup][-window:]
    if len(tail) < 5:
        return [
            f"- not enough cycles after the warm-up of {warmup} for a verdict "
            f"({len(tail)})"
        ], False
    s = slope(tail)
    projected = s * 50
    growth = tail[-1][1] - tail[0][1]
    failed = projected > limit
    return [
        f"- window: cycles {tail[0][0]}–{tail[-1][0]} ({len(tail)}), "
        f"{what} {tail[0][1]} → {tail[-1][1]} MB ({growth:+d} MB)",
        f"- {what} slope: {s:.2f} MB/cycle → {projected:.0f} MB per 50 cycles "
        f"(limit {limit:.0f} MB): {'FAIL' if failed else 'ok'}",
    ], failed


def main(argv: list[str]) -> int:
    if len(argv) != 2:
        print(__doc__, file=sys.stderr)
        return 64
    window = int(os.environ.get("PA_E2E_SOAK_WINDOW", "50"))
    warmup = int(os.environ.get("PA_E2E_SOAK_WARMUP", "10"))
    limit = float(os.environ.get("PA_E2E_SOAK_LIMIT_MB", "50"))
    cycles: list[tuple[int, int]] = []
    inuse: list[tuple[int, int]] = []
    native: list[str] = []
    with open(argv[1], encoding="utf-8", errors="replace") as log:
        for line in log:
            m = CYCLE.match(line.strip())
            if not m:
                continue
            cycles.append((int(m.group(1)), int(m.group(2))))
            fields = dict(FIELD.findall(m.group(3)))
            if "inuse" in fields:
                inuse.append((int(m.group(1)), int(fields["inuse"])))
            if fields:
                native.append(
                    f"{m.group(1)}: rss {m.group(2)}"
                    + "".join(f" {k} {v}" for k, v in fields.items())
                )
    print()
    print("## Memory (BUG-001)")
    print()
    if not cycles:
        print("- no soak cycles in the log")
        return 1
    print("- RSS per cycle, MB: " + " ".join(str(mb) for _, mb in cycles))
    if native:
        print("- native (threads, malloc heap/in use/free, MB):")
        for n in native:
            print(f"  - {n}")
    lines, failed = verdict(cycles, window, warmup, limit)
    if len(inuse) == len(cycles):
        more, leaked = verdict(inuse, window, warmup, limit, "malloc in use")
        if not more[0].startswith("- not enough"):
            lines += more
            failed = failed or leaked
    print("\n".join(lines))
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
