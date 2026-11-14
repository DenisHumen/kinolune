#!/usr/bin/env python3
# SPDX-License-Identifier: MPL-2.0
"""Generates a synthetic test season for intro and credits detection (PLAY-011).

Every episode: a cold open of its own length, the same 40-second intro
(music and picture), an episode body, the same end credits (white text
scrolling on black over the same ending song) and, in some episodes, a
short scene after the credits. One extra episode has no intro and no
credits (a special): nothing may be found there.

Writes `<out>/Show/Season 01/Show - S01Exx.mkv` and `<out>/truth.tsv`
(file, intro start/end, credits start/end in ms; `-` for none).

    tools/make-test-season.py --out /tmp/season                 # ffmpeg on PATH
    tools/make-test-season.py --out /tmp/season --docker IMAGE  # ffmpeg in Docker

GPL encoders (x264) are fine here: the files are test data and never ship.
"""

import argparse
import os
import subprocess
import sys

FPS = 25
SIZE = "640x360"
INTRO_S = 40
CREDITS_S = 50
EPISODE_S = 480
# (cold open seconds, scene after the credits in seconds)
EPISODES = [
    (0.0, 0),
    (12.5, 0),
    (31.0, 12),
    (47.3, 0),
    (5.0, 0),
    (60.7, 12),
    (22.0, 0),
    (38.4, 0),
]
SPECIAL_S = 300

# The intro theme: a melody of 0.2 s notes from a fixed pseudo-random
# sequence over a bass line, plus a hi-hat (noise gated every 0.25 s).
INTRO_AUDIO = (
    "aevalsrc="
    "'0.35*sin(2*PI*220*pow(2,mod(floor(t*5)*floor(t*5)*7+floor(t*5)*3,19)/12)*t)"
    "+0.2*sin(2*PI*110*pow(2,mod(floor(t*2.5)*5,12)/12)*t)"
    "+0.15*(random(0)-0.5)*lt(mod(t,0.25),0.04)'"
    f":s=48000:d={INTRO_S}"
)
# The ending song: another melody, slower.
ENDING_AUDIO = (
    "aevalsrc="
    "'0.3*sin(2*PI*330*pow(2,mod(floor(t*3)*11+floor(t*3)*floor(t*3)*5,17)/12)*t)"
    "+0.25*sin(2*PI*165*pow(2,mod(floor(t*1.5)*7,12)/12)*t)"
    "+0.1*(random(0)-0.5)*lt(mod(t,0.5),0.05)'"
    f":s=48000:d={CREDITS_S}"
)


def content_audio(seed: int, duration: float) -> str:
    """Speech-like audio of one episode: its own note sequence and noise
    (`anoisesrc` with the episode's seed: `random()` of aevalsrc would give
    every episode the same noise)."""
    mul = 5 + 2 * seed
    return (
        "aevalsrc="
        f"'0.25*sin(2*PI*140*pow(2,mod(floor(t*3.7)*{mul}+{seed * 3},23)/12)*t)"
        f"*(0.6+0.4*sin(2*PI*{0.3 + 0.07 * seed}*t))'"
        f":s=48000:d={duration:.3f}[t];"
        f"anoisesrc=d={duration:.3f}:c=pink:r=48000:a=0.12:seed={1000 + seed}[n];"
        "[t][n]amix=inputs=2:normalize=0[out0]"
    )


def content_video(seed: int, duration: float) -> str:
    return f"testsrc2=s={SIZE}:r={FPS}:d={duration:.3f},hue=h={seed * 40}"


def credits_video(text_path: str) -> str:
    # Lines scroll up at 40 px/s.
    return (
        f"color=c=black:s={SIZE}:r={FPS}:d={CREDITS_S},"
        f"drawtext=textfile='{text_path}':fontcolor=white:fontsize=20:line_spacing=14:"
        "x=(w-text_w)/2:y=h-40*t"
    )


def run(cmd: list[str]) -> None:
    print("+", " ".join(cmd[:6]), "…", file=sys.stderr)
    subprocess.run(cmd, check=True)


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--out", required=True)
    ap.add_argument("--docker", help="Docker image whose entrypoint can be ffmpeg")
    ap.add_argument("--threads", type=int, default=4)
    args = ap.parse_args()

    out = os.path.abspath(args.out)
    season = os.path.join(out, "Show", "Season 01")
    os.makedirs(season, exist_ok=True)

    if args.docker:
        prefix = [
            "docker", "run", "--rm", "-u", f"{os.getuid()}:{os.getgid()}",
            "-v", f"{out}:/w", "--entrypoint", "ffmpeg", args.docker,
        ]
        def path(p: str) -> str:
            return "/w/" + os.path.relpath(p, out)
    else:
        prefix = ["ffmpeg"]
        def path(p: str) -> str:
            return p
    ff = prefix + ["-hide_banner", "-loglevel", "error", "-y", "-threads", str(args.threads)]
    enc = [
        "-c:v", "libx264", "-preset", "veryfast", "-pix_fmt", "yuv420p",
        "-c:a", "aac", "-b:a", "128k", "-ac", "2",
    ]

    credits_txt = os.path.join(out, "credits.txt")
    with open(credits_txt, "w", encoding="utf-8") as f:
        roles = ["Directed by", "Written by", "Music by", "Edited by", "Produced by"]
        for i in range(60):
            f.write(f"{roles[i % len(roles)]}   Person Number {i + 1}\n")

    # Shared intro (mandelbrot zoom with a title) and the ending song.
    intro = os.path.join(out, "intro.mkv")
    run(ff + [
        "-f", "lavfi", "-i",
        f"mandelbrot=s={SIZE}:rate={FPS},trim=duration={INTRO_S},"
        "drawtext=text='THE SHOW':fontcolor=white:fontsize=48:x=(w-text_w)/2:y=(h-text_h)/2",
        "-f", "lavfi", "-i", INTRO_AUDIO,
        "-t", str(INTRO_S), *enc, path(intro),
    ])
    ending = os.path.join(out, "ending.wav")
    run(ff + ["-f", "lavfi", "-i", ENDING_AUDIO, "-ac", "2", path(ending)])

    rows = []
    for n, (cold, after) in enumerate(EPISODES, start=1):
        body = EPISODE_S - cold - INTRO_S - CREDITS_S - after
        name = f"Show - S01E{n:02d}.mkv"
        parts_v, parts_a, inputs = [], [], []

        def count_inputs() -> int:
            return inputs.count("-i")

        def add(video: str, audio: str, is_file: bool = False) -> None:
            k = count_inputs()
            if is_file:
                inputs.extend(["-i", video])
                parts_v.append(f"[{k}:v]")
                parts_a.append(f"[{k}:a]")
            else:
                inputs.extend(["-f", "lavfi", "-i", video, "-f", "lavfi", "-i", audio])
                parts_v.append(f"[{k}:v]")
                parts_a.append(f"[{k + 1}:a]")

        if cold > 0:
            add(content_video(n, cold), content_audio(n, cold))
        add(path(intro), "", is_file=True)
        add(content_video(n + 10, body), content_audio(n + 10, body))
        k = count_inputs()
        inputs.extend([
            "-f", "lavfi", "-i", credits_video(path(credits_txt)),
            "-i", path(ending),
        ])
        parts_v.append(f"[{k}:v]")
        parts_a.append(f"[{k + 1}:a]")
        if after:
            add(content_video(n + 20, after), content_audio(n + 20, after))
        count = len(parts_v)
        chain = "".join(
            f"{parts_v[i]}setsar=1,fps={FPS},format=yuv420p[v{i}];"
            f"{parts_a[i]}aformat=sample_rates=48000:channel_layouts=stereo[a{i}];"
            for i in range(count)
        )
        chain += "".join(f"[v{i}][a{i}]" for i in range(count))
        chain += f"concat=n={count}:v=1:a=1[v][a]"
        run(ff + inputs + [
            "-filter_complex", chain, "-map", "[v]", "-map", "[a]",
            *enc, path(os.path.join(season, name)),
        ])
        intro_start = round(cold * 1000)
        credits_start = round((cold + INTRO_S + body) * 1000)
        rows.append((
            name, intro_start, intro_start + INTRO_S * 1000,
            credits_start, credits_start + CREDITS_S * 1000,
        ))

    # A special: no intro, no credits.
    name = "Show - S01E09.mkv"
    run(ff + [
        "-f", "lavfi", "-i", content_video(99, SPECIAL_S),
        "-f", "lavfi", "-i", content_audio(99, SPECIAL_S),
        *enc, path(os.path.join(season, name)),
    ])
    rows.append((name, None, None, None, None))

    with open(os.path.join(out, "truth.tsv"), "w", encoding="utf-8") as f:
        f.write("# file\tintro_start_ms\tintro_end_ms\tcredits_start_ms\tcredits_end_ms\n")
        for row in rows:
            f.write("\t".join("-" if v is None else str(v) for v in row) + "\n")
    print(f"wrote {len(rows)} episodes to {season}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
