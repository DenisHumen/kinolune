// SPDX-License-Identifier: MPL-2.0
//! Normalized music genres (META-021).
//!
//! Genre tags are free text: `Hip-Hop/Rap`, `hip hop`, `(17)`, `RnB`,
//! `Drum'n'Bass`; MusicBrainz genres are lower case (`alternative rock`).
//! [`normalize`] turns one raw value into canonical display names, so the
//! same genre groups together whatever wrote it:
//!
//! - ID3v1 numeric references (`17`, `(17)`, `(17)Rock`) become their names;
//! - values are split on `/`, `;`, `,` and NUL (`Folk/Rock` → two genres);
//! - spelling variants map to one name through a compact key (letters and
//!   digits only, `&` and `n` as "and"): `hiphop`, `Hip Hop` → `Hip-Hop`;
//!   `R&B`, `RnB`, `rhythm and blues` → `R&B`;
//! - other names are title-cased word by word (`post-rock` → `Post-Rock`),
//!   with known acronyms kept upper case (`IDM`);
//! - placeholders (`Other`, `Unknown`, `genre`, `RX`, `CR`) are dropped.
//!
//! ```
//! use pa_library::genre::{normalize, normalize_all};
//!
//! assert_eq!(normalize("Hip-Hop/Rap"), ["Hip-Hop", "Rap"]);
//! assert_eq!(normalize("(17)"), ["Rock"]);
//! assert_eq!(normalize("alternative rock"), ["Alternative Rock"]);
//! assert_eq!(normalize_all(["rnb", "R&B", "Soul"]), ["R&B", "Soul"]);
//! ```

/// ID3v1 genres 0–79 (the standard list; later Winamp extensions are rare
/// as numbers).
const ID3V1: [&str; 80] = [
    "Blues",
    "Classic Rock",
    "Country",
    "Dance",
    "Disco",
    "Funk",
    "Grunge",
    "Hip-Hop",
    "Jazz",
    "Metal",
    "New Age",
    "Oldies",
    "Other",
    "Pop",
    "R&B",
    "Rap",
    "Reggae",
    "Rock",
    "Techno",
    "Industrial",
    "Alternative",
    "Ska",
    "Death Metal",
    "Pranks",
    "Soundtrack",
    "Euro-Techno",
    "Ambient",
    "Trip-Hop",
    "Vocal",
    "Jazz-Funk",
    "Fusion",
    "Trance",
    "Classical",
    "Instrumental",
    "Acid",
    "House",
    "Game",
    "Sound Clip",
    "Gospel",
    "Noise",
    "Alternative Rock",
    "Bass",
    "Soul",
    "Punk",
    "Space",
    "Meditative",
    "Instrumental Pop",
    "Instrumental Rock",
    "Ethnic",
    "Gothic",
    "Darkwave",
    "Techno-Industrial",
    "Electronic",
    "Pop-Folk",
    "Eurodance",
    "Dream",
    "Southern Rock",
    "Comedy",
    "Cult",
    "Gangsta",
    "Top 40",
    "Christian Rap",
    "Pop-Funk",
    "Jungle",
    "Native American",
    "Cabaret",
    "New Wave",
    "Psychedelic",
    "Rave",
    "Showtunes",
    "Trailer",
    "Lo-Fi",
    "Tribal",
    "Acid Punk",
    "Acid Jazz",
    "Polka",
    "Retro",
    "Musical",
    "Rock & Roll",
    "Hard Rock",
];

/// Compact key → canonical name, for spellings that title-casing alone
/// would not unify.
const ALIASES: &[(&str, &str)] = &[
    ("hiphop", "Hip-Hop"),
    ("triphop", "Trip-Hop"),
    ("randb", "R&B"),
    ("rnb", "R&B"),
    ("rhythmandblues", "R&B"),
    ("rockandroll", "Rock & Roll"),
    ("rocknroll", "Rock & Roll"),
    ("drumandbass", "Drum & Bass"),
    ("drumnbass", "Drum & Bass"),
    ("dnb", "Drum & Bass"),
    ("dandb", "Drum & Bass"),
    ("altrock", "Alternative Rock"),
    ("alternrock", "Alternative Rock"),
    ("alternativerock", "Alternative Rock"),
    ("alt", "Alternative"),
    ("electronica", "Electronic"),
    ("electro", "Electro"),
    ("synthpop", "Synth-Pop"),
    ("jpop", "J-Pop"),
    ("kpop", "K-Pop"),
    ("jrock", "J-Rock"),
    ("lofi", "Lo-Fi"),
    ("postrock", "Post-Rock"),
    ("postpunk", "Post-Punk"),
    ("ost", "Soundtrack"),
    ("originalsoundtrack", "Soundtrack"),
    ("soundtracks", "Soundtrack"),
    ("filmscore", "Soundtrack"),
    ("classic", "Classical"),
    ("jazzfunk", "Jazz-Funk"),
    ("popfolk", "Pop-Folk"),
    ("popfunk", "Pop-Funk"),
    ("eurotechno", "Euro-Techno"),
    ("technoindustrial", "Techno-Industrial"),
    ("psychadelic", "Psychedelic"),
    ("showtune", "Showtunes"),
    ("singersongwriter", "Singer-Songwriter"),
    ("worldmusic", "World"),
    ("easylistening", "Easy Listening"),
    ("newage", "New Age"),
    ("newwave", "New Wave"),
];

/// Words kept upper case when title-casing.
const ACRONYMS: &[&str] = &["idm", "edm", "ebm", "uk", "us", "dj", "mpb", "nwobhm"];

/// Placeholders that are not a genre (compact keys).
const JUNK: &[&str] = &[
    "other", "unknown", "genre", "misc", "none", "rx", "cr", "remix", "cover", "null",
];

/// Compact key: lowercase letters and digits only, `&` as `and`
/// (`rock'n'roll` gives `rocknroll`, listed in the aliases).
pub fn key(name: &str) -> String {
    name.to_lowercase()
        .replace('&', "and")
        .chars()
        .filter(|c| c.is_alphanumeric())
        .collect()
}

fn title_case(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut word = String::new();
    let flush = |word: &mut String, out: &mut String| {
        if word.is_empty() {
            return;
        }
        let lower = word.to_lowercase();
        if ACRONYMS.contains(&lower.as_str()) {
            out.push_str(&word.to_uppercase());
        } else if word.chars().skip(1).any(char::is_uppercase)
            && word.chars().any(char::is_lowercase)
        {
            // Mixed case as written (`McCartney`-like spellings): keep.
            out.push_str(word);
        } else {
            let mut chars = lower.chars();
            if let Some(first) = chars.next() {
                out.extend(first.to_uppercase());
                out.push_str(chars.as_str());
            }
        }
        word.clear();
    };
    for c in name.chars() {
        if c.is_alphanumeric() || c == '\'' {
            word.push(c);
        } else {
            flush(&mut word, &mut out);
            out.push(c);
        }
    }
    flush(&mut word, &mut out);
    out
}

/// ID3v1 reference at the start: `17`, `(17)`, `(17)Rock`. Returns the
/// genre name and the rest of the value.
fn id3v1(value: &str) -> Option<(&'static str, &str)> {
    let v = value.trim();
    let (digits, rest) = if let Some(inner) = v.strip_prefix('(') {
        let end = inner.find(')')?;
        (&inner[..end], &inner[end + 1..])
    } else {
        (v, "")
    };
    if digits.is_empty() || digits.len() > 3 || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let n: usize = digits.parse().ok()?;
    ID3V1.get(n).map(|name| (*name, rest))
}

/// Canonical name of one genre (already split), or `None` for a
/// placeholder or an empty value.
pub fn canonical(name: &str) -> Option<String> {
    let name = name
        .trim_matches(|c: char| c.is_whitespace() || (c.is_ascii_punctuation() && c != '&'))
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let k = key(&name);
    if k.is_empty() || JUNK.contains(&k.as_str()) {
        return None;
    }
    if let Some((_, display)) = ALIASES.iter().find(|(a, _)| *a == k) {
        return Some((*display).to_owned());
    }
    if let Some(known) = ID3V1.iter().find(|g| key(g) == k) {
        return Some((*known).to_owned());
    }
    Some(title_case(&name))
}

/// Canonical genres of one raw tag value, in order, without repeats.
pub fn normalize(raw: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut push = |g: Option<String>| {
        if let Some(g) = g
            && !out.iter().any(|o| key(o) == key(&g))
        {
            out.push(g);
        }
    };
    let mut rest = raw;
    // `(17)(18)Rock` and `(17)Rock` chains.
    while let Some((name, tail)) = id3v1(rest) {
        push(canonical(name));
        rest = tail;
        if !tail.trim_start().starts_with('(') {
            break;
        }
    }
    for part in rest.split(['/', ';', ',', '\0', '|']) {
        // A bare number left after splitting is an ID3v1 reference too.
        match id3v1(part) {
            Some((name, tail)) if tail.trim().is_empty() => push(canonical(name)),
            _ => push(canonical(part)),
        }
    }
    out
}

/// Canonical genres of several raw values, in order, without repeats.
pub fn normalize_all<I, S>(raw: I) -> Vec<String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut out: Vec<String> = Vec::new();
    for value in raw {
        for g in normalize(value.as_ref()) {
            if !out.iter().any(|o| key(o) == key(&g)) {
                out.push(g);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spellings_unify() {
        for raw in ["Hip-Hop", "hip hop", "HipHop", "hip_hop", "(7)", "7"] {
            assert_eq!(normalize(raw), ["Hip-Hop"], "{raw}");
        }
        for raw in ["R&B", "RnB", "r'n'b", "Rhythm and Blues", "(14)"] {
            assert_eq!(normalize(raw), ["R&B"], "{raw}");
        }
        for raw in ["Drum & Bass", "drum and bass", "Drum'n'Bass", "DnB"] {
            assert_eq!(normalize(raw), ["Drum & Bass"], "{raw}");
        }
        for raw in ["rock & roll", "Rock'n'Roll", "rock and roll"] {
            assert_eq!(normalize(raw), ["Rock & Roll"], "{raw}");
        }
    }

    #[test]
    fn musicbrainz_lowercase_names_are_title_cased() {
        assert_eq!(normalize("alternative rock"), ["Alternative Rock"]);
        assert_eq!(normalize("post-rock"), ["Post-Rock"]);
        assert_eq!(normalize("idm"), ["IDM"]);
        assert_eq!(normalize("drum and bass"), ["Drum & Bass"]);
        assert_eq!(normalize("русский рок"), ["Русский Рок"]);
    }

    #[test]
    fn splits_and_drops_placeholders() {
        assert_eq!(normalize("Folk/Rock; Pop"), ["Folk", "Rock", "Pop"]);
        assert_eq!(normalize("Other"), Vec::<String>::new());
        assert_eq!(normalize("  "), Vec::<String>::new());
        assert_eq!(normalize("(17)(9)"), ["Rock", "Metal"]);
        assert_eq!(normalize("(17)Hard Rock"), ["Rock", "Hard Rock"]);
        assert_eq!(normalize("Soundtrack / OST"), ["Soundtrack"]);
        assert_eq!(normalize("999"), ["999"]);
    }

    #[test]
    fn normalize_all_dedups_across_values() {
        assert_eq!(
            normalize_all(["Rock", "rock", "Hip Hop", "hip-hop"]),
            ["Rock", "Hip-Hop"]
        );
    }
}
