#!/usr/bin/env python3
# SPDX-License-Identifier: MPL-2.0
"""Offline stand-in for the TMDB title cache (META-007 candidate eval).

Writes one `TitleRecord` (pa_recognition::title) per movie, series and daily
show of facts.json: ids, kind, year and the titles a cached TMDB entry would
carry (original/English title and the ru-RU translation). Anime is not
written here: it comes from the bundled anime-offline-database.

Like tape.py this is NOT a TMDB recording (the project has no TMDB key yet,
DEC-008); it only holds the corpus works, so retrieval among movies and
shows is easier than against a real, larger cache. Re-record from the TMDB
client (pa_metadata::tmdb::titles::title_record) once a key exists.

Each record also carries what a cached TMDB details response would add
(META-014 pair features): `runtime_min` (movie runtime, or the median
episode runtime of a show) and `seasons` (regular episodes per season).

`--anime-tmdb` writes instead the anime series and films TMDB lists, built
from the anime franchises of titles.yaml the way the corpus labels them
(GUIDE §3: one TMDB series per franchise, the entry's own series when it
has one): the TMDB id, the earliest year and every name the franchise and
its entries go by. It feeds the synthetic TMDB of the META-013 matcher
(`pa-eval run --recognizer tmdb --synthetic-tmdb …`) so that baseline is
measured on anime as well; it is not part of the scorer's local index
(anime comes from anime-offline-database there).

Each anime series also carries `seasons`, the regular episodes per TMDB
season as the corpus places the entries (`tmdb_season` + `tmdb_offset` +
the entry's episode count), as a TMDB details response would (META-015:
absolute numbers → TMDB seasons in the library).

`--anime-lists` writes a synthetic list in the Anime-Lists
`anime-list-master.xml` format (META-015) for the numbering module: one
`<anime>` per AniDB entry of titles.yaml with its TMDB series, season and
offset and its TVDB absolute offset (`defaulttvdbseason="a"`) — the
maintainer rules the corpus labels follow, written in the format the device
downloads. It is NOT Anime-Lists data (that list has no licence and is
never committed, docs/legal/anime-mappings.md): TVDB ids are synthetic
(≥ 9 000 000, one per franchise) and only group the entries of a franchise.
Because labels and list come from the same rules, a recognizer scored with
it is measured given correct mappings, not on the quality of real ones.

Usage: catalog.py [--facts facts.json] [--anime-tmdb | --anime-lists] > out
"""

import argparse
import html
import json
import pathlib
import statistics
import sys

HERE = pathlib.Path(__file__).resolve().parent


def records(facts):
    for section, kind in (("movies", "movie"), ("shows", "series"), ("daily", "series")):
        for w in facts[section]:
            ids = {}
            for ns in ("tmdb", "tvdb", "imdb", "kinopoisk", "tvmaze"):
                if w.get(ns) is not None:
                    ids[ns] = str(w[ns])
            if not ids:
                continue
            titles = []
            for t in (w.get("title"), w.get("label_en"), w.get("label_ru")):
                if t and t not in titles:
                    titles.append(t)
            rec = {"ids": ids, "kind": kind}
            if w.get("year"):
                rec["year"] = w["year"]
            rec["titles"] = titles
            episodes = [e for e in w.get("episodes") or [] if e[0] >= 1]
            if episodes:
                seasons = {}
                for e in episodes:
                    seasons[e[0]] = max(seasons.get(e[0], 0), e[1])
                rec["seasons"] = {str(k): v for k, v in sorted(seasons.items())}
                runtimes = [e[3] for e in episodes if len(e) > 3 and e[3]]
                if runtimes:
                    rec["runtime_min"] = int(statistics.median(runtimes))
            elif w.get("runtime_min"):
                rec["runtime_min"] = w["runtime_min"]
            yield rec


def anime_tmdb(titles_path, facts):
    """TMDB anime series and films as the corpus labels them (generate.Data)."""
    import yaml  # noqa: PLC0415 - only this mode needs it

    sys.path.insert(0, str(HERE))
    import generate  # noqa: PLC0415

    titles = yaml.safe_load(pathlib.Path(titles_path).read_text(encoding="utf-8"))
    data = generate.Data(titles, facts)
    by_id = {}
    order = []

    def add(tmdb, kind, year, names):
        if tmdb not in by_id:
            by_id[tmdb] = {"ids": {"tmdb": tmdb}, "kind": kind, "year": year, "titles": []}
            order.append(tmdb)
        rec = by_id[tmdb]
        if year and (not rec["year"] or year < rec["year"]):
            rec["year"] = year
        for n in names:
            if n and n not in rec["titles"]:
                rec["titles"].append(n)

    seasons = {}
    for fr in data.franchises:
        for e in fr["entries"]:
            tm = data.anime_work(fr, e)["ids"].get("tmdb")
            if not tm or not tm.startswith("tv:"):
                continue
            f = e["fact"]
            names = list(fr["names"].values()) + list(e["names"].values()) + [f["title"], f.get("label_ja")]
            add(tm, "series", f.get("year"), names)
            if e.get("tmdb_season") and tm == fr.get("tmdb"):
                per = seasons.setdefault(tm, {})
                end = e.get("tmdb_offset", 0) + f["episodes"]
                per[e["tmdb_season"]] = max(per.get(e["tmdb_season"], 0), end)
    for tm, per in seasons.items():
        by_id[tm]["seasons"] = {str(k): v for k, v in sorted(per.items())}
    for m in data.anime_movies:
        tm = data.anime_movie_work(m)["ids"].get("tmdb")
        if not tm:
            continue
        f = m["fact"]
        add(tm, "movie", f.get("year"), list(m.get("names", {}).values()) + [f["title"], f.get("label_ja")])
    for tm in order:
        rec = by_id[tm]
        if not rec["year"]:
            del rec["year"]
        yield rec


def anime_lists(titles_path, facts):
    """Synthetic Anime-Lists XML of the corpus franchises (see module docs)."""
    import yaml  # noqa: PLC0415 - only this mode needs it

    sys.path.insert(0, str(HERE))
    import generate  # noqa: PLC0415

    titles = yaml.safe_load(pathlib.Path(titles_path).read_text(encoding="utf-8"))
    data = generate.Data(titles, facts)
    out = [
        '<?xml version="1.0" encoding="utf-8"?>',
        "<!-- SPDX-License-Identifier: MPL-2.0 -->",
        "<!-- Synthetic list in the Anime-Lists format, generated by",
        "     tools/recognition-eval/corpus/gen/catalog.py --anime-lists from titles.yaml",
        "     (maintainer rules of corpus v0). Not Anime-Lists data; TVDB ids are",
        "     synthetic (>= 9000000) and only group the entries of a franchise. -->",
        "<anime-list>",
    ]
    for k, fr in enumerate(data.franchises):
        tvdb = 9_000_000 + k
        for e in fr["entries"]:
            tm = data.anime_work(fr, e)["ids"].get("tmdb", "")
            tmdb_tv = tm.split(":", 1)[1] if tm.startswith("tv:") else ""
            season = e.get("tmdb_season") if tmdb_tv and tm == fr.get("tmdb") else None
            attrs = {
                "anidbid": str(e["aid"]),
                "tvdbid": str(tvdb),
                "defaulttvdbseason": "a",
                "episodeoffset": str(e["abs_offset"]) if e["abs_offset"] else "",
                "tmdbtv": tmdb_tv,
                "tmdbseason": str(season) if season else "",
                "tmdboffset": str(e.get("tmdb_offset", "")) if season and e.get("tmdb_offset") else "",
            }
            text = " ".join(f'{a}="{v}"' for a, v in attrs.items())
            out.append(f"  <anime {text}>")
            out.append(f"    <name>{html.escape(e['fact']['title'], quote=False)}</name>")
            out.append("  </anime>")
    out.append("</anime-list>")
    return out


def main():
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--facts", default=str(HERE / "facts.json"))
    ap.add_argument("--titles", default=str(HERE / "titles.yaml"))
    ap.add_argument("--anime-tmdb", action="store_true", help="write the anime TMDB stand-in instead")
    ap.add_argument("--anime-lists", action="store_true", help="write the synthetic Anime-Lists XML instead")
    args = ap.parse_args()
    facts = json.loads(pathlib.Path(args.facts).read_text(encoding="utf-8"))
    if args.anime_lists:
        sys.stdout.write("\n".join(anime_lists(args.titles, facts)) + "\n")
        return
    source = anime_tmdb(args.titles, facts) if args.anime_tmdb else records(facts)
    out = sorted(source, key=lambda r: (r["kind"], r["ids"].get("tmdb", ""), r["titles"][0]))
    seen = set()
    for rec in out:
        line = json.dumps(rec, ensure_ascii=False, sort_keys=False)
        if line not in seen:
            seen.add(line)
            sys.stdout.write(line + "\n")


if __name__ == "__main__":
    main()
