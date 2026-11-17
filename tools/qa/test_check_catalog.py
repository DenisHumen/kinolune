# SPDX-License-Identifier: MPL-2.0
"""Unit tests for check_catalog.py (QA-014):
python3 -B -m unittest discover -s tools/qa -p 'test_check_catalog.py'"""

import sys
import tempfile
import textwrap
import unittest
from pathlib import Path

import check_catalog as cc

HEADER = (
    "| ID | Проверка | Android | Android TV | iOS | iPadOS | tvOS | macOS | Windows | Linux |\n"
    "|---|---|---|---|---|---|---|---|---|---|\n"
)


def row(id_, text, marks="⬜ | ⬜ | ⬜ | ⬜ | — | 🤖 | 🤖 | 🤖"):
    return f"| {id_} | {text} | {marks} |\n"


def ids(dart: str) -> list[str]:
    return [e.id for e in cc.scan_dart(textwrap.dedent(dart), "lib/x.dart")]


class ScanTest(unittest.TestCase):
    def test_plain_and_typed_keys(self):
        self.assertEqual(
            ids("""
                Key('a'); const ValueKey('b'); ValueKey<String>("c");
                GlobalKey(); ValueKey(item.id); ObjectKey('nope');
            """),
            ["a", "b", "c"],
        )

    def test_interpolation_is_a_star(self):
        self.assertEqual(
            ids("""ValueKey('tile-$id'); ValueKey('x-${a.b}-${c ?? 'd'}');"""),
            ["tile-*", "x-*-*"],
        )

    def test_multiline_ternary_and_trailing_comma(self):
        self.assertEqual(
            ids("""
                key: ValueKey(
                  open
                      ? 'pin-locked'
                      : 'pin-error',
                ),
            """),
            ["pin-locked", "pin-error"],
        )

    def test_nested_quotes_in_interpolation(self):
        self.assertEqual(ids("""ValueKey('lang-${l ?? 'system'}')"""), ["lang-*"])

    def test_menu_items_without_keys(self):
        found = ids("""
            PopupMenuButton(itemBuilder: (_) => [
              for (final s in Sort.values)
                PopupMenuItem(value: s, child: Text(label(s))),
              PopupMenuItem(value: 'rename', child: Text(l.rename)),
              PopupMenuItem(value: 'x', child: Text('X')),
              PopupMenuItem(key: const ValueKey('keyed'), value: 1, child: Text(l.y)),
              PopupMenuItem(onTap: go, child: Icon(Icons.add)),
            ]);
        """)
        self.assertIn("menu:Sort.values", found)
        self.assertIn("menu:rename", found)
        self.assertIn("menu:x", found)
        self.assertIn("keyed", found)
        self.assertNotIn("menu:y", found)
        self.assertTrue(any(i.startswith("menu:?") for i in found))


class CheckTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.lib = Path(self.tmp.name) / "lib"
        (self.lib / "l10n" / "gen").mkdir(parents=True)
        (self.lib / "screen.dart").write_text(
            "IconButton(key: const ValueKey('play'));\n"
            "Container(key: ValueKey('page-$id'));\n"
            "PopupMenuItem(value: 'del', child: Text(l.delete));\n"
        )
        (self.lib / "l10n" / "gen" / "x.dart").write_text("Key('generated')")

    def tearDown(self):
        self.tmp.cleanup()

    def run_check(self, catalog, allow="page-*  # container\n"):
        return cc.check([self.lib], catalog, allow)[0]

    def test_covered(self):
        cat = HEADER + row("A-01", "«Играть» (`play`) → плеер") + row(
            "A-02", "«Удалить» (`menu:delete`) → удалено"
        )
        report = self.run_check(cat)
        self.assertEqual(report.errors, [])

    def test_missing_key_and_menu(self):
        report = self.run_check(HEADER + row("A-01", "без ключей"))
        text = "\n".join(report.errors)
        self.assertIn("`play` is not in", text)
        self.assertIn("`menu:delete` is not in", text)
        self.assertNotIn("generated", text)

    def test_stale_citation_and_allowlist(self):
        cat = HEADER + row("A-01", "(`play`) (`menu:delete`) (`gone`)")
        report = self.run_check(cat, "page-*  # container\nold  # gone too\n")
        text = "\n".join(report.errors)
        self.assertIn("`gone` is not a key", text)
        self.assertIn("allowlist: `old` is no longer", text)

    def test_allowlist_needs_reason(self):
        report = self.run_check(HEADER + row("A-01", "(`play`) (`menu:delete`)"), "page-*\n")
        self.assertTrue(any("no `# reason`" in e for e in report.errors))

    def test_table_format(self):
        cat = (
            HEADER
            + row("A-01", "(`play`) (`menu:delete`)", "✅ | ⬜ | ⬜ | ⬜ | — | ⬜ | ⬜ | ⬜")
            + row("A-01", "dup", "✅ 2026-11-20 | ❌ #12 | ⬜ | ⬜ | — | ⬜ | ⬜ | ⬜")
            + row("bad", "id", "⬜ | ⬜ | ⬜ | ⬜ | — | ⬜ | ⬜ | ⬜")
            + "| A-09 | short | ⬜ |\n"
        )
        text = "\n".join(self.run_check(cat).errors)
        self.assertIn("`✅` is not one of", text)  # a date is required
        self.assertIn("`A-01` is used twice", text)
        self.assertIn("`bad` is not like", text)
        self.assertIn("3 cells", text)
        self.assertNotIn("A-01 · Android TV", text)  # ❌ with an issue is fine

    def test_wrong_header(self):
        cat = "| ID | Проверка | Android | Linux |\n|---|---|---|---|\n| A-01 | x | ⬜ | ⬜ |\n"
        self.assertTrue(any("header must be" in e for e in self.run_check(cat).errors))

    def test_counts_match_the_roadmap_bar(self):
        """The catalog the checker accepts is the one the progress bar reads."""
        import importlib.util

        spec = importlib.util.spec_from_file_location(
            "roadmap", Path(__file__).resolve().parents[1] / "roadmap" / "roadmap.py"
        )
        roadmap = importlib.util.module_from_spec(spec)
        sys.modules["roadmap"] = roadmap  # dataclasses look the module up
        spec.loader.exec_module(roadmap)
        cat = HEADER + row(
            "A-01", "(`play`) (`menu:delete`)", "✅ 2026-11-20 | 🤖 | ⬜ | ⬜ | — | 🤖 | ❌ #1 | —"
        )
        self.assertEqual(self.run_check(cat).errors, [])
        verified, auto, total, per = roadmap.catalog_counts(cat)
        self.assertEqual((verified, auto, total), (1, 2, 6))
        self.assertEqual(per["Android"], (1, 1))
        self.assertNotIn("tvOS", per)


class RepoTest(unittest.TestCase):
    def test_repository_catalog_passes(self):
        catalog = cc.DEFAULT_CATALOG.read_text(encoding="utf-8")
        allow = cc.DEFAULT_ALLOWLIST.read_text(encoding="utf-8")
        report, elements = cc.check(list(cc.DEFAULT_LIBS), catalog, allow)
        self.assertEqual(report.errors, [])
        self.assertGreater(len(elements), 100)


if __name__ == "__main__":
    unittest.main()
