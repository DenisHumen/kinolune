# SPDX-License-Identifier: MPL-2.0
"""Plan and synchronize good-first-issue tickets from docs/ROADMAP.md."""

from __future__ import annotations

import json
import os
import re
import shutil
import subprocess
import sys
from dataclasses import dataclass
from pathlib import Path

import roadmap
from roadmap import Task

DEFAULT_REPO = "DenisHumen/kinolune"
LABEL = "good first issue"
ISSUE_ID_RE = re.compile(r"\[([A-Z]+(?:-[A-Z]+)*-\d{3})\]")
CURATED_FILE = Path(__file__).with_name("good_first_issues.txt")


@dataclass(frozen=True)
class Action:
    kind: str
    task: Task
    number: int | None = None
    url: str | None = None


def issue_id(issue: dict) -> str | None:
    match = ISSUE_ID_RE.search(issue.get("title", ""))
    return match.group(1) if match else None


def curated_ids(path: Path = CURATED_FILE) -> set[str]:
    """Read the maintainer-reviewed beginner task allowlist."""
    ids: set[str] = set()
    for number, raw in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
        value = raw.split("#", 1)[0].strip()
        if not value:
            continue
        if not re.fullmatch(r"[A-Z]+(?:-[A-Z]+)*-\d{3}", value):
            raise ValueError(f"{path}:{number}: invalid task ID {value!r}")
        ids.add(value)
    return ids


def eligible(
    tasks: list[Task], open_pr_ids: set[str], curated: set[str]
) -> list[Task]:
    ready = roadmap.ready_tasks(tasks, taken=open_pr_ids)
    return [task for task in ready if task.size == "S" and task.id in curated]


def make_issue(task: Task, repo: str) -> tuple[str, str]:
    title = f"[{task.id}] {task.title}"
    body = "\n".join([
        f"**Roadmap:** [{task.id} — {task.title}](https://github.com/{repo}/blob/main/docs/ROADMAP.md#L{task.line})",
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
    ])
    return title, body


def plan(
    tasks: list[Task], open_pr_ids: set[str], issues: list[dict], curated: set[str]
) -> list[Action]:
    # An empty allowlist means no human has reviewed the candidates yet. In
    # that state, do not create or close issues based on task size alone.
    if not curated:
        return []
    by_id = {issue_id(issue): issue for issue in issues if issue_id(issue)}
    ready = {task.id: task for task in eligible(tasks, open_pr_ids, curated)}
    actions: list[Action] = []
    for task_id, task in ready.items():
        issue = by_id.get(task_id)
        if issue is None:
            actions.append(Action("create", task))
        elif issue.get("state") == "CLOSED":
            continue  # Respect manual closure; do not reopen an issue.
    for task_id, issue in by_id.items():
        if issue.get("state") == "OPEN" and task_id not in ready:
            task = next((item for item in tasks if item.id == task_id), None)
            if task is not None:
                actions.append(Action("close", task, issue.get("number"), issue.get("url")))
    return actions


def gh_json(args: list[str], repo: str) -> list[dict]:
    result = subprocess.run(
        ["gh", *args, "--repo", repo], capture_output=True, text=True,
        check=True, timeout=60,
    )
    return json.loads(result.stdout or "[]")


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
    try:
        reviewed_ids = curated_ids()
    except (OSError, ValueError) as exc:
        print(f"error: cannot read curated good-first-issue IDs: {exc}", file=sys.stderr)
        return 1
    if not reviewed_ids:
        print("no roadmap tasks have been reviewed as beginner-friendly; no changes planned")
        return 0
    try:
        prs = gh_json(["pr", "list", "--state", "open", "--limit", "500", "--json", "title"], repo)
        issues = gh_json(["issue", "list", "--state", "all", "--label", LABEL,
                          "--limit", "500", "--json", "number,title,state,url"], repo)
    except (subprocess.SubprocessError, OSError, json.JSONDecodeError) as exc:
        print(f"error: GitHub query failed: {exc}", file=sys.stderr)
        return 1
    open_pr_ids = {task_id for pr in prs for task_id in roadmap.PR_ID_RE.findall(pr.get("title", ""))}
    actions = plan(tasks, open_pr_ids, issues, reviewed_ids)
    for action in actions:
        if action.kind == "create":
            title, _ = make_issue(action.task, repo)
            print(f"create {title}")
        else:
            print(f"close #{action.number} [{action.task.id}] {action.task.title}")
    if not args.apply:
        print(f"dry run: {len(actions)} changes; pass --apply to synchronize")
        return 0
    for action in actions:
        if action.kind == "create":
            title, body = make_issue(action.task, repo)
            subprocess.run(
                ["gh", "issue", "create", "--repo", repo, "--title", title,
                 "--body", body, "--label", LABEL], check=True, timeout=60,
            )
        else:
            subprocess.run(
                ["gh", "issue", "close", str(action.number), "--repo", repo,
                 "--comment", f"{action.task.id} is no longer an available good first issue."],
                check=True, timeout=60,
            )
    print(f"synchronized {len(actions)} issues")
    return 0
