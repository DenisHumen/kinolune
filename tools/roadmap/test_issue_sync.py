# SPDX-License-Identifier: MPL-2.0
"""Unit tests for good-first-issue planning."""

import unittest

import issue_sync
import roadmap
from test_roadmap import section, task


class IssuePlanTest(unittest.TestCase):
    def task_list(self, mark=" ", extra=""):
        text = section("CORE", task("CORE-001", mark=mark, extra=extra))
        return roadmap.parse(text)

    def test_creates_issue_for_ready_small_task(self):
        (item,) = self.task_list()
        actions = issue_sync.plan([item], set(), [])
        self.assertEqual([(action.kind, action.task.id) for action in actions],
                         [("create", "CORE-001")])

    def test_does_not_duplicate_existing_open_issue(self):
        (item,) = self.task_list()
        issue = {"number": 7, "title": "[CORE-001] Title", "state": "OPEN"}
        self.assertEqual(issue_sync.plan([item], set(), [issue]), [])

    def test_closes_issue_when_task_is_taken_by_pr(self):
        (item,) = self.task_list()
        issue = {"number": 7, "title": "[CORE-001] Title", "state": "OPEN"}
        actions = issue_sync.plan([item], {"CORE-001"}, [issue])
        self.assertEqual([(action.kind, action.number) for action in actions],
                         [("close", 7)])

    def test_closes_issue_when_task_is_done(self):
        (item,) = self.task_list(mark="x")
        issue = {"number": 7, "title": "[CORE-001] Title", "state": "OPEN"}
        self.assertEqual(issue_sync.plan([item], set(), [issue])[0].kind, "close")

    def test_does_not_reopen_manually_closed_issue(self):
        (item,) = self.task_list()
        issue = {"number": 7, "title": "[CORE-001] Title", "state": "CLOSED"}
        self.assertEqual(issue_sync.plan([item], set(), [issue]), [])

    def test_issue_links_to_roadmap_line_and_uses_acceptance_criteria(self):
        (item,) = self.task_list()
        title, body = issue_sync.make_issue(item, "owner/project")
        self.assertIn("[CORE-001", title)
        self.assertIn("docs/ROADMAP.md#L", body)
        self.assertIn("tests pass", body)


if __name__ == "__main__":
    unittest.main()
