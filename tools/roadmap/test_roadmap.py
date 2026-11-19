# SPDX-License-Identifier: MPL-2.0
"""Unit tests for roadmap.py: python3 -m unittest discover -s tools/roadmap"""

import textwrap
import unittest
from pathlib import Path

import roadmap


def doc(body: str) -> str:
    return textwrap.dedent(body).lstrip("\n")


def task(id_, deps="—", mark=" ", title="Title", milestone=0, extra=""):
    return (
        f"- [{mark}] **{id_}** {title}{extra}\n"
        f"  - Что: something.\n"
        f"  - deps: {deps} · size: S · parallel: yes · веха: M{milestone} · ref: —\n"
        f"  - done when: tests pass.\n"
    )


def section(ws, *tasks):
    return f"### 3.1. {ws} — test\n\n" + "".join(tasks) + "\n"


class ParseTest(unittest.TestCase):
    def test_parses_fields(self):
        tasks = roadmap.parse(
            section("CORE", task("CORE-001"), task("CORE-002", "CORE-001", mark="x", milestone=1))
        )
        self.assertEqual([t.id for t in tasks], ["CORE-001", "CORE-002"])
        t = tasks[1]
        self.assertTrue(t.done)
        self.assertEqual(t.deps, ["CORE-001"])
        self.assertEqual((t.size, t.parallel, t.milestone), ("S", "yes", 1))
        self.assertEqual(t.workstream, "CORE")

    def test_multi_part_workstream(self):
        (t,) = roadmap.parse(section("PLAY-AND", task("PLAY-AND-007")))
        self.assertEqual(t.workstream, "PLAY-AND")

    def test_code_blocks_are_ignored(self):
        text = "```\n" + task("CORE-0NN".replace("NN", "01"), "BOGUS-999") + "```\n"
        self.assertEqual(roadmap.parse(text), [])

    def test_owner_cancelled_and_blocked(self):
        text = section(
            "REPO",
            task("REPO-001", extra=" (owner)"),
            "- [ ] ~~**REPO-002** Old thing~~ (отменена: не нужно)\n",
            task("REPO-003", title="New (blocked: DEC-001, DEC-002)"),
        )
        a, b, c = roadmap.parse(text)
        self.assertTrue(a.owner)
        self.assertTrue(b.cancelled)
        self.assertEqual(b.title, "Old thing")
        self.assertEqual(c.blocked_by, ["DEC-001", "DEC-002"])


class LintTest(unittest.TestCase):
    def errors(self, text):
        return roadmap.lint(text)[1].errors

    def test_clean_file(self):
        text = section("CORE", task("CORE-001"), task("CORE-002", "CORE-001"))
        self.assertEqual(self.errors(text), [])

    def test_duplicate_id(self):
        errs = self.errors(section("CORE", task("CORE-001"), task("CORE-001")))
        self.assertTrue(any("duplicate ID CORE-001" in e for e in errs), errs)

    def test_unknown_dependency(self):
        errs = self.errors(section("CORE", task("CORE-001", "CORE-404")))
        self.assertTrue(any("unknown dependency CORE-404" in e for e in errs), errs)

    def test_cycle(self):
        text = section(
            "CORE",
            task("CORE-001", "CORE-003"),
            task("CORE-002", "CORE-001"),
            task("CORE-003", "CORE-002"),
        )
        errs = self.errors(text)
        self.assertTrue(any("dependency cycle" in e for e in errs), errs)

    def test_self_dependency(self):
        errs = self.errors(section("CORE", task("CORE-001", "CORE-001")))
        self.assertTrue(any("depends on itself" in e for e in errs), errs)

    def test_ticked_with_open_dependency(self):
        text = section("CORE", task("CORE-001"), task("CORE-002", "CORE-001", mark="x"))
        errs = self.errors(text)
        self.assertTrue(any("ticked but dependency CORE-001" in e for e in errs), errs)

    def test_owner_ticked_early_is_warning(self):
        text = section(
            "DEC", task("DEC-001"), task("DEC-002", "DEC-001", mark="x", extra=" (owner)")
        )
        tasks, report = roadmap.lint(text)
        self.assertEqual(report.errors, [])
        self.assertTrue(any("decision taken early" in w for w in report.warnings))

    def test_milestone_ordering(self):
        text = section("CORE", task("CORE-001", milestone=2), task("CORE-002", "CORE-001"))
        errs = self.errors(text)
        self.assertTrue(any("later milestone M2" in e for e in errs), errs)

    def test_bad_deps_format(self):
        text = section("CORE", "- [ ] **CORE-001** T\n  - Что: x.\n  - deps: — · size: XL\n"
                       "  - done when: y.\n")
        errs = self.errors(text)
        self.assertTrue(any("deps line must be" in e for e in errs), errs)

    def test_missing_fields(self):
        errs = self.errors(section("CORE", "- [ ] **CORE-001** T\n"))
        self.assertEqual(len([e for e in errs if "missing" in e]), 3, errs)

    def test_wrong_section(self):
        errs = self.errors(section("NET", task("CORE-001")))
        self.assertTrue(any("listed under workstream NET" in e for e in errs), errs)

    def test_malformed_task_line(self):
        errs = self.errors(section("CORE", "- [ ] **core-1** lowercase\n"))
        self.assertTrue(any("malformed task line" in e for e in errs), errs)

    def test_repeated_field_is_warning(self):
        t = task("CORE-001")
        deps_line = t.splitlines()[2] + "\n"
        text = section("CORE", t.replace(deps_line, deps_line * 2))
        tasks, report = roadmap.lint(text)
        self.assertEqual(report.errors, [])
        self.assertTrue(any("repeated" in w for w in report.warnings))

    def test_summary_mismatch_warns(self):
        text = section("CORE", task("CORE-001")) + "## 8. Сводка по количеству задач\n\n" \
            "| Workstream | Задач |\n|---|---|\n| CORE | 2 |\n"
        report = roadmap.lint(text)[1]
        self.assertTrue(any("CORE has 2 tasks, found 1" in w for w in report.warnings))


class NextTest(unittest.TestCase):
    TEXT = section(
        "CORE",
        task("CORE-001", mark="x"),
        task("CORE-002", "CORE-001"),
        task("CORE-003", "CORE-002"),
        task("CORE-004", "CORE-001", extra=" (owner)"),
        task("CORE-005", title="Wait (blocked: CORE-003)"),
    ) + section("NET", task("NET-001", "CORE-001"))

    def ids(self, **kw):
        return [t.id for t in roadmap.ready_tasks(roadmap.parse(self.TEXT), **kw)]

    def test_ready(self):
        self.assertEqual(self.ids(), ["CORE-002", "NET-001"])

    def test_workstream_filter(self):
        self.assertEqual(self.ids(workstreams=["net"]), ["NET-001"])

    def test_taken_by_open_pr(self):
        self.assertEqual(self.ids(taken={"CORE-002"}), ["NET-001"])


class RealRoadmapTest(unittest.TestCase):
    def test_repository_roadmap_is_clean(self):
        path = roadmap.DEFAULT_FILE
        if not Path(path).exists():
            self.skipTest("docs/ROADMAP.md not found")
        tasks, report = roadmap.lint(Path(path).read_text(encoding="utf-8"))
        self.assertEqual(report.errors, [])
        self.assertGreater(len(tasks), 100)



class ProgressTest(unittest.TestCase):
    def test_catalog_cells_and_platforms(self) -> None:
        text = textwrap.dedent(
            """\
            | ID | Проверка | Android | Windows | tvOS |
            |---|---|---|---|---|
            | T-1 | Играет видео | ✅ | 🤖 | — |
            | T-2 | Пауза | ⬜ | ❌ 2026-11-18 | ✅ |

            | Not | a catalog |
            |---|---|
            | x | ✅ |
            """
        )
        v, a, n, per = roadmap.catalog_counts(text)
        self.assertEqual((v, a, n), (2, 1, 5))
        self.assertEqual(per["Android"], (1, 2))
        self.assertEqual(per["tvOS"], (1, 1))

    def test_side_milestones_are_not_in_the_main_bar(self) -> None:
        self.assertIn(13, roadmap.SUBSCRIPTION_MILESTONES)
        self.assertTrue(
            roadmap.SUBSCRIPTION_MILESTONES.isdisjoint(roadmap.EXPERIMENTAL_MILESTONES)
        )

    def test_detail_by_size_theme_and_side_milestones(self) -> None:
        text = section(
            "CORE",
            task("CORE-001", mark="x"),
            task("CORE-002", "CORE-001"),
            task("CORE-003", milestone=13),
        ) + section("TWI", task("TWI-001", milestone=11))
        tasks = roadmap.parse(text)
        lines = roadmap.progress_lines(tasks)
        self.assertTrue(lines[0].startswith("Система"))
        self.assertIn("50% (1/2)", lines[0])
        small = next(l for l in lines if "Маленькие" in l)
        self.assertIn("(1/2)", small)
        self.assertIn("готовы к работе: 1", small)
        theme = next(l for l in lines if "Ядро и источники" in l)
        self.assertIn("S1/2", theme)
        self.assertFalse(any("Twitch" in l and "▓" in l for l in lines[1:-2]))
        self.assertIn("0% (0/1)", next(l for l in lines if l.startswith("Эксперименты")))
        self.assertIn("0% (0/1)", next(l for l in lines if l.startswith("Подписка")))
        self.assertEqual(len(roadmap.progress_lines(tasks, detail=False)), 3)

    def test_weighted(self) -> None:
        tasks = roadmap.parse(section("CORE", task("CORE-001", mark="x"), task("CORE-002")))
        tasks[1].size = "L"
        self.assertEqual(roadmap.weighted(tasks), (1, 9))

    def test_every_workstream_has_a_theme(self) -> None:
        tasks, _ = roadmap.lint(roadmap.DEFAULT_FILE.read_text(encoding="utf-8"))
        side = roadmap.EXPERIMENTAL_MILESTONES | roadmap.SUBSCRIPTION_MILESTONES
        themed = {w for _, ws in roadmap.THEMES for w in ws}
        main = {t.workstream for t in tasks if t.milestone not in side}
        self.assertEqual(main - themed, set())

    def test_bar(self) -> None:
        self.assertEqual(roadmap.bar(1, 4, 4), "▓░░░ 25% (1/4)")
        self.assertEqual(roadmap.bar(0, 0, 2), "░░ 0% (0/0)")


if __name__ == "__main__":
    unittest.main()
