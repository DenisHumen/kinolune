// SPDX-License-Identifier: MPL-2.0
//! What the names of audiobook folders, files and album tags say about a
//! book (MUS-018, MUS-019; ADR-0107, ADR-0116): author, title, narrator,
//! series, number in the series and year.
//!
//! The layouts are the ones people and release groups use:
//!
//! - `Автор - Название (2019) (читает Чтец) [128 kbps]`;
//! - `Фамилия Имя - Название [Чтец Имя, 2015, 96 kbps, MP3]` (tracker
//!   style: the first name in the square brackets reads the book);
//! - `Автор - Название (Имя Чтец)`: a bracketed person name is the
//!   narrator when one of its words is a common first name;
//! - `Автор - Серия 01 - Название`, `Автор - Серия - 01 - Название`,
//!   `Серия. Книга 2. Название`, `Название (Серия #3)`;
//! - `Sanderson, Brandon - Title` (surname first, turned around);
//! - leading `[Аудиокнига]`, `(Audiobook)` and trailing noise
//!   (`[MP3]`, `(Unabridged)`) are dropped.
//!
//! These are pure functions; [`crate::audiobooks`] combines what they find
//! with the tags and the folders above the book.

/// Folders that hold books rather than name them (lowercase).
const GENERIC_FOLDERS: &[&str] = &[
    "audiobooks",
    "audiobook",
    "audio books",
    "audio-books",
    "books",
    "аудиокниги",
    "аудиокнига",
    "аудио книги",
    "аудио",
    "книги",
    "music",
    "audio",
    "media",
    "downloads",
    "download",
    "загрузки",
    "torrents",
    "mp3",
    "m4b",
    "unsorted",
    "разное",
    "misc",
    "new",
    "новое",
    "library",
    "библиотека",
    "various artists",
    // Genres people sort books into.
    "фантастика",
    "фэнтези",
    "детективы",
    "детектив",
    "классика",
    "русская классика",
    "зарубежная классика",
    "проза",
    "детские",
    "детская литература",
    "для детей",
    "нон-фикшн",
    "fantasy",
    "sci-fi",
    "science fiction",
    "classics",
    "fiction",
    "non-fiction",
    "nonfiction",
    "kids",
    "children",
    "thriller",
    "thrillers",
    "mystery",
];

/// A folder that holds books rather than names one.
pub(crate) fn generic(name: &str) -> bool {
    let lower = name.trim().to_lowercase();
    lower.is_empty() || GENERIC_FOLDERS.contains(&lower.as_str())
}

/// What a folder or album name says about a book.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct NameInfo {
    pub title: String,
    pub author: Option<String>,
    pub narrator: Option<String>,
    pub series: Option<String>,
    pub number: Option<f64>,
    pub year: Option<i64>,
}

/// Words before a narrator: `(читает Кирилл Радциг)`.
const NARRATOR_MARKERS: &[&str] = &[
    "читает",
    "читают",
    "читала",
    "читал",
    "чит.",
    "чтец:",
    "чтец",
    "чтецы:",
    "чтецы",
    "исполнитель:",
    "исполнитель",
    "исполняет",
    "narrated by",
    "narrator:",
    "narrator",
    "read by",
];

/// `читает Кирилл Радциг` → `Кирилл Радциг`.
pub(crate) fn narrator_in(group: &str) -> Option<String> {
    let lower = group.trim().to_lowercase();
    NARRATOR_MARKERS.iter().find_map(|m| {
        let rest = lower.strip_prefix(m)?;
        // A whole word: `читает`, not `читатель`.
        if !m.ends_with([':', '.']) && rest.chars().next().is_some_and(char::is_alphanumeric) {
            return None;
        }
        let name: String = group.trim().chars().skip(m.chars().count()).collect();
        let name = name.trim_matches([' ', ':', '-', '.', ',']);
        (!name.is_empty()).then(|| name.to_owned())
    })
}

/// Common first names of narrators and authors (lowercase): a bracketed
/// pair of capitalised words with one of them is a person, not a subtitle.
const FIRST_NAMES: &[&str] = &[
    // Russian
    "александр",
    "александра",
    "алексей",
    "анатолий",
    "андрей",
    "анна",
    "антон",
    "аркадий",
    "артём",
    "артем",
    "борис",
    "вадим",
    "валентин",
    "валентина",
    "валерий",
    "василий",
    "вера",
    "виктор",
    "виктория",
    "владимир",
    "владислав",
    "всеволод",
    "вячеслав",
    "галина",
    "геннадий",
    "георгий",
    "глеб",
    "григорий",
    "дарья",
    "денис",
    "дмитрий",
    "евгений",
    "евгения",
    "егор",
    "екатерина",
    "елена",
    "ефим",
    "иван",
    "игорь",
    "илья",
    "ирина",
    "кирилл",
    "константин",
    "лариса",
    "леонид",
    "лев",
    "любовь",
    "людмила",
    "максим",
    "марина",
    "мария",
    "михаил",
    "надежда",
    "наталья",
    "наталия",
    "никита",
    "николай",
    "нина",
    "олег",
    "ольга",
    "павел",
    "пётр",
    "петр",
    "роман",
    "ростислав",
    "светлана",
    "святослав",
    "семён",
    "семен",
    "сергей",
    "станислав",
    "степан",
    "татьяна",
    "тимур",
    "фёдор",
    "федор",
    "филипп",
    "эдуард",
    "юлия",
    "юрий",
    "яна",
    "ярослав",
    "иннокентий",
    "зиновий",
    "клим",
    "клара",
    "алла",
    "оксана",
    // English
    "adam",
    "alan",
    "andrew",
    "andy",
    "anne",
    "barbara",
    "ben",
    "bill",
    "brian",
    "bronson",
    "charles",
    "chris",
    "christopher",
    "daniel",
    "david",
    "derek",
    "edward",
    "elizabeth",
    "emma",
    "frank",
    "fred",
    "george",
    "grover",
    "hugh",
    "jack",
    "james",
    "jane",
    "jeff",
    "jim",
    "john",
    "jonathan",
    "julia",
    "juliet",
    "kate",
    "kevin",
    "lisa",
    "luke",
    "mark",
    "martin",
    "mary",
    "michael",
    "neil",
    "nathaniel",
    "nicholas",
    "paul",
    "peter",
    "ralph",
    "ray",
    "richard",
    "robert",
    "roy",
    "rosamund",
    "scott",
    "simon",
    "stephen",
    "steven",
    "susan",
    "thomas",
    "tim",
    "tom",
    "travis",
    "will",
    "william",
    "wil",
    "kristen",
    "rc",
    "steve",
    "tavia",
    "davina",
    "dan",
    "campbell",
    "jenny",
    "joe",
    "matt",
    "nick",
    "sam",
    "sean",
    "stephen",
    "anna",
];

fn first_name(word: &str) -> bool {
    FIRST_NAMES.contains(&word.trim_matches(['.', ',']).to_lowercase().as_str())
}

/// An initial: `А.`, `Л.Н.`, `J.R.R.`, `Дж.`.
fn initial(word: &str) -> bool {
    word.ends_with('.')
        && word.chars().count() <= 6
        && word.chars().next().is_some_and(char::is_uppercase)
        && word.chars().all(|c| c.is_alphabetic() || c == '.')
}

/// 1–4 capitalised words (or initials), no digits: how names of people
/// look. Russian titles capitalise only their first word.
pub(crate) fn person_like(s: &str) -> bool {
    let words: Vec<&str> = s
        .split(|c: char| c.is_whitespace() || c == ',')
        .filter(|w| !w.is_empty() && *w != "и" && *w != "&" && *w != "and")
        .collect();
    !words.is_empty()
        && words.len() <= 4
        && !generic(s)
        && !["the", "a", "an"].contains(&words[0].to_lowercase().as_str())
        && words.iter().all(|w| {
            initial(w)
                || (w.chars().next().is_some_and(char::is_uppercase)
                    && w.chars()
                        .all(|c| c.is_alphabetic() || c == '-' || c == '\'' || c == '.'))
        })
}

/// A bracketed narrator without a marker: `(Кирилл Радциг)`,
/// `[Radzig Kirill]`.
fn bare_narrator(group: &str) -> Option<String> {
    let g = group.trim();
    let n = g.split_whitespace().count();
    ((2..=3).contains(&n) && person_like(g) && g.split_whitespace().any(first_name))
        .then(|| g.to_owned())
}

fn year_of(s: &str) -> Option<i64> {
    let s = s.trim();
    (s.len() == 4
        && s.bytes().all(|b| b.is_ascii_digit())
        && (s.starts_with("19") || s.starts_with("20")))
    .then(|| s.parse().ok())
    .flatten()
}

/// `3`, `3.5`, `3/10`, `Book 3`, `#3`.
pub(crate) fn series_number(s: &str) -> Option<f64> {
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

/// Words that mark a number in a series.
const BOOK_WORDS: &[&str] = &["книга", "кн.", "том", "book", "volume", "vol.", "vol", "#"];

/// `Книга 2`, `Book 3`, `#4`, `Vol. 1`: the number.
fn book_number(piece: &str) -> Option<f64> {
    let lower = piece.trim().to_lowercase();
    let rest = BOOK_WORDS.iter().find_map(|w| lower.strip_prefix(w))?;
    let rest = rest.trim_start();
    let digits = rest.trim_end_matches('.');
    (!digits.is_empty()
        && digits.len() <= 4
        && digits.bytes().all(|b| b.is_ascii_digit() || b == b'.'))
    .then(|| series_number(digits))
    .flatten()
}

/// `Серия #3`, `Mistborn, Book 1`, `Ведьмак-1`, `Ведьмак 01`: a series name
/// with a trailing number (`max_digits` long at most for a bare number).
fn series_with_number(s: &str, bare: bool) -> Option<(String, f64)> {
    let t = s.trim();
    // `…, Book 1` / `… Книга 2` / `… #3`.
    let lower = t.to_lowercase();
    for w in BOOK_WORDS {
        if let Some(at) = lower
            .rfind(&format!(" {w}"))
            .or_else(|| lower.rfind(&format!(",{w}")))
        {
            let start = t
                .char_indices()
                .nth(lower[..at].chars().count())
                .map_or(t.len(), |(i, _)| i);
            if let Some(n) = book_number(&t[start..]) {
                let name = t[..start]
                    .trim()
                    .trim_end_matches([',', '.', '-', ':', '–'])
                    .trim();
                if !name.is_empty() {
                    return Some((name.to_owned(), n));
                }
            }
        }
    }
    if !bare {
        return None;
    }
    // `Ведьмак 01`, `Ведьмак-1`.
    let digits = t.bytes().rev().take_while(u8::is_ascii_digit).count();
    if digits == 0 || digits > 2 {
        return None;
    }
    let (name, n) = t.split_at(t.len() - digits);
    if !name.ends_with([' ', '-']) {
        return None;
    }
    let name = name.trim().trim_end_matches(['-', ',', '.', '#']).trim();
    (!name.is_empty() && name.chars().any(char::is_alphabetic))
        .then(|| (name.to_owned(), n.parse().ok()))
        .and_then(|(name, n)| Some((name, n?)))
}

/// A leading `02 - `, `02. `, `2) `, `Книга 2. `, `Book 2 - ` number.
pub(crate) fn leading_number(s: &str) -> Option<(f64, &str)> {
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
    // A number needs a separator after it ("1984" is a title), and a mark
    // unless a word announced it ("451 градус по Фаренгейту" is a title).
    let gap = &after[..after.len() - title.len()];
    let marked = start > 0 || gap.contains(['.', '-', '_', ')', '–', '—']) || raw.ends_with('.');
    if title.len() == after.len() || title.is_empty() || !marked {
        return None;
    }
    if start == 0 && raw.len() > 3 {
        return None;
    }
    Some((raw.parse().ok()?, title))
}

/// Leading noise groups: `[Аудиокнига]`, `(Audiobook)`.
fn noise_group(group: &str) -> bool {
    let l = group.trim().to_lowercase();
    l.is_empty()
        || [
            "аудиокнига",
            "аудиокниги",
            "audiobook",
            "audio book",
            "unabridged",
            "abridged",
            "полная версия",
            "mp3",
            "m4b",
            "aac",
        ]
        .iter()
        .any(|w| l.contains(w))
}

/// What a trailing bracket group says.
fn read_group(group: &str, info: &mut NameInfo) {
    let g = group.trim();
    if let Some(n) = narrator_in(g) {
        info.narrator.get_or_insert(n);
        return;
    }
    if let Some(y) = year_of(g) {
        info.year.get_or_insert(y);
        return;
    }
    if let Some((s, n)) = series_with_number(g, false) {
        // `The Stormlight Archive, Book 1`.
        info.series.get_or_insert(s);
        info.number.get_or_insert(n);
        return;
    }
    if g.contains(',') {
        // `Кирилл Радциг, 2019, 128 kbps, MP3`: tracker style.
        for (i, item) in g.split(',').map(str::trim).enumerate() {
            if let Some(y) = year_of(item) {
                info.year.get_or_insert(y);
            } else if let Some(n) = narrator_in(item) {
                info.narrator.get_or_insert(n);
            } else if i == 0 && person_like(item) && item.split_whitespace().count() >= 2 {
                info.narrator.get_or_insert(item.to_owned());
            }
        }
        return;
    }
    if let Some(n) = bare_narrator(g) {
        info.narrator.get_or_insert(n);
    }
}

/// `Sanderson, Brandon` → `Brandon Sanderson`; other names stay.
pub(crate) fn author_name(raw: &str) -> String {
    let raw = raw.trim();
    if let Some((last, first)) = raw.split_once(", ")
        && !first.contains(',')
        && last.split_whitespace().count() == 1
        && (1..=2).contains(&first.split_whitespace().count())
        && person_like(raw)
    {
        return format!("{} {}", first.trim(), last.trim());
    }
    raw.to_owned()
}

/// `Название. Читает Кирилл Радциг` / `Название, читает …`: the narrator
/// at the end of a segment.
fn trailing_narrator(segment: &str) -> Option<(String, String)> {
    let lower = segment.to_lowercase();
    for sep in [". ", ", "] {
        for m in ["читает ", "читают ", "чтец ", "narrated by ", "read by "] {
            let needle = format!("{sep}{m}");
            if let Some(at) = lower.find(&needle) {
                let start = segment
                    .char_indices()
                    .nth(lower[..at].chars().count())
                    .map_or(segment.len(), |(i, _)| i);
                let head = segment[..start].trim();
                let tail = &segment[start..].trim_start_matches(['.', ',', ' ']);
                if let Some(n) = narrator_in(tail)
                    && !head.is_empty()
                {
                    return Some((head.to_owned(), n));
                }
            }
        }
    }
    None
}

/// `Серия. Книга 2. Название` → series, number, title.
fn dotted_series(segment: &str) -> Option<(String, f64, String)> {
    let pieces: Vec<&str> = segment.split(". ").collect();
    for i in 1..pieces.len().saturating_sub(1) {
        if let Some(n) = book_number(pieces[i]) {
            let series = pieces[..i].join(". ");
            let title = pieces[i + 1..].join(". ");
            if !series.trim().is_empty() && !title.trim().is_empty() {
                return Some((series.trim().to_owned(), n, title.trim().to_owned()));
            }
        }
    }
    None
}

/// Reads `Автор - Серия 02 - Название (2019) (читает Чтец) [128 kbps]`.
pub(crate) fn name_info(name: &str) -> NameInfo {
    let mut info = NameInfo::default();
    let mut s = name.replace('_', " ").trim().to_owned();
    // Leading noise groups: `[Аудиокнига] …`.
    loop {
        let t = s.trim_start();
        let Some(open) = t.chars().next().filter(|c| matches!(c, '(' | '[' | '{')) else {
            break;
        };
        let close = match open {
            '(' => ')',
            '[' => ']',
            _ => '}',
        };
        let Some(end) = t.find(close) else { break };
        let group = &t[1..end];
        let rest = t[end + 1..]
            .trim_start_matches([' ', '-', '.', '–'])
            .to_owned();
        if rest.is_empty() || !noise_group(group) {
            break;
        }
        s = rest;
    }
    // Trailing bracket groups: narrator, year, series, noise.
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
        let group = t[at + 1..t.len() - 1].trim().to_owned();
        let head = t[..at].trim_end().to_owned();
        if head.is_empty() {
            // The name is all brackets: keep it.
            break;
        }
        read_group(&group, &mut info);
        s = head;
    }
    let s = s.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut parts: Vec<String> = Vec::new();
    for sep in [" - ", " – ", " — "] {
        if s.contains(sep) {
            parts = s.split(sep).map(|p| p.trim().to_owned()).collect();
            break;
        }
    }
    if parts.is_empty() {
        parts.push(s.clone());
    }
    parts.retain(|p| !p.is_empty());
    // A narrator segment: `… - читает Кирилл Радциг`.
    parts.retain(|p| match narrator_in(p) {
        Some(n) => {
            info.narrator.get_or_insert(n);
            false
        }
        None => true,
    });
    if let Some(last) = parts.last_mut()
        && let Some((head, n)) = trailing_narrator(last)
    {
        info.narrator.get_or_insert(n);
        *last = head;
    }
    let mut rest: Vec<String> = match parts.split_first() {
        None => Vec::new(),
        Some((first, more)) => {
            if more.is_empty() {
                vec![first.clone()]
            } else if leading_number(&format!("{first} - x")).is_some()
                || first.bytes().all(|b| b.is_ascii_digit())
            {
                // `02 - Название`: no author.
                parts.clone()
            } else if let Some((series, n)) = series_with_number(first, true) {
                // `Mistborn 01 - The Final Empire`.
                info.series.get_or_insert(series);
                info.number.get_or_insert(n);
                more.to_vec()
            } else {
                info.author = Some(author_name(first));
                more.to_vec()
            }
        }
    };
    // `Серия 01 - Название` / `Серия - 01 - Название` / `01 - Название`.
    if rest.len() >= 2 {
        let after = format!("{} - {}", rest[1], rest[2..].join(" - "));
        if let Some((series, n)) = series_with_number(&rest[0], true) {
            info.series.get_or_insert(series);
            info.number.get_or_insert(n);
            rest.remove(0);
        } else if let Some((n, t)) = leading_number(&after)
            && !rest[0].bytes().all(|b| b.is_ascii_digit())
            && leading_number(&format!("{} - x", rest[0])).is_none()
        {
            info.series.get_or_insert(rest[0].clone());
            info.number.get_or_insert(n);
            rest = vec![t.trim_end_matches([' ', '-']).to_owned()];
        }
    }
    let joined = rest.join(" - ");
    let mut title = match leading_number(&joined) {
        Some((n, t)) => {
            info.number.get_or_insert(n);
            t.to_owned()
        }
        None => joined,
    };
    if let Some((series, n, t)) = dotted_series(&title) {
        info.series.get_or_insert(series);
        info.number.get_or_insert(n);
        title = t;
    }
    info.title = if title.trim().is_empty() {
        name.trim().to_owned()
    } else {
        title.trim().to_owned()
    };
    info
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn tracker_style_brackets() {
        let i =
            name_info("Лукьяненко Сергей - Ночной дозор [Владимир Левашёв, 2006, 64 kbps, MP3]");
        assert_eq!(i.author.as_deref(), Some("Лукьяненко Сергей"));
        assert_eq!(i.title, "Ночной дозор");
        assert_eq!(i.narrator.as_deref(), Some("Владимир Левашёв"));
        assert_eq!(i.year, Some(2006));

        let i = name_info("[Аудиокнига] Булгаков - Мастер и Маргарита (Олег Басилашвили)");
        assert_eq!(i.author.as_deref(), Some("Булгаков"));
        assert_eq!(i.title, "Мастер и Маргарита");
        assert_eq!(i.narrator.as_deref(), Some("Олег Басилашвили"));

        // A subtitle in brackets is not a narrator.
        let i = name_info("Ефремов - Туманность Андромеды (Роман)");
        assert_eq!(i.narrator, None);
        assert_eq!(i.title, "Туманность Андромеды");
    }

    #[test]
    fn series_layouts() {
        let i = name_info("Brandon Sanderson - Mistborn 01 - The Final Empire");
        assert_eq!(i.author.as_deref(), Some("Brandon Sanderson"));
        assert_eq!(i.series.as_deref(), Some("Mistborn"));
        assert_eq!(i.number, Some(1.0));
        assert_eq!(i.title, "The Final Empire");

        let i = name_info("Сапковский Анджей - Ведьмак - 02 - Меч предназначения");
        assert_eq!(i.series.as_deref(), Some("Ведьмак"));
        assert_eq!(i.number, Some(2.0));
        assert_eq!(i.title, "Меч предназначения");

        let i = name_info("Пехов Алексей - Хроники Сиалы. Книга 1. Крадущийся в тени");
        assert_eq!(i.series.as_deref(), Some("Хроники Сиалы"));
        assert_eq!(i.number, Some(1.0));
        assert_eq!(i.title, "Крадущийся в тени");

        let i = name_info("The Eye of the World (The Wheel of Time, Book 1)");
        assert_eq!(i.series.as_deref(), Some("The Wheel of Time"));
        assert_eq!(i.number, Some(1.0));
        assert_eq!(i.title, "The Eye of the World");

        let i = name_info("Sanderson, Brandon - Elantris");
        assert_eq!(i.author.as_deref(), Some("Brandon Sanderson"));
        // A four-digit number is a title.
        let i = name_info("Глуховский - Метро 2033");
        assert_eq!(i.title, "Метро 2033");
        assert_eq!(i.series, None);
    }

    #[test]
    fn narrator_segments() {
        let i = name_info("Гоголь - Мёртвые души. Читает Александр Клюквин");
        assert_eq!(i.narrator.as_deref(), Some("Александр Клюквин"));
        assert_eq!(i.title, "Мёртвые души");
        let i = name_info("Чехов - Рассказы - читает Олег Табаков");
        assert_eq!(i.narrator.as_deref(), Some("Олег Табаков"));
        assert_eq!(i.title, "Рассказы");
        assert_eq!(narrator_in("читатель"), None);
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
    fn people() {
        assert!(person_like("Стругацкие"));
        assert!(person_like("Толстой Л.Н."));
        assert!(person_like("J.R.R. Tolkien"));
        assert!(!person_like("Плоский мир"));
        assert!(!person_like("Аудиокниги"));
        assert_eq!(author_name("Tolkien, J.R.R."), "J.R.R. Tolkien");
        assert_eq!(
            author_name("Стругацкий Аркадий, Стругацкий Борис"),
            "Стругацкий Аркадий, Стругацкий Борис"
        );
    }
}
