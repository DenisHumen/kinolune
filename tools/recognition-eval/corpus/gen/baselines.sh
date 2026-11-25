#!/usr/bin/env bash
# SPDX-License-Identifier: MPL-2.0
#
# Regenerate corpus v0 and the pa-eval baseline reports (META-002).
# Offline: uses facts.json, no network. Run from anywhere:
#   tools/recognition-eval/corpus/gen/baselines.sh
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
corpus="$here/../v0"
out="$here/../../baselines/v0"
root="$(cd "$here/../../../.." && pwd)"
mkdir -p "$out"

python3 "$here/generate.py" --out-dir "$corpus"
cargo build --quiet --release --manifest-path "$root/Cargo.toml" -p pa-eval -p pa-recognition-eval
bin="${CARGO_TARGET_DIR:-$root/target}/release"

"$bin/corpus-validate" --quiet --min-per-category 50 "$corpus"/*.jsonl

misses="$(mktemp)"
trap 'rm -f "$misses"' EXIT
"$bin/pa-eval" run --recognizer naive --misses "$misses" --json /dev/null "$corpus"/*.jsonl
python3 "$here/tape.py" "$misses" > "$out/tmdb-search.tape.jsonl"

"$bin/pa-eval" run --recognizer abstain --json "$out/abstain.json" --md "$out/abstain.md" "$corpus"/*.jsonl
# Product filename parser (META-005/META-006): parse metrics only, no work matching.
"$bin/pa-eval" run --recognizer parser --json "$out/parser.json" --md "$out/parser.md" "$corpus"/*.jsonl
"$bin/pa-eval" run --recognizer naive --tape "$out/tmdb-search.tape.jsonl" \
    --json "$out/naive.json" --md "$out/naive.md" "$corpus"/*.jsonl
"$bin/pa-eval" run --recognizer naive --tape "$out/tmdb-search.tape.jsonl" --threshold 0.8 \
    --json "$out/naive-t080.json" --md "$out/naive-t080.md" "$corpus"/*.jsonl

# META-007: candidate recall of the local title index, no provider calls.
# Needs the bundled anime-offline-database (tools/anime-mappings/build.sh).
python3 "$here/catalog.py" > "$out/title-catalog.jsonl"
# META-014: the anime series and films TMDB lists, for the synthetic TMDB of
# the META-013 matcher on all kinds (not part of the scorer's index).
python3 "$here/catalog.py" --anime-tmdb > "$out/tmdb-anime-catalog.jsonl"

# META-013: the library's TMDB matcher with folder groups (pa-catalog) on the
# western subset (movie, show), against a synthetic TMDB search over
# title-catalog.jsonl (not TMDB data). Threshold 0.75 = accepted match, so
# Recall is the accuracy; 0.92 = the library's auto threshold.
tmdb=(--recognizer tmdb --synthetic-tmdb "$out/title-catalog.jsonl" --kinds movie,show)
"$bin/pa-eval" run "${tmdb[@]}" --threshold 0.75 --json "$out/tmdb.json" --md "$out/tmdb.md" \
    "$corpus/train.jsonl" "$corpus/dev.jsonl"
"$bin/pa-eval" run "${tmdb[@]}" --threshold 0.92 --json /dev/null --md "$out/tmdb-t092.md" \
    "$corpus/train.jsonl" "$corpus/dev.jsonl"
"$bin/pa-eval" run "${tmdb[@]}" --threshold 0.75 --json "$out/tmdb-test.json" --md "$out/tmdb-test.md" \
    "$corpus/test.jsonl"
"$bin/pa-eval" run "${tmdb[@]}" --threshold 0.92 --json /dev/null --md "$out/tmdb-test-t092.md" \
    "$corpus/test.jsonl"

aod="${AOD_BUNDLE:-$root/data/anime-mappings/dist/anime-offline-ids.jsonl}"
if [ -f "$aod" ]; then
    "$bin/pa-eval" candidates --anime "$aod" --catalog "$out/title-catalog.jsonl" \
        --json "$out/candidates.json" --md "$out/candidates.md" "$corpus/train.jsonl" "$corpus/dev.jsonl"
    "$bin/pa-eval" candidates --anime "$aod" --catalog "$out/title-catalog.jsonl" \
        --json "$out/candidates-test.json" --md "$out/candidates-test.md" "$corpus/test.jsonl"
    # META-026: scorer v0 over the same index — work, franchise entry, episode.
    "$bin/pa-eval" run --recognizer scorer --anime "$aod" --catalog "$out/title-catalog.jsonl" \
        --json "$out/scorer.json" --md "$out/scorer.md" "$corpus/train.jsonl" "$corpus/dev.jsonl"
    "$bin/pa-eval" run --recognizer scorer --anime "$aod" --catalog "$out/title-catalog.jsonl" \
        --json "$out/scorer-test.json" --md "$out/scorer-test.md" "$corpus/test.jsonl"

    # META-014: errors per category against META-013 at the library's auto
    # threshold 0.92, all kinds. meta013-all*.json are frozen reports of the
    # META-013 matcher (main before META-014) over title-catalog.jsonl plus
    # tmdb-anime-catalog.jsonl; the current scorer (local index) and the
    # current library matcher (pa-catalog, same synthetic TMDB) are compared
    # with them. `pa-eval errors` exits 1 below 30 %: report, do not stop.
    tmp="$(mktemp -d)"
    trap 'rm -rf "$misses" "$tmp"' EXIT
    all=(--synthetic-tmdb "$out/title-catalog.jsonl" --synthetic-tmdb "$out/tmdb-anime-catalog.jsonl")
    for split in train-dev test; do
        if [ "$split" = test ]; then files=("$corpus/test.jsonl"); sfx=-test; else
            files=("$corpus/train.jsonl" "$corpus/dev.jsonl"); sfx=; fi
        "$bin/pa-eval" run --recognizer scorer --anime "$aod" --catalog "$out/title-catalog.jsonl" \
            --threshold 0.92 --json "$tmp/scorer.json" --md /dev/null "${files[@]}"
        "$bin/pa-eval" run --recognizer tmdb "${all[@]}" --threshold 0.92 \
            --json "$tmp/tmdb.json" --md /dev/null "${files[@]}"
        "$bin/pa-eval" errors --baseline "$out/meta013-all$sfx.json" --current "$tmp/scorer.json" \
            --categories A2,A3,A8 --md "$out/meta014-errors$sfx.md" || true
        "$bin/pa-eval" errors --baseline "$out/meta013-all$sfx.json" --current "$tmp/tmdb.json" \
            --md "$out/meta014-library-errors$sfx.md" || true
    done

    # META-015: episode accuracy per category and scheme (AniDB, TMDB) at
    # the auto threshold 0.92, with the synthetic list in the Anime-Lists
    # format (maintainer rules of titles.yaml, not Anime-Lists data): the
    # scorer over the local index and the library matcher (pa-catalog
    # renumbering), the latter also without the list. `pa-eval episodes`
    # exits 1 below 97 %: report, do not stop.
    python3 "$here/catalog.py" --anime-lists > "$out/anime-lists-synthetic.xml"
    lists=(--anime-lists "$out/anime-lists-synthetic.xml")
    eps=(--categories A1,A2,A3,A4,A16 --schemes anidb,tmdb,tvdb_absolute)
    for split in train-dev test; do
        if [ "$split" = test ]; then files=("$corpus/test.jsonl"); sfx=-test; else
            files=("$corpus/train.jsonl" "$corpus/dev.jsonl"); sfx=; fi
        "$bin/pa-eval" run --recognizer scorer --anime "$aod" --catalog "$out/title-catalog.jsonl" \
            "${lists[@]}" --threshold 0.92 --json "$tmp/scorer15.json" --md /dev/null "${files[@]}"
        "$bin/pa-eval" run --recognizer tmdb "${all[@]}" "${lists[@]}" --threshold 0.92 \
            --json "$tmp/lib15.json" --md /dev/null "${files[@]}"
        "$bin/pa-eval" run --recognizer tmdb "${all[@]}" --threshold 0.92 \
            --json "$tmp/lib15-nolist.json" --md /dev/null "${files[@]}"
        "$bin/pa-eval" episodes --report "$tmp/scorer15.json" "${eps[@]}" \
            --md "$out/meta015-scorer-episodes$sfx.md" || true
        "$bin/pa-eval" episodes --report "$tmp/lib15.json" "${eps[@]}" \
            --md "$out/meta015-library-episodes$sfx.md" || true
        "$bin/pa-eval" episodes --report "$tmp/lib15-nolist.json" "${eps[@]}" \
            --md "$out/meta015-library-nolist-episodes$sfx.md" || true
        "$bin/pa-eval" errors --baseline "$out/meta013-all$sfx.json" --current "$tmp/lib15.json" \
            --md "$out/meta015-library-errors$sfx.md" || true

        # META-030: the library matcher with anime-offline-database on the
        # device (the bundle the app ships; plain or .zst) against the same
        # run without it: anime works TMDB search does not find get the TMDB
        # series of their AniDB entry through the (synthetic) list.
        "$bin/pa-eval" run --recognizer tmdb "${all[@]}" "${lists[@]}" --anime "$aod" \
            --threshold 0.92 --json "$tmp/lib30.json" --md /dev/null "${files[@]}"
        "$bin/pa-eval" errors --baseline "$tmp/lib15.json" --current "$tmp/lib30.json" \
            --md "$out/meta030-library-errors$sfx.md" || true
        "$bin/pa-eval" episodes --report "$tmp/lib30.json" --baseline "$tmp/lib15.json" \
            --categories A1,A2,A3,A4,A16 --schemes tmdb,tvdb_absolute \
            --md "$out/meta030-library-episodes$sfx.md" || true
    done
else
    echo "skipping candidate and scorer reports: $aod not found (run tools/anime-mappings/build.sh)" >&2
fi
