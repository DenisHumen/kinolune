# SPDX-License-Identifier: MPL-2.0
"""Plan and synchronize good-first-issue tickets from docs/ROADMAP.md.

A task becomes a good first issue when it is open, not taken by an open PR,
all its deps are done, it is `size: S`, and none of the exclusion rules below
match. The task line can override the rules: `[gfi]` opts a ready task in,
`[no-gfi]` opts it out (opt-out wins).
"""

from __future__ import annotations

import json
import os
import re
import shutil
import subprocess
import sys
import time
from dataclasses import dataclass
from pathlib import Path
from urllib.parse import quote

import roadmap
from roadmap import Task

DEFAULT_REPO = "DenisHumen/kinolune"
LABEL = "good first issue"
ISSUE_ID_RE = re.compile(rf"^\[({roadmap.ID_RE})\]")
REPO_RE = re.compile(r"[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+")
OPT_IN = "[gfi]"
OPT_OUT = "[no-gfi]"
#: Pause between mutating GitHub calls: GitHub's secondary rate limit for
#: content creation asks for at least a second between requests.
MUTATION_PAUSE = 1.5
#: Upper bound of issues created by one run, so a parser bug cannot flood
#: the tracker.
DEFAULT_MAX_CREATE = 20

#: Workstreams that need the owner, legal review, architecture decisions,
#: the server/distro stack or Apple hardware.
EXCLUDED_WORKSTREAMS = {"LEGAL", "SUB", "DIST", "SRV", "SYNC", "DEC", "PLAY-APL"}

#: Phrases that say where a task came from or what it must not contain, not
#: what it needs: "owner's request 2026-11-25", "found by the live run
#: QA-017", "settings without secrets". Removed before the rules run.
NOT_A_REQUIREMENT_RE = re.compile(
    r"(?:запрос|решени[еяю]|по решению)\s+владельца(?:\s+\d{4}-\d{2}-\d{2})?"
    r"|(?:найден\w*\s+)?жив\w+\s+(?:прогон\w*|e2e-тест\w*)\s+QA-\d{3}"
    r"|без\s+секретов",
    re.I,
)
#: (reason, pattern) checked in order on the task text; the first match wins.
RULES: list[tuple[str, re.Pattern[str]]] = [
    ("needs the owner", re.compile(r"владел(?:ец|ьца|ьцу|ьцем|ьце)", re.I)),
    ("research first", re.compile(r"проверить до старта|исследовани\w*|\bresearch\b", re.I)),
    ("needs repo admin or a third-party service",
     re.compile(r"\b(?:weblate|crowdin)\b|github discussions", re.I)),
    ("needs keys, secrets or signing", re.compile(
        r"\bключ(?!ев)\w*|\bsecrets?\b|секрет\w*|minisign|\bclient[ _]id\b|api[ _-]?key"
        r"|подпис(?:ыва\w*|ан\w*|ью|и\b|ь\b)", re.I)),
    ("needs a live run or service", re.compile(
        r"\bжив(?:ой|ом|ых|ым|ого|ые|ая|ую|ыми)\b|\bstand\b|стенд\w*", re.I)),
    ("needs real hardware", re.compile(
        r"реальн\w*\s+(?:устройств|железе|телевизор)|настоящ\w*|оборудовани\w*|приставк\w*"
        r"|физическ\w*", re.I)),
    ("needs Apple hardware", re.compile(
        r"\b(?:apple|macos|mac|ios|tvos|watchos|xcode|xctest|mpvkit|avplayer)\b", re.I)),
]
LEGAL_REF_RE = re.compile(r"\bLEGAL-\d{3}\b")
PRIORITY_RE = re.compile(r"\[P[01]\]")


@dataclass(frozen=True)
class Action:
    kind: str  # "create", "close" or "keep" (an open issue left alone)
    task: Task
    number: int | None = None
    url: str | None = None
    reason: str = ""


def issue_id(issue: dict) -> str | None:
    match = ISSUE_ID_RE.match(issue.get("title", ""))
    return match.group(1) if match else None


def task_text(task: Task) -> str:
    parts = [task.title]
    for key in ("Что", "deps", "done when"):
        parts.extend(task.fields.get(key, []))
    return "\n".join(parts)


def exclusion(task: Task, by_id: dict[str, Task]) -> str | None:
    """Why a ready task is not a good first issue, or None if it is one."""
    if OPT_OUT in task.title:
        return f"opted out with {OPT_OUT}"
    if OPT_IN in task.title:
        return None
    if task.size != "S":
        return f"size {task.size or '?'}, not S"
    if task.workstream in EXCLUDED_WORKSTREAMS:
        return f"workstream {task.workstream}"
    if task.workstream == "BUG" and PRIORITY_RE.search(task.title):
        return "P0/P1 bug"
    text = task_text(task)
    legal = sorted({ref for ref in LEGAL_REF_RE.findall(text)
                    if not (ref in by_id and by_id[ref].done)})
    if legal:
        return "waits for " + ", ".join(legal)
    rest = NOT_A_REQUIREMENT_RE.sub("", text)
    for reason, pattern in RULES:
        if pattern.search(rest):
            return reason
    return None


def eligible(tasks: list[Task], open_pr_ids: set[str]) -> list[Task]:
    by_id = {task.id: task for task in tasks}
    ready = roadmap.ready_tasks(tasks, taken=open_pr_ids)
    return [task for task in ready if exclusion(task, by_id) is None]


def why_not(task: Task | None, tasks: list[Task], open_pr_ids: set[str]) -> str:
    """Why a task with an open issue is no longer a good first issue."""
    if task is None:
        return "task is no longer in the roadmap"
    if task.done or task.cancelled:
        return "task is done" if task.done else "task is cancelled"
    if task.owner:
        return "owner task"
    # A rule that excludes the task outranks open PRs and deps, which change.
    reason = exclusion(task, {t.id: t for t in tasks})
    if reason:
        return reason
    if task.id in open_pr_ids:
        return "taken by an open PR"
    if task not in roadmap.ready_tasks(tasks, taken=open_pr_ids):
        return "deps are not done"
    return "not eligible"


def clean_title(task: Task) -> str:
    title = task.title.replace(OPT_IN, "").replace(OPT_OUT, "")
    return re.sub(r"\s{2,}", " ", title).strip()


def roadmap_url(task: Task, repo: str) -> str:
    """Link to the task by its ID and title, not by a line number that drifts.

    A text fragment scrolls to the rendered task line ("ID Title") in browsers
    that support it; others open the top of ROADMAP, where Ctrl+F by ID works.
    """
    words = re.sub(r"[`*_~]", "", clean_title(task)).split()[:4]
    fragment = quote(" ".join([task.id, *words]), safe="")
    return f"https://github.com/{repo}/blob/main/docs/ROADMAP.md#:~:text={fragment}"


def make_issue(task: Task, repo: str) -> tuple[str, str]:
    title = f"[{task.id}] {clean_title(task)}"
    body = "\n".join([
        f"**Roadmap:** [{task.id} — {clean_title(task)}]({roadmap_url(task, repo)})"
        f" (найдите `{task.id}` в `docs/ROADMAP.md`)",
        "",
        "## Что сделать",
        "",
        *task.fields.get("Что", ["См. задачу в roadmap."]),
        "",
        "## Готово, когда",
        "",
        *task.fields.get("done when", ["См. критерии в roadmap."]),
        "",
        "Please comment if you would like to work on this issue before starting.",
        f"Open the PR with `[{task.id}]` in its title; see `.github/CONTRIBUTING.md`.",
    ])
    return title, body


def plan(tasks: list[Task], open_pr_ids: set[str], issues: list[dict]) -> list[Action]:
    """Actions that bring the labelled issues in line with the roadmap.

    `issues` are all issues of the repo (any label, any state), so a task that
    already has an issue - even closed or relabelled by a human - never gets a
    second one. Only issues with the good-first-issue label are ever closed,
    and never one that someone is assigned to.
    """
    by_id: dict[str, dict] = {}
    for issue in sorted(issues, key=lambda item: item.get("number") or 0):
        task_id = issue_id(issue)
        if task_id is None:
            continue
        # Prefer an open labelled issue, so it is the one kept or closed.
        current = by_id.get(task_id)
        if current is None or (_is_ours(issue) and issue.get("state") == "OPEN"):
            by_id[task_id] = issue
    tasks_by_id = {task.id: task for task in tasks}
    ready = {task.id: task for task in eligible(tasks, open_pr_ids)}
    actions: list[Action] = []
    for task_id, task in ready.items():
        if task_id not in by_id:  # A closed issue stays closed: no reopening.
            actions.append(Action("create", task))
    for task_id, issue in sorted(by_id.items(), key=lambda item: item[1].get("number") or 0):
        if task_id in ready or issue.get("state") != "OPEN" or not _is_ours(issue):
            continue
        task = tasks_by_id.get(task_id)
        reason = why_not(task, tasks, open_pr_ids)
        if task is None:
            task = Task(id=task_id, line=0, done=False, cancelled=False, owner=False,
                        title=issue.get("title", ""), workstream=roadmap.workstream_of(task_id))
        kind = "keep" if issue.get("assignees") else "close"
        if kind == "keep":
            reason += "; assigned, left for a maintainer"
        actions.append(Action(kind, task, issue.get("number"), issue.get("url"), reason))
    return actions


def _is_ours(issue: dict) -> bool:
    labels = issue.get("labels")
    if labels is None:  # Old callers passed only labelled issues.
        return True
    return any((label.get("name") if isinstance(label, dict) else label) == LABEL
               for label in labels)


def gh_json(args: list[str], repo: str) -> list[dict]:
    result = subprocess.run(
        ["gh", *args, "--repo", repo], capture_output=True, text=True,
        check=True, timeout=120,
    )
    return json.loads(result.stdout or "[]")


def describe(action: Action) -> str:
    if action.kind == "create":
        return f"create [{action.task.id}] {clean_title(action.task)}"
    return f"{action.kind} #{action.number} [{action.task.id}]: {action.reason}"


def cmd_sync(args) -> int:
    path = Path(args.file).resolve()
    tasks, report = roadmap.lint(path.read_text(encoding="utf-8"))
    if report.errors:
        print("roadmap has lint errors; run `just roadmap lint`", file=sys.stderr)
        return 1
    if shutil.which("gh") is None:
        print("error: GitHub CLI `gh` is required", file=sys.stderr)
        return 1
    repo = args.repo or os.environ.get("GITHUB_REPOSITORY") or DEFAULT_REPO
    if not REPO_RE.fullmatch(repo):
        print(f"error: --repo must look like owner/name, got {repo!r}", file=sys.stderr)
        return 2
    try:
        prs = gh_json(["pr", "list", "--state", "open", "--limit", "500", "--json", "title"], repo)
        issues = gh_json(["issue", "list", "--state", "all", "--limit", "2000",
                          "--json", "number,title,state,url,labels,assignees"], repo)
    except (subprocess.SubprocessError, OSError, json.JSONDecodeError) as exc:
        print(f"error: GitHub query failed: {exc}", file=sys.stderr)
        return 1
    open_pr_ids = {task_id for pr in prs for task_id in roadmap.PR_ID_RE.findall(pr.get("title", ""))}
    actions = plan(tasks, open_pr_ids, issues)
    creates = [action for action in actions if action.kind == "create"]
    max_create = getattr(args, "max_create", DEFAULT_MAX_CREATE)
    if len(creates) > max_create:
        print(f"note: {len(creates)} issues to create, capped at --max-create {max_create}")
        skipped = {id(action) for action in creates[max_create:]}
        actions = [action for action in actions if id(action) not in skipped]
    for action in actions:
        print(describe(action))
    changes = [action for action in actions if action.kind != "keep"]
    if not args.apply:
        print(f"dry run: {len(changes)} changes; pass --apply to synchronize")
        return 0
    done = 0
    try:
        for action in changes:
            if done:
                time.sleep(MUTATION_PAUSE)
            if action.kind == "create":
                title, body = make_issue(action.task, repo)
                # Arguments go straight to gh, never through a shell.
                subprocess.run(
                    ["gh", "issue", "create", "--repo", repo, "--title", title,
                     "--body-file", "-", "--label", LABEL],
                    input=body, text=True, check=True, timeout=60,
                )
            else:
                subprocess.run(
                    ["gh", "issue", "close", str(action.number), "--repo", repo,
                     "--comment", f"Closed by `just roadmap issues sync`: {action.reason}."],
                    check=True, timeout=60,
                )
            done += 1
    except (subprocess.SubprocessError, OSError) as exc:
        print(f"error: stopped after {done} of {len(changes)} changes: {exc}", file=sys.stderr)
        return 1
    print(f"synchronized {done} issues")
    return 0
