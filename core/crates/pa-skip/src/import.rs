// SPDX-License-Identifier: MPL-2.0
//! Ready-made intro and credits markers of other programs (PLAY-015):
//!
//! - **EDL** — the Kodi/MPlayer edit decision list next to the video
//!   (`Episode.edl`), written by hand, by Comskip or by Jellyfin's
//!   intro-skipper plugin: `start end action` lines, times in seconds
//!   (`12.5`) or as `[hh:]mm:ss[.sss]`, actions 0 (cut), 1 (mute),
//!   2 (scene marker), 3 (commercial break). An EDL does not say what a
//!   range is, so a skip range (0, 2 or 3) near the start is the intro and
//!   one near the end the credits ([`parse_edl`]).
//! - **Plex** — the markers of an episode (`<Marker type="intro"
//!   startTimeOffset=… endTimeOffset=…>`) as Plex Media Server answers
//!   `/library/metadata/<id>?includeMarkers=1`, JSON or XML
//!   ([`parse_plex`]).
//! - **Jellyfin** — the Media Segments API (`/MediaSegments/<id>`:
//!   `Items` with `Type` and `StartTicks`/`EndTicks`) or the intro-skipper
//!   plugin (`IntroStart`/`IntroEnd` in seconds, alone or per mode:
//!   `{"Introduction": …, "Credits": …}`) ([`parse_jellyfin`]).
//!
//! A user keeps a saved answer next to the video as `<video stem>.plex.json`
//! (or `.plex.xml`) and `<video stem>.jellyfin.json`; [`import_season`]
//! reads those sidecars and EDLs for the files of a season. Fetching
//! markers from a running server is NET-012's; whoever does it hands the
//! answer to [`import_markers`]. Imported markers win over chapter names
//! and detection (`pa_library::skip::effective_segments`); one marker per
//! kind, the typed exports (Plex, Jellyfin) before an EDL's guess.

use std::collections::HashMap;

use pa_library::rusqlite::Connection;
use pa_library::skip::{self, SeasonFile, SkipKind, SkipOrigin, SkipProvider, SkipSegment};
use quick_xml::Reader;
use quick_xml::events::{BytesStart, Event};
use serde_json::Value;

use crate::Error;
use crate::job::{Opener, now_ms, season_window};

/// A marker read from another program.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Marker {
    pub kind: SkipKind,
    pub start_ms: u64,
    pub end_ms: u64,
}

/// Why a payload could not be read.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct ParseError(pub String);

fn parse_err(e: impl std::fmt::Display) -> ParseError {
    ParseError(e.to_string())
}

/// Biggest sidecar read.
const MAX_SIDECAR_BYTES: u64 = 1 << 20;

/// The intro of an EDL starts in the first quarter of the file and in its
/// first 10 minutes; like a chapter, it is at most 5 minutes long.
const EDL_INTRO_WINDOW_MS: u64 = 10 * 60_000;
const EDL_INTRO_MAX_MS: u64 = 5 * 60_000;
/// The credits of an EDL start in the second half and end in the last
/// 10 minutes; at most 15 minutes long.
const EDL_CREDITS_TAIL_MS: u64 = 10 * 60_000;
const EDL_CREDITS_MAX_MS: u64 = 15 * 60_000;

/// One time of an EDL line in milliseconds: seconds (`12.5`) or
/// `[hh:]mm:ss[.sss]`. Frame numbers (`#123`) need the frame rate and
/// give `None`.
fn edl_time(s: &str) -> Option<u64> {
    if s.starts_with('#') {
        return None;
    }
    let mut total = 0.0_f64;
    let parts: Vec<&str> = s.split(':').collect();
    if parts.len() > 3 {
        return None;
    }
    for (i, p) in parts.iter().enumerate() {
        let last = i + 1 == parts.len();
        if p.is_empty() || p.starts_with(['+', '-']) || (!last && p.contains('.')) {
            return None;
        }
        let v: f64 = p.parse().ok()?;
        if !v.is_finite() || (i > 0 && v >= 60.0) {
            return None;
        }
        total = total * 60.0 + v;
    }
    Some((total * 1000.0).round() as u64)
}

/// The skip ranges of an EDL file, in file order: actions 0, 2 and 3
/// (mute is not a skip). Lines that do not parse are left out.
#[must_use]
pub fn edl_ranges(text: &str) -> Vec<(u64, u64)> {
    let mut out = Vec::new();
    for line in text.trim_start_matches('\u{feff}').lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with("##") || line.starts_with(';') {
            continue;
        }
        let tokens: Vec<&str> = line.split_whitespace().collect();
        if !(2..=3).contains(&tokens.len()) {
            continue;
        }
        let (Some(start), Some(end)) = (edl_time(tokens[0]), edl_time(tokens[1])) else {
            continue;
        };
        // MPlayer requires the action; Kodi reads a missing one as a cut.
        let action = tokens.get(2).map_or(Some(0), |a| a.parse::<u8>().ok());
        if matches!(action, Some(0 | 2 | 3)) && end > start {
            out.push((start, end));
        }
    }
    out
}

/// Intro and credits of an EDL file: the first skip range that starts
/// in the intro window, and the last one that starts in the second half
/// and ends near the end of the file (needs `duration_ms`). Ranges in
/// between (Comskip's commercial breaks) are left out.
#[must_use]
pub fn parse_edl(text: &str, duration_ms: Option<u64>) -> Vec<Marker> {
    let ranges = edl_ranges(text);
    let intro_window =
        duration_ms.map_or(EDL_INTRO_WINDOW_MS, |d| (d / 4).min(EDL_INTRO_WINDOW_MS));
    let mut out = Vec::new();
    let intro = ranges
        .iter()
        .find(|(s, e)| *s < intro_window && e - s <= EDL_INTRO_MAX_MS);
    if let Some(&(start_ms, end_ms)) = intro {
        out.push(Marker {
            kind: SkipKind::Intro,
            start_ms,
            end_ms,
        });
    }
    if let Some(d) = duration_ms.filter(|d| *d > 0) {
        let credits = ranges.iter().rev().find(|(s, e)| {
            *s >= d / 2
                && e.saturating_add(EDL_CREDITS_TAIL_MS) >= d
                && e - s <= EDL_CREDITS_MAX_MS
                && Some(&(*s, *e)) != intro
        });
        if let Some(&(start_ms, end_ms)) = credits {
            out.push(Marker {
                kind: SkipKind::Credits,
                start_ms,
                end_ms: end_ms.min(d).max(start_ms + 1),
            });
        }
    }
    out
}

fn json_u64(v: &Value) -> Option<u64> {
    match v {
        Value::Number(n) => n
            .as_u64()
            .or_else(|| n.as_f64().filter(|f| *f >= 0.0).map(|f| f.round() as u64)),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

fn json_f64(v: &Value) -> Option<f64> {
    match v {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

fn plex_kind(t: &str) -> Option<SkipKind> {
    match t.to_ascii_lowercase().as_str() {
        "intro" => Some(SkipKind::Intro),
        "credits" => Some(SkipKind::Credits),
        // Commercials of DVR recordings and anything newer.
        _ => None,
    }
}

fn marker(kind: Option<SkipKind>, start_ms: Option<u64>, end_ms: Option<u64>) -> Option<Marker> {
    let (kind, start_ms, end_ms) = (kind?, start_ms?, end_ms?);
    (end_ms > start_ms).then_some(Marker {
        kind,
        start_ms,
        end_ms,
    })
}

/// Every array named `key` in a JSON document, depth first.
fn arrays_named<'a>(v: &'a Value, key: &str, out: &mut Vec<&'a Vec<Value>>) {
    match v {
        Value::Object(map) => {
            for (k, child) in map {
                match child {
                    Value::Array(items) if k == key => out.push(items),
                    _ => arrays_named(child, key, out),
                }
            }
        }
        Value::Array(items) => {
            for child in items {
                arrays_named(child, key, out);
            }
        }
        _ => {}
    }
}

/// Plex markers from an answer of Plex Media Server, JSON
/// (`MediaContainer.Metadata[].Marker[]`, or a bare list of markers) or
/// XML (`<Marker type=… startTimeOffset=… endTimeOffset=…/>`), offsets in
/// milliseconds. An answer about several items is refused: the markers
/// would not be this file's.
///
/// # Errors
///
/// Neither JSON nor XML, or several items with markers.
pub fn parse_plex(text: &str) -> Result<Vec<Marker>, ParseError> {
    let text = text.trim_start_matches('\u{feff}').trim();
    if text.starts_with('<') {
        return parse_plex_xml(text);
    }
    let v: Value = serde_json::from_str(text).map_err(parse_err)?;
    let mut lists = Vec::new();
    if let Value::Array(items) = &v {
        lists.push(items);
    } else {
        arrays_named(&v, "Marker", &mut lists);
    }
    if lists.len() > 1 {
        return Err(ParseError("markers of several items".into()));
    }
    Ok(lists
        .into_iter()
        .flatten()
        .filter_map(|m| {
            marker(
                m.get("type").and_then(Value::as_str).and_then(plex_kind),
                m.get("startTimeOffset").and_then(json_u64),
                m.get("endTimeOffset").and_then(json_u64),
            )
        })
        .collect())
}

fn xml_attrs(e: &BytesStart<'_>) -> Result<Vec<(String, String)>, ParseError> {
    let mut out = Vec::new();
    for a in e.attributes() {
        let a = a.map_err(parse_err)?;
        let key = a.key.as_ref().to_owned();
        let val = a
            .normalized_value(quick_xml::XmlVersion::Implicit1_0)
            .map_err(parse_err)?
            .into_owned();
        out.push((key, val));
    }
    Ok(out)
}

/// Elements that are one item of a Plex answer.
const PLEX_ITEMS: &[&str] = &["Video", "Directory", "Metadata", "Track"];

fn parse_plex_xml(text: &str) -> Result<Vec<Marker>, ParseError> {
    let mut reader = Reader::from_str(text);
    let mut out = Vec::new();
    // Items seen and the item the last marker belonged to.
    let mut items = 0_usize;
    let mut marked_item: Option<usize> = None;
    loop {
        let ev = reader.read_event().map_err(parse_err)?;
        let e = match &ev {
            Event::Start(e) | Event::Empty(e) => e,
            Event::Eof => break,
            _ => continue,
        };
        let qname = e.name();
        let name: &str = qname.as_ref();
        if PLEX_ITEMS.contains(&name) {
            items += 1;
        } else if name == "Marker" {
            if marked_item.is_some_and(|i| i != items) {
                return Err(ParseError("markers of several items".into()));
            }
            marked_item = Some(items);
            let attrs = xml_attrs(e)?;
            let get = |k: &str| attrs.iter().find(|(a, _)| a == k).map(|(_, v)| v.as_str());
            if let Some(m) = marker(
                get("type").and_then(plex_kind),
                get("startTimeOffset").and_then(|v| v.trim().parse().ok()),
                get("endTimeOffset").and_then(|v| v.trim().parse().ok()),
            ) {
                out.push(m);
            }
        }
    }
    Ok(out)
}

/// Jellyfin's ticks (100 ns) to milliseconds.
fn ticks_ms(v: &Value) -> Option<u64> {
    json_u64(v).map(|t| (t + 5_000) / 10_000)
}

fn seconds_ms(v: &Value) -> Option<u64> {
    json_f64(v)
        .filter(|s| s.is_finite() && *s >= 0.0)
        .map(|s| (s * 1000.0).round() as u64)
}

fn jellyfin_kind(t: &str) -> Option<SkipKind> {
    match t.to_ascii_lowercase().as_str() {
        "intro" | "introduction" => Some(SkipKind::Intro),
        "outro" | "credits" => Some(SkipKind::Credits),
        "recap" => Some(SkipKind::Recap),
        "preview" => Some(SkipKind::Preview),
        // `Commercial`, `Unknown`.
        _ => None,
    }
}

/// One intro-skipper segment (`IntroStart`, `IntroEnd` in seconds;
/// `Valid: false` means none).
fn intro_skipper(kind: Option<SkipKind>, v: &Value) -> Option<Marker> {
    if v.get("Valid").and_then(Value::as_bool) == Some(false) {
        return None;
    }
    marker(
        kind,
        v.get("IntroStart").and_then(seconds_ms),
        v.get("IntroEnd").and_then(seconds_ms),
    )
}

/// Jellyfin markers: an answer of the Media Segments API
/// (`{"Items": [{"Type": "Intro", "StartTicks": …, "EndTicks": …}]}` or a
/// bare list of segments; `Outro` is the credits) or of the intro-skipper
/// plugin (`{"IntroStart": 12.3, "IntroEnd": 45.6}` for the intro, or one
/// such object per mode: `{"Introduction": …, "Credits": …}`).
///
/// # Errors
///
/// Not JSON, or segments of several items.
pub fn parse_jellyfin(text: &str) -> Result<Vec<Marker>, ParseError> {
    let v: Value =
        serde_json::from_str(text.trim_start_matches('\u{feff}').trim()).map_err(parse_err)?;
    let segments = match &v {
        Value::Array(items) => Some(items),
        Value::Object(map) => map.get("Items").and_then(Value::as_array),
        _ => None,
    };
    if let Some(items) = segments {
        let mut owners = items
            .iter()
            .filter_map(|s| s.get("ItemId").and_then(Value::as_str))
            .collect::<Vec<_>>();
        owners.sort_unstable();
        owners.dedup();
        if owners.len() > 1 {
            return Err(ParseError("segments of several items".into()));
        }
        return Ok(items
            .iter()
            .filter_map(|s| {
                marker(
                    s.get("Type")
                        .and_then(Value::as_str)
                        .and_then(jellyfin_kind),
                    s.get("StartTicks").and_then(ticks_ms),
                    s.get("EndTicks").and_then(ticks_ms),
                )
            })
            .collect());
    }
    if v.get("IntroStart").is_some() {
        return Ok(intro_skipper(Some(SkipKind::Intro), &v)
            .into_iter()
            .collect());
    }
    let Value::Object(map) = &v else {
        return Ok(Vec::new());
    };
    Ok(map
        .iter()
        .filter_map(|(mode, seg)| intro_skipper(jellyfin_kind(mode), seg))
        .collect())
}

/// Markers of `provider` in `text`; `duration_ms` places an EDL's ranges.
///
/// # Errors
///
/// The payload is not what the provider writes.
pub fn parse(
    provider: SkipProvider,
    text: &str,
    duration_ms: Option<u64>,
) -> Result<Vec<Marker>, ParseError> {
    match provider {
        SkipProvider::Plex => parse_plex(text),
        SkipProvider::Jellyfin => parse_jellyfin(text),
        SkipProvider::Edl => Ok(parse_edl(text, duration_ms)),
    }
}

/// One marker per kind, the first of each kind in `markers`.
fn first_per_kind(
    markers: impl IntoIterator<Item = (Marker, SkipProvider, Option<String>)>,
) -> Vec<SkipSegment> {
    let mut out: Vec<SkipSegment> = Vec::new();
    for (m, provider, sidecar) in markers {
        if out.iter().any(|s| s.kind == m.kind) {
            continue;
        }
        out.push(SkipSegment {
            provider: Some(provider),
            provider_ref: sidecar,
            ..SkipSegment::new(m.kind, SkipOrigin::Import, m.start_ms, m.end_ms, 1.0)
        });
    }
    out
}

/// Stores markers a program handed over for a file (for instance a media
/// server's answer, NET-012): replaces the markers handed over before,
/// keeps those of sidecar files. Returns how many were stored.
///
/// # Errors
///
/// The payload does not parse ([`Error::Import`]), database errors.
pub fn import_markers(
    conn: &Connection,
    file_id: i64,
    provider: SkipProvider,
    text: &str,
) -> Result<usize, Error> {
    let duration = duration_of(conn, file_id, None)?;
    let markers = parse(provider, text, duration).map_err(|e| Error::Import(e.0))?;
    let segments = first_per_kind(markers.into_iter().map(|m| (m, provider, None)));
    skip::replace_imported(conn, file_id, false, &segments, now_ms())?;
    Ok(segments.len())
}

/// Sidecar names of a video, with the provider each holds, in the order
/// they win: typed exports before an EDL's guess.
fn sidecar_suffixes() -> [(&'static str, SkipProvider); 4] {
    [
        (".plex.json", SkipProvider::Plex),
        (".plex.xml", SkipProvider::Plex),
        (".jellyfin.json", SkipProvider::Jellyfin),
        (".edl", SkipProvider::Edl),
    ]
}

fn split_dir(path: &str) -> (&str, &str) {
    path.rsplit_once('/').unwrap_or(("", path))
}

fn stem(name: &str) -> &str {
    name.rsplit_once('.')
        .map_or(name, |(s, _)| if s.is_empty() { name } else { s })
}

/// The sidecars of `video` (a file name) among the names of its folder,
/// in the order they win.
#[must_use]
pub fn sidecars_of(video: &str, names: &[String]) -> Vec<(String, SkipProvider)> {
    let stem = stem(video).to_lowercase();
    let mut out = Vec::new();
    for (suffix, provider) in sidecar_suffixes() {
        let want = format!("{stem}{suffix}");
        if let Some(n) = names.iter().find(|n| n.to_lowercase() == want) {
            out.push((n.clone(), provider));
        }
    }
    out
}

fn duration_of(conn: &Connection, file_id: i64, known: Option<u64>) -> Result<Option<u64>, Error> {
    if known.is_some() {
        return Ok(known);
    }
    let from_files: Option<i64> = conn
        .query_row(
            "SELECT duration_ms FROM files WHERE id = ?1",
            [file_id],
            |r| r.get(0),
        )
        .ok()
        .flatten();
    if let Some(d) = from_files.and_then(|d| u64::try_from(d).ok()) {
        return Ok(Some(d));
    }
    Ok(skip::analysis(conn, file_id)?.and_then(|a| a.duration_ms))
}

fn read_text(opener: &dyn Opener, source_id: i64, path: &str) -> Result<String, String> {
    let source = opener.open(source_id, path)?;
    let size = source.size().min(MAX_SIDECAR_BYTES);
    let mut buf = vec![0_u8; usize::try_from(size).unwrap_or(0)];
    source
        .read_exact_at(0, &mut buf)
        .map_err(|e| e.to_string())?;
    Ok(String::from_utf8_lossy(&buf).into_owned())
}

/// What [`import_season`] did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ImportReport {
    /// Files of the season.
    pub files: usize,
    /// Files with imported markers now.
    pub imported: usize,
    /// Files left as they were: their folder could not be listed or a
    /// sidecar could not be read.
    pub skipped: usize,
    /// Sidecars that did not parse.
    pub invalid: usize,
}

/// Reads the marker sidecars (`<stem>.edl`, `<stem>.plex.json`,
/// `<stem>.plex.xml`, `<stem>.jellyfin.json`) of the files of the season
/// (or folder) of `file_id` and stores what they hold. A folder that
/// cannot be listed or a sidecar that cannot be read (a network source
/// away) leaves the file's imported markers as they were; a sidecar that
/// is gone takes its markers with it.
///
/// # Errors
///
/// Database errors.
pub fn import_season(
    conn: &Connection,
    opener: &dyn Opener,
    file_id: i64,
) -> Result<ImportReport, Error> {
    let files = season_window(conn, file_id)?;
    let mut report = ImportReport {
        files: files.len(),
        ..ImportReport::default()
    };
    let mut listings: HashMap<(i64, String), Option<Vec<String>>> = HashMap::new();
    for file in &files {
        let (dir, name) = split_dir(&file.path);
        let names = listings
            .entry((file.source_id, dir.to_owned()))
            .or_insert_with(|| opener.list(file.source_id, dir).ok());
        let Some(names) = names else {
            report.skipped += 1;
            continue;
        };
        let sidecars = sidecars_of(name, names);
        match import_file(conn, opener, file, dir, &sidecars)? {
            Some((n, invalid)) => {
                report.imported += usize::from(n > 0);
                report.invalid += invalid;
            }
            None => report.skipped += 1,
        }
    }
    Ok(report)
}

/// Imports the sidecars of one file: `Some((stored, invalid))`, or `None`
/// if one could not be read.
fn import_file(
    conn: &Connection,
    opener: &dyn Opener,
    file: &SeasonFile,
    dir: &str,
    sidecars: &[(String, SkipProvider)],
) -> Result<Option<(usize, usize)>, Error> {
    let mut found = Vec::new();
    let mut invalid = 0;
    let mut duration = file.duration_ms;
    for (name, provider) in sidecars {
        let path = format!("{dir}/{name}");
        let Ok(text) = read_text(opener, file.source_id, &path) else {
            return Ok(None);
        };
        if *provider == SkipProvider::Edl {
            duration = duration_of(conn, file.file_id, duration)?;
        }
        match parse(*provider, &text, duration) {
            Ok(markers) => {
                found.extend(
                    markers
                        .into_iter()
                        .map(|m| (m, *provider, Some(name.clone()))),
                );
            }
            Err(_) => invalid += 1,
        }
    }
    let segments = first_per_kind(found);
    skip::replace_imported(conn, file.file_id, true, &segments, now_ms())?;
    Ok(Some((segments.len(), invalid)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(kind: SkipKind, start_ms: u64, end_ms: u64) -> Marker {
        Marker {
            kind,
            start_ms,
            end_ms,
        }
    }

    #[test]
    fn edl_times() {
        assert_eq!(edl_time("12.5"), Some(12_500));
        assert_eq!(edl_time("0"), Some(0));
        assert_eq!(edl_time("01:02.250"), Some(62_250));
        assert_eq!(edl_time("1:00:01.5"), Some(3_601_500));
        assert_eq!(edl_time("#250"), None);
        assert_eq!(edl_time("1:75"), None);
        assert_eq!(edl_time("-3"), None);
        assert_eq!(edl_time("1.5:20"), None);
        assert_eq!(edl_time("abc"), None);
    }

    #[test]
    fn edl_intro_and_credits() {
        // Jellyfin intro-skipper with "commercial break", a Comskip break
        // in the middle, a mute, credits before an after-credits scene.
        let edl = "\u{feff}\
            12.48\t42.976\t3\n\
            ## a comment\n\
            600 660 3\n\
            700 705 1\n\
            1300.2 1385.75 3\n\
            garbage line\n";
        let markers = parse_edl(edl, Some(1_440_000));
        assert_eq!(
            markers,
            [
                m(SkipKind::Intro, 12_480, 42_976),
                m(SkipKind::Credits, 1_300_200, 1_385_750)
            ]
        );
        // Without a duration only the intro is placed.
        assert_eq!(parse_edl(edl, None), [m(SkipKind::Intro, 12_480, 42_976)]);
        // Kodi times and a missing action (a cut).
        let kodi = "00:00:05.5 00:01:35.5\n00:23:00 00:24:00 0\n";
        assert_eq!(
            parse_edl(kodi, Some(1_450_000)),
            [
                m(SkipKind::Intro, 5_500, 95_500),
                m(SkipKind::Credits, 1_380_000, 1_440_000)
            ]
        );
        // A long cut at the start is not an intro; a mute never is.
        assert!(parse_edl("0 400 0\n10 40 1\n", Some(3_000_000)).is_empty());
        // Frame lines need a frame rate.
        assert!(parse_edl("#10 #500 0\n", Some(1_000_000)).is_empty());
    }

    #[test]
    fn plex_json_and_xml() {
        let json = r#"{"MediaContainer":{"size":1,"Metadata":[{"ratingKey":"7",
            "type":"episode","Marker":[
              {"id":1,"type":"intro","startTimeOffset":63520,"endTimeOffset":123480},
              {"id":2,"type":"credits","startTimeOffset":2526000,"endTimeOffset":2581000,"final":true},
              {"id":3,"type":"commercial","startTimeOffset":900000,"endTimeOffset":960000}
            ]}]}}"#;
        let want = [
            m(SkipKind::Intro, 63_520, 123_480),
            m(SkipKind::Credits, 2_526_000, 2_581_000),
        ];
        assert_eq!(parse_plex(json).unwrap(), want);
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
            <MediaContainer size="1">
              <Video ratingKey="7" type="episode" title="Pilot &amp; more">
                <Media id="1"><Part id="1" file="/x.mkv"/></Media>
                <Marker id="1" type="intro" startTimeOffset="63520" endTimeOffset="123480"/>
                <Marker id="2" type="credits" startTimeOffset="2526000" endTimeOffset="2581000" final="1"></Marker>
              </Video>
            </MediaContainer>"#;
        assert_eq!(parse_plex(xml).unwrap(), want);
        // A bare list of markers.
        let list = r#"[{"type":"intro","startTimeOffset":"1000","endTimeOffset":"31000"}]"#;
        assert_eq!(
            parse_plex(list).unwrap(),
            [m(SkipKind::Intro, 1_000, 31_000)]
        );
        // Markers of a whole season are not one file's.
        let two = r#"{"MediaContainer":{"Metadata":[
            {"Marker":[{"type":"intro","startTimeOffset":1,"endTimeOffset":2}]},
            {"Marker":[{"type":"intro","startTimeOffset":3,"endTimeOffset":4}]}]}}"#;
        assert!(parse_plex(two).is_err());
        let two_xml = r#"<MediaContainer><Video><Marker type="intro" startTimeOffset="1" endTimeOffset="2"/></Video>
            <Video><Marker type="intro" startTimeOffset="3" endTimeOffset="4"/></Video></MediaContainer>"#;
        assert!(parse_plex(two_xml).is_err());
        assert!(parse_plex("not json").is_err());
        // Inverted or missing offsets are dropped.
        let bad = r#"[{"type":"intro","startTimeOffset":5,"endTimeOffset":5},{"type":"credits"}]"#;
        assert!(parse_plex(bad).unwrap().is_empty());
    }

    #[test]
    fn jellyfin_media_segments_and_intro_skipper() {
        let segments = r#"{"Items":[
            {"Id":"a","ItemId":"e1","Type":"Intro","StartTicks":635200000,"EndTicks":1234800000},
            {"Id":"b","ItemId":"e1","Type":"Outro","StartTicks":25260000000,"EndTicks":25810000000},
            {"Id":"c","ItemId":"e1","Type":"Recap","StartTicks":0,"EndTicks":300000000},
            {"Id":"d","ItemId":"e1","Type":"Commercial","StartTicks":9000000000,"EndTicks":9600000000}
        ],"TotalRecordCount":4,"StartIndex":0}"#;
        assert_eq!(
            parse_jellyfin(segments).unwrap(),
            [
                m(SkipKind::Intro, 63_520, 123_480),
                m(SkipKind::Credits, 2_526_000, 2_581_000),
                m(SkipKind::Recap, 0, 30_000)
            ]
        );
        let other = r#"[{"ItemId":"e1","Type":"Intro","StartTicks":0,"EndTicks":10},
                        {"ItemId":"e2","Type":"Intro","StartTicks":0,"EndTicks":10}]"#;
        assert!(parse_jellyfin(other).is_err());
        let v1 = r#"{"EpisodeId":"e1","Valid":true,"IntroStart":63.52,"IntroEnd":123.48,
                     "ShowSkipPromptAt":58.52,"HideSkipPromptAt":73.52}"#;
        assert_eq!(
            parse_jellyfin(v1).unwrap(),
            [m(SkipKind::Intro, 63_520, 123_480)]
        );
        let modes = r#"{"Introduction":{"EpisodeId":"e1","Valid":true,"IntroStart":63.52,"IntroEnd":123.48},
                        "Credits":{"EpisodeId":"e1","Valid":true,"IntroStart":2526.0,"IntroEnd":2581.0},
                        "Recap":{"EpisodeId":"e1","Valid":false,"IntroStart":0,"IntroEnd":0}}"#;
        let mut got = parse_jellyfin(modes).unwrap();
        got.sort_by_key(|m| m.kind);
        assert_eq!(
            got,
            [
                m(SkipKind::Intro, 63_520, 123_480),
                m(SkipKind::Credits, 2_526_000, 2_581_000)
            ]
        );
        assert!(parse_jellyfin("<xml/>").is_err());
    }

    #[test]
    fn sidecar_names() {
        let names: Vec<String> = [
            "Show S01E01.mkv",
            "Show S01E01.EDL",
            "Show S01E01.plex.json",
            "Show S01E01.jellyfin.json",
            "Show S01E02.edl",
            "Show S01E01.en.srt",
        ]
        .map(str::to_owned)
        .to_vec();
        assert_eq!(
            sidecars_of("Show S01E01.mkv", &names),
            [
                ("Show S01E01.plex.json".to_owned(), SkipProvider::Plex),
                (
                    "Show S01E01.jellyfin.json".to_owned(),
                    SkipProvider::Jellyfin
                ),
                ("Show S01E01.EDL".to_owned(), SkipProvider::Edl),
            ]
        );
        assert!(sidecars_of("Show S01E03.mkv", &names).is_empty());
    }

    #[test]
    fn typed_exports_win_over_edl() {
        let found = vec![
            (m(SkipKind::Intro, 1_000, 31_000), SkipProvider::Plex, None),
            (m(SkipKind::Intro, 2_000, 32_000), SkipProvider::Edl, None),
            (
                m(SkipKind::Credits, 90_000, 99_000),
                SkipProvider::Edl,
                None,
            ),
        ];
        let segs = first_per_kind(found);
        assert_eq!(segs.len(), 2);
        assert_eq!(segs[0].provider, Some(SkipProvider::Plex));
        assert_eq!(segs[1].provider, Some(SkipProvider::Edl));
        assert!(segs.iter().all(|s| s.origin == SkipOrigin::Import));
    }
}
