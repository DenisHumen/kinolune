// SPDX-License-Identifier: MPL-2.0
//! File names for links and server items that have only a title.

use std::collections::HashSet;

use url::Url;

/// Media extensions a link's path may end with (video, then audio).
pub const MEDIA_EXTENSIONS: &[&str] = &[
    "mkv", "mp4", "m4v", "avi", "mov", "wmv", "mpg", "mpeg", "ts", "m2ts", "mts", "webm", "flv",
    "ogv", "3gp", "vob", "mp3", "flac", "m4a", "m4b", "aac", "ogg", "oga", "opus", "wav", "wma",
    "aiff", "aif", "ape", "wv", "mka", "dsf",
];

/// The extension of a media file from its MIME type.
#[must_use]
pub fn extension_of_mime(mime: &str) -> Option<&'static str> {
    let mime = mime.split(';').next()?.trim().to_ascii_lowercase();
    Some(match mime.as_str() {
        "video/x-matroska" | "video/matroska" | "video/x-mkv" => "mkv",
        "video/mp4" => "mp4",
        "video/x-m4v" => "m4v",
        "video/x-msvideo" | "video/avi" | "video/divx" => "avi",
        "video/quicktime" => "mov",
        "video/x-ms-wmv" | "video/x-ms-asf" => "wmv",
        "video/mpeg" | "video/mpeg2" => "mpg",
        "video/mp2t" | "video/vnd.dlna.mpeg-tts" => "ts",
        "video/webm" => "webm",
        "video/x-flv" => "flv",
        "video/ogg" => "ogv",
        "video/3gpp" => "3gp",
        "audio/mpeg" | "audio/mp3" => "mp3",
        "audio/flac" | "audio/x-flac" => "flac",
        "audio/mp4" | "audio/x-m4a" | "audio/m4a" => "m4a",
        "audio/aac" | "audio/x-aac" => "aac",
        "audio/ogg" | "audio/vorbis" => "ogg",
        "audio/opus" => "opus",
        "audio/wav" | "audio/x-wav" | "audio/wave" => "wav",
        "audio/x-ms-wma" => "wma",
        "audio/aiff" | "audio/x-aiff" => "aiff",
        "audio/x-matroska" => "mka",
        _ => return None,
    })
}

/// Whether a MIME type is media that plays (not a web page, an image or
/// a playlist).
#[must_use]
pub fn is_media_mime(mime: &str) -> bool {
    let m = mime.trim().to_ascii_lowercase();
    (m.starts_with("video/") || m.starts_with("audio/"))
        && !m.contains("mpegurl")
        && !m.contains("x-scpls")
}

/// The media extension of a link's path, lowercase.
#[must_use]
pub fn extension_of_url(url: &Url) -> Option<&'static str> {
    let last = url.path_segments()?.next_back()?;
    let (_, ext) = last.rsplit_once('.')?;
    let ext = ext.to_ascii_lowercase();
    MEDIA_EXTENSIONS.iter().copied().find(|e| *e == ext)
}

/// The decoded last path segment of a link without its extension.
#[must_use]
pub fn stem_of_url(url: &Url) -> Option<String> {
    let last = url.path_segments()?.next_back()?;
    let decoded = percent_decode(last);
    let stem = match decoded.rsplit_once('.') {
        Some((s, _)) if !s.is_empty() => s.to_owned(),
        _ => decoded,
    };
    Some(stem).filter(|s| !s.trim().is_empty())
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    let hex = |b: u8| char::from(b).to_digit(16);
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 3 <= bytes.len()
            && let (Some(h), Some(l)) = (hex(bytes[i + 1]), hex(bytes[i + 2]))
        {
            out.push(u8::try_from(h * 16 + l).unwrap_or(b'?'));
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// A title as one file or folder name: no `/`, `\` or control
/// characters, no leading dots, at most 200 bytes.
#[must_use]
pub fn sanitize(title: &str) -> String {
    let mut s: String = title
        .chars()
        .map(|c| match c {
            '/' | '\\' => '-',
            c if c.is_control() => ' ',
            c => c,
        })
        .collect();
    s = s.split_whitespace().collect::<Vec<_>>().join(" ");
    let s = s.trim_start_matches('.').trim();
    let mut out = String::new();
    for c in s.chars() {
        if out.len() + c.len_utf8() > 200 {
            break;
        }
        out.push(c);
    }
    let out = out.trim().to_owned();
    if out.is_empty() {
        "Untitled".to_owned()
    } else {
        out
    }
}

/// `stem.ext`, unless the stem already ends with `.ext`.
#[must_use]
pub fn file_name(stem: &str, ext: &str) -> String {
    let stem = sanitize(stem);
    let suffix = format!(".{ext}");
    if stem.len() > suffix.len() && stem[stem.len() - suffix.len()..].eq_ignore_ascii_case(&suffix)
    {
        stem
    } else {
        format!("{stem}{suffix}")
    }
}

/// Hands out unique names within one folder: `Name.mkv`, `Name (2).mkv`…
/// (case-insensitive, like the file systems the scanner knows).
#[derive(Debug, Default)]
pub struct UniqueNames {
    taken: HashSet<String>,
}

impl UniqueNames {
    /// `name`, or `name (2)`, `name (3)`… before the extension.
    pub fn take(&mut self, name: &str) -> String {
        if self.taken.insert(name.to_lowercase()) {
            return name.to_owned();
        }
        let (stem, ext) = match name.rsplit_once('.') {
            Some((s, e)) if !s.is_empty() => (s, format!(".{e}")),
            _ => (name, String::new()),
        };
        (2..)
            .map(|n| format!("{stem} ({n}){ext}"))
            .find(|n| self.taken.insert(n.to_lowercase()))
            .expect("a free name")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_from_titles_and_links() {
        assert_eq!(sanitize("  AC/DC \t Live\n"), "AC-DC Live");
        assert_eq!(sanitize("..hidden"), "hidden");
        assert_eq!(sanitize(""), "Untitled");
        assert_eq!(file_name("Movie (2020)", "mkv"), "Movie (2020).mkv");
        assert_eq!(file_name("movie.MKV", "mkv"), "movie.MKV");
        let u = Url::parse("http://h/path/My%20Movie.MP4?x=1").unwrap();
        assert_eq!(extension_of_url(&u), Some("mp4"));
        assert_eq!(stem_of_url(&u).as_deref(), Some("My Movie"));
        let u = Url::parse("http://h/stream?id=1").unwrap();
        assert_eq!(extension_of_url(&u), None);
        assert_eq!(stem_of_url(&u).as_deref(), Some("stream"));
        assert_eq!(extension_of_mime("video/x-matroska"), Some("mkv"));
        assert_eq!(extension_of_mime("audio/mpeg; charset=x"), Some("mp3"));
        assert!(is_media_mime("video/mp4"));
        assert!(!is_media_mime("application/vnd.apple.mpegurl"));
        assert!(!is_media_mime("audio/x-mpegurl"));
        assert!(!is_media_mime("text/html"));
    }

    #[test]
    fn unique_names() {
        let mut n = UniqueNames::default();
        assert_eq!(n.take("A.mkv"), "A.mkv");
        assert_eq!(n.take("a.mkv"), "a (2).mkv");
        assert_eq!(n.take("A.mkv"), "A (3).mkv");
        assert_eq!(n.take("Dir"), "Dir");
        assert_eq!(n.take("Dir"), "Dir (2)");
    }
}
