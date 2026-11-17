// SPDX-License-Identifier: MPL-2.0
//! The audiobook library (MUS-018, ADR-0107).
//!
//! The user never says what is a book: the content kind classifier
//! (LIB-025, `file_content_kinds.kind = 'audiobook'`) marks the tracks, and
//! [`Catalog::audiobooks`] puts them together:
//!
//! - a **book is a folder** of parts (mp3, flac, ogg…); `CD1`, `Disc 2`,
//!   `Диск 3`, `Часть 1` subfolders belong to the folder above;
//! - in a folder, every **album tag** is its own book, and a file without
//!   one that has **chapters** (m4b, m4a) is a book by itself;
//! - **chapters** are the chapters inside a file (`"chapter_list"` of
//!   `files.embedded_tags_json`, written by `pa-probe`), else one chapter
//!   per part;
//! - **author, narrator, series and number** come from the tags (album
//!   artist, `narrator`, `series`, `series-part`, MP4 movement), then from
//!   the path (`Автор - Название (читает Чтец)`, `Серия/02 - Название`);
//!   the composer of a book is its narrator when it is not the author
//!   (how Audible files keep it).
//!
//! **Where the listener stopped** is one position per book, in
//! milliseconds from the start of the book, stored and synced like any
//! other progress (`pa_library::progress`, SYNC-001) under the **book
//! key**: the `file:` key (content fingerprint) of the book's first part.
//! It is a protocol v1 key, so every paired device keeps and forwards it;
//! a device with the same first file finds the same book.

use std::cmp::Ordering;
use std::collections::HashMap;

use pa_library::progress::{self as lib_progress};
use pa_library::rusqlite::Connection;
use pa_model::{MediaKey, MediaKind};

use crate::book_metadata::{BookQuery, stored_books};
use crate::book_names::{NameInfo, author_name, generic, name_info, person_like, series_number};
use crate::items::{Item, ItemRef};
use crate::music::{MusicTrack, music_tracks};
use crate::progress::profile;
use crate::{Catalog, Error};

/// A chapter of a book.
#[derive(Debug, Clone, PartialEq)]
pub struct BookChapter {
    /// The chapter's own title; `None` when the file has none (the app
    /// numbers it).
    pub title: Option<String>,
    /// Index of the part (into [`Audiobook::parts`]) the chapter is in.
    pub part: u32,
    /// Start within the part, milliseconds.
    pub start_ms: i64,
    /// Start within the whole book, milliseconds.
    pub book_start_ms: i64,
    /// 0 if unknown.
    pub duration_ms: i64,
}

/// Where the listener stopped in a book (on the active profile).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BookProgress {
    /// Milliseconds from the start of the book.
    pub position_ms: i64,
    /// The book's length when the position was saved; 0 if unknown.
    pub duration_ms: i64,
    pub updated_at_ms: i64,
    /// Listened to the end (past 95 %).
    pub finished: bool,
}

/// An audiobook: its parts in play order and its chapters.
#[derive(Debug, Clone, PartialEq)]
pub struct Audiobook {
    /// The book key (`file:<fingerprint>` of the first part) the position
    /// is stored and synced under; `None` until the first part is
    /// fingerprinted (then nothing is saved).
    pub key: Option<String>,
    pub title: String,
    pub author: Option<String>,
    pub narrator: Option<String>,
    pub series: Option<String>,
    /// Number in the series (`2`, `2.5`).
    pub series_number: Option<f64>,
    pub year: Option<i64>,
    pub source_id: i64,
    /// The book's folder (a single-file book: the file), relative to the
    /// source root.
    pub rel_path: String,
    /// Folder cover of the first part (relative to the source root), as
    /// [`Item::local_poster`].
    pub local_poster: Option<String>,
    /// Open Library cover of the matched work (MUS-019), else the Cover Art
    /// Archive cover of the album, when the music metadata run found one.
    pub cover_url: Option<String>,
    /// Description of the matched Open Library work (MUS-019).
    pub description: Option<String>,
    /// The matched Open Library work (`OL123W`).
    pub open_library_id: Option<String>,
    /// Not sure what the book is: no author in the tags or names, or Open
    /// Library has a close but not the same work. Nothing from the
    /// internet is shown for it.
    pub needs_review: bool,
    /// Sum of the parts; 0 if no part length is known.
    pub duration_ms: i64,
    /// Newest file of the book.
    pub added_at_ms: Option<i64>,
    /// The parts (tracks) in play order.
    pub parts: Vec<Item>,
    pub chapters: Vec<BookChapter>,
    pub progress: Option<BookProgress>,
}

/// Chapters inside a file: start, end (ms) and title.
type FileChapters = Vec<(i64, i64, Option<String>)>;

/// One audiobook file with what [`books_of`] needs.
#[derive(Debug, Clone)]
struct BookFile {
    track: MusicTrack,
    fingerprint: Option<String>,
    chapters: FileChapters,
    narrator: Option<String>,
    series: Option<String>,
    series_part: Option<String>,
}

/// `chapter_list` and `book` of `embedded_tags_json` (`pa_probe::store`).
fn embedded(json: Option<&str>) -> (FileChapters, HashMap<String, String>) {
    let Some(v) = json.and_then(|j| serde_json::from_str::<serde_json::Value>(j).ok()) else {
        return (Vec::new(), HashMap::new());
    };
    let chapters = v["chapter_list"]
        .as_array()
        .map(|list| {
            list.iter()
                .filter_map(|c| {
                    let start = c.get(0)?.as_i64()?;
                    let end = c.get(1)?.as_i64()?;
                    let title = c.get(2).and_then(|t| t.as_str()).map(str::to_owned);
                    Some((start.max(0), end.max(start), title))
                })
                .collect()
        })
        .unwrap_or_default();
    let book = v["book"]
        .as_object()
        .map(|o| {
            o.iter()
                .filter_map(|(k, v)| v.as_str().map(|v| (k.clone(), v.to_owned())))
                .collect()
        })
        .unwrap_or_default();
    (chapters, book)
}

/// Fingerprint and embedded tags of every audiobook track, by track id.
const BOOK_FILES_SQL: &str = "
SELECT t.id, f.fingerprint, f.embedded_tags_json
FROM music_tracks t
JOIN files f ON f.id = t.file_id AND f.missing_since_ms IS NULL
JOIN file_content_kinds k ON k.file_id = f.id AND k.kind = 'audiobook'";

fn book_files(conn: &Connection) -> Result<Vec<BookFile>, Error> {
    let mut extra: HashMap<i64, (Option<String>, Option<String>)> = HashMap::new();
    let mut stmt = conn.prepare_cached(BOOK_FILES_SQL)?;
    let mut rows = stmt.query([])?;
    while let Some(r) = rows.next()? {
        extra.insert(r.get(0)?, (r.get(1)?, r.get(2)?));
    }
    drop(rows);
    let mut out = Vec::new();
    for track in music_tracks(conn)? {
        let ItemRef::Track(id) = track.item.id else {
            continue;
        };
        if !track.audiobook {
            continue;
        }
        let Some((fingerprint, json)) = extra.remove(&id) else {
            continue;
        };
        let (chapters, mut tags) = embedded(json.as_deref());
        out.push(BookFile {
            track,
            fingerprint,
            chapters,
            narrator: tags.remove("narrator"),
            series: tags.remove("series"),
            series_part: tags.remove("series_part"),
        });
    }
    Ok(out)
}

// -- paths ---------------------------------------------------------------------

/// `CD1`, `CD 2`, `Disc 1`, `Disk2`, `Диск 1`, `Часть 2`, `Part 3`: a
/// subfolder of one book.
fn disc_folder(name: &str) -> bool {
    let lower = name.trim().to_lowercase();
    let rest = ["cd", "disc", "disk", "диск", "часть", "part", "pt"]
        .iter()
        .find_map(|p| lower.strip_prefix(p));
    match rest {
        Some(r) => {
            let r = r.trim_start_matches([' ', '.', '_', '-']);
            !r.is_empty() && r.len() <= 3 && r.bytes().all(|b| b.is_ascii_digit())
        }
        None => false,
    }
}

fn parent(path: &str) -> &str {
    path.rsplit_once('/').map_or("", |(p, _)| p)
}

fn last(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

/// The folder of the book a file at `rel_path` belongs to.
fn book_folder(rel_path: &str) -> &str {
    let dir = parent(rel_path);
    if disc_folder(last(dir)) {
        parent(dir)
    } else {
        dir
    }
}

/// Natural order: digit runs compare by value (`2` before `10`), the rest
/// without case.
fn natural(a: &str, b: &str) -> Ordering {
    let (mut x, mut y) = (a.chars().peekable(), b.chars().peekable());
    loop {
        match (x.peek().copied(), y.peek().copied()) {
            (None, None) => return a.cmp(b),
            (None, _) => return Ordering::Less,
            (_, None) => return Ordering::Greater,
            (Some(c), Some(d)) if c.is_ascii_digit() && d.is_ascii_digit() => {
                let mut n = String::new();
                while let Some(c) = x.next_if(char::is_ascii_digit) {
                    n.push(c);
                }
                let mut m = String::new();
                while let Some(d) = y.next_if(char::is_ascii_digit) {
                    m.push(d);
                }
                let (n, m) = (n.trim_start_matches('0'), m.trim_start_matches('0'));
                let o = n.len().cmp(&m.len()).then_with(|| n.cmp(m));
                if o != Ordering::Equal {
                    return o;
                }
            }
            (Some(c), Some(d)) => {
                let o = c.to_lowercase().cmp(d.to_lowercase());
                if o != Ordering::Equal {
                    return o;
                }
                x.next();
                y.next();
            }
        }
    }
}

/// The value most parts agree on (ties: the first in play order).
fn most_common<'a>(values: impl Iterator<Item = &'a str>) -> Option<String> {
    let mut counts: Vec<(&str, usize)> = Vec::new();
    for v in values {
        let v = v.trim();
        if v.is_empty() {
            continue;
        }
        match counts.iter_mut().find(|(x, _)| x.eq_ignore_ascii_case(v)) {
            Some((_, n)) => *n += 1,
            None => counts.push((v, 1)),
        }
    }
    let best = counts.iter().map(|(_, n)| *n).max()?;
    counts
        .into_iter()
        .find(|(_, n)| *n == best)
        .map(|(v, _)| v.to_owned())
}

fn is_various(name: &str) -> bool {
    let l = name.trim().to_lowercase();
    l == "various artists" || l == "various" || l == "сборник" || l == "разные исполнители"
}

// -- grouping ------------------------------------------------------------------

fn sort_parts(files: &mut [BookFile]) {
    files.sort_by(|a, b| {
        let (ia, ib) = (&a.track.item, &b.track.item);
        a.track
            .disc_number
            .unwrap_or(0)
            .cmp(&b.track.disc_number.unwrap_or(0))
            .then_with(|| natural(parent(&ia.rel_path), parent(&ib.rel_path)))
            .then_with(|| {
                ia.track_number
                    .unwrap_or(0)
                    .cmp(&ib.track_number.unwrap_or(0))
            })
            .then_with(|| natural(last(&ia.rel_path), last(&ib.rel_path)))
    });
}

/// Same name, ignoring case, word order and punctuation (`Сапковский
/// Анджей` = `Анджей Сапковский`).
fn same_name(a: &str, b: &str) -> bool {
    let words = |s: &str| {
        let mut w: Vec<String> = s
            .split(|c: char| !c.is_alphanumeric())
            .filter(|w| !w.is_empty())
            .map(str::to_lowercase)
            .collect();
        w.sort();
        w
    };
    let (x, y) = (words(a), words(b));
    !x.is_empty() && x == y
}

fn word_count(s: &str) -> usize {
    s.split_whitespace().count()
}

/// The artist of a file when a tag gave it (not the scanner's guess from
/// the path, `pa_scanner::with_path_fallback`).
fn tagged_artist(f: &BookFile) -> Option<&str> {
    not_from_path(f, f.track.artists.first()?)
}

/// The album artist of a file when a tag gave it: without one, the album
/// takes the track artist, which may be the scanner's guess from the path.
fn tagged_album_artist(f: &BookFile) -> Option<&str> {
    not_from_path(f, f.track.album_artists.first()?)
}

fn not_from_path<'a>(f: &BookFile, artist: &'a str) -> Option<&'a str> {
    let artist = artist.trim();
    let from_path = pa_scanner::path_info(&f.track.item.rel_path).artist;
    (!artist.is_empty() && from_path.as_deref() != Some(artist)).then_some(artist)
}

/// Artist tags that name nobody.
fn unknown_artist(name: &str) -> bool {
    let l = name.trim().to_lowercase();
    is_various(name)
        || generic(name)
        || [
            "unknown",
            "unknown artist",
            "неизвестен",
            "неизвестный",
            "неизвестный исполнитель",
            "аудиокнига",
            "audiobook",
        ]
        .contains(&l.as_str())
}

/// One book of `files` (in play order), found in `folder`.
fn book(folder: &str, files: Vec<BookFile>) -> Audiobook {
    let first = &files[0];
    let single = files.len() == 1 && !first.chapters.is_empty();
    let item = &first.track.item;
    let rel_path = if single {
        item.rel_path.clone()
    } else {
        folder.to_owned()
    };
    let folder_name = if folder.is_empty() {
        item.title.clone()
    } else {
        last(folder).to_owned()
    };
    let album = most_common(files.iter().filter_map(tagged_album));
    // Named by the album tag, else a single file by its own title, else
    // by the folder.
    let raw_title = album.clone().unwrap_or_else(|| {
        if single {
            item.title.clone()
        } else {
            folder_name.clone()
        }
    });
    let from_title = name_info(&raw_title);
    // A single file also says something by its name; a folder by its own.
    let from_file = single.then(|| {
        let name = last(&item.rel_path);
        name_info(name.rsplit_once('.').map_or(name, |(s, _)| s))
    });
    let from_folder = name_info(&folder_name);
    let names: Vec<&NameInfo> = [Some(&from_title), from_file.as_ref(), Some(&from_folder)]
        .into_iter()
        .flatten()
        .collect();
    let named =
        |pick: fn(&NameInfo) -> Option<&String>| names.iter().find_map(|n| pick(n)).cloned();

    let tag = |pick: fn(&BookFile) -> Option<&str>| most_common(files.iter().filter_map(pick));
    let usable = |a: &String| !unknown_artist(a);
    let title = if from_title.title.trim().is_empty() {
        raw_title.clone()
    } else {
        from_title.title.clone()
    };
    let mut author = tag(tagged_album_artist)
        .filter(usable)
        .or_else(|| tag(tagged_artist).filter(usable))
        .or_else(|| named(|n| n.author.as_ref()).filter(usable))
        .map(|a| author_name(&a));
    let narrator = tag(|f| f.narrator.as_deref())
        .or_else(|| named(|n| n.narrator.as_ref()))
        .or_else(|| {
            tag(|f| f.track.composers.first().map(String::as_str))
                .filter(|c| author.as_ref().is_none_or(|a| !same_name(a, c)))
        });
    let mut series = tag(|f| f.series.as_deref()).or_else(|| named(|n| n.series.as_ref()));
    let mut number = tag(|f| f.series_part.as_deref())
        .and_then(|s| series_number(&s))
        .or_else(|| names.iter().find_map(|n| n.series.as_ref().and(n.number)));
    let path_number = names.iter().find_map(|n| n.number);

    // The folders above: `Автор/Серия/02 - Название`, `Автор/Название`.
    // A single file in a folder named after it: the folder is the book's.
    let up1 = if !single || same_name(last(folder), &title) {
        parent(folder)
    } else {
        folder
    };
    fn naming(a: &str) -> Option<&str> {
        Some(a).filter(|a| !a.is_empty() && !generic(a))
    }
    let a0 = naming(last(up1));
    let a1 = naming(last(parent(up1)));
    let person = |a: &str| person_like(a) && !unknown_artist(a);
    let not_author =
        |a: &str, author: &Option<String>| author.as_ref().is_none_or(|x| !same_name(x, a));
    if author.is_none() {
        match (a0, a1) {
            // `Сапковский Анджей/Ведьмак/…`: an author above a series.
            (Some(s), Some(a))
                if person(a)
                    && (path_number.is_some()
                        || series.is_some()
                        || (word_count(a) >= 2 && !(person(s) && word_count(s) >= 2))) =>
            {
                author = Some(author_name(a));
                if series.is_none() {
                    series = Some(name_info(s).title);
                }
            }
            (Some(a), _) if person(a) && path_number.is_none() => {
                author = Some(author_name(a));
            }
            _ => {}
        }
    }
    if series.is_none()
        && let Some(s) = a0
        && not_author(s, &author)
        && (path_number.is_some()
            || a1.is_some_and(|a| author.as_ref().is_some_and(|x| same_name(x, a))))
    {
        // `Серия/02 - Название`, `Автор/Серия/Название`.
        series = Some(name_info(s).title);
    }
    if series.is_some() {
        number = number.or(path_number);
    }
    let year = files
        .iter()
        .find_map(|f| f.track.album_year.or(f.track.item.year))
        .or_else(|| names.iter().find_map(|n| n.year));

    // Parts and chapters.
    let mut chapters = Vec::new();
    let mut offset = 0i64;
    for (i, f) in files.iter().enumerate() {
        let part = u32::try_from(i).unwrap_or(u32::MAX);
        let len = f.track.item.duration_ms.unwrap_or(0).max(0);
        if f.chapters.len() > 1 {
            for (start, end, t) in &f.chapters {
                chapters.push(BookChapter {
                    title: t.clone(),
                    part,
                    start_ms: *start,
                    book_start_ms: offset + start,
                    duration_ms: (end - start).max(0),
                });
            }
        } else {
            let t = &f.track.item.title;
            chapters.push(BookChapter {
                title: (!t.trim().is_empty()).then(|| t.clone()),
                part,
                start_ms: 0,
                book_start_ms: offset,
                duration_ms: len,
            });
        }
        // The probe knows the file length; chapters end where it ends.
        let chapter_end = f.chapters.last().map_or(0, |c| c.1);
        offset += len.max(chapter_end);
    }

    let has_author = author.is_some();
    Audiobook {
        key: first
            .fingerprint
            .clone()
            .map(|fingerprint| MediaKey::File { fingerprint }.to_string()),
        title,
        author,
        narrator,
        series,
        series_number: number,
        year,
        source_id: item.source_id,
        rel_path,
        local_poster: files.iter().find_map(|f| f.track.item.local_poster.clone()),
        cover_url: files.iter().find_map(|f| f.track.cover_url.clone()),
        description: None,
        open_library_id: None,
        needs_review: !has_author,
        duration_ms: offset,
        added_at_ms: files.iter().filter_map(|f| f.track.item.added_at_ms).max(),
        parts: files.iter().map(|f| f.track.item.clone()).collect(),
        chapters,
        progress: None,
    }
}

/// The album of a file when a tag gave it: the scanner names the album of
/// an untagged file after its folder (`pa_scanner::path_info`), which says
/// nothing about books.
fn tagged_album(f: &BookFile) -> Option<&str> {
    let album = f.track.item.album.as_deref().map(str::trim)?;
    let from_path = pa_scanner::path_info(&f.track.item.rel_path).album;
    (!album.is_empty() && from_path.as_deref() != Some(album)).then_some(album)
}

/// Audiobook files grouped into books (see the module docs), ordered by
/// author, series, number in the series and title.
fn books_of(files: Vec<BookFile>) -> Vec<Audiobook> {
    // (source, folder) → album tag (lowercase) or the file itself → files.
    let mut groups: Vec<((i64, String, String), Vec<BookFile>)> = Vec::new();
    for f in files {
        let item = &f.track.item;
        let folder = book_folder(&item.rel_path).to_owned();
        let album = tagged_album(&f).map(str::to_lowercase);
        let sub = match album {
            Some(a) => format!("album:{a}"),
            None if f.chapters.len() > 1 => format!("file:{}", item.rel_path),
            None => String::new(),
        };
        let key = (item.source_id, folder, sub);
        match groups.iter_mut().find(|(k, _)| *k == key) {
            Some((_, v)) => v.push(f),
            None => groups.push((key, vec![f])),
        }
    }
    let mut books: Vec<Audiobook> = groups
        .into_iter()
        .map(|((_, folder, _), mut files)| {
            sort_parts(&mut files);
            book(&folder, files)
        })
        .collect();
    books.sort_by(|a, b| {
        let key = |x: &Audiobook| {
            (
                x.author.clone().unwrap_or_default().to_lowercase(),
                x.series.clone().unwrap_or_default().to_lowercase(),
            )
        };
        key(a)
            .cmp(&key(b))
            .then_with(|| {
                a.series_number
                    .unwrap_or(f64::MAX)
                    .total_cmp(&b.series_number.unwrap_or(f64::MAX))
            })
            .then_with(|| natural(&a.title, &b.title))
            .then_with(|| a.rel_path.cmp(&b.rel_path))
    });
    books
}

/// The Open Library query of a book: only books with an author and a
/// title are asked for.
fn query_of(b: &Audiobook) -> Option<BookQuery> {
    let author = b.author.as_deref()?.trim();
    let title = b.title.trim();
    (!author.is_empty() && title.chars().filter(|c| c.is_alphanumeric()).count() >= 2).then(|| {
        BookQuery {
            title: title.to_owned(),
            author: Some(author.to_owned()),
        }
    })
}

/// A book key as given by the app: a `file:` key.
fn book_key(key: &str) -> Result<MediaKey, Error> {
    match key.parse::<MediaKey>() {
        Ok(k @ MediaKey::File { .. }) => Ok(k),
        _ => Err(Error::Invalid(format!("book key {key:?}"))),
    }
}

fn stored(conn: &Connection, key: &MediaKey) -> Result<Option<BookProgress>, Error> {
    Ok(
        lib_progress::newest(conn, profile(conn)?, std::slice::from_ref(key))?.map(|p| {
            BookProgress {
                position_ms: i64::try_from(p.position_ms).unwrap_or(i64::MAX),
                duration_ms: p
                    .duration_ms
                    .map_or(0, |d| i64::try_from(d).unwrap_or(i64::MAX)),
                updated_at_ms: p.updated_at_ms,
                finished: p.watched,
            }
        }),
    )
}

impl Catalog {
    /// The audiobooks of the library the active profile sees, with their
    /// parts, chapters and where the listener stopped.
    pub fn audiobooks(&self) -> Result<Vec<Audiobook>, Error> {
        let conn = self.lock();
        let mut books = books_of(book_files(&conn)?);
        let found = stored_books(&conn)?;
        for b in &mut books {
            if let Some(key) = b.key.as_deref().and_then(|k| k.parse::<MediaKey>().ok()) {
                b.progress = stored(&conn, &key)?;
            }
            if let Some(q) = query_of(b)
                && let Some(m) = found.get(&q.key())
            {
                if m.matched {
                    b.cover_url = m.cover_url.clone().or(b.cover_url.take());
                    b.description = m.description.clone();
                    b.open_library_id = m.work_id.clone();
                } else if m.review {
                    b.needs_review = true;
                }
            }
        }
        Ok(books)
    }

    /// What Open Library is asked for each book with an author and a
    /// title (MUS-019).
    pub(crate) fn book_queries(&self) -> Result<Vec<BookQuery>, Error> {
        let conn = self.lock();
        Ok(books_of(book_files(&conn)?)
            .iter()
            .filter_map(query_of)
            .collect())
    }

    /// Where the listener stopped in the book with this key, on the active
    /// profile (the newest report of any device).
    pub fn book_progress(&self, key: &str) -> Result<Option<BookProgress>, Error> {
        let key = book_key(key)?;
        stored(&self.lock(), &key)
    }

    /// Saves where the listener is in a book: milliseconds from the start
    /// of the book and its length. Goes out to paired devices like any
    /// progress (SYNC-001). Past 95 % (or with `finished`) the book counts
    /// as listened to.
    pub fn save_book_progress(
        &self,
        key: &str,
        position_ms: i64,
        duration_ms: i64,
        updated_at_ms: i64,
        finished: bool,
    ) -> Result<(), Error> {
        let key = book_key(key)?;
        let mut conn = self.lock();
        self.report(
            &mut conn,
            key,
            MediaKind::Track,
            Vec::new(),
            position_ms,
            duration_ms,
            updated_at_ms,
            finished,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn natural_order() {
        let mut v = vec!["10.mp3", "2.mp3", "01.mp3", "Part 1/03.mp3", "1a.mp3"];
        v.sort_by(|a, b| natural(a, b));
        assert_eq!(v, ["01.mp3", "1a.mp3", "2.mp3", "10.mp3", "Part 1/03.mp3"]);
    }

    #[test]
    fn disc_folders_belong_to_the_book() {
        assert_eq!(book_folder("A/Book/CD1/01.mp3"), "A/Book");
        assert_eq!(book_folder("A/Book/Диск 2/01.mp3"), "A/Book");
        assert_eq!(book_folder("A/Book/Часть 3/01.mp3"), "A/Book");
        assert_eq!(book_folder("A/Book/01.mp3"), "A/Book");
        assert_eq!(book_folder("A/CDs of 1999/01.mp3"), "A/CDs of 1999");
        assert_eq!(book_folder("01.mp3"), "");
    }

    #[test]
    fn embedded_chapters_and_tags() {
        let (ch, tags) = embedded(Some(
            r#"{"tags":{},"chapters":2,"chapter_list":[[0,1000,"Пролог"],[1000,5000,null]],
                "book":{"narrator":"Чтец","series":"Серия","series_part":"2"}}"#,
        ));
        assert_eq!(
            ch,
            vec![(0, 1000, Some("Пролог".into())), (1000, 5000, None)]
        );
        assert_eq!(tags["narrator"], "Чтец");
        assert_eq!(tags["series_part"], "2");
        assert_eq!(embedded(Some(r#"{"tags":{},"chapters":0}"#)).0, vec![]);
        assert_eq!(embedded(None).0, vec![]);
    }

    #[test]
    fn only_file_keys_are_book_keys() {
        assert!(book_key("file:abc123").is_ok());
        assert!(book_key("tmdb:movie:1").is_err());
        assert!(book_key("file:XYZ").is_err());
    }
}
