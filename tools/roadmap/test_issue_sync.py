# SPDX-License-Identifier: MPL-2.0
"""Unit tests for good-first-issue planning."""

import argparse
import io
import unittest
from contextlib import redirect_stdout
from unittest import mock
from urllib.parse import unquote

import issue_sync
import roadmap
from test_roadmap import section, task

LABELS = [{"name": issue_sync.LABEL}]


def gfi_task(id_="CORE-001", what="something.", done_when="tests pass.",
             title="Title", deps="—", size="S", mark=" "):
    return (
        f"- [{mark}] **{id_}** {title}\n"
        f"  - Что: {what}\n"
        f"  - deps: {deps} · size: {size} · parallel: yes · веха: M0 · ref: —\n"
        f"  - done when: {done_when}\n"
    )


def parse(ws, *lines):
    return roadmap.parse(section(ws, *lines))


def kinds(actions):
    return [(action.kind, action.task.id) for action in actions]


class EligibilityTest(unittest.TestCase):
    def excluded(self, ws="CORE", **kwargs):
        (item,) = parse(ws, gfi_task(id_=f"{ws}-001", **kwargs))
        return issue_sync.exclusion(item, {item.id: item})

    def test_plain_small_task_is_eligible(self):
        self.assertIsNone(self.excluded())

    def test_medium_task_is_not(self):
        self.assertIn("size M", self.excluded(size="M"))

    def test_excluded_workstreams(self):
        for ws in ("LEGAL", "SUB", "DIST", "SRV", "SYNC", "DEC", "PLAY-APL"):
            with self.subTest(ws=ws):
                self.assertEqual(self.excluded(ws=ws), f"workstream {ws}")

    def test_high_priority_bugs_are_excluded_but_p2_is_not(self):
        self.assertEqual(self.excluded(ws="BUG", title="[P1] Stall"), "P0/P1 bug")
        self.assertEqual(self.excluded(ws="BUG", title="[P0] Crash"), "P0/P1 bug")
        self.assertIsNone(self.excluded(ws="BUG", title="[P2] Wrong label"))

    def test_bug_needing_live_repro_is_excluded(self):
        reason = self.excluded(ws="BUG", title="[P2] Click",
                               done_when="повторено на release-сборке Windows настоящей мышью.")
        self.assertEqual(reason, "needs real hardware")
        reason = self.excluded(ws="BUG", title="[P2] Seek",
                               done_when="в журнале живого прогона нет исключения.")
        self.assertEqual(reason, "needs a live run or service")

    def test_where_a_bug_was_found_is_not_a_requirement(self):
        self.assertIsNone(self.excluded(
            ws="BUG", title="[P2] Title match",
            what="живой прогон QA-017 на Windows 2026-11-21: фильм ушёл на проверку.",
            done_when="тест на корпус; config.txt с настройками без секретов."))

    def test_owner_only_steps(self):
        self.assertEqual(self.excluded(what="Подписывает владелец своим ключом."),
                         "needs the owner")
        self.assertEqual(self.excluded(what="записать плёнку ключом владельца."),
                         "needs the owner")
        # Provenance of the task is not an owner step.
        self.assertIsNone(self.excluded(what="запрос владельца 2026-11-25. Кнопка в плеере."))

    def test_keys_research_services_and_hardware(self):
        cases = {
            "needs keys, secrets or signing": "нужен API key сервиса.",
            "research first": "**Проверить до старта**, есть ли эндпоинт.",
            "needs repo admin or a third-party service": "подключить Weblate.",
            "needs a live run or service": "прогон на SMB-стенде.",
            "needs real hardware": "проверка на реальном устройстве.",
            "needs Apple hardware": "XCTest на macOS.",
        }
        for reason, what in cases.items():
            with self.subTest(reason=reason):
                self.assertEqual(self.excluded(what=what), reason)

    def test_words_that_only_look_like_blockers(self):
        # "включить" contains "ключ", "подписка" starts like "подпись",
        # "ключевые" is "key" as in "main".
        self.assertIsNone(self.excluded(
            what="включить экран подписки; ключевые сценарии; сочетание клавиш."))

    def test_open_legal_dependency_excludes_done_one_does_not(self):
        tasks = parse("CORE", gfi_task(what="без client-id (LEGAL-006) не проверить."))
        legal = parse("LEGAL", gfi_task(id_="LEGAL-006", mark=" "))
        by_id = {t.id: t for t in tasks + legal}
        self.assertEqual(issue_sync.exclusion(tasks[0], by_id), "waits for LEGAL-006")
        legal_done = parse("LEGAL", gfi_task(id_="LEGAL-006", mark="x"))
        by_id = {t.id: t for t in tasks + legal_done}
        self.assertIsNone(issue_sync.exclusion(tasks[0], by_id))

    def test_opt_in_and_opt_out_markers(self):
        self.assertIsNone(self.excluded(ws="SUB", title="Small docs fix [gfi]"))
        self.assertIsNone(self.excluded(size="M", title="Bigger but easy [gfi]"))
        self.assertEqual(self.excluded(title="Looks easy [no-gfi]"), "opted out with [no-gfi]")
        self.assertEqual(self.excluded(title="Both [gfi] [no-gfi]"), "opted out with [no-gfi]")

    def test_owner_tasks_never_become_issues(self):
        tasks = parse("CORE", gfi_task(title="(owner) Decide [gfi]"))
        self.assertEqual(issue_sync.eligible(tasks, set()), [])


class IssuePlanTest(unittest.TestCase):
    def task_list(self, mark=" ", extra=""):
        return parse("CORE", task("CORE-001", mark=mark, extra=extra))

    def issue(self, state="OPEN", labels=LABELS, assignees=(), number=7):
        return {"number": number, "title": "[CORE-001] Title", "state": state,
                "url": f"https://example.invalid/{number}", "labels": list(labels),
                "assignees": list(assignees)}

    def test_creates_issue_for_ready_small_task(self):
        self.assertEqual(kinds(issue_sync.plan(self.task_list(), set(), [])),
                         [("create", "CORE-001")])

    def test_does_not_duplicate_existing_open_issue(self):
        self.assertEqual(issue_sync.plan(self.task_list(), set(), [self.issue()]), [])

    def test_does_not_duplicate_issue_without_the_label(self):
        issue = self.issue(labels=[{"name": "bug"}])
        self.assertEqual(issue_sync.plan(self.task_list(), set(), [issue]), [])

    def test_does_not_reopen_or_recreate_manually_closed_issue(self):
        self.assertEqual(issue_sync.plan(self.task_list(), set(), [self.issue("CLOSED")]), [])

    def test_closes_issue_when_task_is_taken_by_pr(self):
        actions = issue_sync.plan(self.task_list(), {"CORE-001"}, [self.issue()])
        self.assertEqual([(a.kind, a.number, a.reason) for a in actions],
                         [("close", 7, "taken by an open PR")])

    def test_closes_issue_when_task_is_done(self):
        actions = issue_sync.plan(self.task_list(mark="x"), set(), [self.issue()])
        self.assertEqual([(a.kind, a.reason) for a in actions], [("close", "task is done")])

    def test_closes_issue_when_rules_exclude_the_task(self):
        actions = issue_sync.plan(self.task_list(extra=" [no-gfi]"), set(), [self.issue()])
        self.assertEqual([(a.kind, a.reason) for a in actions],
                         [("close", "opted out with [no-gfi]")])

    def test_closes_issue_of_task_removed_from_roadmap(self):
        actions = issue_sync.plan([], set(), [self.issue()])
        self.assertEqual([(a.kind, a.task.id) for a in actions], [("close", "CORE-001")])

    def test_keeps_assigned_issue_open(self):
        issue = self.issue(assignees=[{"login": "contributor"}])
        actions = issue_sync.plan(self.task_list(mark="x"), set(), [issue])
        self.assertEqual([a.kind for a in actions], ["keep"])
        self.assertIn("assigned", actions[0].reason)

    def test_never_closes_issue_without_the_label(self):
        issue = self.issue(labels=[{"name": "bug"}])
        self.assertEqual(issue_sync.plan(self.task_list(mark="x"), set(), [issue]), [])

    def test_issue_id_must_start_the_title(self):
        self.assertEqual(issue_sync.issue_id({"title": "[CORE-001] x"}), "CORE-001")
        self.assertIsNone(issue_sync.issue_id({"title": "Follow-up of [CORE-001]"}))


class IssueTextTest(unittest.TestCase):
    def test_links_by_id_not_by_line_and_uses_acceptance_criteria(self):
        (item,) = parse("CORE", gfi_task(title="`Entitlements`: одна точка [gfi]"))
        title, body = issue_sync.make_issue(item, "owner/project")
        self.assertEqual(title, "[CORE-001] `Entitlements`: одна точка")
        self.assertNotIn("#L", body)
        self.assertIn("blob/main/docs/ROADMAP.md#:~:text=", body)
        url = body.split("](", 1)[1].split(")", 1)[0]
        self.assertEqual(unquote(url.split("text=", 1)[1]), "CORE-001 Entitlements: одна точка")
        self.assertIn("tests pass", body)


class CommandTest(unittest.TestCase):
    def run_sync(self, *, apply, issues, prs=()):
        args = argparse.Namespace(file=str(self.path), repo="owner/project",
                                  apply=apply, max_create=20)
        responses = {"pr": list(prs), "issue": issues}
        calls = []

        def fake_run(cmd, **kwargs):
            calls.append((cmd, kwargs))
            out = mock.Mock(stdout="")
            if cmd[1:3] == ["pr", "list"] or cmd[1:3] == ["issue", "list"]:
                import json
                out.stdout = json.dumps(responses[cmd[1]])
            return out

        stdout = io.StringIO()
        with mock.patch.object(issue_sync.subprocess, "run", side_effect=fake_run), \
                mock.patch.object(issue_sync.shutil, "which", return_value="/usr/bin/gh"), \
                mock.patch.object(issue_sync.time, "sleep"), redirect_stdout(stdout):
            code = issue_sync.cmd_sync(args)
        return code, stdout.getvalue(), calls

    def setUp(self):
        from pathlib import Path
        from tempfile import TemporaryDirectory

        self.tmp = TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.path = Path(self.tmp.name) / "ROADMAP.md"
        self.path.write_text(section("CORE", gfi_task(), gfi_task(id_="CORE-002", mark="x")),
                             encoding="utf-8")

    def test_dry_run_only_reads(self):
        closed = {"number": 9, "title": "[CORE-002] Done", "state": "OPEN", "labels": LABELS}
        code, out, calls = self.run_sync(apply=False, issues=[closed])
        self.assertEqual(code, 0)
        self.assertIn("create [CORE-001] Title", out)
        self.assertIn("close #9 [CORE-002]: task is done", out)
        self.assertIn("dry run: 2 changes", out)
        self.assertTrue(all(cmd[1:3] in (["pr", "list"], ["issue", "list"]) for cmd, _ in calls))

    def test_apply_passes_untrusted_text_as_arguments(self):
        code, _, calls = self.run_sync(apply=True, issues=[])
        self.assertEqual(code, 0)
        (create,) = [(cmd, kw) for cmd, kw in calls if cmd[1:3] == ["issue", "create"]]
        cmd, kwargs = create
        self.assertNotIn("shell", kwargs)
        self.assertEqual(cmd[cmd.index("--title") + 1], "[CORE-001] Title")
        self.assertEqual(cmd[cmd.index("--body-file") + 1], "-")
        self.assertIn("tests pass", kwargs["input"])

    def test_rejects_malformed_repo(self):
        args = argparse.Namespace(file=str(self.path), repo="--evil", apply=False)
        with mock.patch.object(issue_sync.shutil, "which", return_value="/usr/bin/gh"), \
                redirect_stdout(io.StringIO()), mock.patch("sys.stderr", io.StringIO()):
            self.assertEqual(issue_sync.cmd_sync(args), 2)


if __name__ == "__main__":
    unittest.main()
