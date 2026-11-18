// SPDX-License-Identifier: MPL-2.0
//! M3U and M3U8 playlists, and `.strm` files.

use pa_vfs::{ErrorKind, VfsError};
use url::Url;

/// One link of a playlist.
#[derive(Debug, Clone, PartialEq)]
pub struct PlaylistEntry {
    /// Title from `#EXTINF:…,Title`.
    pub title: Option<String>,
    /// Group from `group-title="…"` or `#EXTGRP:`.
    pub group: Option<String>,
    /// Absolute `http(s)` link (relative ones resolved against the
    /// playlist).
    pub url: Url,
    /// Duration from `#EXTINF`, seconds; `None` when not given.
    pub duration_secs: Option<f64>,
}

impl PlaylistEntry {
    /// A live channel or an HLS stream rather than a file: `#EXTINF:-1`
    /// (or 0), or a link to another `.m3u8`/`.m3u` playlist. These have no
    /// size and cannot be a file of the source (ADR-0124).
    #[must_use]
    pub fn is_live(&self) -> bool {
        let path = self.url.path().to_ascii_lowercase();
        self.duration_secs.is_some_and(|d| d <= 0.0)
            || path.ends_with(".m3u8")
            || path.ends_with(".m3u")
    }
}

/// What [`parse_m3u`] read.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Playlist {
    /// The `http(s)` links in order.
    pub entries: Vec<PlaylistEntry>,
    /// Lines that were links but not playable `http(s)` ones (local paths,
    /// `rtsp://`, `plugin://`, links with a password).
    pub skipped: usize,
}

/// Reads an M3U or extended M3U (`#EXTM3U`) playlist. Relative links are
/// resolved against `base` (the playlist's own URL); without a base they
/// are skipped.
///
/// # Errors
///
/// [`ErrorKind::Unsupported`] for an HLS media or master playlist (one
/// stream cut into segments, not a list of files).
pub fn parse_m3u(text: &str, base: Option<&Url>) -> pa_vfs::Result<Playlist> {
    let mut out = Playlist::default();
    let mut title = None;
    let mut duration = None;
    let mut group: Option<String> = None;
    let mut ext_group: Option<String> = None;
    for line in text.lines() {
        let line = line.trim().trim_start_matches('\u{feff}');
        if line.is_empty() {
            continue;
        }
        if let Some(tag) = line.strip_prefix('#') {
            let upper = tag.to_ascii_uppercase();
            if upper.starts_with("EXT-X-TARGETDURATION")
                || upper.starts_with("EXT-X-STREAM-INF")
                || upper.starts_with("EXT-X-MEDIA-SEQUENCE")
            {
                return Err(VfsError::new(
                    ErrorKind::Unsupported,
                    "this is one HLS stream, not a playlist of files",
                ));
            }
            if let Some(info) = tag.strip_prefix("EXTINF:") {
                let (head, name) = split_extinf(info);
                duration = head
                    .split_whitespace()
                    .next()
                    .and_then(|d| d.parse::<f64>().ok());
                group = attribute(head, "group-title");
                title = Some(name.trim().to_owned()).filter(|t| !t.is_empty());
            } else if let Some(g) = tag.strip_prefix("EXTGRP:") {
                ext_group = Some(g.trim().to_owned()).filter(|g| !g.is_empty());
            }
            continue;
        }
        let url = match base {
            Some(b) => b.join(line),
            None => Url::parse(line),
        };
        let entry_group = group.take().or_else(|| ext_group.clone());
        let entry_title = title.take();
        let entry_duration = duration.take();
        match url {
            Ok(url)
                if matches!(url.scheme(), "http" | "https")
                    && url.username().is_empty()
                    && url.password().is_none()
                    && url.host_str().is_some_and(|h| !h.is_empty()) =>
            {
                out.entries.push(PlaylistEntry {
                    title: entry_title,
                    group: entry_group,
                    url,
                    duration_secs: entry_duration,
                });
            }
            _ => out.skipped += 1,
        }
    }
    Ok(out)
}

/// Splits `-1 tvg-id="x" group-title="A, B",Title` at the comma that ends
/// the attributes (commas inside quotes do not count).
fn split_extinf(info: &str) -> (&str, &str) {
    let mut quoted = false;
    for (i, c) in info.char_indices() {
        match c {
            '"' => quoted = !quoted,
            ',' if !quoted => return (&info[..i], &info[i + 1..]),
            _ => {}
        }
    }
    (info, "")
}

/// The value of `name="value"` in the attributes of `#EXTINF`.
fn attribute(head: &str, name: &str) -> Option<String> {
    let lower = head.to_ascii_lowercase();
    let key = format!("{name}=\"");
    let start = lower.find(&key)? + key.len();
    let len = head[start..].find('"')?;
    Some(head[start..start + len].trim().to_owned()).filter(|v| !v.is_empty())
}

/// The link of a `.strm` file (Kodi, Jellyfin, Emby): the first line that
/// is not empty or a comment.
///
/// # Errors
///
/// [`ErrorKind::InvalidPath`] if there is no line or it is not an absolute
/// `http(s)` link without credentials (`plugin://` links of Kodi add-ons
/// cannot be played outside Kodi).
pub fn parse_strm(text: &str) -> pa_vfs::Result<Url> {
    let line = text
        .lines()
        .map(|l| l.trim().trim_start_matches('\u{feff}'))
        .find(|l| !l.is_empty() && !l.starts_with('#'))
        .ok_or_else(|| VfsError::new(ErrorKind::InvalidPath, "the .strm file is empty"))?;
    let url = Url::parse(line)
        .map_err(|_| VfsError::new(ErrorKind::InvalidPath, "the .strm file holds no link"))?;
    crate::client::check_link(&url)?;
    Ok(url)
}

/// Whether `name` is a `.strm` file.
#[must_use]
pub fn is_strm(name: &str) -> bool {
    name.len() > 5 && name[name.len() - 5..].eq_ignore_ascii_case(".strm")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_extended_m3u() {
        let base = Url::parse("http://media.example/lists/movies.m3u").unwrap();
        let text = "\u{feff}#EXTM3U\n\
            #EXTINF:5400 tvg-id=\"m1\" group-title=\"Movies, old\",The Movie (1999)\n\
            /files/movie.mkv\n\
            \n\
            #EXTINF:-1,Live channel\n\
            http://tv.example/live/1.m3u8\n\
            #EXTGRP:Music\n\
            #EXTINF:200,Song\n\
            song.mp3\n\
            C:\\Movies\\local.mkv\n\
            rtsp://cam/stream\n\
            http://user:pw@media.example/secret.mkv\n";
        let p = parse_m3u(text, Some(&base)).unwrap();
        assert_eq!(p.entries.len(), 3);
        assert_eq!(p.skipped, 3);
        let m = &p.entries[0];
        assert_eq!(m.title.as_deref(), Some("The Movie (1999)"));
        assert_eq!(m.group.as_deref(), Some("Movies, old"));
        assert_eq!(m.url.as_str(), "http://media.example/files/movie.mkv");
        assert_eq!(m.duration_secs, Some(5400.0));
        assert!(!m.is_live());
        assert!(p.entries[1].is_live());
        let s = &p.entries[2];
        assert_eq!(s.url.as_str(), "http://media.example/lists/song.mp3");
        assert_eq!(s.group.as_deref(), Some("Music"));
        assert_eq!(s.title.as_deref(), Some("Song"));
    }

    #[test]
    fn plain_m3u_without_base_keeps_absolute_links() {
        let p = parse_m3u("http://a.example/1.mp4\nrelative.mp4\n", None).unwrap();
        assert_eq!(p.entries.len(), 1);
        assert_eq!(p.entries[0].title, None);
        assert_eq!(p.skipped, 1);
    }

    #[test]
    fn refuses_hls_streams() {
        let e = parse_m3u(
            "#EXTM3U\n#EXT-X-TARGETDURATION:6\n#EXTINF:6,\nseg1.ts\n",
            None,
        )
        .unwrap_err();
        assert_eq!(e.kind(), ErrorKind::Unsupported);
        let e = parse_m3u(
            "#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=1\nhttp://a/b.m3u8\n",
            None,
        )
        .unwrap_err();
        assert_eq!(e.kind(), ErrorKind::Unsupported);
    }

    #[test]
    fn reads_strm_files() {
        assert_eq!(
            parse_strm("\n# Kodi\n  https://cdn.example/m.mkv?t=1 \n")
                .unwrap()
                .as_str(),
            "https://cdn.example/m.mkv?t=1"
        );
        for bad in [
            "",
            "plugin://plugin.video.x/?id=1",
            "/mnt/movie.mkv",
            "http://u:p@h/x",
        ] {
            assert_eq!(
                parse_strm(bad).unwrap_err().kind(),
                ErrorKind::InvalidPath,
                "{bad}"
            );
        }
        assert!(is_strm("Movie (2010).STRM") && !is_strm(".strm") && !is_strm("a.mkv"));
    }
}
