-- SPDX-License-Identifier: MPL-2.0
--
-- MUS-019 (ADR-NNNN): what Open Library answered for an audiobook. Written
-- by `pa_catalog::Catalog::fetch_book_metadata`, read by
-- `Catalog::audiobooks`. Expand-only (LIB-002).
--
-- One row per question, not per book: `query` is the folded author and
-- title (`pa_catalog::book_metadata::BookQuery::key`), so the same book on
-- two sources is asked for once and a renamed book is asked for again.
-- Books are made from files on every read (ADR-0107); a row no book asks
-- for any more is harmless and a reset deletes everything.
--
-- outcome   'matched'   same title and author: cover and description shown;
--           'review'    close but not sure: nothing shown, book marked;
--           'not_found' nothing close (asked again after a month);
--           'failed'    network or provider error (asked again soon).

CREATE TABLE audiobook_metadata (
    query         TEXT    PRIMARY KEY,
    outcome       TEXT    NOT NULL
                  CHECK (outcome IN ('matched', 'review', 'not_found', 'failed')),
    confidence    REAL    CHECK (confidence BETWEEN 0 AND 1),
    -- Open Library work id (`OL123W`).
    work_id       TEXT    CHECK (work_id IS NULL OR work_id GLOB 'OL[0-9]*W'),
    title         TEXT,
    -- Authors as Open Library names them, JSON array of strings.
    authors       TEXT    NOT NULL DEFAULT '[]'
                  CHECK (json_valid(authors) AND json_type(authors) = 'array'),
    first_year    INTEGER CHECK (first_year IS NULL OR first_year BETWEEN -3000 AND 3000),
    -- `https://covers.openlibrary.org/b/id/<id>-L.jpg`; the image is
    -- downloaded by the image cache on the device only.
    cover_url     TEXT    CHECK (cover_url IS NULL OR cover_url GLOB 'https://*'),
    description   TEXT,
    error         TEXT,
    fetched_at_ms INTEGER NOT NULL,
    CHECK (outcome IN ('matched', 'review') OR (work_id IS NULL AND cover_url IS NULL
                                                 AND description IS NULL)),
    CHECK (outcome NOT IN ('matched', 'review') OR work_id IS NOT NULL)
) STRICT, WITHOUT ROWID;
