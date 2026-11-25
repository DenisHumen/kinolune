# SPDX-License-Identifier: MPL-2.0
"""BUG-001: the soak memory verdict of tools/qa/e2e/run.sh."""

from __future__ import annotations

import io
import os
import sys
import tempfile
import unittest
from contextlib import redirect_stdout
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import soak_memory  # noqa: E402


def log(rss: list[int], native: bool = False, inuse: list[int] | None = None) -> str:
    def extra(i: int) -> str:
        if inuse is not None:
            return f" threads=60 heap=900MB inuse={inuse[i]}MB free=600MB"
        return " threads=60 heap=900MB inuse=300MB free=600MB" if native else ""

    return "".join(
        f"E2E SOAK cycle {i} ok Sintel 16s rss={mb}MB{extra(i - 1)} play=[0:00 → 0:06]\n"
        for i, mb in enumerate(rss, 1)
    )


class SoakMemoryTest(unittest.TestCase):
    def run_on(self, text: str, **env: str) -> tuple[int, str]:
        with tempfile.TemporaryDirectory() as d:
            path = Path(d) / "run.log"
            path.write_text("noise\n" + text + "E2E MEM end rss=1MB\n", encoding="utf-8")
            old = dict(os.environ)
            os.environ.update(env)
            try:
                out = io.StringIO()
                with redirect_stdout(out):
                    rc = soak_memory.main(["soak_memory.py", str(path)])
            finally:
                os.environ.clear()
                os.environ.update(old)
        return rc, out.getvalue()

    def test_the_leak_of_the_bug_fails(self) -> None:
        # The 40-cycle run that found BUG-001 (debug build, MB).
        rss = [1348, 1475, 1556, 1674, 1716, 1777, 1818, 1842, 1908, 1943,
               1986, 1988, 2008, 2049, 2050, 2075, 2089, 2107, 2145, 2170,
               2198, 2205, 2203, 2245, 2232, 2246, 2273, 2286, 2319, 2298,
               2316, 2320, 2326, 2352, 2347, 2368, 2369, 2376, 2418, 2434]
        rc, out = self.run_on(log(rss))
        self.assertEqual(rc, 1)
        self.assertIn("cycles 11–40 (30)", out)
        self.assertIn("FAIL", out)

    def test_a_plateau_after_the_warm_up_passes(self) -> None:
        rss = [300 + 40 * i for i in range(10)] + [700 + (i % 3) for i in range(190)]
        rc, out = self.run_on(log(rss, native=True))
        self.assertEqual(rc, 0, out)
        self.assertIn("cycles 151–200 (50)", out)
        self.assertIn("threads 60 heap 900 inuse 300 free 600", out)

    def test_a_leak_under_flat_rss_fails_on_malloc_in_use(self) -> None:
        rss = [700 + (i % 3) for i in range(60)]
        inuse = [300 + 2 * i for i in range(60)]  # 100 MB / 50 cycles held
        rc, out = self.run_on(log(rss, inuse=inuse))
        self.assertEqual(rc, 1, out)
        self.assertIn("RSS slope: ", out)
        self.assertIn("malloc in use slope: 2.00 MB/cycle", out)

    def test_flat_malloc_in_use_passes(self) -> None:
        rss = [700 + (i % 3) for i in range(60)]
        rc, out = self.run_on(log(rss, native=True))
        self.assertEqual(rc, 0, out)
        self.assertIn("malloc in use slope: 0.00 MB/cycle", out)

    def test_slow_steady_growth_over_the_limit_fails(self) -> None:
        rss = [500 + 2 * i for i in range(100)]  # 2 MB/cycle = 100 MB / 50
        rc, _ = self.run_on(log(rss))
        self.assertEqual(rc, 1)

    def test_the_limit_and_window_are_configurable(self) -> None:
        rss = [500 + 2 * i for i in range(100)]
        rc, out = self.run_on(log(rss), PA_E2E_SOAK_LIMIT_MB="150", PA_E2E_SOAK_WINDOW="20")
        self.assertEqual(rc, 0, out)
        self.assertIn("cycles 81–100 (20)", out)

    def test_too_few_cycles_give_no_verdict(self) -> None:
        rc, out = self.run_on(log([300, 400, 500, 600, 700, 800, 900, 1000, 1100, 1200, 1300, 1400]))
        self.assertEqual(rc, 0)
        self.assertIn("not enough cycles", out)

    def test_no_cycles_fail(self) -> None:
        rc, _ = self.run_on("")
        self.assertEqual(rc, 1)


if __name__ == "__main__":
    unittest.main()
