# SPDX-License-Identifier: MPL-2.0
"""Check that the test catalog covers every interactive element (QA-014).

Reads the widget keys (`Key('…')`, `ValueKey('…')`) and the popup-menu
items of the Flutter app (`clients/flutter/app/lib` and the player package
`clients/flutter/packages/pa_player/lib`) and reports those that
are neither cited in `docs/qa/test-catalog.md` nor listed in the allowlist of
non-interactive keys (`tools/qa/catalog_allowlist.txt`). It also checks the
catalog tables themselves: unique row IDs, one mark per platform cell, and
no citation of a key that is gone from the code.

Usage: python3 tools/qa/check_catalog.py [--lib DIR]... [--catalog FILE]
                                         [--allowlist FILE] [--list]

Standard library only (Python ≥ 3.10). Exit code 1 when something is wrong.
"""

from __future__ import annotations

import argparse
import re
import sys
from dataclasses import dataclass, field
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
#: Dart sources whose widgets the catalog covers: the app and the player
#: package that draws the OSD, the track panels and the music screens.
DEFAULT_LIBS = (
    ROOT / "clients" / "flutter" / "app" / "lib",
    ROOT / "clients" / "flutter" / "packages" / "pa_player" / "lib",
)
DEFAULT_CATALOG = ROOT / "docs" / "qa" / "test-catalog.md"
DEFAULT_ALLOWLIST = Path(__file__).resolve().parent / "catalog_allowlist.txt"

#: Generated Dart that has no hand-written widgets.
SKIP_PARTS = ("/l10n/gen/", "/src/rust/")
SKIP_SUFFIXES = (".g.dart", ".freezed.dart")

#: `Key('a')`, `ValueKey('a-$id')`, `ValueKey<String>("a")`, `const Key('a')`,
#: also split over lines or `ValueKey(open ? 'a' : 'b')` (both ids count).
#: An interpolation (`$name`, `${expr}`) is any value, `*` in the catalog.
KEY_RE = re.compile(r"(?<![\w.])(?:Value)?Key(?:<\w+>)?\(")
IDENT_RE = re.compile(r"[A-Za-z_]\w*")
#: Menu entries a user picks; their keys are found by KEY_RE, the rest get
#: an id from their label, their enum or their value.
#: `PlatformMenuItem` is an item of the macOS menu bar (UI-022), labelled by
#: `label: l.x` rather than a `Text`.
MENU_RE = re.compile(
    r"\b(PopupMenuItem|CheckedPopupMenuItem|MenuItemButton|PlatformMenuItem)\s*(?:<[^>]*>)?\("
)
LABEL_RE = re.compile(r"(?:Text\(|\blabel:)\s*(?:l|l10n|context\.l10n)\.(\w+)")
VALUE_RE = re.compile(r"\bvalue:\s*'([^']*)'")
LOOP_RE = re.compile(r"for\s*\(\s*(?:final\s+)?(\w+)\s+in\s+(\w+)\.values\s*\)")

#: Catalog marks (the same as tools/roadmap/roadmap.py `catalog_counts`).
PLATFORMS = ("Android", "Android TV", "iOS", "iPadOS", "tvOS", "macOS", "Windows", "Linux")
MARK_RE = re.compile(
    r"^(?:—|⬜|🤖|✅ \d{4}-\d{2}-\d{2}|❌ (?:\[[^\]]+\]\([^)]+\)|#\d+|https?://\S+))$"
)
CITE_RE = re.compile(r"`([^`]+)`")


@dataclass(frozen=True)
class Element:
    """An interactive element found in the code: `id` as the catalog cites it."""

    id: str
    path: str
    line: int


@dataclass
class Report:
    errors: list[str] = field(default_factory=list)
    warnings: list[str] = field(default_factory=list)


def _is_string_start(text: str, i: int) -> bool:
    c = text[i]
    if c in "'\"":
        return True
    # r'raw' — but not the r ending an identifier such as `bar'`.
    return (
        c == "r"
        and text[i + 1 : i + 2] in ("'", '"')
        and not (i > 0 and (text[i - 1].isalnum() or text[i - 1] == "_"))
    )


def _string_end(text: str, i: int) -> tuple[int, str]:
    """Reads the Dart string literal that starts at `text[i]`. Returns the
    index past it and its value, every interpolation (`$name`, `${expr}`)
    written as `*`."""
    raw = text[i] == "r"
    if raw:
        i += 1
    q = text[i]
    close = q * 3 if text.startswith(q * 3, i) else q
    i += len(close)
    out: list[str] = []
    while i < len(text) and not text.startswith(close, i):
        c = text[i]
        if c == "\\" and not raw:
            out.append(text[i + 1 : i + 2])
            i += 2
        elif c == "$" and not raw and text.startswith("${", i):
            i = _call_end(text, i + 2)  # nested strings and braces included
            out.append("*")
        elif c == "$" and not raw and IDENT_RE.match(text, i + 1):
            i = IDENT_RE.match(text, i + 1).end()
            out.append("*")
        else:
            out.append(c)
            i += 1
    return i + len(close), "".join(out)


def _call_end(text: str, start: int) -> int:
    """Index just past the bracket that closes the one at `text[start-1]`."""
    depth = 1
    i = start
    while i < len(text) and depth:
        if _is_string_start(text, i):
            i, _ = _string_end(text, i)
            continue
        c = text[i]
        if c in "([{":
            depth += 1
        elif c in ")]}":
            depth -= 1
        i += 1
    return i


def key_strings(args: str) -> list[str]:
    """String literals among a key's arguments, outside nested calls:
    `'a-$id'` → `a-*`; `open ? 'a' : 'b'` → `a`, `b`; `item.id` → none."""
    found: list[str] = []
    i = depth = 0
    while i < len(args):
        if _is_string_start(args, i):
            i, value = _string_end(args, i)
            if depth == 0:
                found.append(value)
            continue
        c = args[i]
        if c in "([{":
            depth += 1
        elif c in ")]}":
            depth -= 1
        i += 1
    return found


def scan_dart(text: str, path: str) -> list[Element]:
    """Keys and key-less menu items of one Dart file."""
    found: list[Element] = []

    def line_of(pos: int) -> int:
        return text.count("\n", 0, pos) + 1

    for m in KEY_RE.finditer(text):
        args = text[m.end() : _call_end(text, m.end()) - 1]
        for value in key_strings(args):
            found.append(Element(value, path, line_of(m.start())))
    for m in MENU_RE.finditer(text):
        body = text[m.end() : _call_end(text, m.end())]
        if re.search(r"\bkey:", body):
            continue  # cited by its key
        if label := LABEL_RE.search(body):
            ident = f"menu:{label.group(1)}"
        elif (loops := LOOP_RE.findall(text[max(0, m.start() - 300) : m.start()])) and re.search(
            rf"\bvalue:\s*{loops[-1][0]}\b", body
        ):
            ident = f"menu:{loops[-1][1]}.values"
        elif value := VALUE_RE.search(body):
            ident = f"menu:{value.group(1)}"
        else:
            ident = f"menu:?{path}:{line_of(m.start())}"
        found.append(Element(ident, path, line_of(m.start())))
    return found


def _label(lib: Path) -> str:
    """`app/lib`, `pa_player/lib`: short enough for messages."""
    return f"{lib.parent.name}/{lib.name}"


def scan_lib(libs: list[Path]) -> dict[str, list[Element]]:
    """Every element id of the app, with where it is used."""
    out: dict[str, list[Element]] = {}
    for lib in libs:
        for p in sorted(lib.rglob("*.dart")):
            rel = p.relative_to(lib).as_posix()
            if any(s in f"/{rel}" for s in SKIP_PARTS) or rel.endswith(SKIP_SUFFIXES):
                continue
            for e in scan_dart(p.read_text(encoding="utf-8"), f"{_label(lib)}/{rel}"):
                out.setdefault(e.id, []).append(e)
    return out


def parse_allowlist(text: str, report: Report) -> set[str]:
    """`key  # why it is not interactive`, one per line."""
    keys: set[str] = set()
    for n, raw in enumerate(text.splitlines(), 1):
        line = raw.strip()
        if not line or line.startswith("#"):
            continue
        key, _, why = line.partition("#")
        key = key.strip()
        if not why.strip():
            report.errors.append(f"allowlist:{n}: `{key}` has no `# reason`")
        if key in keys:
            report.errors.append(f"allowlist:{n}: `{key}` is listed twice")
        keys.add(key)
    return keys


def parse_catalog(text: str, report: Report) -> dict[str, str]:
    """Cited ids → the row ID that cites them; checks the tables on the way."""
    cited: dict[str, str] = {}
    rows: set[str] = set()
    header: list[str] | None = None
    for n, line in enumerate(text.splitlines(), 1):
        if not line.startswith("|"):
            header = None
            continue
        cells = [c.strip() for c in line.strip().strip("|").split("|")]
        if header is None:
            header = cells if cells and cells[0] == "ID" else None
            if header is not None and tuple(header[2:]) != PLATFORMS:
                report.errors.append(
                    f"catalog:{n}: header must be `| ID | Проверка | "
                    f"{' | '.join(PLATFORMS)} |`"
                )
            continue
        if set("".join(cells)) <= set("-: "):
            continue
        if len(cells) != len(header):
            report.errors.append(f"catalog:{n}: {len(cells)} cells, header has {len(header)}")
            continue
        row = cells[0]
        if not re.fullmatch(r"[A-Z]+(?:-[A-Z]+)*-\d{2,3}", row):
            report.errors.append(f"catalog:{n}: row ID `{row}` is not like `HOME-01`")
        if row in rows:
            report.errors.append(f"catalog:{n}: row ID `{row}` is used twice")
        rows.add(row)
        for platform, cell in zip(header[2:], cells[2:]):
            if not MARK_RE.match(cell):
                report.errors.append(
                    f"catalog:{n}: {row} · {platform}: `{cell}` is not one of "
                    "— ⬜ 🤖 `✅ YYYY-MM-DD` `❌ <issue link>`"
                )
        for cite in CITE_RE.findall(cells[1]):
            cited.setdefault(cite, row)
    if not rows:
        report.errors.append("catalog: no table with an `| ID | Проверка | … |` header")
    return cited


def check(
    libs: list[Path], catalog: str, allowlist: str
) -> tuple[Report, dict[str, list[Element]]]:
    report = Report()
    elements = scan_lib(libs)
    allowed = parse_allowlist(allowlist, report)
    cited = parse_catalog(catalog, report)
    for ident, uses in sorted(elements.items()):
        if ident.startswith("menu:?"):
            u = uses[0]
            report.errors.append(
                f"{u.path}:{u.line}: menu item without a key, label or value — "
                "give it a ValueKey"
            )
        elif ident not in cited and ident not in allowed:
            u = uses[0]
            report.errors.append(
                f"{u.path}:{u.line}: `{ident}` is not in docs/qa/test-catalog.md "
                "(add a row) or in tools/qa/catalog_allowlist.txt (not interactive)"
            )
    for ident in sorted(allowed - elements.keys()):
        report.errors.append(f"allowlist: `{ident}` is no longer in the code")
    for ident, row in sorted(cited.items()):
        if ident not in elements:
            report.errors.append(f"catalog {row}: `{ident}` is not a key or menu item in the code")
        elif ident in allowed:
            report.warnings.append(f"catalog {row}: `{ident}` is also in the allowlist")
    return report, elements


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    parser.add_argument(
        "--lib", type=Path, action="append", help="Dart lib dir (repeatable; default: app + pa_player)"
    )
    parser.add_argument("--catalog", type=Path, default=DEFAULT_CATALOG)
    parser.add_argument("--allowlist", type=Path, default=DEFAULT_ALLOWLIST)
    parser.add_argument("--list", action="store_true", help="print every element id and exit")
    args = parser.parse_args(argv)
    libs = args.lib or list(DEFAULT_LIBS)

    if args.list:
        for ident, uses in sorted(scan_lib(libs).items()):
            print(f"{ident}\t{uses[0].path}:{uses[0].line}")
        return 0
    allowlist = args.allowlist.read_text(encoding="utf-8") if args.allowlist.exists() else ""
    report, elements = check(libs, args.catalog.read_text(encoding="utf-8"), allowlist)
    for w in report.warnings:
        print(f"warning: {w}", file=sys.stderr)
    for e in report.errors:
        print(f"error: {e}", file=sys.stderr)
    status = "FAILED" if report.errors else "ok"
    print(f"catalog check {status}: {len(elements)} elements, {len(report.errors)} errors")
    return 1 if report.errors else 0


if __name__ == "__main__":
    sys.exit(main())
