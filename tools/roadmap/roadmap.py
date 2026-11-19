#!/usr/bin/env python3
# SPDX-License-Identifier: MPL-2.0
"""Lint docs/ROADMAP.md and list tasks that are ready to start.

Standard library only (Python >= 3.10), so it runs on every CI runner and dev
machine without a virtualenv. Task format: docs/ROADMAP.md §0.1.

    python3 tools/roadmap/roadmap.py lint [--file PATH]
    python3 tools/roadmap/roadmap.py next [WORKSTREAM ...] [--offline] [--json]
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


def progress_lines(tasks: list[Task], detail: bool = True) -> list[str]:
    """The main, experimental and subscription bars and, with `detail`, the
    main bar by task size and by theme plus what is ready to start now."""
    live = [t for t in tasks if not t.cancelled]
    side = EXPERIMENTAL_MILESTONES | SUBSCRIPTION_MILESTONES
    main = [t for t in live if t.milestone not in side]
    exp = [t for t in live if t.milestone in EXPERIMENTAL_MILESTONES]
    sub = [t for t in live if t.milestone in SUBSCRIPTION_MILESTONES]
    wd, wt = weighted(main)
    out = [
        f"Система      {bar(sum(t.done for t in main), len(main))}"
        f" · по объёму {pct(wd, wt)}%"
    ]
    if detail:
        ready = ready_tasks(main)
        for size in ("S", "M", "L"):
            group = [t for t in main if t.size == size]
            n_ready = sum(t.size == size for t in ready)
            out.append(
                f"  {SIZE_LABEL[size]:<22} {bar(sum(t.done for t in group), len(group), 10)}"
                f" · готовы к работе: {n_ready}"
            )
        owner = [t for t in main if t.owner and not t.done]
        if owner:
            out.append(f"  {'Ждут владельца':<22} {len(owner)}: " + ", ".join(t.id for t in owner[:6])
                       + (" …" if len(owner) > 6 else ""))
        out.append("  По темам:")
        width = max(len(name) for name, _ in THEMES)
        for name, streams in THEMES:
            group = [t for t in main if t.workstream in streams]
            if not group:
                continue
            gd, gt = weighted(group)
            sizes = " ".join(
                f"{s}{sum(t.done for t in group if t.size == s)}/{sum(t.size == s for t in group)}"
                for s in ("S", "M", "L")
            )
            out.append(
                f"    {name:<{width}} {bar(sum(t.done for t in group), len(group), 10)}"
                f" · объём {pct(gd, gt):>3}% · {sizes}"
            )
    out.append(f"Эксперименты {bar(sum(t.done for t in exp), len(exp))}")
    out.append(f"Подписка     {bar(sum(t.done for t in sub), len(sub))}")
    return out


def cmd_progress(args: argparse.Namespace) -> int:
    tasks, report = lint(Path(args.file).read_text(encoding="utf-8"))
    if report.errors:
        print("roadmap has lint errors; run `just roadmap lint`", file=sys.stderr)
        return 1
    for line in progress_lines(tasks, detail=not args.short):
        print(line)
    path = Path(args.catalog)
    if path.exists():
        v, a, n, per = catalog_counts(path.read_text(encoding="utf-8"))
        print(f"Проверки     {bar(v, n)} · только автотесты: {a} ({pct(a, n)}%)")
        if args.platforms:
            for name, (pv, pn) in per.items():
                print(f"  {name:<12} {bar(pv, pn, 10)}")
    else:
        print("Проверки     — (каталог docs/qa/test-catalog.md ещё не создан)")
    return 0


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
    p_prog.add_argument("--short", action="store_true", help="only the four main bars")
    p_prog.set_defaults(func=cmd_progress)

    args = parser.parse_args(argv)
    return args.func(args)


if __name__ == "__main__":
    sys.exit(main())
