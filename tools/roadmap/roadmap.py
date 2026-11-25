#!/usr/bin/env python3
# SPDX-License-Identifier: MPL-2.0
"""Lint docs/ROADMAP.md and list tasks that are ready to start.

Standard library only (Python >= 3.10), so it runs on every CI runner and dev
machine without a virtualenv. Task format: docs/ROADMAP.md §0.1.

    python3 tools/roadmap/roadmap.py lint [--file PATH]
    python3 tools/roadmap/roadmap.py next [WORKSTREAM ...] [--offline] [--json]
    python3 tools/roadmap/roadmap.py progress [--platforms] [--deep]
    python3 tools/roadmap/roadmap.py page --out DIR [--prs FILE]
"""

from __future__ import annotations

import argparse
import json
import re
import shutil
import subprocess
import sys
from dataclasses import dataclass, field
from pathlib import Path

DEFAULT_FILE = Path(__file__).resolve().parents[2] / "docs" / "ROADMAP.md"

ID_RE = r"[A-Z]+(?:-[A-Z]+)*-\d{3}"
TASK_RE = re.compile(
    rf"^- \[(?P<mark>[ xX])\] (?P<strike>~~)?\*\*(?P<id>{ID_RE})\*\*\s*(?P<rest>.*)$"
)
# Anything that looks like a task line, used to report malformed ones.
TASK_LIKE_RE = re.compile(r"^- \[[^\]]*\]")
FIELD_RE = re.compile(r"^\s+- (?P<key>Что|deps|done when):\s*(?P<value>.*)$")
SECTION_RE = re.compile(rf"^###\s+[\d.]+\s+(?P<ws>[A-Z]+(?:-[A-Z]+)*)\s+—")
H2_RE = re.compile(r"^##\s")
DEPS_RE = re.compile(
    r"^(?P<deps>.+?) · size: (?P<size>[SML]) · parallel: (?P<parallel>yes|no)"
    r" · веха: M(?P<milestone>\d+) · ref: (?P<ref>.+)$"
)
BLOCKED_RE = re.compile(rf"\(blocked: (?P<ids>{ID_RE}(?:,\s*{ID_RE})*)\)")
PR_ID_RE = re.compile(rf"\[({ID_RE})\]")
NO_DEPS = "—"
SUMMARY_HEADING = "Сводка по количеству задач"
SUMMARY_ROW_RE = re.compile(r"\|\s*([A-Z]+(?:-[A-Z]+)*)\s*\|\s*(\d+)\s*(?=\|)")


@dataclass
class Task:
    id: str
    line: int
    done: bool
    cancelled: bool
    owner: bool
    title: str
    workstream: str
    section: str | None = None
    deps: list[str] = field(default_factory=list)
    blocked_by: list[str] = field(default_factory=list)
    size: str | None = None
    parallel: str | None = None
    milestone: int | None = None
    fields: dict[str, list[str]] = field(default_factory=dict)


@dataclass
class Report:
    errors: list[str] = field(default_factory=list)
    warnings: list[str] = field(default_factory=list)

    def error(self, line: int, msg: str) -> None:
        self.errors.append(f"line {line}: {msg}")

    def warn(self, line: int, msg: str) -> None:
        self.warnings.append(f"line {line}: {msg}")


def workstream_of(task_id: str) -> str:
    return task_id.rsplit("-", 1)[0]


def parse(text: str, report: Report | None = None) -> list[Task]:
    """Parse task entries, skipping fenced code blocks."""
    report = report if report is not None else Report()
    tasks: list[Task] = []
    current: Task | None = None
    section: str | None = None
    in_fence = False

    for lineno, line in enumerate(text.splitlines(), start=1):
        if line.lstrip().startswith("```"):
            in_fence = not in_fence
            continue
        if in_fence:
            continue

        if line.startswith("#"):
            current = None
            m = SECTION_RE.match(line)
            if m:
                section = m.group("ws")
            elif H2_RE.match(line):
                section = None
            continue

        m = TASK_RE.match(line)
        if m:
            rest = m.group("rest")
            cancelled = m.group("strike") is not None
            title = rest.split("~~", 1)[0].strip() if cancelled else rest.strip()
            blocked = BLOCKED_RE.search(rest)
            current = Task(
                id=m.group("id"),
                line=lineno,
                done=m.group("mark") in "xX",
                cancelled=cancelled,
                owner="(owner)" in rest,
                title=title,
                workstream=workstream_of(m.group("id")),
                section=section,
                blocked_by=re.split(r",\s*", blocked.group("ids")) if blocked else [],
            )
            tasks.append(current)
            continue
        if TASK_LIKE_RE.match(line):
            report.error(lineno, f"malformed task line: {line.strip()[:60]}")
            current = None
            continue

        f = FIELD_RE.match(line)
        if f and current is not None:
            current.fields.setdefault(f.group("key"), []).append(f.group("value"))
            if f.group("key") == "deps" and len(current.fields["deps"]) == 1:
                _parse_deps(current, f.group("value"), lineno, report)
            continue
        if line.strip() and not line.startswith(" "):
            current = None

    return tasks


def _parse_deps(task: Task, value: str, lineno: int, report: Report) -> None:
    m = DEPS_RE.match(value.strip())
    if not m:
        report.error(
            lineno,
            f"{task.id}: deps line must be "
            "'deps: … · size: S|M|L · parallel: yes|no · веха: MN · ref: …'",
        )
        return
    task.size = m.group("size")
    task.parallel = m.group("parallel")
    task.milestone = int(m.group("milestone"))
    deps = m.group("deps").strip()
    if deps == NO_DEPS:
        return
    for dep in re.split(r",\s*", deps):
        if re.fullmatch(ID_RE, dep):
            task.deps.append(dep)
        else:
            report.error(lineno, f"{task.id}: bad dependency ID {dep!r}")


def find_cycles(tasks: dict[str, Task]) -> list[list[str]]:
    """Return one representative path per dependency cycle (iterative DFS)."""
    white, grey, black = 0, 1, 2
    color = dict.fromkeys(tasks, white)
    cycles: list[list[str]] = []
    for root in sorted(tasks):
        if color[root] != white:
            continue
        stack: list[tuple[str, int]] = [(root, 0)]
        path: list[str] = []
        while stack:
            node, idx = stack.pop()
            if idx == 0:
                color[node] = grey
                path.append(node)
            deps = [d for d in tasks[node].deps + tasks[node].blocked_by if d in tasks]
            if idx < len(deps):
                stack.append((node, idx + 1))
                dep = deps[idx]
                if color[dep] == grey:
                    cycles.append(path[path.index(dep):] + [dep])
                elif color[dep] == white:
                    stack.append((dep, 0))
            else:
                color[node] = black
                path.pop()
    return cycles


def lint(text: str) -> tuple[list[Task], Report]:
    report = Report()
    tasks = parse(text, report)
    by_id: dict[str, Task] = {}

    for t in tasks:
        if t.id in by_id:
            report.error(t.line, f"duplicate ID {t.id} (first at line {by_id[t.id].line})")
            continue
        by_id[t.id] = t

    for t in tasks:
        if t.section is not None and t.workstream != t.section:
            report.error(t.line, f"{t.id} is listed under workstream {t.section}")
        if t.cancelled:
            if t.done:
                report.error(t.line, f"{t.id}: a cancelled task stays '- [ ]'")
            continue
        if not t.title:
            report.error(t.line, f"{t.id}: empty title")
        for key in ("Что", "deps", "done when"):
            values = t.fields.get(key, [])
            if not values:
                report.error(t.line, f"{t.id}: missing '- {key}:' line")
            elif len(values) > 1:
                if len(set(values)) == 1:
                    report.warn(t.line, f"{t.id}: '- {key}:' line is repeated")
                else:
                    report.error(t.line, f"{t.id}: conflicting '- {key}:' lines")
        for dep in t.deps + t.blocked_by:
            if dep == t.id:
                report.error(t.line, f"{t.id} depends on itself")
            elif dep not in by_id:
                report.error(t.line, f"{t.id}: unknown dependency {dep}")
            elif by_id[dep].cancelled:
                report.error(t.line, f"{t.id}: depends on cancelled task {dep}")
            elif t.done and not by_id[dep].done:
                msg = f"{t.id} is ticked but dependency {dep} is open"
                # The owner may decide ahead of the research a decision waits for.
                if t.owner:
                    report.warn(t.line, msg + " (owner decision taken early)")
                else:
                    report.error(t.line, msg)
        for dep in t.deps:
            d = by_id.get(dep)
            if (
                d is not None
                and d.milestone is not None
                and t.milestone is not None
                and d.milestone > t.milestone
            ):
                report.error(
                    t.line,
                    f"{t.id} (M{t.milestone}) depends on {dep} from a later "
                    f"milestone M{d.milestone}",
                )

    _check_summary(text, tasks, report)

    for cycle in find_cycles(by_id):
        report.error(by_id[cycle[0]].line, "dependency cycle: " + " -> ".join(cycle))

    return tasks, report


def _check_summary(text: str, tasks: list[Task], report: Report) -> None:
    """Compare the per-workstream table in the summary section with real counts."""
    in_summary = False
    counts: dict[str, int] = {}
    for t in tasks:
        counts[t.workstream] = counts.get(t.workstream, 0) + 1
    for lineno, line in enumerate(text.splitlines(), start=1):
        if line.startswith("## "):
            in_summary = SUMMARY_HEADING in line
            continue
        if not in_summary:
            continue
        for ws, n in SUMMARY_ROW_RE.findall(line):
            if counts.get(ws, 0) != int(n):
                report.warn(
                    lineno, f"summary says {ws} has {n} tasks, found {counts.get(ws, 0)}"
                )


def ready_tasks(
    tasks: list[Task], workstreams: list[str] | None = None, taken: set[str] | None = None
) -> list[Task]:
    """Open, non-owner tasks whose deps (and blockers) are all ticked."""
    by_id = {t.id: t for t in tasks}
    wanted = {w.upper() for w in workstreams} if workstreams else None
    taken = taken or set()
    out = []
    for t in tasks:
        if t.done or t.cancelled or t.owner or t.id in taken:
            continue
        if wanted is not None and t.workstream not in wanted:
            continue
        if all(by_id.get(d) is not None and by_id[d].done for d in t.deps + t.blocked_by):
            out.append(t)
    return out


def open_pr_ids() -> set[str] | None:
    """IDs in titles of open PRs, or None when `gh` is unavailable."""
    if shutil.which("gh") is None:
        return None
    try:
        res = subprocess.run(
            ["gh", "pr", "list", "--state", "open", "--limit", "500", "--json", "title"],
            capture_output=True,
            text=True,
            check=True,
            timeout=60,
        )
    except (subprocess.SubprocessError, OSError):
        return None
    ids: set[str] = set()
    for pr in json.loads(res.stdout or "[]"):
        ids.update(PR_ID_RE.findall(pr.get("title", "")))
    return ids


def cmd_lint(args: argparse.Namespace) -> int:
    tasks, report = lint(Path(args.file).read_text(encoding="utf-8"))
    for w in report.warnings:
        print(f"warning: {w}", file=sys.stderr)
    for e in report.errors:
        print(f"error: {e}", file=sys.stderr)
    done = sum(t.done for t in tasks)
    print(
        f"roadmap lint: {len(tasks)} tasks ({done} done), "
        f"{len(report.errors)} errors, {len(report.warnings)} warnings"
    )
    return 1 if report.errors else 0


def cmd_next(args: argparse.Namespace) -> int:
    tasks, report = lint(Path(args.file).read_text(encoding="utf-8"))
    if report.errors:
        print("roadmap has lint errors; run `just roadmap lint`", file=sys.stderr)
        return 1
    taken: set[str] = set()
    if not args.offline:
        ids = open_pr_ids()
        if ids is None:
            print(
                "warning: `gh` unavailable, tasks with open PRs are not filtered "
                "(use --offline to silence)",
                file=sys.stderr,
            )
        else:
            taken = ids
    ready = ready_tasks(tasks, args.workstream, taken)
    if args.size:
        ready = [t for t in ready if t.size in args.size]
    if args.json:
        rows = [
            {
                "id": t.id,
                "workstream": t.workstream,
                "title": t.title,
                "size": t.size,
                "parallel": t.parallel,
                "milestone": f"M{t.milestone}" if t.milestone is not None else None,
                "deps": t.deps,
            }
            for t in ready
        ]
        print(json.dumps(rows, ensure_ascii=False, indent=2))
        return 0
    for t in ready:
        print(f"{t.id:<14} M{t.milestone} {t.size} par:{t.parallel:<3} {t.title}")
    print(f"{len(ready)} ready tasks", file=sys.stderr)
    return 0


#: Milestones of experimental work (owner, 2026-11-17): Apple Watch and
#: visionOS (M11), the distro TV shell (M12). They are counted in their
#: own bar so the main bar shows when the system itself is done.
EXPERIMENTAL_MILESTONES = frozenset({11, 12})

#: The Kinolune+ subscription launch (M13, owner 2026-11-19, ADR-0136): only
#: after the main plan is done and there is an audience, so it has its own
#: bar too. The preparation tasks (SUB-002…SUB-008) are in M10 and count in
#: the main bar.
SUBSCRIPTION_MILESTONES = frozenset({13})

#: Physical discs (M14, owner 2026-11-19, ADR-0139): playing Blu-ray/DVD
#: from a drive, copying discs, the AACS licence — last of all, after the
#: main plan, the redesign and the launch. Files of Blu-ray stay in the main plan.
DISC_MILESTONES = frozenset({14})

#: Kinolune Social (M15, owner 2026-11-22): public profiles, friends, their
#: watching and achievements, E2E file transfer between friends — on the
#: cloud account of M13, after the subscription, so it has its own bar.
SOCIAL_MILESTONES = frozenset({15})

#: The app ecosystem (M16, owner 2026-11-23): the server announces itself,
#: provider apps (Universal Video Downloader, Kinolune desktop) pair once and
#: send downloads to it. Started only on the owner's command; own bar.
ECOSYSTEM_MILESTONES = frozenset({16})

#: Milestones outside the main ("Система") bar.
SIDE_MILESTONES = (
    EXPERIMENTAL_MILESTONES | SUBSCRIPTION_MILESTONES | DISC_MILESTONES | SOCIAL_MILESTONES | ECOSYSTEM_MILESTONES
)

#: Found bugs (owner 2026-11-19): the BUG workstream is tracked as tasks but
#: has its own bar, so fixing bugs does not inflate the system bar.
BUG_WORKSTREAMS = frozenset({"BUG"})
BUG_PRIORITY_RE = re.compile(r"\[(P[0-2])\]")

#: Relative effort of a task size for the "по объёму" figure: an L task is
#: about two to three M tasks, an M task about three S tasks.
SIZE_WEIGHT = {"S": 1, "M": 3, "L": 8}
SIZE_LABEL = {
    "S": "Маленькие (S)",
    "M": "Средние  (M)",
    "L": "Большие  (L)",
}

#: Themes of the main bar (owner, 2026-11-19): workstreams grouped so the
#: report shows where the system stands, area by area.
THEMES: list[tuple[str, tuple[str, ...]]] = [
    ("Ядро и источники", ("CORE", "NET", "LIB")),
    ("Распознавание", ("META",)),
    ("Плеер", ("PLAY", "PLAY-AND", "PLAY-DSK", "PLAY-APL")),
    ("Интерфейс и ТВ", ("UI", "TV", "DSN")),
    ("Синхронизация и сервер", ("SYNC", "SRV")),
    ("Музыка", ("MUS",)),
    ("Twitch", ("TWI",)),
    ("Дистрибутив", ("DIST",)),
    ("Сборка и релизы", ("PKG", "CI", "REPO")),
    ("Качество и тесты", ("QA",)),
    ("Документы и право", ("DOC", "LEGAL", "SITE", "COMM", "DEC")),
    ("Заготовки подписки", ("SUB",)),
]

#: The manual test catalog (QA-014): rows of features × platforms.
DEFAULT_CATALOG = Path(__file__).resolve().parents[2] / "docs" / "qa" / "test-catalog.md"

#: Catalog cell marks: verified on a real device, covered only by automated
#: tests, not tested yet, failed; "—" means not applicable.
CATALOG_DONE = "✅"
CATALOG_AUTO = "🤖"
CATALOG_OPEN = ("⬜", "❌")


def bar(done: int, total: int, width: int = 20) -> str:
    """`▓▓▓░░░ 45% (90/200)`."""
    pct = 0 if total == 0 else round(100 * done / total)
    filled = 0 if total == 0 else round(width * done / total)
    return f"{'▓' * filled}{'░' * (width - filled)} {pct}% ({done}/{total})"


def catalog_counts(text: str) -> tuple[int, int, int, dict[str, tuple[int, int]]]:
    """Verified, automated-only and total cells of the catalog tables, and
    verified/total per platform column. A table counts when its header has
    an `ID` column; `—` cells are not applicable."""
    verified = auto = total = 0
    per: dict[str, list[int]] = {}
    header: list[str] | None = None
    for line in text.splitlines():
        if not line.startswith("|"):
            header = None
            continue
        cells = [c.strip() for c in line.strip().strip("|").split("|")]
        if header is None:
            header = cells if cells and cells[0] == "ID" else None
            continue
        if set("".join(cells)) <= set("-: "):
            continue
        for name, cell in zip(header[2:], cells[2:]):
            if not cell or cell == "—":
                continue
            p = per.setdefault(name, [0, 0])
            total += 1
            p[1] += 1
            if cell.startswith(CATALOG_DONE):
                verified += 1
                p[0] += 1
            elif cell.startswith(CATALOG_AUTO):
                auto += 1
    return verified, auto, total, {k: (v[0], v[1]) for k, v in per.items()}


def weighted(tasks: list[Task]) -> tuple[int, int]:
    """Done and total effort of `tasks` by SIZE_WEIGHT (unknown size = M)."""
    w = [SIZE_WEIGHT.get(t.size or "M", 3) for t in tasks]
    return sum(x for x, t in zip(w, tasks) if t.done), sum(w)


def pct(done: int, total: int) -> int:
    return 0 if total == 0 else round(100 * done / total)


def done_ids_by_day(path: Path, since_days: int) -> list[tuple[str, set[str]]]:
    """(date, ticked task IDs) at the last commit of each day that touched the
    roadmap in the last `since_days` days, oldest first; [] without git."""
    if shutil.which("git") is None:
        return []
    try:
        res = subprocess.run(
            ["git", "log", "--first-parent", "--format=%H %ad", "--date=short",
             f"--since={since_days + 1} days ago", "--", path.name],
            capture_output=True, text=True, check=True, timeout=60, cwd=path.parent,
        )
    except (subprocess.SubprocessError, OSError):
        return []
    last: dict[str, str] = {}
    for line in res.stdout.splitlines():  # newest first: keep the first per day
        sha, day = line.split()
        last.setdefault(day, sha)
    out = []
    for day in sorted(last):
        try:
            text = subprocess.run(
                ["git", "show", f"{last[day]}:./{path.name}"],
                capture_output=True, text=True, check=True, timeout=60, cwd=path.parent,
            ).stdout
        except (subprocess.SubprocessError, OSError):
            continue
        out.append((day, {t.id for t in parse(text) if t.done}))
    return out


def forecast(tasks: list[Task], history: list[tuple[str, set[str]]], today: str | None = None) -> str | None:
    """«≈ N дн. (до D) · скорость …» for the main bar: remaining effort of the
    system divided by the effort ticked per day over `history`."""
    import datetime as dt

    side = SIDE_MILESTONES
    main = {t.id: t for t in tasks if not t.cancelled and t.milestone not in side}
    if len(history) < 2:
        return None
    w = lambda ids: sum(SIZE_WEIGHT.get(main[i].size or "M", 3) for i in ids if i in main)
    first_day, first_ids = history[0]
    last_day, last_ids = history[-1]
    d0 = dt.date.fromisoformat(first_day)
    d1 = dt.date.fromisoformat(today or last_day)
    days = max((d1 - d0).days, 1)
    gained = w(last_ids) - w(first_ids)
    n_gained = len([i for i in last_ids - first_ids if i in main])
    if gained <= 0:
        return f"прогноз: нет данных о скорости за {days} дн."
    done_w = w({i for i, t in main.items() if t.done})
    total_w = w(set(main))
    left = max(total_w - done_w, 0)
    eta_days = left / (gained / days)
    end = d1 + dt.timedelta(days=round(eta_days))
    return (
        f"≈ {round(eta_days)} дн. (≈ {end.strftime('%d.%m.%Y')}) при скорости "
        f"{n_gained / days:.1f} задач/день за {days} дн."
    )


def tree_lines(
    tasks: list[Task],
    detail: bool = True,
    deep: bool = False,
    catalog: tuple[int, int, int, dict[str, tuple[int, int]]] | None = None,
    platforms: bool = False,
    eta: str | None = None,
) -> list[str]:
    """Progress as a tree of bars (owner, 2026-11-19): the project at the
    root, then the system with its sizes and themes (and, with `deep`, the
    workstreams of each theme), experiments, subscription and device checks."""
    live = [t for t in tasks if not t.cancelled]
    side = SIDE_MILESTONES
    bugs = [t for t in live if t.workstream in BUG_WORKSTREAMS]
    live = [t for t in live if t.workstream not in BUG_WORKSTREAMS]
    main = [t for t in live if t.milestone not in side]
    exp = [t for t in live if t.milestone in EXPERIMENTAL_MILESTONES]
    sub = [t for t in live if t.milestone in SUBSCRIPTION_MILESTONES]
    disc = [t for t in live if t.milestone in DISC_MILESTONES]
    social = [t for t in live if t.milestone in SOCIAL_MILESTONES]
    eco = [t for t in live if t.milestone in ECOSYSTEM_MILESTONES]
    ready = ready_tasks(main)
    rows: list[tuple[str, str, str]] = []  # (prefix + label, bar, note)

    def node(prefix: str, label: str, group: list[Task], note: str = "", width: int = 16) -> None:
        rows.append((prefix + label, bar(sum(t.done for t in group), len(group), width), note))

    def sizes(group: list[Task]) -> str:
        return " ".join(
            f"{s}{sum(t.done for t in group if t.size == s)}/{sum(t.size == s for t in group)}"
            for s in ("S", "M", "L")
            if any(t.size == s for t in group)
        )

    node("", "Kinolune", live, "весь план")
    wd, wt = weighted(main)
    node("├─ ", "Система", main, f"по объёму {pct(wd, wt)}%")
    if eta:
        rows.append((f"│  {'├─' if detail else '└─'} Прогноз", "", eta))
    if detail:
        rows.append(("│  ├─ По размеру", "", "S — для малого лимита, L — после сброса"))
        for i, size in enumerate(("S", "M", "L")):
            group = [t for t in main if t.size == size]
            n_ready = sum(t.size == size for t in ready)
            branch = "└─ " if i == 2 else "├─ "
            node(f"│  │  {branch}", SIZE_LABEL[size].replace("  ", " "), group,
                 f"готовы к работе: {n_ready}")
        rows.append(("│  ├─ По темам", "", ""))
        themes = [(n, ws, [t for t in main if t.workstream in ws]) for n, ws in THEMES]
        themes = [x for x in themes if x[2]]
        for i, (name, streams, group) in enumerate(themes):
            last = i == len(themes) - 1
            gd, gt = weighted(group)
            node(f"│  │  {'└─ ' if last else '├─ '}", name, group,
                 f"объём {pct(gd, gt)}% · {sizes(group)}")
            if deep:
                ws = [w for w in streams if any(t.workstream == w for t in group)]
                for j, w in enumerate(ws):
                    wg = [t for t in group if t.workstream == w]
                    pre = "│  │     " if last else "│  │  │  "
                    node(f"{pre}{'└─ ' if j == len(ws) - 1 else '├─ '}", w, wg, sizes(wg))
        owner = [t for t in main if t.owner and not t.done]
        rows.append((
            "│  └─ Ждут владельца",
            str(len(owner)),
            ", ".join(t.id for t in owner[:6]) + (" …" if len(owner) > 6 else ""),
        ))
    node("├─ ", "Эксперименты", exp, "M11, M12")
    node("├─ ", "Подписка", sub, "M13")
    node("├─ ", "Социальное", social, "M15, после подписки")
    node("├─ ", "Экосистема", eco, "M16, по команде владельца")
    node("├─ ", "Диски", disc, "M14, последним")
    open_bugs = [b for b in bugs if not b.done]
    by_p = {p: sum(1 for b in open_bugs if (m := BUG_PRIORITY_RE.search(b.title)) and m.group(1) == p) for p in ("P0", "P1", "P2")}
    node("├─ ", "Баги", bugs, f"исправлено; открыто: {len(open_bugs)} (P0 {by_p['P0']} · P1 {by_p['P1']} · P2 {by_p['P2']})")
    if detail and open_bugs:
        for i, b in enumerate(open_bugs):
            last = i == len(open_bugs) - 1
            rows.append((f"│  {'└─' if last else '├─'} {b.id}", "", b.title[:70]))
    if catalog is None:
        rows.append(("└─ Проверки", "—", "каталог docs/qa/test-catalog.md не найден"))
    else:
        v, a, n, per = catalog
        rows.append(("└─ Проверки", bar(v, n, 16), f"на устройствах; только автотесты: {a} ({pct(a, n)}%)"))
        if platforms:
            names = list(per)
            for i, name in enumerate(names):
                pv, pn = per[name]
                rows.append((f"   {'└─ ' if i == len(names) - 1 else '├─ '}{name}", bar(pv, pn, 16), ""))
    w = max(len(r[0]) for r in rows) + 1
    out = []
    for label, b, note in rows:
        line = f"{label:<{w}} {b}" if b else label
        if note:
            line += (" · " if b else f"{'':<{w - len(label)}}  ") + note
        out.append(line.rstrip())
    return out


def progress_lines(tasks: list[Task], detail: bool = True) -> list[str]:
    """Flat view kept for scripts: the tree without the device checks."""
    return [l for l in tree_lines(tasks, detail=detail) if "Проверки" not in l]


def cmd_progress(args: argparse.Namespace) -> int:
    tasks, report = lint(Path(args.file).read_text(encoding="utf-8"))
    if report.errors:
        print("roadmap has lint errors; run `just roadmap lint`", file=sys.stderr)
        return 1
    path = Path(args.catalog)
    catalog = catalog_counts(path.read_text(encoding="utf-8")) if path.exists() else None
    history = done_ids_by_day(Path(args.file).resolve(), args.window)
    eta = forecast(tasks, history)
    for line in tree_lines(
        tasks, detail=not args.short, deep=args.deep, catalog=catalog,
        platforms=args.platforms, eta=eta,
    ):
        print(line)
    return 0


def git_head(path: Path) -> str:
    """Full SHA of HEAD in the repository that holds `path`; "" without git."""
    try:
        return subprocess.run(
            ["git", "rev-parse", "HEAD"], capture_output=True, text=True, check=True,
            timeout=30, cwd=path.parent,
        ).stdout.strip()
    except (subprocess.SubprocessError, OSError):
        return ""


def cmd_page(args: argparse.Namespace) -> int:
    """Live progress page (REPO-015): DIR/index.html and DIR/README.md."""
    import datetime as dt
    import os

    import progress_page

    path = Path(args.file).resolve()
    tasks, report = lint(path.read_text(encoding="utf-8"))
    if report.errors:
        print("roadmap has lint errors; run `just roadmap lint`", file=sys.stderr)
        return 1
    repo = args.repo or os.environ.get("GITHUB_REPOSITORY") or progress_page.DEFAULT_REPO
    prs = progress_page.load_prs(Path(args.prs) if args.prs else None, repo)
    if prs is None:
        if args.require_prs:
            print("error: open PRs unavailable (`gh` missing or not signed in)", file=sys.stderr)
            return 1
        print("warning: `gh` unavailable, the «в работе» status is skipped", file=sys.stderr)
    catalog = Path(args.catalog)
    catalog_text = catalog.read_text(encoding="utf-8") if catalog.exists() else None
    now = dt.datetime.now(dt.timezone.utc)
    eta = forecast(tasks, done_ids_by_day(path, args.window), today=now.date().isoformat())
    page = progress_page.build(
        tasks, catalog_text, prs, repo=repo, sha=args.sha or git_head(path),
        generated=args.now or now.strftime("%Y-%m-%d %H:%M UTC"), eta=eta,
    )
    out = Path(args.out)
    digest = progress_page.write(page, out)
    c = progress_page.counts(page.entries)
    print(
        f"progress page: {out}/index.html, {out}/README.md (digest {digest}); "
        + ", ".join(f"{progress_page.LABEL[s]} {c[s]}" for s in progress_page.STATUSES)
    )
    return 0


def cmd_issues_sync(args: argparse.Namespace) -> int:
    """Synchronize community issues; changes require the explicit --apply flag."""
    import issue_sync

    return issue_sync.cmd_sync(args)


def main(argv: list[str] | None = None) -> int:
    common = argparse.ArgumentParser(add_help=False)
    common.add_argument("--file", default=str(DEFAULT_FILE), help="path to ROADMAP.md")
    parser = argparse.ArgumentParser(prog="roadmap", description=__doc__.split("\n")[0])
    sub = parser.add_subparsers(dest="command", required=True)

    p_lint = sub.add_parser("lint", parents=[common], help="validate the task list")
    p_lint.set_defaults(func=cmd_lint)

    p_next = sub.add_parser("next", parents=[common], help="list tasks ready to start")
    p_next.add_argument("workstream", nargs="*", help="filter, e.g. CORE PLAY-AND")
    p_next.add_argument("--offline", action="store_true", help="do not query open PRs")
    p_next.add_argument("--json", action="store_true", help="machine-readable output")
    p_next.add_argument(
        "--size", choices=["S", "M", "L"], action="append",
        help="only tasks of this size (repeatable): S when limits are low, L after a reset",
    )
    p_next.set_defaults(func=cmd_next)

    p_prog = sub.add_parser(
        "progress", parents=[common], help="system, experimental and test bars"
    )
    p_prog.add_argument("--catalog", default=str(DEFAULT_CATALOG), help="test catalog")
    p_prog.add_argument("--platforms", action="store_true", help="per-platform test bars")
    p_prog.add_argument("--short", action="store_true", help="only the main bars")
    p_prog.add_argument("--deep", action="store_true", help="also every workstream of a theme")
    p_prog.add_argument("--window", type=int, default=3, help="days of history for the forecast")
    p_prog.set_defaults(func=cmd_progress)

    p_page = sub.add_parser(
        "page", parents=[common], help="write the live progress page (HTML + Markdown)"
    )
    p_page.add_argument("--out", required=True, help="output directory, e.g. .progress")
    p_page.add_argument("--catalog", default=str(DEFAULT_CATALOG), help="test catalog")
    p_page.add_argument(
        "--prs", help="JSON of `gh pr list --json number,title,headRefName,isDraft,url` "
        "instead of calling gh",
    )
    p_page.add_argument("--require-prs", action="store_true",
                        help="fail when the open PRs are unavailable (CI)")
    p_page.add_argument("--repo", help="owner/name for links (default $GITHUB_REPOSITORY)")
    p_page.add_argument("--sha", help="main commit shown on the page (default git HEAD)")
    p_page.add_argument("--now", help="timestamp text instead of the current UTC time")
    p_page.add_argument("--window", type=int, default=3, help="days of history for the forecast")
    p_page.set_defaults(func=cmd_page)

    p_issues = sub.add_parser(
        "issues", parents=[common], help="manage roadmap-derived community issues"
    )
    issue_commands = p_issues.add_subparsers(dest="issue_command", required=True)
    p_sync = issue_commands.add_parser(
        "sync", parents=[common], help="plan or apply good-first-issue synchronization"
    )
    p_sync.add_argument("--repo", help="owner/name (default $GITHUB_REPOSITORY)")
    p_sync.add_argument(
        "--apply", action="store_true",
        help="create and close GitHub issues (default is a read-only dry run)",
    )
    p_sync.add_argument(
        "--max-create", type=int, default=20, metavar="N",
        help="create at most N issues in one run (default 20)",
    )
    p_sync.set_defaults(func=cmd_issues_sync)

    args = parser.parse_args(argv)
    return args.func(args)


if __name__ == "__main__":
    # progress_page imports this module as `roadmap`: share one copy.
    sys.modules.setdefault("roadmap", sys.modules["__main__"])
    sys.exit(main())
