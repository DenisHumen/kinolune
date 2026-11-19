// SPDX-License-Identifier: MPL-2.0
//! Placeholder names (BUG-005): `clip.mp4`, `video (copy).mp4`,
//! `movie.final.FINAL(1).mkv`, `Untitled 3.mov`, `New folder (2)/`,
//! `Новое видео.mp4`, `VID_20240817_191512.mp4`, `0001.mp4`. Such a name
//! says nothing about a work: no movie is ever made from it.
//!
//! [`is_placeholder_title`] is a word rule, not a list of whole names: a
//! title is a placeholder when every word of it is a generic word of a
//! default file or folder name ([`GENERIC_WORDS`]) or a number, and at
//! least one is a generic word. One real word is enough to keep a title
//! (`Clip of the Day`, `Video Games`). A film named by generic words and
//! numbers alone (`Movie 43`, `Clip`) is told by its year, which callers
//! check: `Movie 43 (2013)` stays a movie.
//!
//! A bare number is a title (`300`, `1408`, `1917`, `2046`, `1883` are
//! films and shows of corpus v0); only a counter ([`is_counter`]: `1`,
//! `07`, `0001`, `123456`) is not.

/// Words of default file and folder names (lowercase): what cameras, phones,
/// editors, browsers and file managers write when nobody named the file,
/// and the copy and version marks people add to such names.
pub const GENERIC_WORDS: &[&str] = &[
    // What it is.
    "clip",
    "clips",
    "video",
    "videos",
    "vid",
    "vids",
    "movie",
    "movies",
    "film",
    "films",
    "media",
    "file",
    "files",
    "recording",
    "record",
    "footage",
    "videoclip",
    "img",
    "mov",
    "mvi",
    "dsc",
    "pxl",
    "gopr",
    // Nobody named it.
    "untitled",
    "unnamed",
    "noname",
    "no",
    "name",
    "new",
    "folder",
    // Copies and versions.
    "copy",
    "final",
    "edit",
    "edited",
    "output",
    "export",
    "exported",
    "render",
    "rendered",
    "compressed",
    "converted",
    // Where it landed.
    "download",
    "downloads",
    "downloaded",
    "desktop",
    "temp",
    "tmp",
    "misc",
    "unsorted",
    "incoming",
    "complete",
    "completed",
    "torrents",
    // Russian.
    "видео",
    "видеоролик",
    "ролик",
    "клип",
    "фильм",
    "фильмы",
    "кино",
    "запись",
    "файл",
    "новый",
    "новое",
    "новая",
    "папка",
    "копия",
    "без",
    "названия",
    "имени",
    "безымянный",
    "загрузки",
    "загрузка",
    "разное",
    "рабочий",
    "стол",
    "финал",
    "финальный",
];

/// `true` if `title` names no work: every word is a [generic
/// word](GENERIC_WORDS) or a number, and one at least is a generic word.
/// Case and separators do not matter (`VID_20240817_191512`,
/// `movie.final.FINAL(1)`, `New folder (2)`). A bare number is not a
/// placeholder (`300`); see [`is_counter`].
///
/// ```
/// use pa_recognition::placeholder::is_placeholder_title;
///
/// assert!(is_placeholder_title("clip"));
/// assert!(is_placeholder_title("clip (copy)"));
/// assert!(is_placeholder_title("New folder (2)"));
/// assert!(!is_placeholder_title("Clip of the Day"));
/// assert!(!is_placeholder_title("Arrival"));
/// ```
#[must_use]
pub fn is_placeholder_title(title: &str) -> bool {
    let mut generic = false;
    for w in title
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
    {
        if GENERIC_WORDS.contains(&w.to_lowercase().as_str()) {
            generic = true;
        } else if !w.chars().all(|c| c.is_ascii_digit()) {
            return false;
        }
    }
    generic
}

/// `true` if `name` is a bare counter, not a numeric title: one or two
/// digits (`1`, `07`), a leading zero (`0001`) or five digits and more
/// (`123456`). Three- and four-digit numbers are titles (`300`, `1408`,
/// `2046`) and years.
///
/// ```
/// use pa_recognition::placeholder::is_counter;
///
/// assert!(is_counter("0001"));
/// assert!(!is_counter("300"));
/// ```
#[must_use]
pub fn is_counter(name: &str) -> bool {
    let n = name.trim();
    !n.is_empty()
        && n.bytes().all(|b| b.is_ascii_digit())
        && (n.len() <= 2 || n.starts_with('0') || n.len() >= 5)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_names_are_placeholders() {
        for t in [
            "clip",
            "Clip",
            "clip (copy)",
            "clip (1)",
            "video",
            "Video 2",
            "movie",
            "movie.final.FINAL(1)",
            "movie final FINAL",
            "untitled",
            "Untitled 3",
            "New folder (2)",
            "New Video",
            "VID_20240817_191512",
            "VID 20240817 191512",
            "IMG_1234",
            "Downloads",
            "Новое видео",
            "Видео 1",
            "Без названия",
            "клип (копия)",
            "Новая папка (3)",
        ] {
            assert!(is_placeholder_title(t), "{t:?} should be a placeholder");
        }
    }

    #[test]
    fn real_titles_are_kept() {
        for t in [
            "Clip of the Day",
            "Video Games",
            "Arrival",
            "Some Film",
            "Inception",
            "Movie 43 Extended",
            "Final Destination",
            "The Final Countdown",
            "Новый Человек-паук",
            "Без границ",
            "Кино про Алексеева",
            "1080p",
            "Dark",
            // Numbers are titles.
            "300",
            "1408",
            "2046",
            "1",
            "",
        ] {
            assert!(!is_placeholder_title(t), "{t:?} should stay a title");
        }
    }

    #[test]
    fn counters_are_not_titles() {
        for n in ["1", "07", "0001", "00800", "123456"] {
            assert!(is_counter(n), "{n}");
        }
        for n in ["300", "1408", "1917", "2046", "", "12a", "Dark"] {
            assert!(!is_counter(n), "{n}");
        }
    }
}
