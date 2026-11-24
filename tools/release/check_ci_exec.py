#!/usr/bin/env python3
# SPDX-License-Identifier: MPL-2.0
"""Every repository script that CI runs by its path must be executable (PKG-025).

The alpha.17 release failed after an hour of builds because
packaging/linux/import-key.sh, which release-app.yml runs as a command, was
committed without the executable bit. This check reads the `run:` scripts of
every workflow (and local composite action), finds the repository files used
in command position (`tools/x.sh args`, `out=$(packaging/y.sh)`,
`"$GITHUB_WORKSPACE/tools/z.sh"`, after `&&`, `||`, `;`, `if`, `then`, env
assignments ...) and checks their mode in git (100755) and, with
--worktree, on disk. Files given to an interpreter (`python3 -I x.py`,
`node x.mjs`, `bash x.sh`) are not commands and need no bit. A command path
that is not in git at all is reported too.

    check_ci_exec.py [--root DIR] [--worktree] [--list]

Exit 0 when everything is fine, 1 with a list of offenders. --list prints
every command script found (for review). Needs PyYAML and git.
"""

from __future__ import annotations

import argparse
import os
import re
import subprocess
import sys
from pathlib import Path

import yaml

# A path-like token: a/b, ./a/b, "$GITHUB_WORKSPACE/a/b", ${{ github.workspace }}/a/b.
PREFIXES = r"(?:\$GITHUB_WORKSPACE/|\$\{GITHUB_WORKSPACE\}/|\$\{\{\s*github\.workspace\s*\}\}/|\./)?"
TOKEN = re.compile(r"(?<![\w./$-])" + PREFIXES + r"((?:\.?[\w-][\w.-]*/)+[\w.-]+)")
# What may stand right before a command: start of line, an operator, a
# keyword, `$(`, `(`, `{`, `!`, followed by any number of NAME=value pairs
# and an optional opening quote.
ASSIGN = (
    r"""(?:(?:[A-Za-z_][A-Za-z0-9_]*=(?:"(?:[^"\\]|\\.)*"|'[^']*'|[^\s;&|]*)"""
    r"""|"[A-Za-z_][A-Za-z0-9_]*=[^"]*"|'[A-Za-z_][A-Za-z0-9_]*=[^']*')\s+)*"""
)
COMMAND_START = re.compile(
    r"(?:^|[;&|({]|\$\(|`|(?:^|\s)(?:if|then|else|elif|do|while|until|exec|time|nice|xargs|env|sudo|!))\s*"
    + ASSIGN
    + r"""["']?$"""
)


def git_modes(root: Path) -> dict[str, str]:
    out = subprocess.run(
        ["git", "-C", str(root), "ls-files", "-s", "-z"], check=True, capture_output=True
    ).stdout.decode()
    modes = {}
    for rec in out.split("\0"):
        if not rec:
            continue
        meta, path = rec.split("\t", 1)
        modes[path] = meta.split()[0]
    return modes


def logical_lines(script: str):
    """Shell lines with `\\` continuations joined; comments dropped."""
    script = re.sub(r"\\\n", " ", script)
    for line in script.splitlines():
        stripped = line.lstrip()
        if stripped.startswith("#"):
            continue
        yield line


def steps_of(doc: dict):
    """(job/step label, run script, shell, working-directory) for a workflow or action."""
    if not isinstance(doc, dict):
        return
    runs = doc.get("runs")
    if isinstance(runs, dict) and runs.get("using") == "composite":
        for i, step in enumerate(runs.get("steps") or []):
            if isinstance(step, dict) and "run" in step:
                yield f"step {i + 1}", str(step["run"]), step.get("shell", "bash"), step.get("working-directory")
    defaults = ((doc.get("defaults") or {}).get("run") or {}) if isinstance(doc.get("defaults"), dict) else {}
    wf_env = doc.get("env") if isinstance(doc.get("env"), dict) else {}
    for job_id, job in (doc.get("jobs") or {}).items():
        if not isinstance(job, dict):
            continue
        jdef = ((job.get("defaults") or {}).get("run") or {}) if isinstance(job.get("defaults"), dict) else {}
        env = {**wf_env, **(job.get("env") if isinstance(job.get("env"), dict) else {})}

        def expand(value, env=env):
            # ${{ env.NAME }} from the workflow and job env; anything else stays.
            return re.sub(
                r"\$\{\{\s*env\.(\w+)\s*\}\}", lambda m: str(env.get(m.group(1), m.group(0))), str(value)
            )

        for i, step in enumerate(job.get("steps") or []):
            if not isinstance(step, dict):
                continue
            name = step.get("name") or f"step {i + 1}"
            # Actions that run a script of ours (android-emulator-runner `script:`).
            with_ = step.get("with") if isinstance(step.get("with"), dict) else {}
            if "script" in with_:
                yield f"{job_id}: {name}", expand(with_["script"]), "bash", None
            if "run" not in step:
                continue
            shell = step.get("shell") or jdef.get("shell") or defaults.get("shell") or "bash"
            wd = step.get("working-directory") or jdef.get("working-directory") or defaults.get("working-directory")
            yield f"{job_id}: {name}", expand(step["run"]), shell, expand(wd) if wd else None


SCRIPT_EXT = (".sh", ".bash", ".py", ".pl", ".rb", ".mjs", ".js")


def is_script(root: Path, path: str) -> bool:
    """A shell/Python/... script by name, or an extension-less file with a shebang."""
    if path.endswith(SCRIPT_EXT):
        return True
    name = path.rsplit("/", 1)[-1]
    if "." in name:
        return False
    try:
        with open(root / path, "rb") as f:
            return f.read(2) == b"#!"
    except OSError:
        return False


def command_paths(script: str):
    for line in logical_lines(script):
        for m in TOKEN.finditer(line):
            if COMMAND_START.search(line[: m.start()]):
                yield m.group(1)


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--root", default=".", help="repository root (default: .)")
    ap.add_argument("--worktree", action="store_true", help="also require the bit on the files on disk")
    ap.add_argument("--list", action="store_true", help="print every command script found")
    args = ap.parse_args()
    root = Path(args.root).resolve()
    modes = git_modes(root)

    files = sorted((root / ".github" / "workflows").glob("*.y*ml"))
    files += sorted((root / ".github" / "actions").glob("**/action.y*ml"))
    problems: list[str] = []
    found: dict[str, set[str]] = {}
    for wf in files:
        rel_wf = wf.relative_to(root).as_posix()
        try:
            doc = yaml.safe_load(wf.read_text(encoding="utf-8"))
        except yaml.YAMLError as e:
            problems.append(f"{rel_wf}: not valid YAML ({e})")
            continue
        for label, script, shell, wd in steps_of(doc):
            if str(shell).split()[0] in ("pwsh", "powershell", "cmd", "python"):
                continue
            for token in command_paths(script):
                # Relative to the step's working-directory first, then the root.
                candidates = []
                if wd and not str(wd).startswith(("$", "/")):
                    candidates.append(os.path.normpath(f"{wd}/{token}"))
                candidates.append(os.path.normpath(token))
                path = next((c for c in candidates if c in modes), None)
                if not is_script(root, path or candidates[-1]):
                    continue
                if path is None:
                    # Only paths into the repository count: a tracked parent directory.
                    if any(k.startswith(candidates[-1].rsplit("/", 1)[0] + "/") for k in modes) and not (
                        root / candidates[-1]
                    ).is_dir():
                        problems.append(f"{rel_wf} ({label}): runs {token}, which is not in git")
                    continue
                found.setdefault(path, set()).add(rel_wf)
                if modes[path] != "100755":
                    problems.append(
                        f"{rel_wf} ({label}): runs {path}, but git has it as {modes[path]} (needs 100755: "
                        f"git update-index --chmod=+x {path})"
                    )
                elif args.worktree and not os.access(root / path, os.X_OK):
                    problems.append(f"{rel_wf} ({label}): runs {path}, which is not executable on disk (chmod +x)")

    if args.list:
        for path, wfs in sorted(found.items()):
            print(f"{path}  <- {', '.join(sorted(wfs))}")
    # One line per problem, without duplicates from several steps of a file.
    seen = set()
    for p in problems:
        if p not in seen:
            seen.add(p)
            print(f"::error::{p}" if os.environ.get("GITHUB_ACTIONS") else f"error: {p}", file=sys.stderr)
    if problems:
        return 1
    print(f"check_ci_exec: {len(found)} scripts that CI runs by path are executable ({len(files)} workflow files)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
