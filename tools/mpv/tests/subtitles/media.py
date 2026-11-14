#!/usr/bin/env python3
# SPDX-License-Identifier: MPL-2.0
"""Test media for the subtitle golden-frame tests (PLAY-006).

Writes into OUT_DIR (default: ./media next to this file):

- ``ass.mkv``    styled ASS (positioning, rotation, fades, karaoke, CJK line)
                 with the DejaVu Sans font attached, as anime releases do;
- ``pgs.mkv``    a PGS (HDMV presentation graphics) track muxed by FFmpeg;
- ``vobsub.mkv`` the same bitmaps as a VobSub (``dvd_subtitle``) track.

The ASS script and the PGS writer are the QA-002 ones (tools/qa/media). The
picture is a flat colour encoded losslessly (FFV1), so the golden frames only
depend on libmpv and the subtitle renderers, not on a video encoder version.

Needs an ``ffmpeg`` CLI with the ffv1 and dvdsub encoders (any distribution
build: it only makes test inputs, nothing of it is shipped) and DejaVu Sans.
"""

from __future__ import annotations

import argparse
import os
import shutil
import subprocess
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parents[2] / "qa" / "media"))

import pgs  # noqa: E402
import texts  # noqa: E402

SIZE = (640, 360)
DURATION = 5.0
# (start, end) of the PGS/VobSub events: event N shows N yellow blocks.
BITMAP_EVENTS = [(0.5, 1.5), (2.0, 3.0), (3.5, 4.5)]

FONT_CANDIDATES = [
    "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
    "/usr/share/fonts/TTF/DejaVuSans.ttf",
    "/usr/share/fonts/dejavu/DejaVuSans.ttf",
    "/usr/share/fonts/dejavu-sans-fonts/DejaVuSans.ttf",
]


def find_font() -> Path:
    for p in FONT_CANDIDATES:
        if Path(p).exists():
            return Path(p)
    raise SystemExit("DejaVuSans.ttf not found (install fonts-dejavu-core)")


def ffmpeg(ffmpeg_bin: str, out: Path, *args: str) -> None:
    # Relative paths: the work dir is the only thing a containerised ffmpeg
    # wrapper has to mount.
    cmd = [ffmpeg_bin, "-hide_banner", "-loglevel", "error", "-y", *args]
    res = subprocess.run(cmd, cwd=out, capture_output=True, text=True)
    if res.returncode != 0:
        raise SystemExit(f"ffmpeg failed: {' '.join(cmd)}\n{res.stderr}")


def make(out: Path, ffmpeg_bin: str) -> None:
    out.mkdir(parents=True, exist_ok=True)
    shutil.copyfile(find_font(), out / "DejaVuSans.ttf")
    (out / "styled.ass").write_text(texts.ass(), encoding="utf-8")
    pgs.write_sup(out / "events.sup", BITMAP_EVENTS, SIZE)

    w, h = SIZE
    video = ["-f", "lavfi", "-i", f"color=c=0x2f4f6f:size={w}x{h}:rate=24:duration={DURATION}"]
    common = ["-c:v", "ffv1", "-pix_fmt", "yuv420p", "-fflags", "+bitexact", "-map_metadata", "-1"]

    ffmpeg(ffmpeg_bin, out, *video, "-i", "styled.ass", "-attach", "DejaVuSans.ttf",
           "-metadata:s:t", "mimetype=font/ttf", "-map", "0:v", "-map", "1:s", *common,
           "-c:s", "copy", "-metadata:s:s:0", "language=rus", "ass.mkv")
    for name, codec in [("pgs.mkv", "copy"), ("vobsub.mkv", "dvdsub")]:
        ffmpeg(ffmpeg_bin, out, *video, "-fix_sub_duration", "-i", "events.sup", "-map", "0:v",
               "-map", "1:s", *common, "-c:s", codec, "-metadata:s:s:0", "language=eng", name)
    for tmp in ["events.sup", "styled.ass", "DejaVuSans.ttf"]:
        (out / tmp).unlink()


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("out", nargs="?", type=Path, default=HERE / "media")
    ap.add_argument("--ffmpeg", default=os.environ.get("FFMPEG", "ffmpeg"))
    a = ap.parse_args()
    make(a.out.resolve(), a.ffmpeg)
    print(f"media written to {a.out}")


if __name__ == "__main__":
    main()
