#!/usr/bin/env python3
# SPDX-License-Identifier: MPL-2.0
"""QA-016: the library of the e2e media stand.

Builds on tools/qa/media/library.py (movies, series with seasons, anime,
music albums with tags and covers — synthetic clips with realistic names)
and adds what a real home share has:

* the five Blender open movies as REAL files (CC-BY, Blender Foundation),
  downloaded once into --cache from the official URLs and checked by size
  and sha256, then put in place of their synthetic stand-ins under
  release-style names;
* a Blu-ray folder (BDMV/PLAYLIST, CLIPINF, STREAM with an .m2ts);
* a messy «dump» folder: camera names, a half-downloaded .part, a sample,
  a readme, nested «New folder (2)».

    python3 tools/qa/e2e/library.py --out DIR --cache DIR [--seconds 90] [--no-real]

Needs ffmpeg and unzip (the stand runs it in the runner container).
Idempotent: real files are reused from --cache, the tree is rebuilt.
"""

from __future__ import annotations

import argparse
import hashlib
import os
import shutil
import subprocess
import sys
import urllib.request
import zipfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
REPO = HERE.parents[2]
FF = ["nice", "-n", "10", "ffmpeg", "-hide_banner", "-nostdin", "-loglevel", "error", "-y"]

# Real Blender open movies (CC-BY; attribution: (c) Blender Foundation |
# www.blender.org; Cosmos Laundromat: (c) Blender Institute, CC-BY 4.0).
# (url, size, sha256 of the downloaded file, member inside the zip or None,
#  folder in the library to replace, file name in the library)
REAL = [
    ("https://download.blender.org/peach/bigbuckbunny_movies/big_buck_bunny_480p_h264.mov.zip",
     249230089, "9735a4124928237f6f011a8a1f492a7c8139d918ddcea86ed0d438406addcbe9", "big_buck_bunny_480p_h264.mov",
     "Movies/Big Buck Bunny (2008)", "Big.Buck.Bunny.2008.480p.h264.mov"),
    ("https://download.blender.org/ED/elephantsdream-480-h264-st-aac.mov",
     103636531, "a7dbacae5fa2bc345dbd7b10588575f3e2bba36012e9a93df5346467053b0634", None,
     "Movies/Elephants Dream (2006)", "Elephants Dream (2006) 480p.mov"),
    ("https://download.blender.org/durian/movies/Sintel.2010.720p.mkv.zip",
     681285470, "d252f898641b20f51c64206f31063e659c28b152a9d988936736d1b56e6e4db3", "Sintel.2010.720p.mkv",
     "Movies/Sintel (2010)", "Sintel.2010.720p.mkv"),
    ("https://download.blender.org/demo/movies/ToS/tears_of_steel_720p.mov",
     372178639, "efa9062d9cdb7a338e40ad530dfdf234806743f29ae6a1a136b97ece4e588e8f", None,
     "Movies/Tears of Steel (2012)", "Tears.of.Steel.2012.720p.mov"),
    ("https://archive.org/download/CosmosLaundromatFirstCycle/"
     "Cosmos%20Laundromat%20-%20First%20Cycle%20%281080p%29.mp4",
     220087570, "ed4f43b895920072f8e995520d85f8470708a2909258ab5b6d102e7a3f5fca0f", None,
     "Movies/Cosmos Laundromat (2015)", "Cosmos.Laundromat.2015.1080p.mp4"),
]

DUMP = [
    "Downloads/VID_20240817_191512.mp4",
    "Downloads/movie.final.FINAL(1).mkv",
    "Downloads/[rus] Some.Film.2019.WEB-DLRip.avi",
    "Downloads/sample-some.film.2019.mkv",
    "Downloads/New folder (2)/clip.mp4",
    "Downloads/New folder (2)/clip (copy).mp4",
]


def ff(*args) -> None:
    res = subprocess.run(FF + [str(a) for a in args], capture_output=True, text=True)
    if res.returncode != 0:
        raise SystemExit(f"ffmpeg failed: {' '.join(map(str, args))}\n{res.stderr}")


def sha256(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def fetch(url: str, size: int, digest: str, cache: Path) -> Path:
    name = urllib.request.unquote(url.rsplit("/", 1)[1])
    dest = cache / name
    if not (dest.exists() and dest.stat().st_size == size):
        print(f"downloading {url}")
        tmp = dest.with_suffix(dest.suffix + ".part")
        subprocess.run(["curl", "-fsSL", "-C", "-", "-o", str(tmp), url], check=True)
        tmp.rename(dest)
    if dest.stat().st_size != size:
        raise SystemExit(f"{dest}: size {dest.stat().st_size}, expected {size}")
    ok = dest.with_name(dest.name + ".sha256-ok")
    if not ok.exists():
        got = sha256(dest)
        if got != digest:
            raise SystemExit(f"{dest}: sha256 {got}, expected {digest}")
        ok.write_text(got + "\n")
    return dest


def real_movies(out: Path, cache: Path) -> int:
    cache.mkdir(parents=True, exist_ok=True)
    for url, size, digest, member, folder, name in REAL:
        src = fetch(url, size, digest, cache)
        if member:
            unpacked = cache / member
            if not unpacked.exists():
                with zipfile.ZipFile(src) as z:
                    info = next(i for i in z.infolist() if i.filename.endswith(member))
                    with z.open(info) as zin, (cache / (member + ".part")).open("wb") as zout:
                        shutil.copyfileobj(zin, zout, 1 << 20)
                (cache / (member + ".part")).rename(unpacked)
            src = unpacked
        target = out / folder
        # The synthetic stand-in of the main film goes; extras and subtitles stay.
        for p in target.glob("*"):
            if p.is_file() and p.suffix in {".mkv", ".mp4", ".avi", ".mov"} and "trailer" not in p.name:
                p.unlink()
        target.mkdir(parents=True, exist_ok=True)
        dest = target / name
        if dest.exists():
            dest.unlink()
        try:
            os.link(src, dest)
        except OSError:
            shutil.copyfile(src, dest)
    return len(REAL)


def bdmv(out: Path, seconds: int) -> int:
    root = out / "Movies" / "Caminandes Llama Drama (2013) [BDMV]" / "BDMV"
    for d in ("PLAYLIST", "CLIPINF", "STREAM", "BACKUP", "AUXDATA", "META"):
        (root / d).mkdir(parents=True, exist_ok=True)
    ff("-f", "lavfi", "-i", f"testsrc2=size=1280x720:rate=24:duration={seconds}",
       "-f", "lavfi", "-i", f"sine=frequency=330:sample_rate=48000:duration={seconds}",
       "-c:v", "libx264", "-preset", "veryfast", "-crf", "35", "-pix_fmt", "yuv420p",
       "-c:a", "ac3", "-b:a", "192k", "-f", "mpegts", "-mpegts_m2ts_mode", "1",
       root / "STREAM" / "00000.m2ts")
    # Header-only placeholders: enough for folder detection, not a valid disc.
    (root / "index.bdmv").write_bytes(b"INDX0200" + bytes(32))
    (root / "MovieObject.bdmv").write_bytes(b"MOBJ0200" + bytes(32))
    (root / "PLAYLIST" / "00000.mpls").write_bytes(b"MPLS0200" + bytes(32))
    (root / "CLIPINF" / "00000.clpi").write_bytes(b"HDMV0200" + bytes(32))
    return 5


def dump(out: Path, seconds: int) -> int:
    clip = out / "Downloads" / ".clip.mp4"
    clip.parent.mkdir(parents=True, exist_ok=True)
    ff("-f", "lavfi", "-i", f"testsrc=size=640x360:rate=30:duration={seconds}",
       "-f", "lavfi", "-i", f"sine=frequency=550:sample_rate=48000:duration={seconds}",
       "-c:v", "libx264", "-preset", "veryfast", "-crf", "35", "-pix_fmt", "yuv420p", "-c:a", "aac",
       "-movflags", "+faststart", clip)
    for rel in DUMP:
        dest = out / rel
        dest.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(clip, dest)
    clip.unlink()
    (out / "Downloads" / "Sintel.2010.1080p.mkv.part").write_bytes(os.urandom(256 * 1024))
    (out / "Downloads" / "readme.txt").write_text("скачано / downloaded\n", encoding="utf-8")
    (out / "Downloads" / "Thumbs.db").write_bytes(bytes(1024))
    return len(DUMP) + 3


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--out", type=Path, required=True)
    ap.add_argument("--cache", type=Path, required=True, help="downloads of the real films")
    ap.add_argument("--seconds", type=int, default=90, help="length of every synthetic clip")
    ap.add_argument("--no-real", action="store_true", help="synthetic stand-ins only (no downloads)")
    args = ap.parse_args()

    if args.out.exists():
        shutil.rmtree(args.out)
    subprocess.run([sys.executable, str(REPO / "tools" / "qa" / "media" / "library.py"),
                    "--out", str(args.out), "--seconds", str(args.seconds)], check=True)
    n = 0 if args.no_real else real_movies(args.out, args.cache)
    n += bdmv(args.out, args.seconds)
    n += dump(args.out, args.seconds)
    (args.out / "CREDITS.txt").write_text(
        "Big Buck Bunny, Elephants Dream, Sintel, Tears of Steel: (c) copyright Blender Foundation | "
        "www.blender.org, CC-BY.\nCosmos Laundromat: (c) Blender Institute, CC-BY 4.0.\n"
        "Everything else: synthetic test patterns (CC0), tools/qa/media/library.py and tools/qa/e2e/library.py.\n",
        encoding="utf-8")
    files = [p for p in args.out.rglob("*") if p.is_file()]
    size = sum(p.stat().st_size for p in files)
    print(f"QA-016 library: {len(files)} files, {size / 1e9:.2f} GB ({n} real/extra) in {args.out}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
