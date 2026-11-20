# SPDX-License-Identifier: MPL-2.0
"""Live progress page (REPO-015): every ROADMAP task with one of four
statuses, as a self-contained HTML page and as GitHub-flavoured Markdown.

    python3 tools/roadmap/roadmap.py page --out .progress [--prs FILE]

Statuses:

- «не начато» — the box is `[ ]` and no open PR names the task;
- «в работе» — the box is `[ ]` and an open PR has `[ID]` in its title or
  the ID right after the type in its head branch (`feat/ID-…`, `fix/ID-…`);
- «сделано, не проверено» — the box is `[x]`, but the device checks of the
  task in docs/qa/test-catalog.md are not all ✅ (see `catalog_links` for
  how catalog rows are tied to tasks, and DEVICE_WORKSTREAMS for done tasks
  that no catalog row names);
- «готово» — the box is `[x]` and its device checks are verified.

Standard library only, like roadmap.py.
"""

from __future__ import annotations

import hashlib
import html
import json
import re
import shutil
import subprocess
from dataclasses import dataclass, field
from pathlib import Path

import roadmap
from roadmap import ID_RE, PR_ID_RE, Task

TODO, WIP, UNVERIFIED, DONE = "todo", "wip", "unverified", "done"
#: Lifecycle order: how the counts and filters are listed.
STATUSES = (TODO, WIP, UNVERIFIED, DONE)
LABEL = {
    TODO: "не начато",
    WIP: "в работе",
    UNVERIFIED: "сделано, не проверено",
    DONE: "готово",
}
EMOJI = {TODO: "⚪", WIP: "🔵", UNVERIFIED: "🟡", DONE: "✅"}

#: Workstreams whose result a person sees or uses on a device: a ticked task
#: of these that no catalog row names stays «сделано, не проверено» — nothing
#: shows it was checked on a device. Ticked tasks of the other workstreams
#: (decisions, repository, CI, documents, law, site, community, QA tooling,
#: subscription seams) have nothing to check on a device and count as «готово».
DEVICE_WORKSTREAMS = frozenset({
    "CORE", "NET", "LIB", "META", "PLAY", "PLAY-AND", "PLAY-DSK", "PLAY-APL",
    "UI", "TV", "DSN", "SYNC", "SRV", "MUS", "TWI", "DIST", "PKG", "EXT", "BUG",
})

DEFAULT_REPO = "DenisHumen/kinolune"
REPO_RE = re.compile(r"^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+$")
#: `feat/REPO-015-progress-page`, `fix/BUG-009`: the ID right after the type.
BRANCH_ID_RE = re.compile(rf"^[a-z]+/({ID_RE})(?=-|$)", re.IGNORECASE)
WORD_ID_RE = re.compile(rf"(?<![A-Za-z0-9-])({ID_RE})(?![0-9])")
#: Split so that REUSE does not read the string as this file's header.
SPDX_COMMENT = "<!-- SPDX-" + "License-Identifier: MPL-2.0 -->"
DIGEST_RE = re.compile(r"progress-digest: ([0-9a-f]{16,64})")


@dataclass
class PullRequest:
    number: int
    title: str
    head: str
    draft: bool
    url: str


@dataclass
class CatalogRow:
    """One check of the test catalog: its row ID and the applicable cells."""

    id: str
    line: int
    cells: list[str]

    @property
    def verified(self) -> int:
        return sum(c.startswith(roadmap.CATALOG_DONE) for c in self.cells)


@dataclass
class Entry:
    task: Task
    status: str
    prs: list[PullRequest] = field(default_factory=list)
    rows: list[CatalogRow] = field(default_factory=list)
    note: str = ""

    @property
    def checks(self) -> tuple[int, int]:
        return sum(r.verified for r in self.rows), sum(len(r.cells) for r in self.rows)


# --- inputs ---------------------------------------------------------------


def prs_from_json(rows: list[dict]) -> list[PullRequest]:
    out = []
    for r in rows:
        try:
            out.append(PullRequest(
                number=int(r["number"]),
                title=str(r.get("title", "")),
                head=str(r.get("headRefName", "")),
                draft=bool(r.get("isDraft", False)),
                url=str(r.get("url", "")),
            ))
        except (KeyError, TypeError, ValueError):
            continue
    return out


def load_prs(path: Path | None, repo: str) -> list[PullRequest] | None:
    """Open PRs from `path` (the JSON of `gh pr list --json …`) or from `gh`;
    None when neither is available."""
    if path is not None:
        return prs_from_json(json.loads(path.read_text(encoding="utf-8")))
    if shutil.which("gh") is None:
        return None
    try:
        res = subprocess.run(
            ["gh", "pr", "list", "--repo", repo, "--state", "open", "--limit", "500",
             "--json", "number,title,headRefName,isDraft,url"],
            capture_output=True, text=True, check=True, timeout=120,
        )
        return prs_from_json(json.loads(res.stdout or "[]"))
    except (subprocess.SubprocessError, OSError, ValueError):
        return None


def prs_by_task(prs: list[PullRequest], ids: set[str]) -> dict[str, list[PullRequest]]:
    """Task ID → open PRs that name it in the title (`[ID]`) or head branch."""
    out: dict[str, list[PullRequest]] = {}
    for pr in prs:
        named = set(PR_ID_RE.findall(pr.title))
        m = BRANCH_ID_RE.match(pr.head)
        if m:
            named.add(m.group(1).upper())
        for tid in sorted(named & ids):
            out.setdefault(tid, []).append(pr)
    return out


def catalog_links(text: str, ids: set[str]) -> dict[str, list[CatalogRow]]:
    """Task ID → catalog rows that check it.

    The catalog has no task column, so a row belongs to a task when the task
    ID is written
      - in the row itself (e.g. «… (SYNC-014)» in the «Проверка» cell), or
      - in the heading of its `##`/`###` section, or in a prose paragraph of
        that section (the intro that says which task built the screen,
        e.g. «Первый запуск в три шага (UI-042)»): then every row of the
        section belongs to the task.
    «Автотесты: …» lines list catalog row IDs and test files, not tasks, and
    are skipped; IDs that are not ROADMAP tasks (catalog row IDs such as
    SRC-120 share the format) are ignored. Rows whose task is not named
    anywhere stay unlinked."""
    rows: list[tuple[CatalogRow, set[str], tuple[int, int]]] = []
    section_ids: dict[tuple[int, int], set[str]] = {}
    h2 = h3 = 0
    header: list[str] | None = None
    in_fence = False
    for lineno, line in enumerate(text.splitlines(), start=1):
        if line.lstrip().startswith("```"):
            in_fence = not in_fence
            continue
        if in_fence:
            continue
        found = {i for i in WORD_ID_RE.findall(line) if i in ids}
        if line.startswith("## "):
            h2, h3, header = h2 + 1, 0, None
            section_ids.setdefault((h2, 0), set()).update(found)
            continue
        if line.startswith("### "):
            h3, header = h3 + 1, None
            section_ids.setdefault((h2, h3), set()).update(found)
            continue
        if not line.startswith("|"):
            header = None
            if found and not line.lstrip().startswith("Автотесты"):
                section_ids.setdefault((h2, h3), set()).update(found)
            continue
        cells = [c.strip() for c in line.strip().strip("|").split("|")]
        if header is None:
            header = cells if cells and cells[0] == "ID" else []
            continue
        if not header or set("".join(cells)) <= set("-: "):
            continue
        own = {i for i in WORD_ID_RE.findall(" ".join(cells[1:2])) if i in ids}
        applicable = [c for c in cells[2:len(header)] if c and c != "—"]
        rows.append((CatalogRow(cells[0], lineno, applicable), own, (h2, h3)))
    out: dict[str, list[CatalogRow]] = {}
    for row, own, (a, b) in rows:
        tids = own | section_ids.get((a, 0), set()) | (section_ids.get((a, b), set()) if b else set())
        for tid in sorted(tids):
            out.setdefault(tid, []).append(row)
    return out


# --- statuses -------------------------------------------------------------


def classify(
    tasks: list[Task],
    prs: dict[str, list[PullRequest]] | None,
    links: dict[str, list[CatalogRow]],
) -> list[Entry]:
    """One entry per non-cancelled task. `prs` None means the open PRs are
    unknown: then no task is «в работе»."""
    out = []
    for t in tasks:
        if t.cancelled:
            continue
        e = Entry(t, TODO, prs=(prs or {}).get(t.id, []), rows=links.get(t.id, []))
        ver, total = e.checks
        if not t.done:
            e.status = WIP if e.prs else TODO
        elif total:
            e.status = DONE if ver == total else UNVERIFIED
            e.note = f"на устройствах ✅ {ver}/{total}"
        elif t.workstream in DEVICE_WORKSTREAMS:
            e.status = UNVERIFIED
            e.note = "нет строк в каталоге проверок"
        else:
            e.status = DONE
            e.note = "проверка на устройстве не нужна"
        out.append(e)
    return out


def counts(entries: list[Entry]) -> dict[str, int]:
    return {s: sum(e.status == s for e in entries) for s in STATUSES}


def groups(entries: list[Entry]) -> list[tuple[str, str, list[Entry]]]:
    """(name, note, entries) by theme for the system, then the side bars."""
    side = roadmap.EXPERIMENTAL_MILESTONES | roadmap.SUBSCRIPTION_MILESTONES | roadmap.DISC_MILESTONES
    bugs = [e for e in entries if e.task.workstream in roadmap.BUG_WORKSTREAMS]
    rest = [e for e in entries if e.task.workstream not in roadmap.BUG_WORKSTREAMS]
    main = [e for e in rest if e.task.milestone not in side]
    out: list[tuple[str, str, list[Entry]]] = []
    seen: set[str] = set()
    for name, streams in roadmap.THEMES:
        g = [e for e in main if e.task.workstream in streams]
        seen.update(streams)
        if g:
            out.append((name, ", ".join(w for w in streams if any(e.task.workstream == w for e in g)), g))
    other = [e for e in main if e.task.workstream not in seen]
    if other:
        out.append(("Прочее", "", other))
    for name, note, ms in (
        ("Эксперименты", "M11, M12", roadmap.EXPERIMENTAL_MILESTONES),
        ("Подписка", "M13", roadmap.SUBSCRIPTION_MILESTONES),
        ("Диски", "M14", roadmap.DISC_MILESTONES),
    ):
        g = [e for e in rest if e.task.milestone in ms]
        if g:
            out.append((name, note, g))
    if bugs:
        out.append(("Баги", "BUG", bugs))
    return out


@dataclass
class Page:
    """Everything both renderers show."""

    entries: list[Entry]
    tree: list[str]
    bars: list[tuple[str, int, int, str]]  # (label, done, total, note)
    forecast: str | None
    repo: str
    sha: str
    generated: str  # «2026-11-20 14:05 UTC»
    prs_known: bool
    cancelled: int = 0


def build(
    tasks: list[Task],
    catalog_text: str | None,
    prs: list[PullRequest] | None,
    repo: str,
    sha: str,
    generated: str,
    eta: str | None = None,
) -> Page:
    ids = {t.id for t in tasks}
    links = catalog_links(catalog_text, ids) if catalog_text is not None else {}
    entries = classify(tasks, prs_by_task(prs, ids) if prs is not None else None, links)
    catalog = roadmap.catalog_counts(catalog_text) if catalog_text is not None else None
    tree = roadmap.tree_lines(tasks, deep=True, catalog=catalog, platforms=True, eta=eta)

    live = [t for t in tasks if not t.cancelled]
    side = roadmap.EXPERIMENTAL_MILESTONES | roadmap.SUBSCRIPTION_MILESTONES | roadmap.DISC_MILESTONES
    bugs = [t for t in live if t.workstream in roadmap.BUG_WORKSTREAMS]
    nobug = [t for t in live if t.workstream not in roadmap.BUG_WORKSTREAMS]
    main = [t for t in nobug if t.milestone not in side]
    wd, wt = roadmap.weighted(main)

    def b(label: str, group: list[Task], note: str = "") -> tuple[str, int, int, str]:
        return (label, sum(t.done for t in group), len(group), note)

    bars = [
        b("Kinolune", nobug, "весь план без багов"),
        b("Система", main, f"по объёму {roadmap.pct(wd, wt)}%"),
        b("Эксперименты", [t for t in nobug if t.milestone in roadmap.EXPERIMENTAL_MILESTONES], "M11, M12"),
        b("Подписка", [t for t in nobug if t.milestone in roadmap.SUBSCRIPTION_MILESTONES], "M13"),
        b("Диски", [t for t in nobug if t.milestone in roadmap.DISC_MILESTONES], "M14"),
        b("Баги", bugs, f"открыто: {sum(not t.done for t in bugs)}"),
    ]
    if catalog is not None:
        v, a, n, _ = catalog
        bars.append(("Проверки", v, n, f"ячейки каталога на устройствах; только автотесты: {a}"))
    return Page(
        entries=entries, tree=tree, bars=bars, forecast=eta, repo=repo, sha=sha,
        generated=generated, prs_known=prs is not None,
        cancelled=sum(t.cancelled for t in tasks),
    )


# --- links ----------------------------------------------------------------


def _repo(page: Page) -> str:
    return page.repo if REPO_RE.match(page.repo) else DEFAULT_REPO


def task_url(page: Page, t: Task) -> str:
    ref = page.sha if re.fullmatch(r"[0-9a-f]{7,40}", page.sha or "") else "main"
    return f"https://github.com/{_repo(page)}/blob/{ref}/docs/ROADMAP.md#L{t.line}"


def pr_url(page: Page, pr: PullRequest) -> str:
    """The PR link; anything but a github.com PR URL of this repo is rebuilt."""
    want = f"https://github.com/{_repo(page)}/pull/{pr.number}"
    return pr.url if pr.url == want else want


def short_sha(page: Page) -> str:
    return (page.sha or "")[:7] or "?"


# --- Markdown -------------------------------------------------------------


def _md_text(s: str) -> str:
    return s.replace("<", "&lt;").replace(">", "&gt;")


def _status_line(c: dict[str, int]) -> str:
    return " · ".join(f"{EMOJI[s]} {c[s]}" for s in STATUSES)


def render_markdown(page: Page, digest: str = "") -> str:
    c = counts(page.entries)
    repo = _repo(page)
    out = [
        SPDX_COMMENT,
        f"<!-- progress-digest: {digest} -->",
        "",
        "# Kinolune — прогресс",
        "",
        f"Обновлено: **{page.generated}** · main "
        f"[`{short_sha(page)}`](https://github.com/{repo}/commit/{page.sha or 'main'})"
        " · страница собирается сама из `docs/ROADMAP.md` и открытых PR"
        " ([как](https://github.com/" + repo + "/blob/main/tools/roadmap/README.md#страница-прогресса)).",
        "",
    ]
    if not page.prs_known:
        out += ["> ⚠️ Список открытых PR недоступен: статус «в работе» не показан.", ""]
    for label, d, n, note in page.bars:
        out.append(f"- **{label}** `{roadmap.bar(d, n, 20)}`" + (f" · {note}" if note else ""))
    if page.forecast:
        out.append(f"- **Прогноз** {page.forecast}")
    out += [
        "",
        "| " + " | ".join(f"{EMOJI[s]} {LABEL[s]}" for s in STATUSES) + " |",
        "|" + "---|" * len(STATUSES),
        "| " + " | ".join(f"**{c[s]}**" for s in STATUSES) + " |",
        "",
        "<details><summary><b>Дерево прогресса</b> (как <code>roadmap.py progress</code>)</summary>",
        "",
        "```text",
        *page.tree,
        "```",
        "",
        "</details>",
        "",
        "## По темам",
        "",
    ]
    for name, note, group in groups(page.entries):
        gc = counts(group)
        done = sum(e.task.done for e in group)
        out += [
            f"<details><summary><b>{_md_text(name)}</b> "
            f"<code>{roadmap.bar(done, len(group), 10)}</code> · {_status_line(gc)}</summary>",
            "",
        ]
        if note:
            out += [f"_{_md_text(note)}_", ""]
        for e in group:
            t = e.task
            bits = [f"{EMOJI[e.status]} [**{t.id}**]({task_url(page, t)}) {_md_text(t.title)}"]
            meta = [t.size or "?"]
            if t.milestone is not None:
                meta.append(f"M{t.milestone}")
            if t.owner and not t.done:
                meta.append("владелец")
            meta += [f"[#{p.number}]({pr_url(page, p)})" + (" черновик" if p.draft else "") for p in e.prs]
            if e.note:
                meta.append(e.note)
            out.append(f"- {bits[0]} · " + " · ".join(meta))
        out += ["", "</details>", ""]
    out += [
        "---",
        "",
        "**Статусы.** "
        f"{EMOJI[TODO]} не начато — `[ ]` и нет открытого PR; "
        f"{EMOJI[WIP]} в работе — `[ ]` и открыт PR с `[ID]` в заголовке или ветка `тип/ID-…`; "
        f"{EMOJI[UNVERIFIED]} сделано, не проверено — `[x]`, но проверки задачи в "
        "`docs/qa/test-catalog.md` не все ✅ (или у задачи приложения нет строк в каталоге); "
        f"{EMOJI[DONE]} готово — `[x]` и проверки на устройствах ✅ (или проверять на устройстве нечего: "
        "решения, репозиторий, CI, документы)."
        + (f" Отменённых задач: {page.cancelled}, они не показаны." if page.cancelled else ""),
        "",
    ]
    return "\n".join(out)


# --- HTML -----------------------------------------------------------------

CSS = """
:root{--bg:#f6f7f9;--card:#fff;--fg:#1b1f24;--muted:#5d6670;--line:#e3e6ea;
--accent:#3b6fd8;--todo:#8a939d;--wip:#2f6fde;--unv:#c98a00;--done:#1f9d55;
--todo-bg:#eef0f2;--wip-bg:#e6eefc;--unv-bg:#fdf3dc;--done-bg:#e3f5ea;--track:#e6e9ed}
@media (prefers-color-scheme:dark){:root{--bg:#0f1216;--card:#171b21;--fg:#e6e9ee;
--muted:#9aa3ad;--line:#262c34;--accent:#7aa2f7;--todo:#8d96a0;--wip:#6f9cf5;
--unv:#e0b040;--done:#45c27a;--todo-bg:#232930;--wip-bg:#1b2840;--unv-bg:#352b14;
--done-bg:#15301f;--track:#262c34}}
*{box-sizing:border-box}
html{-webkit-text-size-adjust:100%}
body{margin:0;background:var(--bg);color:var(--fg);
font:15px/1.45 system-ui,-apple-system,"Segoe UI",Roboto,sans-serif}
main{max-width:920px;margin:0 auto;padding:16px 16px 48px}
h1{font-size:22px;margin:4px 0 2px}
h2{font-size:17px;margin:24px 0 8px}
a{color:var(--accent);text-decoration:none}
a:hover{text-decoration:underline}
code{font:12.5px/1.3 ui-monospace,SFMono-Regular,Menlo,Consolas,monospace;
background:var(--todo-bg);padding:0 3px;border-radius:4px}
.meta{color:var(--muted);font-size:13px}
.card{background:var(--card);border:1px solid var(--line);border-radius:12px;padding:14px;margin:12px 0}
.warn{border-color:var(--unv);background:var(--unv-bg)}
.bar{display:grid;grid-template-columns:minmax(90px,8.5em) 1fr auto;gap:4px 10px;align-items:center;margin:6px 0}
.bar .name{font-weight:600}
.bar .pct{font-variant-numeric:tabular-nums;font-size:13px;color:var(--muted);text-align:right}
.bar .note{grid-column:2/4;color:var(--muted);font-size:12.5px;margin-top:-3px}
.track{height:10px;border-radius:5px;background:var(--track);overflow:hidden;display:flex}
.track i{display:block;height:100%}
.f-done{background:var(--done)}.f-unv{background:var(--unv)}.f-wip{background:var(--wip)}
.big .track{height:14px;border-radius:7px}
.forecast{margin-top:8px;font-size:13.5px}
.stack{margin:10px 0 4px}
.filters{display:flex;flex-wrap:wrap;gap:6px;margin:10px 0}
.filters button{font:inherit;font-size:13.5px;border:1px solid var(--line);background:var(--card);color:var(--fg);
border-radius:999px;padding:5px 11px;cursor:pointer}
.filters button[aria-pressed=true]{border-color:var(--accent);box-shadow:inset 0 0 0 1px var(--accent)}
.filters b{font-variant-numeric:tabular-nums}
input[type=search]{width:100%;font:inherit;padding:9px 12px;border-radius:10px;border:1px solid var(--line);
background:var(--card);color:var(--fg)}
details.sec{background:var(--card);border:1px solid var(--line);border-radius:12px;margin:8px 0}
details.sec>summary{cursor:pointer;padding:12px 14px;list-style:none;display:grid;
grid-template-columns:1fr auto;gap:4px 10px;align-items:center}
details.sec>summary::-webkit-details-marker{display:none}
details.sec>summary .t{font-weight:600}
details.sec>summary .t::before{content:"▸";display:inline-block;width:1em;color:var(--muted);transition:transform .15s}
details.sec[open]>summary .t::before{transform:rotate(90deg)}
details.sec>summary .track{grid-column:1/3}
details.sec>summary .sc{font-size:12.5px;color:var(--muted);font-variant-numeric:tabular-nums}
.sec .ws{padding:0 14px;color:var(--muted);font-size:12.5px}
ul.tasks{list-style:none;margin:0;padding:4px 6px 8px}
ul.tasks li{display:flex;flex-wrap:wrap;gap:4px 8px;align-items:baseline;padding:8px;border-top:1px solid var(--line)}
ul.tasks li:first-child{border-top:0}
.id{font:600 12.5px ui-monospace,SFMono-Regular,Menlo,Consolas,monospace;white-space:nowrap}
.title{flex:1 1 14em;min-width:0;overflow-wrap:anywhere}
.tags{display:flex;flex-wrap:wrap;gap:4px 6px;font-size:12px;color:var(--muted);width:100%;padding-left:2px}
.tag{border:1px solid var(--line);border-radius:6px;padding:0 5px}
.chip{font-size:11.5px;font-weight:600;border-radius:999px;padding:1px 8px;white-space:nowrap}
.s-todo{color:var(--todo);background:var(--todo-bg)}.s-wip{color:var(--wip);background:var(--wip-bg)}
.s-unverified{color:var(--unv);background:var(--unv-bg)}.s-done{color:var(--done);background:var(--done-bg)}
details.tree>summary{cursor:pointer;font-weight:600}
pre{overflow-x:auto;font:11.5px/1.35 ui-monospace,SFMono-Regular,Menlo,Consolas,monospace;margin:10px 0 0}
.hidden{display:none!important}
.empty{color:var(--muted);text-align:center;padding:16px}
footer{color:var(--muted);font-size:12.5px;margin-top:24px}
"""

JS = """
(function(){
var st='all',q='';
var btns=document.querySelectorAll('.filters button');
var search=document.getElementById('q');
var secs=document.querySelectorAll('details.sec');
var empty=document.getElementById('empty');
function apply(){
  var any=0,active=(st!=='all'||q!=='');
  secs.forEach(function(sec){
    var n=0;
    sec.querySelectorAll('li[data-s]').forEach(function(li){
      var ok=(st==='all'||li.dataset.s===st)&&(q===''||li.dataset.q.indexOf(q)>=0);
      li.classList.toggle('hidden',!ok);if(ok)n++;
    });
    sec.classList.toggle('hidden',n===0);
    if(active){sec.open=n>0;}else{sec.open=sec.dataset.open==='1';}
    any+=n;
  });
  empty.classList.toggle('hidden',any>0);
}
btns.forEach(function(b){b.addEventListener('click',function(){
  st=b.dataset.f;btns.forEach(function(x){x.setAttribute('aria-pressed',x===b?'true':'false');});apply();
});});
search.addEventListener('input',function(){q=search.value.trim().toLowerCase();apply();});
})();
"""


def _e(s: str) -> str:
    return html.escape(s, quote=True)


def _title_html(title: str) -> str:
    """Escape, then show `code` spans; ** and other Markdown stay as text."""
    parts = re.split(r"`([^`]+)`", title)
    return "".join(_e(p) if i % 2 == 0 else f"<code>{_e(p)}</code>" for i, p in enumerate(parts))


def _track(parts: list[tuple[str, int]], total: int) -> str:
    if total <= 0:
        return '<div class="track"></div>'
    seg = "".join(
        f'<i class="{cls}" style="width:{100 * n / total:.2f}%"></i>' for cls, n in parts if n
    )
    return f'<div class="track">{seg}</div>'


def render_html(page: Page, digest: str = "") -> str:
    c = counts(page.entries)
    repo = _repo(page)
    total = len(page.entries)
    h: list[str] = [
        "<!DOCTYPE html>",
        SPDX_COMMENT,
        f"<!-- progress-digest: {_e(digest)} -->",
        '<html lang="ru"><head><meta charset="utf-8">',
        '<meta name="viewport" content="width=device-width,initial-scale=1">',
        '<meta name="color-scheme" content="light dark">',
        f'<meta name="progress-digest" content="{_e(digest)}">',
        "<title>Kinolune — прогресс</title>",
        f"<style>{CSS}</style></head><body><main>",
        "<h1>Kinolune — прогресс</h1>",
        f'<div class="meta">обновлено {_e(page.generated)} · main '
        f'<a href="https://github.com/{_e(repo)}/commit/{_e(page.sha or "main")}"><code>{_e(short_sha(page))}</code></a>'
        " · из <code>docs/ROADMAP.md</code>, каталога проверок и открытых PR</div>",
    ]
    if not page.prs_known:
        h.append('<div class="card warn">Список открытых PR недоступен: статус «в работе» не показан.</div>')

    h.append('<section class="card">')
    for i, (label, d, n, note) in enumerate(page.bars):
        big = " big" if i < 2 else ""
        h.append(
            f'<div class="bar{big}"><span class="name">{_e(label)}</span>'
            f'{_track([("f-done", d)], n)}<span class="pct">{roadmap.pct(d, n)}% · {d}/{n}</span>'
            + (f'<span class="note">{_e(note)}</span>' if note else "")
            + "</div>"
        )
    if page.forecast:
        h.append(f'<div class="forecast"><b>Прогноз:</b> {_e(page.forecast)}</div>')
    h.append(
        '<div class="stack"><div class="meta">Все задачи по статусам</div>'
        + _track([("f-done", c[DONE]), ("f-unv", c[UNVERIFIED]), ("f-wip", c[WIP])], total)
        + "</div>"
    )
    h.append('<details class="tree"><summary>Дерево прогресса</summary><pre>'
             + _e("\n".join(page.tree)) + "</pre></details></section>")

    h.append('<div class="filters" role="group" aria-label="Фильтр по статусу">')
    h.append(f'<button type="button" data-f="all" aria-pressed="true">все <b>{total}</b></button>')
    for s in STATUSES:
        h.append(
            f'<button type="button" data-f="{s}" aria-pressed="false">'
            f'<span class="chip s-{s}">{_e(LABEL[s])}</span> <b>{c[s]}</b></button>'
        )
    h.append("</div>")
    h.append('<input id="q" type="search" placeholder="Поиск: ID или слово из названия" '
             'aria-label="Поиск задач" autocomplete="off">')

    h.append("<h2>По темам</h2>")
    for name, note, group in groups(page.entries):
        gc = counts(group)
        # Open the themes that have work in progress.
        is_open = "1" if gc[WIP] else "0"
        h.append(
            f'<details class="sec" data-open="{is_open}"{" open" if gc[WIP] else ""}><summary>'
            f'<span class="t">{_e(name)}</span>'
            f'<span class="sc">{EMOJI[WIP]} {gc[WIP]} · {EMOJI[UNVERIFIED]} {gc[UNVERIFIED]} · '
            f'{EMOJI[DONE]} {gc[DONE]} · {EMOJI[TODO]} {gc[TODO]}</span>'
            + _track([("f-done", gc[DONE]), ("f-unv", gc[UNVERIFIED]), ("f-wip", gc[WIP])], len(group))
            + "</summary>"
        )
        if note:
            h.append(f'<div class="ws">{_e(note)}</div>')
        h.append('<ul class="tasks">')
        for e in group:
            t = e.task
            q = f"{t.id} {t.title}".lower()
            tags = [f'<span class="tag">{_e(t.size or "?")}</span>']
            if t.milestone is not None:
                tags.append(f'<span class="tag">M{t.milestone}</span>')
            if t.owner and not t.done:
                tags.append('<span class="tag">владелец</span>')
            for p in e.prs:
                tags.append(f'<a href="{_e(pr_url(page, p))}">#{p.number}</a>'
                            + (" черновик" if p.draft else ""))
            if e.note:
                tags.append(f"<span>{_e(e.note)}</span>")
            h.append(
                f'<li data-s="{e.status}" data-q="{_e(q)}">'
                f'<span class="chip s-{e.status}">{_e(LABEL[e.status])}</span>'
                f'<a class="id" href="{_e(task_url(page, t))}">{_e(t.id)}</a>'
                f'<span class="title">{_title_html(t.title)}</span>'
                f'<span class="tags">{"".join(tags)}</span></li>'
            )
        h.append("</ul></details>")
    h.append('<div id="empty" class="empty hidden">Ничего не найдено</div>')
    h.append(
        "<footer><p><b>Статусы.</b> «не начато» — <code>[ ]</code> и нет открытого PR; "
        "«в работе» — открыт PR с <code>[ID]</code> в заголовке или веткой <code>тип/ID-…</code>; "
        "«сделано, не проверено» — <code>[x]</code>, но проверки задачи в каталоге "
        "<code>docs/qa/test-catalog.md</code> не все ✅ (или у задачи приложения нет строк в каталоге); "
        "«готово» — <code>[x]</code> и проверки на устройствах ✅ (или проверять на устройстве нечего)."
        + (f" Отменённых задач: {page.cancelled}, они не показаны." if page.cancelled else "")
        + "</p><p>Перегенерировать: <code>python3 tools/roadmap/roadmap.py page --out .progress</code></p></footer>"
    )
    h.append(f"</main><script>{JS}</script></body></html>")
    return "\n".join(h) + "\n"


NOTE = """Generated by .github/workflows/progress.yml from main (REPO-015).
Do not edit: every run replaces this branch with one fresh commit.
Open README.md; index.html is the same page for a browser.
Regenerate locally: python3 tools/roadmap/roadmap.py page --out .progress
"""


def digest_of(page: Page) -> str:
    """Hash of the content without the timestamp: unchanged data, same digest."""
    stamp = page.generated
    page.generated = ""
    try:
        text = render_markdown(page)
    finally:
        page.generated = stamp
    return hashlib.sha256(text.encode("utf-8")).hexdigest()[:32]


def write(page: Page, out: Path) -> str:
    out.mkdir(parents=True, exist_ok=True)
    digest = digest_of(page)
    (out / "README.md").write_text(render_markdown(page, digest), encoding="utf-8")
    (out / "index.html").write_text(render_html(page, digest), encoding="utf-8")
    (out / "NOTE.txt").write_text(NOTE, encoding="utf-8")
    return digest
