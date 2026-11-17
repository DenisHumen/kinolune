#!/usr/bin/env python3
# SPDX-License-Identifier: MPL-2.0
"""Writes a small AVI for the audio-device smoke tests (PKG-021).

Uncompressed BGR24 video (a coloured background with a moving bar and a
seconds counter strip) and a quiet 440 Hz PCM stereo tone, so libmpv opens
a real audio output and a video decoder. Pure stdlib, no ffmpeg needed:

    make_sample.py OUT.avi [--seconds 40]
"""

from __future__ import annotations

import argparse
import math
import struct
from pathlib import Path

W, H, FPS = 160, 90, 10
RATE, CHANNELS = 48000, 2
BLOCK = CHANNELS * 2  # s16


def chunk(fourcc: bytes, data: bytes) -> bytes:
    pad = b"\0" if len(data) % 2 else b""
    return fourcc + struct.pack("<I", len(data)) + data + pad


def riff_list(kind: bytes, fourcc: bytes, data: bytes) -> bytes:
    return kind + struct.pack("<I", len(data) + 4) + fourcc + data


def frame(i: int) -> bytes:
    # Bottom-up BGR rows (biHeight > 0). Background hue changes every second.
    sec = i // FPS
    bg = [(140, 60, 30), (40, 120, 40), (30, 50, 150), (120, 40, 120)][sec % 4]
    bar_x = (i * 4) % W
    row = bytearray(bytes(bg) * W)
    for x in range(bar_x, min(bar_x + 12, W)):
        row[3 * x : 3 * x + 3] = b"\xff\xff\xff"
    row = bytes(row)
    # A strip at the top whose length counts the seconds.
    strip = bytearray(bytes(bg) * W)
    for x in range(min((sec + 1) * 4, W)):
        strip[3 * x : 3 * x + 3] = b"\xf0\xf0\xf0"
    strip = bytes(strip)
    return row * (H - 8) + strip * 8


def tone(n0: int, count: int) -> bytes:
    out = bytearray()
    for n in range(n0, n0 + count):
        s = int(3000 * math.sin(2 * math.pi * 440 * n / RATE))
        out += struct.pack("<hh", s, s)
    return bytes(out)


def write_avi(path: Path, seconds: int) -> None:
    frames = seconds * FPS
    vsize = W * H * 3
    per_frame = RATE // FPS
    asize = per_frame * BLOCK

    avih = struct.pack(
        "<14I",
        1_000_000 // FPS,  # dwMicroSecPerFrame
        (vsize + asize) * FPS,  # dwMaxBytesPerSec
        0,
        0x10 | 0x100,  # AVIF_HASINDEX | AVIF_ISINTERLEAVED
        frames,
        0,
        2,  # streams
        vsize,
        W,
        H,
        0, 0, 0, 0,
    )
    strh_v = struct.pack(
        "<4s4sIHHIIIIIIII4h",
        b"vids", b"\0\0\0\0", 0, 0, 0, 0, 1, FPS, 0, frames, vsize, 0xFFFFFFFF, 0,
        0, 0, W, H,
    )
    strf_v = struct.pack("<IiiHHIIiiII", 40, W, H, 1, 24, 0, vsize, 0, 0, 0, 0)
    strh_a = struct.pack(
        "<4s4sIHHIIIIIIII4h",
        b"auds", b"\0\0\0\0", 0, 0, 0, 0, BLOCK, RATE * BLOCK, 0,
        frames * per_frame, asize, 0xFFFFFFFF, BLOCK, 0, 0, 0, 0,
    )
    strf_a = struct.pack("<HHIIHHH", 1, CHANNELS, RATE, RATE * BLOCK, BLOCK, 16, 0)
    hdrl = riff_list(
        b"LIST",
        b"hdrl",
        chunk(b"avih", avih)
        + riff_list(b"LIST", b"strl", chunk(b"strh", strh_v) + chunk(b"strf", strf_v))
        + riff_list(b"LIST", b"strl", chunk(b"strh", strh_a) + chunk(b"strf", strf_a)),
    )

    movi = bytearray()
    index = bytearray()
    for i in range(frames):
        for fourcc, data in ((b"00db", frame(i)), (b"01wb", tone(i * per_frame, per_frame))):
            # idx1 offsets count from the "movi" fourcc.
            index += struct.pack("<4sIII", fourcc, 0x10, len(movi) + 4, len(data))
            movi += chunk(fourcc, data)
    body = hdrl + riff_list(b"LIST", b"movi", bytes(movi)) + chunk(b"idx1", bytes(index))
    path.write_bytes(riff_list(b"RIFF", b"AVI ", body))


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("out", type=Path)
    ap.add_argument("--seconds", type=int, default=40)
    args = ap.parse_args()
    write_avi(args.out, args.seconds)
    print(f"{args.out}: {args.out.stat().st_size} bytes, {args.seconds} s")


if __name__ == "__main__":
    main()
