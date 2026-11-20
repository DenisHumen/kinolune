# SPDX-License-Identifier: MPL-2.0
"""Unit tests for progress_page.py (REPO-015): python3 -m unittest discover -s tools/roadmap"""

import contextlib
import io
import json
import re
import tempfile
import unittest
from pathlib import Path

import progress_page as pp
import roadmap
from test_roadmap import section, task

REPO = "DenisHumen/kinolune"
SHA = "0123456789abcdef0123456789abcdef01234567"

ROADMAP = (
    section(
        "UI",
        task("UI-001", title="Not started"),
        task("UI-002", title="Has a PR"),
        task("UI-003", title="Branch only"),
        task("UI-004", mark="x", title="Checked everywhere"),
        task("UI-005", mark="x", title="Half checked"),
        task("UI-006", mark="x", title="No catalog row"),
        task("UI-007", mark="x", title="Whole section"),
        "- [ ] ~~**UI-008** Dropped~~ (отменена)\n",
    )
    + section("DOC", task("DOC-001", mark="x", title="Docs `code` <b>bold</b>"))
    + section("BUG", task("BUG-001", title="[P0] Leak"))
)

CATALOG = """\
# Каталог проверок (QA-014)

## 1. Экран

| ID | Проверка | Android | Linux |
|---|---|---|---|
| SCR-01 | Кнопка (UI-004) → работает | ✅ 2026-11-20 | — |
| SCR-02 | Другая кнопка (UI-005) → работает | ✅ 2026-11-20 | 🤖 |
| SCR-03 | Без задачи → ничего | ⬜ | ⬜ |

Автотесты: UI-006 — `test/x_test.dart`.

## 2. Раздел

Экран сделан в UI-007; здесь же упомянут SCR-120, это не задача.

### Подраздел

| ID | Проверка | Android | Linux |
|---|---|---|---|
| SUB-01 | Что-то → есть | ✅ 2026-11-20 | ⬜ |
"""

PRS = [
    {"number": 10, "title": "feat(ui): thing [UI-002]", "headRefName": "feat/whatever",
     "isDraft": True, "url": f"https://github.com/{REPO}/pull/10"},
    {"number": 11, "title": "feat(ui): no id in title", "headRefName": "feat/UI-003-slug",
     "isDraft": False, "url": "https://evil.example/pull/11"},
    {"number": 12, "title": "fix: ticked task [UI-004]", "headRefName": "fix/ui-004",
     "isDraft": False, "url": f"https://github.com/{REPO}/pull/12"},
    {"number": 13, "title": "build(deps): bump x", "headRefName": "dependabot/cargo/base64-0.23.1",
     "isDraft": False, "url": f"https://github.com/{REPO}/pull/13"},
]


def build(prs=PRS, catalog=CATALOG):
    tasks = roadmap.parse(ROADMAP)
    return pp.build(
        tasks, catalog, pp.prs_from_json(prs) if prs is not None else None,
        repo=REPO, sha=SHA, generated="2026-11-20 10:00 UTC", eta="≈ 4 дн.",
    )


def status(page, tid):
    return next(e.status for e in page.entries if e.task.id == tid)


class StatusTest(unittest.TestCase):
    def test_four_statuses(self):
        page = build()
        self.assertEqual(status(page, "UI-001"), pp.TODO)
        self.assertEqual(status(page, "UI-002"), pp.WIP)  # [ID] in the title
        self.assertEqual(status(page, "UI-003"), pp.WIP)  # feat/ID-… branch
        self.assertEqual(status(page, "UI-004"), pp.DONE)  # all applicable cells ✅
        self.assertEqual(status(page, "UI-005"), pp.UNVERIFIED)  # 🤖 is not a device check
        self.assertEqual(status(page, "UI-006"), pp.UNVERIFIED)  # app task, no row
        self.assertEqual(status(page, "UI-007"), pp.UNVERIFIED)  # section intro links rows
        self.assertEqual(status(page, "DOC-001"), pp.DONE)  # nothing to check on a device
        self.assertEqual(status(page, "BUG-001"), pp.TODO)

    def test_cancelled_tasks_are_left_out(self):
        page = build()
        self.assertNotIn("UI-008", {e.task.id for e in page.entries})
        self.assertEqual(page.cancelled, 1)

    def test_ticked_task_with_open_pr_keeps_done_status(self):
        e = next(e for e in build().entries if e.task.id == "UI-004")
        self.assertEqual([p.number for p in e.prs], [12])
        self.assertEqual(e.status, pp.DONE)

    def test_without_prs_nothing_is_in_progress(self):
        page = build(prs=None)
        self.assertFalse(page.prs_known)
        self.assertEqual(status(page, "UI-002"), pp.TODO)
        self.assertIn("недоступен", pp.render_markdown(page))
        self.assertIn("недоступен", pp.render_html(page))

    def test_counts(self):
        c = pp.counts(build().entries)
        self.assertEqual(c, {pp.TODO: 2, pp.WIP: 2, pp.UNVERIFIED: 3, pp.DONE: 2})


class LinkTest(unittest.TestCase):
    def test_catalog_links(self):
        ids = {t.id for t in roadmap.parse(ROADMAP)}
        links = pp.catalog_links(CATALOG, ids)
        self.assertEqual([r.id for r in links["UI-004"]], ["SCR-01"])
        self.assertEqual(links["UI-004"][0].cells, ["✅ 2026-11-20"])  # «—» is not applicable
        self.assertEqual([r.id for r in links["UI-007"]], ["SUB-01"])
        self.assertNotIn("UI-006", links)  # «Автотесты:» lines are not links
        self.assertNotIn("SCR-120", links)

    def test_prs_by_task(self):
        ids = {"UI-002", "UI-003", "UI-004"}
        m = pp.prs_by_task(pp.prs_from_json(PRS), ids)
        self.assertEqual(sorted(m), ["UI-002", "UI-003", "UI-004"])
        self.assertTrue(m["UI-002"][0].draft)

    def test_pr_url_is_rebuilt_unless_github(self):
        page = build()
        e = next(e for e in page.entries if e.task.id == "UI-003")
        self.assertEqual(pp.pr_url(page, e.prs[0]), f"https://github.com/{REPO}/pull/11")

    def test_bad_pr_rows_are_skipped(self):
        self.assertEqual(pp.prs_from_json([{"title": "no number"}, {"number": "x"}]), [])


class RenderTest(unittest.TestCase):
    def test_markdown(self):
        page = build()
        md = pp.render_markdown(page, "abc")
        self.assertIn("progress-digest: abc", md)
        self.assertIn("# Kinolune — прогресс", md)
        self.assertIn("main [`0123456`]", md)
        self.assertIn("**Система** `", md)
        self.assertIn("**Прогноз** ≈ 4 дн.", md)
        self.assertIn("<details><summary><b>Интерфейс и ТВ</b>", md)
        self.assertIn("<details><summary><b>Баги</b>", md)
        self.assertIn(f"🔵 [**UI-002**](https://github.com/{REPO}/blob/{SHA}/docs/ROADMAP.md#L", md)
        self.assertIn(f"[#10](https://github.com/{REPO}/pull/10) черновик", md)
        self.assertIn("&lt;b&gt;bold&lt;/b&gt;", md)
        self.assertIn("Отменённых задач: 1", md)
        self.assertEqual(md.count("<details>"), md.count("</details>"))
        tree = md.split("```text\n", 1)[1].split("```", 1)[0]
        self.assertTrue(tree.startswith("Kinolune"))

    def test_html_is_self_contained(self):
        page = build()
        doc = pp.render_html(page, "abc")
        self.assertTrue(doc.startswith("<!DOCTYPE html>"))
        self.assertIn('name="viewport"', doc)
        self.assertIn("prefers-color-scheme:dark", doc)
        self.assertIn('content="abc"', doc)
        for s in pp.STATUSES:
            self.assertIn(f'data-f="{s}"', doc)
            self.assertIn(f'data-s="{s}"', doc)
        self.assertIn('id="q" type="search"', doc)
        self.assertIn("обновлено 2026-11-20 10:00 UTC", doc)
        self.assertIn("<code>code</code> &lt;b&gt;bold&lt;/b&gt;", doc)
        self.assertNotIn("<b>bold</b>", doc)
        # No external resources: no src=, no @import, no url(); every link
        # goes to a PR, a ROADMAP line or a commit of this repository.
        self.assertNotRegex(doc, r"(?i)\bsrc\s*=")
        self.assertNotRegex(doc, r"(?i)@import|url\(")
        self.assertEqual(doc.count("<script>"), 1)
        allowed = re.compile(
            rf"^https://github\.com/{re.escape(REPO)}/"
            r"(pull/\d+|blob/[0-9a-f]{40}/docs/ROADMAP\.md#L\d+|commit/[0-9a-f]{40})$"
        )
        urls = re.findall(r"https?://[^\s\"'<>)]+", doc)
        self.assertTrue(urls)
        for u in urls:
            self.assertRegex(u, allowed)
        self.assertNotIn("evil.example", doc)

    def test_digest_ignores_the_timestamp(self):
        a, b = build(), build()
        b.generated = "2030-01-01 00:00 UTC"
        self.assertEqual(pp.digest_of(a), pp.digest_of(b))
        self.assertEqual(a.generated, "2026-11-20 10:00 UTC")
        c = build(prs=[])
        self.assertNotEqual(pp.digest_of(a), pp.digest_of(c))

    def test_groups_cover_every_entry(self):
        page = build()
        names = [g[0] for g in pp.groups(page.entries)]
        self.assertEqual(names, ["Интерфейс и ТВ", "Документы и право", "Баги"])
        self.assertEqual(sum(len(g[2]) for g in pp.groups(page.entries)), len(page.entries))


class CommandTest(unittest.TestCase):
    def test_page_command_writes_files(self):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            (root / "ROADMAP.md").write_text(ROADMAP, encoding="utf-8")
            (root / "catalog.md").write_text(CATALOG, encoding="utf-8")
            (root / "prs.json").write_text(json.dumps(PRS), encoding="utf-8")
            out = root / "out"
            with contextlib.redirect_stdout(io.StringIO()):
                rc = roadmap.main([
                    "page", "--file", str(root / "ROADMAP.md"), "--catalog", str(root / "catalog.md"),
                    "--prs", str(root / "prs.json"), "--out", str(out), "--repo", REPO,
                    "--sha", SHA, "--now", "2026-11-20 10:00 UTC",
                ])
            self.assertEqual(rc, 0)
            self.assertEqual(sorted(p.name for p in out.iterdir()), ["NOTE.txt", "README.md", "index.html"])
            md = (out / "README.md").read_text(encoding="utf-8")
            html = (out / "index.html").read_text(encoding="utf-8")
            d1 = pp.DIGEST_RE.search(md).group(1)
            self.assertEqual(d1, pp.DIGEST_RE.search(html).group(1))


if __name__ == "__main__":
    unittest.main()
