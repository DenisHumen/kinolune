// SPDX-License-Identifier: MPL-2.0
//! Covers and descriptions of audiobooks from Open Library (MUS-019,
//! ADR-0114).
//!
//! [`Catalog::fetch_book_metadata`] runs in the background after a scan,
//! together with the music metadata run (META-021):
//!
//! 1. every book with an author and a title ([`Catalog::audiobooks`]) is a
//!    [`BookQuery`]; its folded author and title are the row key, so the
//!    same book on two sources is asked for once, and a book is asked for
//!    again only when its name changes or its answer gets old
//!    ([`BookMetadataOptions`]);
//! 2. Open Library is searched by title and author, then by title alone
//!    (Russian authors are often written in Latin letters there); results
//!    are scored by [`title_score`] and [`author_matches`] (surnames
//!    compared by their consonants after transliteration: «Стругацкие» =
//!    «Strugatsky»);
//! 3. **matched** — the same title and author: the work's description and
//!    cover are kept and shown on the book page; **review** — the title is
//!    close but not the same, or there is no author to compare: nothing is
//!    shown, the book is marked for review; **not found** — nothing close.
//!
//! Only the title and the author leave the device. Open Library allows one
//! request per second without a contact e-mail; the run is cancellable and
//! later runs continue where it stopped.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use pa_library::rusqlite::{Connection, OptionalExtension, params};
use pa_metadata::cache::ProviderCache;
use pa_metadata::fetch::FetchError;
use pa_metadata::openlibrary::{self as ol, OpenLibraryClient, SearchDoc};
use pa_scanner::CancelToken;

use crate::{Catalog, Error, now_ms};

/// Lowest [`title_score`] of a work that goes to review.
pub const REVIEW_TITLE: f64 = 0.6;

/// Network failures in a row that end a run (offline).
const MAX_FAILURES_IN_A_ROW: usize = 5;

/// Longest description kept, in characters.
const MAX_DESCRIPTION: usize = 4000;

const DAY_MS: i64 = 24 * 3600 * 1000;

/// The clients a book metadata run uses.
#[derive(Clone, Default)]
pub struct BookProviders {
    pub open_library: Option<Arc<OpenLibraryClient>>,
}

impl std::fmt::Debug for BookProviders {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BookProviders")
            .field("open_library", &self.open_library.is_some())
            .finish()
    }
}

impl BookProviders {
    /// The live client (HTTPS) with the given response cache.
    pub fn live(cache: Arc<dyn ProviderCache>) -> Self {
        Self {
            open_library: Some(Arc::new(OpenLibraryClient::live(Default::default(), cache))),
        }
    }
}

/// When books are asked for again.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BookMetadataOptions {
    /// Matched books (and books in review) are refreshed after this long.
    pub refresh_after_ms: i64,
    /// Books Open Library did not know are retried after this long (or
    /// when their name changes).
    pub retry_not_found_after_ms: i64,
    /// Failed fetches (network) are retried after this long.
    pub retry_failed_after_ms: i64,
}

impl Default for BookMetadataOptions {
    fn default() -> Self {
        Self {
            refresh_after_ms: 90 * DAY_MS,
            retry_not_found_after_ms: 30 * DAY_MS,
            retry_failed_after_ms: 3600 * 1000,
        }
    }
}

/// Result of [`Catalog::fetch_book_metadata`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BookMetadataReport {
    /// Books that needed metadata.
    pub books: usize,
    pub matched: usize,
    pub review: usize,
    pub not_found: usize,
    pub failed: usize,
    /// The run was cancelled or stopped early.
    pub stopped: bool,
    /// Why the run stopped early (offline, no client), if it did.
    pub error: Option<String>,
}

/// What is asked for a book.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BookQuery {
    pub title: String,
    pub author: Option<String>,
}

impl BookQuery {
    /// Row key: folded author and title.
    pub fn key(&self) -> String {
        format!(
            "{}\u{1f}{}",
            fold(self.author.as_deref().unwrap_or_default()),
            fold(&self.title)
        )
    }
}

/// A work Open Library has for a book.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct BookMatch {
    /// `OL123W`.
    pub work_id: String,
    pub title: String,
    pub authors: Vec<String>,
    pub first_year: Option<i64>,
    pub cover_url: Option<String>,
    pub description: Option<String>,
    pub confidence: f64,
}

/// The answer for one book.
#[derive(Debug, Clone, PartialEq)]
pub enum BookOutcome {
    /// Same title and author.
    Matched(BookMatch),
    /// Close, but not sure: nothing is shown.
    Review(BookMatch),
    NotFound,
    /// Network or provider failure.
    Failed(String),
}

/// Stored metadata of a book, as [`Catalog::audiobooks`] shows it.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct StoredBook {
    pub matched: bool,
    pub review: bool,
    pub work_id: Option<String>,
    pub cover_url: Option<String>,
    pub description: Option<String>,
}

// -- matching --------------------------------------------------------------------

/// Lowercase words of letters and digits, `ё` as `е`, joined by spaces.
pub fn fold(s: &str) -> String {
    let lower = s.to_lowercase().replace('ё', "е");
    lower
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

fn translit(c: char) -> &'static str {
    match c {
        'а' => "a",
        'б' => "b",
        'в' => "v",
        'г' | 'ґ' => "g",
        'д' => "d",
        'е' | 'ё' | 'э' | 'є' => "e",
        'ж' => "zh",
        'з' => "z",
        'и' | 'і' | 'ї' => "i",
        'й' => "y",
        'к' => "k",
        'л' => "l",
        'м' => "m",
        'н' => "n",
        'о' => "o",
        'п' => "p",
        'р' => "r",
        'с' => "s",
        'т' => "t",
        'у' => "u",
        'ф' => "f",
        'х' => "h",
        'ц' => "ts",
        'ч' => "ch",
        'ш' => "sh",
        'щ' => "shch",
        'ы' => "y",
        'ю' => "yu",
        'я' => "ya",
        _ => "",
    }
}

/// The consonants of a name after transliteration, with spelling variants
/// merged: «Стругацкие», `Strugatsky` and `Strugatski` are all `strgk`.
pub fn skeleton(word: &str) -> String {
    let mut latin = String::new();
    for c in word.to_lowercase().chars() {
        if c.is_ascii_alphabetic() {
            latin.push(c);
        } else if let Some(d) = deunicode(c) {
            latin.push(d);
        } else {
            latin.push_str(translit(c));
        }
    }
    let mut s = latin
        .replace("shch", "s")
        .replace("sch", "s")
        .replace("ch", "c")
        .replace("kh", "h")
        .replace("sh", "s")
        .replace("zh", "z")
        .replace("ts", "c")
        .replace("tz", "c")
        .replace("ck", "k")
        .replace("ph", "f")
        .replace('x', "ks");
    s = s
        .chars()
        .map(|c| match c {
            'c' | 'q' => 'k',
            'z' => 's',
            other => other,
        })
        .filter(|c| !"aeiouyhjvw".contains(*c))
        .collect();
    let mut out = String::new();
    for c in s.chars() {
        if out.ends_with(c) {
            continue;
        }
        out.push(c);
    }
    out
}

/// Latin letters with diacritics as plain ones (`é` → `e`, `ł` → `l`).
fn deunicode(c: char) -> Option<char> {
    let plain = match c {
        'à' | 'á' | 'â' | 'ä' | 'ã' | 'å' | 'ą' => 'a',
        'ç' | 'č' | 'ć' => 'c',
        'è' | 'é' | 'ê' | 'ë' | 'ę' | 'ě' => 'e',
        'ì' | 'í' | 'î' | 'ï' => 'i',
        'ñ' | 'ń' | 'ň' => 'n',
        'ò' | 'ó' | 'ô' | 'ö' | 'õ' | 'ø' => 'o',
        'ù' | 'ú' | 'û' | 'ü' | 'ů' => 'u',
        'ý' | 'ÿ' => 'y',
        'š' | 'ś' => 's',
        'ž' | 'ź' | 'ż' => 'z',
        'ł' => 'l',
        'ř' => 'r',
        'đ' | 'ď' => 'd',
        'ť' => 't',
        _ => return None,
    };
    Some(plain)
}

/// Words of a person's name worth comparing: three letters or more, not
/// initials and not joiners.
fn name_words(name: &str) -> Vec<String> {
    name.split(|c: char| !c.is_alphabetic())
        .filter(|w| w.chars().count() >= 3)
        .map(str::to_lowercase)
        .filter(|w| !["and", "the", "von", "van", "der", "del", "mit"].contains(&w.as_str()))
        .collect()
}

/// Whether one of `doc_authors` is the `author` of the query: a word of the
/// name (in practice the surname) has the same [`skeleton`].
pub fn author_matches(author: &str, doc_authors: &[String]) -> bool {
    // A one-consonant skeleton (`Чехов` → `k`) also needs the same first
    // letter: `Yuval` and `Lie` are both `l`.
    let key = |w: &String| {
        let s = skeleton(w);
        let first = skeleton_first(w);
        (s, first)
    };
    let ours: Vec<(String, Option<char>)> = name_words(author)
        .iter()
        .map(key)
        .filter(|(s, _)| !s.is_empty())
        .collect();
    doc_authors.iter().any(|a| {
        name_words(a).iter().map(key).any(|(s, first)| {
            ours.iter()
                .any(|(o, f)| !s.is_empty() && *o == s && (s.chars().count() >= 2 || *f == first))
        })
    })
}

/// First Latin letter of a word after transliteration.
fn skeleton_first(word: &str) -> Option<char> {
    let c = word.to_lowercase().chars().next()?;
    if c.is_ascii_alphabetic() {
        return Some(c);
    }
    deunicode(c).or_else(|| translit(c).chars().next())
}

/// 1 for the same title (also with the subtitle, or without a leading
/// article), else the share of common words.
pub fn title_score(title: &str, doc_title: &str, doc_subtitle: Option<&str>) -> f64 {
    let q = fold(title);
    if q.is_empty() {
        return 0.0;
    }
    let strip = |s: &str| {
        ["the ", "a ", "an "]
            .iter()
            .find_map(|a| s.strip_prefix(a))
            .unwrap_or(s)
            .to_owned()
    };
    let d = fold(doc_title);
    let full = doc_subtitle.map(|s| fold(&format!("{doc_title} {s}")));
    // `Ночной дозор (Nochnoĭ dozor)`: a bracketed transliteration or note.
    let bare = doc_title
        .trim_end()
        .strip_suffix(')')
        .and_then(|t| t.rsplit_once('('))
        .map(|(head, _)| fold(head));
    if q == d || Some(&q) == full.as_ref() || Some(&q) == bare.as_ref() || strip(&q) == strip(&d) {
        return 1.0;
    }
    let a: HashSet<&str> = q.split(' ').collect();
    let b: HashSet<&str> = d.split(' ').collect();
    let common = a.intersection(&b).count();
    let all = a.union(&b).count();
    if all == 0 {
        0.0
    } else {
        common as f64 / all as f64
    }
}

/// Titles that name a kind of book, not a book: «Рассказы» of an author is
/// one of many collections, so a match is only a guess.
const GENERIC_TITLES: &[&str] = &[
    "рассказы",
    "повести",
    "повести и рассказы",
    "стихи",
    "стихотворения",
    "избранное",
    "сборник",
    "сказки",
    "пьесы",
    "сочинения",
    "собрание сочинений",
    "stories",
    "short stories",
    "selected stories",
    "collected stories",
    "poems",
    "selected works",
    "collected works",
    "essays",
];

/// How a work answers a query: (accepted, confidence, the query's author
/// is the work's first author), or not at all.
fn judge(q: &BookQuery, doc: &SearchDoc) -> Option<(bool, f64, bool)> {
    let t = title_score(&q.title, &doc.title, doc.subtitle.as_deref());
    let generic = GENERIC_TITLES.contains(&fold(&q.title).as_str());
    let author = q
        .author
        .as_deref()
        .map(|a| author_matches(a, &doc.author_name));
    let first = q
        .author
        .as_deref()
        .is_some_and(|a| author_matches(a, &doc.author_name[..doc.author_name.len().min(1)]));
    match author {
        Some(false) => None,
        Some(true) if t >= 0.999 && !generic => Some((true, 0.95, first)),
        Some(true) if t >= REVIEW_TITLE => Some((false, 0.5 + t * 0.3, first)),
        None if t >= 0.999 => Some((false, 0.6, first)),
        _ => None,
    }
}

/// The best work of `docs`: accepted ones first, then the ones whose first
/// author is the query's (not an illustrator's adaptation), then the most
/// editions.
fn best(q: &BookQuery, docs: &[SearchDoc]) -> Option<(bool, f64, SearchDoc)> {
    docs.iter()
        .filter_map(|d| judge(q, d).map(|(ok, c, first)| (ok, c, first, d)))
        .max_by(|a, b| {
            a.0.cmp(&b.0)
                .then(a.1.total_cmp(&b.1))
                .then(a.2.cmp(&b.2))
                .then(
                    a.3.edition_count
                        .unwrap_or(0)
                        .cmp(&b.3.edition_count.unwrap_or(0)),
                )
        })
        .map(|(ok, c, _, d)| (ok, c, d.clone()))
}

/// The description without Open Library's reference links and source
/// notes, at most [`MAX_DESCRIPTION`] characters.
pub fn clean_description(text: &str) -> Option<String> {
    let mut out = Vec::new();
    let text = text.replace("\r\n", "\n");
    for line in text.lines() {
        let t = line.trim();
        // `----------` ends the description; what follows are links.
        if t.starts_with("----") || t.starts_with("([source]") {
            break;
        }
        // `[1]: https://…` reference definitions.
        if t.starts_with('[') && t.contains("]: http") {
            continue;
        }
        out.push(line.trim_end());
    }
    let mut s = out.join("\n");
    // Inline `([source][1])`.
    while let Some(at) = s.find("([source]") {
        let end = s[at..].find(')').map_or(s.len(), |e| at + e + 1);
        s.replace_range(at..end, "");
    }
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    let mut cut: String = s.chars().take(MAX_DESCRIPTION).collect();
    if cut.len() < s.len() {
        cut.push('…');
    }
    Some(cut)
}

fn as_match(doc: &SearchDoc, confidence: f64) -> BookMatch {
    BookMatch {
        work_id: doc.work_id().to_owned(),
        title: doc.title.clone(),
        authors: doc.author_name.clone(),
        first_year: doc.first_publish_year,
        cover_url: doc.cover_i.filter(|i| *i > 0).map(ol::cover_url),
        description: None,
        confidence,
    }
}

fn search(
    client: &OpenLibraryClient,
    title: &str,
    author: Option<&str>,
) -> Result<Vec<SearchDoc>, String> {
    match client.search(title, author) {
        Ok(d) => Ok(d),
        Err(FetchError::NotFound { .. }) => Ok(Vec::new()),
        Err(e) => Err(e.to_string()),
    }
}

/// Finds the work of a book: a search by title and author, then by title
/// alone when that gave nothing certain; the description of a matched work.
pub fn match_book(client: &OpenLibraryClient, q: &BookQuery) -> BookOutcome {
    let mut found = None;
    if let Some(author) = q.author.as_deref() {
        match search(client, &q.title, Some(author)) {
            Ok(docs) => found = best(q, &docs),
            Err(e) => return BookOutcome::Failed(e),
        }
    }
    if !found.as_ref().is_some_and(|f| f.0) {
        match search(client, &q.title, None) {
            Ok(docs) => {
                let other = best(q, &docs);
                let better = match (&found, &other) {
                    (None, Some(_)) => true,
                    (Some(a), Some(b)) => (b.0, b.1) > (a.0, a.1),
                    _ => false,
                };
                if better {
                    found = other;
                }
            }
            Err(e) => return BookOutcome::Failed(e),
        }
    }
    let Some((accepted, confidence, doc)) = found else {
        return BookOutcome::NotFound;
    };
    let mut m = as_match(&doc, confidence);
    if !accepted {
        return BookOutcome::Review(m);
    }
    match client.work(doc.work_id()) {
        Ok(w) => {
            m.description = w
                .description
                .as_ref()
                .and_then(|d| clean_description(d.as_str()));
            if m.cover_url.is_none() {
                m.cover_url = w.covers.iter().find(|i| **i > 0).map(|i| ol::cover_url(*i));
            }
        }
        Err(FetchError::NotFound { .. }) => {}
        Err(e) => return BookOutcome::Failed(e.to_string()),
    }
    BookOutcome::Matched(m)
}

// -- storage ---------------------------------------------------------------------

/// Stored answers by query key.
pub(crate) fn stored_books(conn: &Connection) -> Result<HashMap<String, StoredBook>, Error> {
    let mut stmt = conn.prepare_cached(
        "SELECT query, outcome, work_id, cover_url, description FROM audiobook_metadata
         WHERE outcome IN ('matched', 'review')",
    )?;
    let rows = stmt.query_map([], |r| {
        let outcome: String = r.get(1)?;
        Ok((
            r.get::<_, String>(0)?,
            StoredBook {
                matched: outcome == "matched",
                review: outcome == "review",
                work_id: r.get(2)?,
                cover_url: r.get(3)?,
                description: r.get(4)?,
            },
        ))
    })?;
    let mut out = HashMap::new();
    for row in rows {
        let (k, v) = row?;
        out.insert(k, v);
    }
    Ok(out)
}

fn pending(
    conn: &Connection,
    queries: Vec<BookQuery>,
    options: &BookMetadataOptions,
    now: i64,
) -> Result<Vec<BookQuery>, Error> {
    let mut stmt = conn
        .prepare_cached("SELECT outcome, fetched_at_ms FROM audiobook_metadata WHERE query = ?1")?;
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for q in queries {
        let key = q.key();
        if !seen.insert(key.clone()) {
            continue;
        }
        let row: Option<(String, i64)> = stmt
            .query_row([&key], |r| Ok((r.get(0)?, r.get(1)?)))
            .optional()?;
        let due = match row {
            None => true,
            Some((outcome, at)) => {
                let after = match outcome.as_str() {
                    "not_found" => options.retry_not_found_after_ms,
                    "failed" => options.retry_failed_after_ms,
                    _ => options.refresh_after_ms,
                };
                now - at >= after
            }
        };
        if due {
            out.push(q);
        }
    }
    Ok(out)
}

fn write(conn: &Connection, q: &BookQuery, outcome: &BookOutcome, now: i64) -> Result<(), Error> {
    let (state, m, error) = match outcome {
        BookOutcome::Matched(m) => ("matched", Some(m), None),
        BookOutcome::Review(m) => ("review", Some(m), None),
        BookOutcome::NotFound => ("not_found", None, None),
        BookOutcome::Failed(e) => ("failed", None, Some(e.as_str())),
    };
    conn.execute(
        "INSERT OR REPLACE INTO audiobook_metadata
             (query, outcome, confidence, work_id, title, authors, first_year, cover_url,
              description, error, fetched_at_ms)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        params![
            q.key(),
            state,
            m.map(|m| m.confidence.clamp(0.0, 1.0)),
            m.map(|m| m.work_id.as_str()),
            m.map(|m| m.title.as_str()),
            serde_json::to_string(&m.map(|m| m.authors.clone()).unwrap_or_default())
                .unwrap_or_else(|_| "[]".into()),
            m.and_then(|m| m.first_year),
            m.and_then(|m| m.cover_url.as_deref()),
            m.and_then(|m| m.description.as_deref()),
            error,
            now
        ],
    )?;
    Ok(())
}

impl Catalog {
    /// Asks Open Library for the books that need it (see the module docs)
    /// and stores the answers. `progress` gets (done, total).
    pub fn fetch_book_metadata(
        &self,
        providers: &BookProviders,
        options: &BookMetadataOptions,
        cancel: &CancelToken,
        progress: &mut dyn FnMut(usize, usize),
    ) -> Result<BookMetadataReport, Error> {
        let mut report = BookMetadataReport::default();
        let Some(client) = providers.open_library.as_deref() else {
            report.stopped = true;
            report.error = Some("no Open Library client".into());
            return Ok(report);
        };
        let queries = self.book_queries()?;
        let todo = pending(&self.lock(), queries, options, now_ms())?;
        report.books = todo.len();
        progress(0, todo.len());
        let mut failures = 0;
        for (i, q) in todo.iter().enumerate() {
            if cancel.is_cancelled() {
                report.stopped = true;
                break;
            }
            let outcome = match_book(client, q);
            match &outcome {
                BookOutcome::Matched(_) => report.matched += 1,
                BookOutcome::Review(_) => report.review += 1,
                BookOutcome::NotFound => report.not_found += 1,
                BookOutcome::Failed(_) => report.failed += 1,
            }
            write(&self.lock(), q, &outcome, now_ms())?;
            if let BookOutcome::Failed(e) = &outcome {
                failures += 1;
                if failures >= MAX_FAILURES_IN_A_ROW {
                    report.stopped = true;
                    report.error = Some(e.clone());
                    break;
                }
            } else {
                failures = 0;
            }
            progress(i + 1, todo.len());
        }
        Ok(report)
    }

    /// Makes the next run ask Open Library again for every book.
    pub fn reset_book_metadata(&self) -> Result<(), Error> {
        self.lock().execute("DELETE FROM audiobook_metadata", [])?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skeletons_merge_spellings() {
        for (a, b) in [
            ("Стругацкие", "Strugatsky"),
            ("Стругацкий", "Strugatski"),
            ("Булгаков", "Bulgakov"),
            ("Толстой", "Tolstoy"),
            ("Достоевский", "Dostoyevsky"),
            ("Сапковский", "Sapkowski"),
            ("Глуховский", "Glukhovsky"),
            ("Хайнлайн", "Heinlein"),
            ("Пушкин", "Pushkin"),
            ("Чехов", "Chekhov"),
            ("Роулинг", "Rowling"),
            ("Кристи", "Christie"),
            ("Лукьяненко", "Lukyanenko"),
            ("Brontë", "Bronte"),
        ] {
            assert_eq!(skeleton(a), skeleton(b), "{a} / {b}");
        }
        assert_ne!(skeleton("Пелевин"), skeleton("Pratchett"));
    }

    #[test]
    fn authors_match_by_surname() {
        assert!(author_matches(
            "Стругацкие Аркадий и Борис",
            &["Arkady Strugatsky".into(), "Boris Strugatsky".into()]
        ));
        assert!(author_matches(
            "Sanderson, Brandon",
            &["Brandon Sanderson".into()]
        ));
        assert!(!author_matches("Лукьяненко", &["Brandon Sanderson".into()]));
    }

    #[test]
    fn titles() {
        assert_eq!(
            title_score("Мастер и Маргарита", "Мастер и Маргарита", None),
            1.0
        );
        assert_eq!(title_score("Ёлка", "елка", None), 1.0);
        assert_eq!(
            title_score(
                "The Hobbit: There and Back Again",
                "The Hobbit",
                Some("There and Back Again")
            ),
            1.0
        );
        assert_eq!(title_score("Hobbit", "The Hobbit", None), 1.0);
        assert!(title_score("Пикник на обочине", "Пикник на обочине вселенной", None) < 1.0);
    }

    #[test]
    fn descriptions_lose_links() {
        let d = "A story.\r\n\r\n([source][1])\r\n\r\n----------\r\nAlso: [x][2]\r\n\r\n[1]: https://a\r\n";
        assert_eq!(clean_description(d).as_deref(), Some("A story."));
        assert_eq!(clean_description("  "), None);
        let inline = "Text ([source][1]) more.";
        assert_eq!(clean_description(inline).as_deref(), Some("Text  more."));
    }

    #[test]
    fn keys_fold() {
        let a = BookQuery {
            title: "Пикник на обочине!".into(),
            author: Some("СТРУГАЦКИЕ".into()),
        };
        let b = BookQuery {
            title: "пикник  на обочине".into(),
            author: Some("Стругацкие".into()),
        };
        assert_eq!(a.key(), b.key());
    }
}
