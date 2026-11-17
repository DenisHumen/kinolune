// SPDX-License-Identifier: MPL-2.0
//! The audiobook library (MUS-018, ADR-NNNN).
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
    /// Cover Art Archive cover of the album, when the music metadata run
    /// found one.
    pub cover_url: Option<String>,
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

/// Folders that hold books rather than name them.
const GENERIC_FOLDERS: &[&str] = &[
    "audiobooks",
    "audiobook",
    "audio books",
    "books",
    "аудиокниги",
    "аудиокнига",
    "аудио книги",
    "книги",
    "music",
    "audio",
    "media",
    "downloads",
    "mp3",
    "unsorted",
    "various artists",
];

fn generic(name: &str) -> bool {
    let lower = name.trim().to_lowercase();
    lower.is_empty() || GENERIC_FOLDERS.contains(&lower.as_str())
}

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

/// What a folder or album name says about a book.
#[derive(Debug, Clone, Default, PartialEq)]
struct NameInfo {
    title: String,
    author: Option<String>,
    narrator: Option<String>,
    number: Option<f64>,
    year: Option<i64>,
}

/// Words before a narrator in brackets: `(читает Кирилл Радциг)`.
const NARRATOR_MARKERS: &[&str] = &[
    "читает",
    "читают",
    "чит.",
    "чтец:",
    "чтец",
    "чтецы:",
    "чтецы",
    "исполнитель:",
    "исполняет",
    "narrated by",
    "narrator:",
    "narrator",
    "read by",
];

fn narrator_in(group: &str) -> Option<String> {
    let lower = group.to_lowercase();
    NARRATOR_MARKERS.iter().find_map(|m| {
        if !lower.starts_with(m) {
            return None;
        }
        let name: String = group.chars().skip(m.chars().count()).collect();
        let name = name.trim_matches([' ', ':', '-', '.']);
        (!name.is_empty()).then(|| name.to_owned())
    })
}

/// A leading `02 - `, `02. `, `2) `, `Книга 2. `, `Book 2 - ` number.
fn leading_number(s: &str) -> Option<(f64, &str)> {
    let lower = s.to_lowercase();
    let mut start = 0;
    for p in ["книга", "том", "book", "volume", "vol.", "vol"] {
        if lower.starts_with(p) {
            start = s
                .char_indices()
                .nth(p.chars().count())
                .map_or(s.len(), |(i, _)| i);
            break;
        }
    }
    let rest = &s[start..];
    let rest_trim = rest.trim_start();
    let digits = rest_trim
        .bytes()
        .take_while(|b| b.is_ascii_digit() || *b == b'.')
        .count();
    let raw = rest_trim[..digits].trim_end_matches('.');
    if raw.is_empty() || raw.len() > 6 || !raw.as_bytes()[0].is_ascii_digit() {
        return None;
    }
    let after = &rest_trim[raw.len()..];
    let title = after.trim_start_matches([' ', '.', '-', '_', ')', '–', '—']);
    // A number needs a separator after it ("1984" is a title).
    if title.len() == after.len() || title.is_empty() {
        return None;
    }
    if start == 0 && raw.len() > 3 {
        return None;
    }
    Some((raw.parse().ok()?, title))
}

/// Reads `Автор - 02 - Название (2019) (читает Чтец) [128 kbps]`.
fn name_info(name: &str) -> NameInfo {
    let mut info = NameInfo::default();
    let mut s = name.replace('_', " ").trim().to_owned();
    // Trailing bracket groups: narrator, year, noise.
    loop {
        let t = s.trim_end();
        let Some(close) = t.chars().last().filter(|c| matches!(c, ')' | ']' | '}')) else {
            break;
        };
        let open = match close {
            ')' => '(',
            ']' => '[',
            _ => '{',
        };
        let Some(at) = t.rfind(open) else { break };
        let group = t[at + 1..t.len() - 1].trim();
        if let Some(n) = narrator_in(group) {
            info.narrator.get_or_insert(n);
        } else if group.len() == 4
            && group.bytes().all(|b| b.is_ascii_digit())
            && (group.starts_with("19") || group.starts_with("20"))
        {
            info.year.get_or_insert(group.parse().unwrap_or_default());
        }
        let head = t[..at].trim_end().to_owned();
        if head.is_empty() {
            // The name is all brackets: keep it.
            break;
        }
        s = head;
    }
    let s = s.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut parts: Vec<&str> = s.split(" - ").map(str::trim).collect();
    if parts.len() == 1 {
        parts = s.split(" – ").map(str::trim).collect();
    }
    if parts.len() == 1 {
        parts = s.split(" — ").map(str::trim).collect();
    }
    parts.retain(|p| !p.is_empty());
    match parts.as_slice() {
        [] => info.title = s.clone(),
        [one] => match leading_number(one) {
            Some((n, t)) => {
                info.number = Some(n);
                info.title = t.to_owned();
            }
            None => info.title = (*one).to_owned(),
        },
        [first, rest @ ..] => {
            let rest = rest.join(" - ");
            if let Some((n, _)) = leading_number(&format!("{first} - x")) {
                // `02 - Название`
                info.number = Some(n);
                info.title = rest;
            } else {
                info.author = Some((*first).to_owned());
                match leading_number(&rest) {
                    Some((n, t)) => {
                        info.number = Some(n);
                        info.title = t.to_owned();
                    }
                    None => info.title = rest,
                }
            }
        }
    }
    if info.title.is_empty() {
        info.title = name.trim().to_owned();
    }
    info
}

/// `3`, `3.5`, `3/10`, `Book 3`, `#3`.
fn series_number(s: &str) -> Option<f64> {
    let s = s.trim().trim_start_matches('#');
    let start = s.find(|c: char| c.is_ascii_digit())?;
    let raw: String = s[start..]
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.' || *c == ',')
        .map(|c| if c == ',' { '.' } else { c })
        .collect();
    raw.trim_end_matches('.')
        .parse()
        .ok()
        .filter(|n: &f64| *n >= 0.0 && *n < 10_000.0)
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

/// One book of `files` (in play order), found in `folder`.
fn book(folder: &str, files: Vec<BookFile>) -> Audiobook {
    let first = &files[0];
    let single = files.len() == 1;
    let item = &first.track.item;
    let rel_path = if single && !first.chapters.is_empty() {
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
        if single && !first.chapters.is_empty() {
            item.title.clone()
        } else {
            folder_name.clone()
        }
    });
    let from_title = name_info(&raw_title);
    let from_folder = name_info(&folder_name);

    let tag = |pick: fn(&BookFile) -> Option<&str>| most_common(files.iter().filter_map(pick));
    let usable = |a: &String| !generic(a) && !is_various(a);
    let author = tag(|f| f.track.album_artists.first().map(String::as_str))
        .filter(usable)
        .or_else(|| tag(|f| f.track.artists.first().map(String::as_str)).filter(usable))
        .or_else(|| from_title.author.clone().filter(usable))
        .or_else(|| from_folder.author.clone().filter(usable));
    let narrator = tag(|f| f.narrator.as_deref())
        .or_else(|| from_title.narrator.clone())
        .or_else(|| from_folder.narrator.clone())
        .or_else(|| {
            tag(|f| f.track.composers.first().map(String::as_str))
                .filter(|c| author.as_ref().is_none_or(|a| !a.eq_ignore_ascii_case(c)))
        });
    let mut series = tag(|f| f.series.as_deref());
    let mut number = tag(|f| f.series_part.as_deref()).and_then(|s| series_number(&s));
    let path_number = from_title.number.or(from_folder.number);
    if series.is_none() && path_number.is_some() && !folder.is_empty() {
        // `Серия/02 - Название`: the folder above names the series.
        let above = last(parent(folder));
        if !above.is_empty() && !generic(above) {
            let s = name_info(above);
            let named = s.title.trim();
            if author
                .as_ref()
                .is_none_or(|a| !a.eq_ignore_ascii_case(named))
            {
                series = Some(named.to_owned());
            }
        }
    }
    if series.is_some() {
        number = number.or(path_number);
    }
    let title = if from_title.title.trim().is_empty() {
        raw_title.clone()
    } else {
        from_title.title.clone()
    };
    let year = files
        .iter()
        .find_map(|f| f.track.album_year.or(f.track.item.year))
        .or(from_title.year)
        .or(from_folder.year);

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
        for b in &mut books {
            if let Some(key) = b.key.as_deref().and_then(|k| k.parse::<MediaKey>().ok()) {
                b.progress = stored(&conn, &key)?;
            }
        }
        Ok(books)
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
    fn names_give_author_narrator_number_and_year() {
        let i = name_info("Стругацкие - Пикник на обочине (2019) (читает Кирилл Радциг) [128]");
        assert_eq!(i.author.as_deref(), Some("Стругацкие"));
        assert_eq!(i.title, "Пикник на обочине");
        assert_eq!(i.narrator.as_deref(), Some("Кирилл Радциг"));
        assert_eq!(i.year, Some(2019));
        assert_eq!(i.number, None);

        let i = name_info("02 - Трудно быть богом");
        assert_eq!(i.number, Some(2.0));
        assert_eq!(i.title, "Трудно быть богом");
        assert_eq!(i.author, None);

        let i = name_info("Brandon Sanderson - 01. The Way of Kings (Narrated by Michael Kramer)");
        assert_eq!(i.author.as_deref(), Some("Brandon Sanderson"));
        assert_eq!(i.number, Some(1.0));
        assert_eq!(i.title, "The Way of Kings");
        assert_eq!(i.narrator.as_deref(), Some("Michael Kramer"));

        let i = name_info("Книга 3. Волны гасят ветер");
        assert_eq!(i.number, Some(3.0));
        assert_eq!(i.title, "Волны гасят ветер");

        // A year is a title, not a number in a series.
        assert_eq!(name_info("1984").title, "1984");
        assert_eq!(name_info("1984").number, None);
        assert_eq!(name_info("(Чтец Иванов)").title, "(Чтец Иванов)");
    }

    #[test]
    fn series_numbers() {
        assert_eq!(series_number("3"), Some(3.0));
        assert_eq!(series_number("3,5"), Some(3.5));
        assert_eq!(series_number("3/10"), Some(3.0));
        assert_eq!(series_number("Book 4"), Some(4.0));
        assert_eq!(series_number("#2"), Some(2.0));
        assert_eq!(series_number("none"), None);
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
