# SPDX-License-Identifier: MPL-2.0
"""Regenerates the synthetic Subdl fixtures (PLAY-016): python3 -I generate.py <this folder>."""
import json, os, sys, zipfile

out = sys.argv[1]
os.makedirs(out, exist_ok=True)
P = lambda n: os.path.join(out, n)

ass = """[Script Info]
; Synthetic subtitle for tests (PLAY-016)
Title: Lunar Harbor
ScriptType: v4.00+
PlayResX: 1920
PlayResY: 1080

[V4+ Styles]
Format: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, BackColour, Bold, Italic, Underline, StrikeOut, ScaleX, ScaleY, Spacing, Angle, BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding
Style: Default,Arial,64,&H00FFFFFF,&H000000FF,&H00000000,&H64000000,0,0,0,0,100,100,0,0,1,3,1,2,40,40,50,1
Style: Sign,Georgia,56,&H0000FFFF,&H000000FF,&H00000000,&H64000000,1,0,0,0,100,100,0,0,1,2,0,8,40,40,60,1

[Events]
Format: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text
Dialogue: 0,0:00:01.00,0:00:03.00,Default,,0,0,0,,{\\i1}Маяк снова горит.{\\i0}
Dialogue: 0,0:00:04.00,0:00:06.00,Sign,,0,0,0,,{\\pos(960,120)\\fad(200,200)}ГАВАНЬ
"""
with open(P("sample.ass"), "w", encoding="utf-8", newline="\n") as f:
    f.write(ass)


def srt(n, sdh=False):
    return f"1\n00:00:01,000 --> 00:00:02,500\n{'[ветер] ' if sdh else ''}Серия {n}, реплика.\n\n".encode()


def z(name, files):
    with zipfile.ZipFile(P(name), "w") as f:
        for n, data in files:
            zi = zipfile.ZipInfo(n, date_time=(2026, 11, 24, 0, 0, 0))
            zi.compress_type = zipfile.ZIP_DEFLATED if len(data) > 64 else zipfile.ZIP_STORED
            f.writestr(zi, data)


z("movie.zip", [("__MACOSX/._Lunar.Harbor.2019.1080p.BluRay.ass", b"\x00\x05\x16\x07junk"),
                ("Lunar.Harbor.2019.1080p.BluRay.ass", ass.encode())])
z("season.zip", [("Quiet.Coast.S01/", b""),
                 ("Quiet.Coast.S01/Quiet.Coast.S01E01.1080p.WEB.srt", srt(1)),
                 ("Quiet.Coast.S01/Quiet.Coast.S01E02.1080p.WEB.srt", srt(2)),
                 ("Quiet.Coast.S01/Quiet.Coast.S01E02.1080p.WEB.SDH.srt", srt(2, True)),
                 ("Quiet.Coast.S01/Quiet.Coast.S01E03.1080p.WEB.srt", srt(3))])
z("single.zip", [("readme.txt", b"thanks for downloading"), ("quiet coast ru.srt", srt(4))])


def sub(**kw):
    base = {"release_name": None, "name": None, "lang": None, "author": "synthetic", "url": None,
            "subtitlePage": None, "season": None, "episode": None, "episode_from": None,
            "episode_end": None, "full_season": False, "hi": False, "comment": "", "releases": []}
    base.update(kw)
    return base


common = {"comment": "1", "hi": "1", "releases": "1", "subs_per_page": "30"}
lines = []


def ex(query, status, body, headers=None):
    q = dict(common)
    q.update(query)
    lines.append({"request": {"path": "/subtitles", "query": q},
                  "response": {"status": status, "headers": headers or {}, "body": body}})


movie_subs = [
    sub(release_name="Lunar.Harbor.2019.1080p.BluRay", name="SUBDL::subtitle_900101.zip", lang="russian",
        language="RU", url="/subtitle/900101-900201.zip", subtitlePage="/s/info/SYNTH01/lunar-harbor",
        releases=["Lunar.Harbor.2019.1080p.BluRay"], download_count=120),
    sub(release_name="Lunar.Harbor.2019.720p.WEB", name="SUBDL::subtitle_900102.zip", lang="english",
        language="EN", url="/subtitle/900102-900202.zip", hi=1, comment="SDH",
        releases=["Lunar.Harbor.2019.720p.WEB"]),
    sub(release_name="Lunar.Harbor.2019.BRRip", name="SUBDL::subtitle_900103.zip",
        lang="brazillian portuguese", language="BR_PT", url="/subtitle/900103-900203.zip",
        releases=["Lunar.Harbor.2019.BRRip"]),
]
ex({"imdb_id": "tt0900001", "languages": "EN,RU", "type": "movie"}, 200,
   {"status": True, "results": [{"sd_id": 900001, "type": "movie", "name": "Lunar Harbor",
                                 "imdb_id": "tt0900001", "tmdb_id": 800001, "first_air_date": None,
                                 "year": 2019}], "subtitles": movie_subs})
ep_subs = [
    sub(release_name="Quiet.Coast.S01E02.1080p.WEB", name="SUBDL::subtitle_900301.zip", lang="english",
        language="EN", url="/subtitle/900301-900401.zip", season=1, episode=2,
        releases=["Quiet.Coast.S01E02.1080p.WEB"]),
    sub(release_name="Quiet.Coast.S01.1080p.WEB", name="SUBDL::subtitle_900302.zip", lang="russian",
        language="RU", url="/subtitle/900302-900402.zip", season=1, episode=0, full_season=True,
        releases=["Quiet.Coast.S01.1080p.WEB"]),
    sub(release_name="Quiet.Coast.S01E03.1080p.WEB", name="SUBDL::subtitle_900303.zip", lang="russian",
        language="RU", url="/subtitle/900303-900403.zip", season=1, episode=3,
        releases=["Quiet.Coast.S01E03.1080p.WEB"]),
]
epq = {"imdb_id": "tt0900002", "languages": "EN,RU", "type": "tv", "season_number": "1",
       "episode_number": "2"}
ex(epq, 429, {"status": False, "error": "rate_limit", "message": "Too many requests"}, {"retry-after": "3"})
ex(epq, 200, {"status": True, "results": [], "subtitles": ep_subs})
ex({"film_name": "Lunar Harbor", "year": "2019", "languages": "EN,RU", "type": "movie"}, 200,
   {"status": False, "error": "Can't find movie or TV"})
ex({"tmdb_id": "800009", "languages": "EN", "type": "movie"}, 429,
   {"status": False, "error": "daily_limit", "message": "Daily limit reached"})
ex({"tmdb_id": "800010", "languages": "EN", "type": "movie"}, 403,
   {"status": False, "statusCode": 403, "error": "not_authorized", "message": "Not Authorized"})
with open(P("cassette.jsonl"), "w", encoding="utf-8") as f:
    for line in lines:
        f.write(json.dumps(line, ensure_ascii=False) + "\n")
