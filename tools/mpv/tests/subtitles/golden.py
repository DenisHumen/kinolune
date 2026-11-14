#!/usr/bin/env python3
# SPDX-License-Identifier: MPL-2.0
"""Golden-frame tests of subtitle rendering in our libmpv (PLAY-006).

Opens the files from media.py in libmpv through its software render API
(ctypes, no GPU or display needed), seeks to fixed times and compares the
rendered frames (video + subtitles) with PNGs in testdata/golden:

- ASS: styles, \\pos and \\frz, \\fad fades, \\k karaoke (libass, attached font);
- PGS: two events and the gap between them (FFmpeg's pgssub decoder);
- VobSub: two events (FFmpeg's dvdsub decoder).

    golden.py --lib out/linux-x86_64/lib/libmpv.so.2 [--media DIR] [--update]

On a mismatch the actual frame and a diff image are written to --out. The
comparison tolerates small differences (scaler rounding, anti-aliasing):
at most 0.5 % of the pixels may differ by more than 40 levels per channel and
the mean difference must stay under 1.5 levels. Pure stdlib (Python 3.9+).
"""

from __future__ import annotations

import argparse
import ctypes as C
import struct
import sys
import time
import zlib
from pathlib import Path

HERE = Path(__file__).resolve().parent
WIDTH, HEIGHT = 640, 360

# (golden name, media file, time in seconds)
CASES = [
    ("ass-fade-in", "ass.mkv", 0.6),  # \fad half way, sign at \pos with \frz
    ("ass-dialogue-sign", "ass.mkv", 1.0),  # line fully visible + sign
    ("ass-karaoke", "ass.mkv", 2.6),  # \k: two syllables sung, the rest not yet
    ("pgs-event1", "pgs.mkv", 1.0),
    ("pgs-gap", "pgs.mkv", 1.75),  # cleared between events
    ("pgs-event2", "pgs.mkv", 2.5),
    ("vobsub-event1", "vobsub.mkv", 1.0),
    ("vobsub-event3", "vobsub.mkv", 4.0),
]

# mpv options: deterministic software path, no system fonts (the ASS file
# carries its font), no OSD.
OPTIONS = {
    "vo": "libmpv",
    "ao": "null",
    "config": "no",
    "terminal": "no",
    "msg-level": "all=error",
    "idle": "yes",
    "pause": "yes",
    "keep-open": "yes",
    "hwdec": "no",
    "hr-seek": "yes",
    "osd-level": "0",
    "sub-auto": "no",
    "sid": "1",
    "sub-visibility": "yes",
    "sub-ass": "yes",
    "embeddedfonts": "yes",
    "sub-font-provider": "none",
    "demuxer-mkv-subtitle-preroll": "yes",
    "vd-lavc-threads": "1",
}

MPV_EVENT_SHUTDOWN = 1
MPV_EVENT_START_FILE = 6
MPV_EVENT_END_FILE = 7
MPV_EVENT_FILE_LOADED = 8
MPV_EVENT_PLAYBACK_RESTART = 21
MPV_RENDER_PARAM_API_TYPE = 1
MPV_RENDER_PARAM_SW_SIZE = 17
MPV_RENDER_PARAM_SW_FORMAT = 18
MPV_RENDER_PARAM_SW_STRIDE = 19
MPV_RENDER_PARAM_SW_POINTER = 20
MPV_RENDER_UPDATE_FRAME = 1


class RenderParam(C.Structure):
    _fields_ = [("type", C.c_int), ("data", C.c_void_p)]


class Event(C.Structure):
    _fields_ = [("event_id", C.c_int), ("error", C.c_int), ("reply_userdata", C.c_uint64), ("data", C.c_void_p)]


class Mpv:
    """Just enough of the libmpv client and render API."""

    def __init__(self, lib: Path):
        m = self.m = C.CDLL(str(lib))
        m.mpv_create.restype = C.c_void_p
        m.mpv_set_option_string.argtypes = [C.c_void_p, C.c_char_p, C.c_char_p]
        m.mpv_initialize.argtypes = [C.c_void_p]
        m.mpv_command.argtypes = [C.c_void_p, C.POINTER(C.c_char_p)]
        m.mpv_wait_event.argtypes = [C.c_void_p, C.c_double]
        m.mpv_wait_event.restype = C.POINTER(Event)
        m.mpv_error_string.restype = C.c_char_p
        m.mpv_render_context_create.argtypes = [C.POINTER(C.c_void_p), C.c_void_p, C.POINTER(RenderParam)]
        m.mpv_render_context_update.argtypes = [C.c_void_p]
        m.mpv_render_context_update.restype = C.c_uint64
        m.mpv_render_context_render.argtypes = [C.c_void_p, C.POINTER(RenderParam)]
        m.mpv_render_context_free.argtypes = [C.c_void_p]
        m.mpv_terminate_destroy.argtypes = [C.c_void_p]

        self.h = m.mpv_create()
        if not self.h:
            raise SystemExit("mpv_create failed")
        for k, v in OPTIONS.items():
            self._check(m.mpv_set_option_string(self.h, k.encode(), v.encode()), f"option {k}={v}")
        self._check(m.mpv_initialize(self.h), "mpv_initialize")
        self._api = C.c_char_p(b"sw")
        params = (RenderParam * 2)((MPV_RENDER_PARAM_API_TYPE, C.cast(self._api, C.c_void_p)), (0, None))
        self.ctx = C.c_void_p()
        self._check(m.mpv_render_context_create(C.byref(self.ctx), self.h, params), "render context")

    def _check(self, err: int, what: str) -> None:
        if err < 0:
            raise SystemExit(f"libmpv: {what}: {self.m.mpv_error_string(err).decode()}")

    def command(self, *args: str) -> None:
        argv = (C.c_char_p * (len(args) + 1))(*[a.encode() for a in args], None)
        self._check(self.m.mpv_command(self.h, argv), " ".join(args))

    def wait_for(self, event_id: int, timeout: float = 15.0) -> None:
        end = time.monotonic() + timeout
        started = False
        while time.monotonic() < end:
            # Rendering keeps the VO going while we wait.
            self.m.mpv_render_context_update(self.ctx)
            ev = self.m.mpv_wait_event(self.h, 0.05).contents
            if ev.event_id == event_id:
                return
            started = started or ev.event_id == MPV_EVENT_START_FILE
            # END_FILE of the previous file comes first on "loadfile replace".
            if ev.event_id == MPV_EVENT_SHUTDOWN or (ev.event_id == MPV_EVENT_END_FILE and started):
                raise SystemExit(f"libmpv: file did not load (event {ev.event_id})")
        raise SystemExit(f"libmpv: timeout waiting for event {event_id}")

    def render(self) -> bytes:
        size = (C.c_int * 2)(WIDTH, HEIGHT)
        fmt = C.c_char_p(b"rgb0")
        stride = C.c_size_t(WIDTH * 4)
        buf = C.create_string_buffer(WIDTH * HEIGHT * 4)
        params = (RenderParam * 5)(
            (MPV_RENDER_PARAM_SW_SIZE, C.cast(size, C.c_void_p)),
            (MPV_RENDER_PARAM_SW_FORMAT, C.cast(fmt, C.c_void_p)),
            (MPV_RENDER_PARAM_SW_STRIDE, C.cast(C.byref(stride), C.c_void_p)),
            (MPV_RENDER_PARAM_SW_POINTER, C.cast(buf, C.c_void_p)),
            (0, None),
        )
        self._check(self.m.mpv_render_context_render(self.ctx, params), "render")
        rgb0 = buf.raw
        out = bytearray(WIDTH * HEIGHT * 3)
        out[0::3] = rgb0[0::4]
        out[1::3] = rgb0[1::4]
        out[2::3] = rgb0[2::4]
        return bytes(out)

    def frame_at(self, path: Path, t: float) -> bytes:
        self.command("loadfile", str(path), "replace")
        self.wait_for(MPV_EVENT_FILE_LOADED)
        self.command("seek", f"{t:.3f}", "absolute+exact")
        self.wait_for(MPV_EVENT_PLAYBACK_RESTART)
        # The frame (and an embedded subtitle packet read after the seek) can
        # land a little after the restart: render until two frames agree.
        prev = None
        end = time.monotonic() + 5.0
        while time.monotonic() < end:
            self.m.mpv_render_context_update(self.ctx)
            cur = self.render()
            if cur == prev:
                return cur
            prev = cur
            self.m.mpv_wait_event(self.h, 0.15)
        return prev

    def close(self) -> None:
        self.m.mpv_render_context_free(self.ctx)
        self.m.mpv_terminate_destroy(self.h)


# ---- PNG (RGB8, no interlace), stdlib only ---------------------------------


def png_write(path: Path, rgb: bytes, w: int = WIDTH, h: int = HEIGHT) -> None:
    def chunk(kind: bytes, data: bytes) -> bytes:
        return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data))

    stride = w * 3
    raw = b"".join(b"\x00" + rgb[y * stride:(y + 1) * stride] for y in range(h))
    path.write_bytes(
        b"\x89PNG\r\n\x1a\n"
        + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 2, 0, 0, 0))
        + chunk(b"IDAT", zlib.compress(raw, 9))
        + chunk(b"IEND", b"")
    )


def png_read(path: Path) -> tuple[int, int, bytes]:
    data = path.read_bytes()
    if data[:8] != b"\x89PNG\r\n\x1a\n":
        raise ValueError(f"{path}: not a PNG")
    pos, idat, w, h = 8, b"", 0, 0
    while pos < len(data):
        (n,) = struct.unpack(">I", data[pos:pos + 4])
        kind, body = data[pos + 4:pos + 8], data[pos + 8:pos + 8 + n]
        if kind == b"IHDR":
            w, h, depth, ctype, _, _, interlace = struct.unpack(">IIBBBBB", body)
            if (depth, ctype, interlace) != (8, 2, 0):
                raise ValueError(f"{path}: only 8-bit RGB non-interlaced PNGs are supported")
        elif kind == b"IDAT":
            idat += body
        pos += 12 + n
    raw = zlib.decompress(idat)
    stride, bpp = w * 3, 3
    out = bytearray(h * stride)
    prev = bytearray(stride)
    for y in range(h):
        f = raw[y * (stride + 1)]
        line = bytearray(raw[y * (stride + 1) + 1:(y + 1) * (stride + 1)])
        for x in range(stride):
            a = line[x - bpp] if x >= bpp else 0
            b = prev[x]
            c = prev[x - bpp] if x >= bpp else 0
            if f == 1:
                line[x] = (line[x] + a) & 0xFF
            elif f == 2:
                line[x] = (line[x] + b) & 0xFF
            elif f == 3:
                line[x] = (line[x] + ((a + b) >> 1)) & 0xFF
            elif f == 4:
                p = a + b - c
                pa, pb, pc = abs(p - a), abs(p - b), abs(p - c)
                pr = a if pa <= pb and pa <= pc else (b if pb <= pc else c)
                line[x] = (line[x] + pr) & 0xFF
        out[y * stride:(y + 1) * stride] = line
        prev = line
    return w, h, bytes(out)


# ---- comparison ------------------------------------------------------------

MAX_BAD_FRACTION = 0.005
BAD_LEVEL = 40
MAX_MEAN = 1.5


def compare(actual: bytes, golden: bytes) -> tuple[bool, str, bytes]:
    n = WIDTH * HEIGHT
    bad = 0
    total = 0
    diff = bytearray(n * 3)
    for i in range(n):
        o = i * 3
        d = max(abs(actual[o] - golden[o]), abs(actual[o + 1] - golden[o + 1]), abs(actual[o + 2] - golden[o + 2]))
        total += d
        if d > BAD_LEVEL:
            bad += 1
            diff[o] = 255
        else:
            diff[o] = diff[o + 1] = diff[o + 2] = min(255, d * 8)
    frac, mean = bad / n, total / n
    ok = frac <= MAX_BAD_FRACTION and mean <= MAX_MEAN
    return ok, f"{bad} px over {BAD_LEVEL} ({frac:.3%}), mean {mean:.2f}", bytes(diff)


def blank(frame: bytes) -> bool:
    """True if the frame is the flat background only (no subtitle drawn)."""
    first = frame[WIDTH * 3 * (HEIGHT // 2):WIDTH * 3 * (HEIGHT // 2) + 3]
    return frame == first * (WIDTH * HEIGHT)


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--lib", type=Path, required=True, help="libmpv.so.2 (or .so/.dylib/.dll) to test")
    ap.add_argument("--media", type=Path, default=HERE / "media", help="output of media.py")
    ap.add_argument("--golden", type=Path, default=HERE / "testdata" / "golden")
    ap.add_argument("--out", type=Path, default=HERE / "out", help="actual frames and diffs on failure")
    ap.add_argument("--update", action="store_true", help="write the rendered frames as the new goldens")
    ap.add_argument("--only", nargs="*", help="case names to run")
    a = ap.parse_args()

    mpv = Mpv(a.lib)
    failed = 0
    try:
        for name, media, t in CASES:
            if a.only and name not in a.only:
                continue
            frame = mpv.frame_at((a.media / media).resolve(), t)
            golden = a.golden / f"{name}.png"
            # A frame without any subtitle is a renderer failure, except where
            # the case is the gap between events.
            if blank(frame) != name.endswith("-gap"):
                a.out.mkdir(parents=True, exist_ok=True)
                png_write(a.out / f"{name}.actual.png", frame)
                print(f"FAIL {name}: {'no subtitle drawn' if blank(frame) else 'subtitle drawn in a gap'}")
                failed += 1
                continue
            if a.update:
                a.golden.mkdir(parents=True, exist_ok=True)
                png_write(golden, frame)
                print(f"updated {golden}")
                continue
            if not golden.exists():
                print(f"FAIL {name}: no golden {golden} (run with --update)")
                failed += 1
                continue
            gw, gh, ref = png_read(golden)
            if (gw, gh) != (WIDTH, HEIGHT):
                print(f"FAIL {name}: golden is {gw}x{gh}, expected {WIDTH}x{HEIGHT}")
                failed += 1
                continue
            ok, summary, diff = compare(frame, ref)
            print(f"{'ok  ' if ok else 'FAIL'} {name}: {summary}")
            if not ok:
                failed += 1
                a.out.mkdir(parents=True, exist_ok=True)
                png_write(a.out / f"{name}.actual.png", frame)
                png_write(a.out / f"{name}.diff.png", diff)
    finally:
        mpv.close()
    if failed:
        print(f"{failed} case(s) failed; frames in {a.out}")
        return 1
    print("all subtitle golden frames match")
    return 0


if __name__ == "__main__":
    sys.exit(main())
