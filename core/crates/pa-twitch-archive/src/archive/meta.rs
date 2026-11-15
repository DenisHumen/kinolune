// SPDX-License-Identifier: MPL-2.0
//! File names, the `ffmetadata` document and the `.info.json` sidecar.

use pa_twitch::Video;
use pa_twitch::chapters::Chapter;

use super::playlist::{Selection, TimeRange};

/// Longest file or folder name we produce (06 §4.3: ≤ 180, room for
/// suffixes such as `.chat.json.gz`).
pub const MAX_NAME: usize = 160;

/// Makes `name` safe as one path component on Windows, SMB, macOS and
/// Linux: no `<>:"/\|?*` or control characters, no trailing dots or
/// spaces, no reserved DOS device names, at most [`MAX_NAME`] bytes (cut on
/// a character boundary). Never empty.
pub fn sanitize(name: &str) -> String {
    let mut out: String = name
        .chars()
        .map(|c| match c {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => '_',
            c if c.is_control() => ' ',
            c => c,
        })
        .collect();
    out = out.split_whitespace().collect::<Vec<_>>().join(" ");
    if out.len() > MAX_NAME {
        let mut cut = MAX_NAME;
        while !out.is_char_boundary(cut) {
            cut -= 1;
        }
        out.truncate(cut);
    }
    let trimmed = out.trim_end_matches(['.', ' ']).trim_start();
    let mut out = trimmed.to_owned();
    let stem = out.split('.').next().unwrap_or("").to_ascii_uppercase();
    let reserved = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || ((stem.starts_with("COM") || stem.starts_with("LPT"))
            && stem.len() == 4
            && stem.as_bytes()[3].is_ascii_digit());
    if reserved {
        out.insert(0, '_');
    }
    if out.is_empty() {
        out.push('_');
    }
    out
}

/// `YYYY-MM-DD` of an RFC 3339 time, if it has one.
pub fn date_of(rfc3339: &str) -> Option<&str> {
    let d = rfc3339.get(..10)?;
    let b = d.as_bytes();
    (b[4] == b'-' && b[7] == b'-' && d.chars().all(|c| c.is_ascii_digit() || c == '-')).then_some(d)
}

/// Where a download goes, relative to its storage target, and the base
/// name of its files: `<login> [<user_id>]/<year>/<date> <title>[ — <part>]
/// [v<id>]` (06 §4.3, one folder per VOD so retention keeps or drops the
/// video and its sidecars together).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Layout {
    /// Folder relative to the target, `/`-separated.
    pub folder: String,
    /// Base name of the files in it.
    pub base: String,
}

impl Layout {
    pub fn new(video: &Video, part: Option<&str>) -> Self {
        let date = date_of(&video.created_at).unwrap_or("0000-00-00");
        let year = &date[..4];
        let title = if video.title.trim().is_empty() {
            "Twitch VOD"
        } else {
            video.title.trim()
        };
        let id = format!(" [v{}]", video.id);
        let part = part
            .map(str::trim)
            .filter(|p| !p.is_empty())
            .map(|p| format!(" — {p}"))
            .unwrap_or_default();
        // The id must survive truncation: shorten the title, not the id.
        let head = sanitize(&format!("{date} {title}{part}"));
        let room = MAX_NAME.saturating_sub(id.len());
        let mut head = head;
        if head.len() > room {
            let mut cut = room;
            while !head.is_char_boundary(cut) {
                cut -= 1;
            }
            head.truncate(cut);
            head = head.trim_end_matches(['.', ' ']).to_owned();
        }
        let base = format!("{head}{id}");
        let channel = sanitize(&format!("{} [{}]", video.user_login, video.user_id));
        Self {
            folder: format!("{channel}/{year}/{base}"),
            base,
        }
    }

    /// The same layout with ` (n)` appended to the folder, for a second
    /// download of the same VOD.
    pub fn numbered(&self, n: u32) -> Self {
        Self {
            folder: format!("{} ({n})", self.folder),
            base: self.base.clone(),
        }
    }
}

/// Escapes an `ffmetadata` key or value: `=`, `;`, `#`, `\` and newlines
/// get a backslash.
pub fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if matches!(c, '=' | ';' | '#' | '\\' | '\n') {
            out.push('\\');
        }
        if c != '\r' {
            out.push(c);
        }
    }
    out
}

/// Chapters of the selection: shifted to the output start, clipped to it,
/// empty ones dropped. Without chapters the output gets none.
pub fn output_chapters(chapters: &[Chapter], sel: &Selection) -> Vec<(u64, u64, String)> {
    let mut out = Vec::new();
    for c in chapters {
        let end = c.end_ms.unwrap_or(sel.end_ms).min(sel.end_ms);
        let start = c.start_ms.max(sel.start_ms);
        if end <= start {
            continue;
        }
        let title = if c.game.name.is_empty() {
            c.title.clone().unwrap_or_default()
        } else {
            c.game.name.clone()
        };
        out.push((start - sel.start_ms, end - sel.start_ms, title));
    }
    out
}

/// Category names of the chapters, first appearance order.
pub fn categories(chapters: &[Chapter]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for c in chapters {
        if !c.game.name.is_empty() && !out.contains(&c.game.name) {
            out.push(c.game.name.clone());
        }
    }
    out
}

/// The `ffmetadata` document (FFmpeg's `FFMETADATA1`): global tags
/// (`title`, `artist`, `date`, `comment`, `TWITCH_*`) and chapters in ms.
pub fn ffmetadata(
    video: &Video,
    chapters: &[(u64, u64, String)],
    categories: &[String],
    muted: &[TimeRange],
    part: Option<&str>,
) -> String {
    let mut tags: Vec<(&str, String)> = vec![
        ("title", video.title.clone()),
        ("artist", video.user_name.clone()),
        ("comment", video.url.clone()),
        ("TWITCH_VIDEO_ID", video.id.clone()),
        ("TWITCH_USER_ID", video.user_id.clone()),
        ("TWITCH_LOGIN", video.user_login.clone()),
        (
            "TWITCH_VIDEO_TYPE",
            format!("{:?}", video.kind).to_ascii_lowercase(),
        ),
    ];
    if let Some(date) = date_of(&video.created_at) {
        tags.push(("date", date.to_owned()));
    }
    if !video.created_at.is_empty() {
        tags.push(("TWITCH_CREATED_AT", video.created_at.clone()));
    }
    if let Some(stream) = video.stream_id.as_deref().filter(|s| !s.is_empty()) {
        tags.push(("TWITCH_STREAM_ID", stream.to_owned()));
    }
    if !categories.is_empty() {
        tags.push(("TWITCH_CATEGORIES", categories.join(", ")));
    }
    if !muted.is_empty() {
        let ranges: Vec<String> = muted
            .iter()
            .map(|r| format!("{}-{}", r.start_ms, r.end_ms))
            .collect();
        tags.push(("TWITCH_MUTED_MS", ranges.join(",")));
    }
    if let Some(part) = part {
        tags.push(("TWITCH_PART", part.to_owned()));
    }
    let mut out = String::from(";FFMETADATA1\n");
    for (k, v) in tags {
        if !v.is_empty() {
            out.push_str(&format!("{}={}\n", escape(k), escape(&v)));
        }
    }
    for (start, end, title) in chapters {
        out.push_str(&format!(
            "\n[CHAPTER]\nTIMEBASE=1/1000\nSTART={start}\nEND={end}\ntitle={}\n",
            escape(title)
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use pa_twitch::chapters::{ChapterGame, ChapterSource};

    fn video() -> Video {
        serde_json::from_value(serde_json::json!({
            "id": "2290012345", "stream_id": "51234567890", "user_id": "1001",
            "user_login": "somestreamer", "user_name": "SomeStreamer",
            "title": "Boss rush: part 2/3 = fun; #1", "created_at": "2026-11-12T18:00:00Z",
            "url": "https://www.twitch.tv/videos/2290012345", "type": "archive",
            "duration": "3h0m0s"
        }))
        .unwrap()
    }

    #[test]
    fn names_are_safe_and_keep_the_id() {
        assert_eq!(sanitize("a/b:c*?  d. "), "a_b_c__ d");
        assert_eq!(sanitize("CON.txt"), "_CON.txt");
        assert_eq!(sanitize(""), "_");
        let l = Layout::new(&video(), None);
        assert_eq!(l.folder, format!("somestreamer [1001]/2026/{}", l.base));
        assert_eq!(
            l.base,
            "2026-11-12 Boss rush_ part 2_3 = fun; #1 [v2290012345]"
        );
        let mut long = video();
        long.title = "ж".repeat(200);
        let l = Layout::new(&long, Some("Elden Ring"));
        assert!(l.base.len() <= MAX_NAME);
        assert!(l.base.ends_with(" [v2290012345]"));
        assert!(
            Layout::new(&video(), None)
                .numbered(2)
                .folder
                .ends_with(" (2)")
        );
    }

    #[test]
    fn ffmetadata_escapes_and_lists_chapters() {
        let ch = |name: &str, s: u64, e: Option<u64>| Chapter {
            game: ChapterGame::new("1", name).unwrap(),
            title: None,
            start_ms: s,
            end_ms: e,
            source: ChapterSource::EventSub,
        };
        let chapters = [
            ch("Elden Ring", 0, Some(3_600_000)),
            ch("Hades II", 3_600_000, None),
        ];
        let sel = Selection {
            first: 300,
            count: 600,
            start_ms: 3_000_000,
            end_ms: 9_000_000,
            cut_end_ms: None,
        };
        let out = output_chapters(&chapters, &sel);
        assert_eq!(
            out,
            vec![
                (0, 600_000, "Elden Ring".to_owned()),
                (600_000, 6_000_000, "Hades II".to_owned())
            ]
        );
        let cats = categories(&chapters);
        let text = ffmetadata(
            &video(),
            &out,
            &cats,
            &[TimeRange {
                start_ms: 10,
                end_ms: 20,
            }],
            None,
        );
        assert!(text.starts_with(";FFMETADATA1\n"));
        assert!(text.contains("title=Boss rush: part 2/3 \\= fun\\; \\#1\n"));
        assert!(text.contains("TWITCH_VIDEO_ID=2290012345\n"));
        assert!(text.contains("TWITCH_STREAM_ID=51234567890\n"));
        assert!(text.contains("TWITCH_CATEGORIES=Elden Ring, Hades II\n"));
        assert!(text.contains("TWITCH_MUTED_MS=10-20\n"));
        assert!(text.contains("date=2026-11-12\n"));
        assert!(
            text.contains(
                "[CHAPTER]\nTIMEBASE=1/1000\nSTART=600000\nEND=6000000\ntitle=Hades II\n"
            )
        );
    }
}
