// SPDX-License-Identifier: MPL-2.0
//! Site junk in names saved from streaming and tracker sites (META-032):
//! `Title — смотреть аниме онлайн бесплатно на SomeSite`, `Title 2015 1080p
//! tracker.org`, `[site.info] Title`.
//!
//! [`strip`] runs before the tokenizer. It is generic, not a list of sites:
//!
//! - **call-to-action phrases** — the name is cut at the first word of
//!   `смотреть`, `скачать`, `бесплатно`, `торрент`, `torrent`, `watch online`,
//!   `watch free`, `free download` that has a title word before it (`Мастера
//!   меча онлайн — смотреть онлайн` keeps `онлайн` of the title);
//! - **domains** — a chunk `label.tld` or `www.label.tld` (`tld` from a short
//!   list of generic and post-Soviet country domains) is dropped wherever it
//!   stands, also in brackets; known dubbing studios (`LostFilm.TV`) are kept
//!   because they carry the audio language. In a dotted scene name only a
//!   lowercase `.label.org`-style tail is dropped.

use std::borrow::Cow;

use super::lexicon;

/// Domains a site name ends with. `tv` is left out: `Gantz.TV` and scene tags.
const TLDS: &[&str] = &[
    "org", "com", "net", "info", "ru", "su", "ua", "by", "kz", "uz", "me", "cc", "to", "io",
    "club", "biz", "xyz", "top", "pro", "online", "site", "ws", "lol", "fm",
];

/// Generic domains: the only ones dropped from the tail of a dotted name.
const GENERIC_TLDS: &[&str] = &["org", "com", "net", "info", "club", "biz"];

/// Words that start the call-to-action tail of a saved page title.
const CUT_WORDS: &[&str] = &["смотреть", "скачать", "бесплатно", "торрент", "torrent"];

/// Removes site junk from a name (see the [module docs](self)).
pub(crate) fn strip(name: &str) -> Cow<'_, str> {
    let cut = cut_phrase(name);
    let dropped = drop_domains(cut);
    match dropped {
        Some(s) => Cow::Owned(s),
        None if cut.len() == name.len() => Cow::Borrowed(name),
        None => Cow::Borrowed(cut),
    }
}

/// Words of `s` (runs of letters and digits) with their byte ranges.
fn words(s: &str) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut start = None;
    for (i, c) in s.char_indices() {
        if c.is_alphanumeric() {
            start.get_or_insert(i);
        } else if let Some(st) = start.take() {
            out.push((st, i));
        }
    }
    if let Some(st) = start {
        out.push((st, s.len()));
    }
    out
}

/// `name` up to the call-to-action phrase, separators trimmed.
fn cut_phrase(name: &str) -> &str {
    let ws = words(name);
    let lower: Vec<String> = ws.iter().map(|&(a, b)| name[a..b].to_lowercase()).collect();
    for i in 1..ws.len() {
        let w = lower[i].as_str();
        let next = lower.get(i + 1).map_or("", String::as_str);
        let hit = CUT_WORDS.contains(&w)
            || (w == "watch" && matches!(next, "online" | "free"))
            || (w == "free" && next == "download");
        if !hit {
            continue;
        }
        let head = name[..ws[i].0].trim_end_matches(|c: char| {
            c.is_whitespace()
                || matches!(c, '-' | '—' | '–' | '|' | ',' | ':' | '_' | '.' | '(' | '[')
        });
        if words(head).is_empty() {
            return name;
        }
        return head;
    }
    name
}

/// `true` for `label.tld` / `www.label.tld` (lowercased).
fn is_domain(chunk: &str) -> bool {
    let parts: Vec<&str> = chunk.split('.').collect();
    let parts = match parts.as_slice() {
        ["www", rest @ ..] => rest,
        all => all,
    };
    let [label, tld] = parts else {
        return false;
    };
    TLDS.contains(tld)
        && label.chars().count() >= 3
        && label.chars().any(char::is_alphabetic)
        && label.chars().all(|c| c.is_alphanumeric() || c == '-')
}

/// The `.label.org` tail of a lowercase dotted name, if it is a site.
fn dotted_site_tail(s: &str) -> Option<&str> {
    let (head, tld) = s.rsplit_once('.')?;
    let (_, label) = head.rsplit_once('.')?;
    let domain = format!("{label}.{tld}");
    (GENERIC_TLDS.contains(&tld) && is_domain(&domain) && !lexicon::is_dub_studio(&domain))
        .then(|| &s[head.len() - label.len() - 1..])
}

/// Drops domain chunks; `None` when nothing changes.
fn drop_domains(name: &str) -> Option<String> {
    let mut changed = false;
    let mut kept: Vec<String> = Vec::new();
    for chunk in name.split(' ') {
        let bare = chunk.trim_matches(|c: char| matches!(c, '[' | ']' | '(' | ')' | '{' | '}'));
        let lower = bare.to_lowercase();
        if !bare.is_empty() && is_domain(&lower) && !lexicon::is_dub_studio(&lower) {
            changed = true;
            continue;
        }
        // A dotted scene name: `title.2010.1080p.tracker.org`.
        if lower.matches('.').count() >= 3
            && bare == lower
            && let Some(tail) = dotted_site_tail(bare)
        {
            changed = true;
            kept.push(chunk.replacen(tail, "", 1));
            continue;
        }
        kept.push(chunk.to_owned());
    }
    changed.then(|| {
        kept.retain(|c| !c.is_empty());
        kept.join(" ")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn call_to_action_tails_are_cut() {
        assert_eq!(
            strip("Сказка о звезде — смотреть аниме онлайн бесплатно на ПримерСайт Пример"),
            "Сказка о звезде"
        );
        assert_eq!(strip("Title - watch online free HD"), "Title");
        assert_eq!(strip("Фильм (2010) скачать торрент"), "Фильм (2010)");
        // `онлайн` of the title stays; the phrase starts at `смотреть`.
        assert_eq!(
            strip("Мастера меча онлайн смотреть онлайн"),
            "Мастера меча онлайн"
        );
        // Nothing before the phrase: the name stays as it is.
        assert_eq!(strip("Смотреть"), "Смотреть");
    }

    #[test]
    fn domains_are_dropped() {
        assert_eq!(
            strip("Fjord 2015 1080p Blu-ray ReMuX tracker.org"),
            "Fjord 2015 1080p Blu-ray ReMuX"
        );
        assert_eq!(strip("[example.info] Some Title 01"), "Some Title 01");
        assert_eq!(strip("Some Title www.example-site.ru 05"), "Some Title 05");
        assert_eq!(
            strip("some.title.2010.1080p.tracker.org"),
            "some.title.2010.1080p"
        );
        // Dubbing studios carry the audio language and stay.
        assert_eq!(strip("Show S01E01 LostFilm.TV"), "Show S01E01 LostFilm.TV");
        // Scene names and short labels are not domains.
        assert_eq!(
            strip("Dark.City.1998.Directors.Cut"),
            "Dark.City.1998.Directors.Cut"
        );
        assert_eq!(strip("Show.Title.S01E01.RU"), "Show.Title.S01E01.RU");
        assert_eq!(strip("Mr. Robot"), "Mr. Robot");
    }
}
