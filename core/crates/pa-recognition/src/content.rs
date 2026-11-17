// SPDX-License-Identifier: MPL-2.0
//! What a media file holds (LIB-025, ADR-0099): the user never picks a
//! content type, the core decides it for every file and folder.
//!
//! - Video: a titled work (movie, show, anime: recognized as before) or the
//!   user's own footage from a phone or camera («Мои видео»: never sent to
//!   a provider). Signals: camera and phone file names (`IMG_`, `VID_`,
//!   `DSC`, `GOPR`, `PXL_`, `20240612_183000`…), camera folders (`DCIM`,
//!   `Camera`), the make and model a phone writes into the container, and
//!   against them what a release name says (year, season and episode,
//!   resolution, source, codec, release group).
//! - Audio: music or an audiobook. Signals: `.m4b`, chapters in a long
//!   file, the genre `Audiobook` or `Speech`, words such as «аудиокнига»,
//!   «читает», `audiobook`, `narrated by` in the path and the tags,
//!   «Глава»/`Chapter` numbering, long tracks across the folder.
//!
//! Files are decided per folder ([`classify_videos`], [`classify_audio`]):
//! the folder's other files say what a file with no signal of its own is.
//! Everything here is a pure function of its input; the catalog reads the
//! paths and tags, stores the verdicts and applies the user's «Это не то»
//! ([`ContentKind::flipped`]).

use crate::parse;

/// Version of the rules; stored with every verdict, so a newer build
/// decides the files again.
pub const CLASSIFIER_VERSION: u32 = 1;

/// What a media file holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ContentKind {
    /// A movie, show or anime: recognized and matched with providers.
    Video,
    /// The user's own footage from a phone or a camera: «Мои видео», never
    /// sent to a provider.
    OwnVideo,
    /// A music track.
    Music,
    /// A track of an audiobook.
    Audiobook,
}

impl ContentKind {
    /// The stored value: `video`, `own_video`, `music`, `audiobook`.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Video => "video",
            Self::OwnVideo => "own_video",
            Self::Music => "music",
            Self::Audiobook => "audiobook",
        }
    }

    /// From [`as_str`](Self::as_str).
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "video" => Some(Self::Video),
            "own_video" => Some(Self::OwnVideo),
            "music" => Some(Self::Music),
            "audiobook" => Some(Self::Audiobook),
            _ => None,
        }
    }

    /// An audio kind (music or audiobook).
    #[must_use]
    pub fn is_audio(self) -> bool {
        matches!(self, Self::Music | Self::Audiobook)
    }

    /// «Это не то»: the other kind of the same medium.
    #[must_use]
    pub fn flipped(self) -> Self {
        match self {
            Self::Video => Self::OwnVideo,
            Self::OwnVideo => Self::Video,
            Self::Music => Self::Audiobook,
            Self::Audiobook => Self::Music,
        }
    }
}

/// A video file as the classifier sees it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct VideoFile<'a> {
    /// Path relative to the source root (`/`-separated).
    pub path: &'a str,
    /// From the container probe, if known.
    pub duration_ms: Option<u64>,
    /// Container-level tags, keys in lowercase (`make`, `model`,
    /// `com.apple.quicktime.make`, `creation_time`…).
    pub tags: Vec<(&'a str, &'a str)>,
}

/// An audio file as the classifier sees it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AudioFile<'a> {
    /// Path relative to the source root (`/`-separated).
    pub path: &'a str,
    /// From the tags or the probe, if known.
    pub duration_ms: Option<u64>,
    /// Chapters in the file (probe).
    pub chapters: u32,
    /// Genre tags.
    pub genres: Vec<&'a str>,
    /// Title, album, artist, album artist, comment… as tagged: searched
    /// for audiobook words.
    pub texts: Vec<&'a str>,
}

/// The decision for one file.
#[derive(Debug, Clone, PartialEq)]
pub struct Verdict {
    /// What the file holds.
    pub kind: ContentKind,
    /// Evidence for the other kind of the medium is negative: own footage
    /// against a titled video, an audiobook against music.
    pub score: f32,
    /// Short names of the signals that decided, for the explanation.
    pub reasons: Vec<&'static str>,
}

/// A file is the "other" kind (own video, audiobook) at this score.
const THRESHOLD: f32 = 3.0;

/// Folder evidence counts for a file without strong evidence of its own.
const STRONG: f32 = 4.0;

// --- Video ---------------------------------------------------------------

/// Camera and phone file name prefixes followed by a digit (`IMG_1234`,
/// `VID_20240612`, `GOPR0042`, `PXL_2024…`, `DSC01234`, `MVI_0042`,
/// `DJI_0001`), lowercase.
const CAMERA_PREFIXES: &[&str] = &[
    "img", "vid", "mvi", "mov", "dsc", "dscf", "dscn", "dji", "gopr", "gp", "gx", "gh", "pxl",
    "mah", "wp", "sam", "hpim", "pict", "cimg", "p", "c", "clip", "video", "pano", "rec",
];

/// Screen recorders and apps whose file names carry a date or a counter
/// somewhere after the prefix (`Screen Recording 2024-06-12 at 18.30.00`,
/// `RPReplay_Final1718210000`), lowercase.
const RECORDER_PREFIXES: &[&str] = &[
    "screen recording",
    "screenrecording",
    "screenrecorder",
    "screen_recording",
    "запись экрана",
    "screencast",
    "rpreplay",
    "bandicam",
    "snapchat-",
    "signal-",
    "insta360",
    "trim.",
];

/// Folder names (lowercase) that hold own footage.
const CAMERA_FOLDERS: &[&str] = &[
    "dcim",
    "camera",
    "camera roll",
    "cameraroll",
    "camera uploads",
    "100apple",
    "100media",
    "100gopro",
    "100andro",
    "whatsapp video",
    "whatsapp videos",
    "whatsapp animated gifs",
    "screen recordings",
    "screenrecordings",
    "screenrecorder",
    "screenshots",
    "записи экрана",
    "видео с телефона",
    "с телефона",
    "телефон",
    "камера",
    "домашнее видео",
    "домашние видео",
    "home videos",
    "home video",
    "phone",
    "iphone",
    "android",
    "gopro",
    "dashcam",
    "регистратор",
    "видеорегистратор",
    "фото",
    "фотографии",
    "photos",
    "pictures",
    "изображения",
    "telegram video",
    "snapchat",
    "instagram",
    "inshot",
    "youcut",
];

/// Container tag keys a phone or camera writes (lowercase).
const CAMERA_TAGS: &[&str] = &[
    "make",
    "model",
    "com.apple.quicktime.make",
    "com.apple.quicktime.model",
    "com.apple.quicktime.software",
    "com.apple.quicktime.location.iso6709",
    "com.android.version",
    "com.android.capture.fps",
    "com.android.manufacturer",
    "com.android.model",
    "location",
    "location-eng",
    "firmware",
];

/// `true` if `stem` (lowercase) starts with a digit run that reads as a
/// camera timestamp: `20240612_183000`, `20240612-183000`,
/// `2024-06-12 18.30.00`, `2024-06-12_18-30-00`, `20240612183000`.
fn dated_name(stem: &str) -> bool {
    let b = stem.as_bytes();
    let digits = |from: usize, n: usize| {
        b.len() >= from + n && b[from..from + n].iter().all(u8::is_ascii_digit)
    };
    let plausible = |y: &[u8], m: &[u8], d: &[u8]| {
        let n = |s: &[u8]| s.iter().fold(0u32, |a, c| a * 10 + u32::from(c - b'0'));
        (1990..=2099).contains(&n(y)) && (1..=12).contains(&n(m)) && (1..=31).contains(&n(d))
    };
    // 20240612 then a separator and 6 digits, or 14 digits.
    if digits(0, 8) && plausible(&b[0..4], &b[4..6], &b[6..8]) {
        if digits(0, 14) {
            return true;
        }
        if b.len() >= 15 && matches!(b[8], b'_' | b'-' | b' ' | b'.') && digits(9, 6) {
            return true;
        }
        // WhatsApp: `20240612-wa0003`.
        if b.len() >= 11 && b[8] == b'-' && b[9..].starts_with(b"wa") {
            return true;
        }
    }
    // Dash cams: `2024_0612_183000_001`.
    if b.len() >= 16
        && digits(0, 4)
        && b[4] == b'_'
        && digits(5, 4)
        && b[9] == b'_'
        && digits(10, 6)
        && plausible(&b[0..4], &b[5..7], &b[7..9])
    {
        return true;
    }
    // 2024-06-12 18.30.00 (Dropbox, Google Takeout), 2024-06-12_18-30-00.
    if b.len() >= 19
        && digits(0, 4)
        && b[4] == b'-'
        && digits(5, 2)
        && b[7] == b'-'
        && digits(8, 2)
        && matches!(b[10], b' ' | b'_' | b't')
        && digits(11, 2)
        && matches!(b[13], b'.' | b'-' | b':' | b'_')
        && digits(14, 2)
        && matches!(b[16], b'.' | b'-' | b':' | b'_')
        && digits(17, 2)
        && plausible(&b[0..4], &b[5..7], &b[8..10])
    {
        return true;
    }
    false
}

/// `true` if `stem` (lowercase) is a camera or phone file name: a known
/// prefix, an optional separator and a digit, or a dated name after the
/// prefix (`vid_20240612_183000`, `pxl_20240612_183000123`).
fn camera_name(stem: &str) -> bool {
    if dated_name(stem) {
        return true;
    }
    if RECORDER_PREFIXES
        .iter()
        .any(|p| stem.starts_with(p) && stem[p.len()..].chars().any(|c| c.is_ascii_digit()))
    {
        return true;
    }
    // GoPro HERO5+: `gx010123`, `gh010123`; DJI: `dji_0001`.
    for prefix in CAMERA_PREFIXES {
        let Some(rest) = stem.strip_prefix(prefix) else {
            continue;
        };
        let rest = rest.trim_start_matches(['_', '-', ' ', '.']);
        if rest.is_empty() {
            continue;
        }
        if dated_name(rest) {
            return true;
        }
        // A short prefix needs at least four digits right after it
        // (`IMG_1234`, `P1010001`, `C0001`), so «Pi», «Clip of the day»,
        // «Video 2» or «Mov 1» are not camera names.
        let run = rest.bytes().take_while(u8::is_ascii_digit).count();
        let tail = &rest[run..];
        if run >= 4
            && (tail.is_empty()
                || tail.starts_with(['_', '-', ' ', '.', '~', '('])
                || tail.starts_with("wa"))
        {
            return true;
        }
    }
    false
}

fn file_name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

fn stem(name: &str) -> &str {
    match name.rfind('.') {
        Some(i) if i > 0 => &name[..i],
        _ => name,
    }
}

fn folders(path: &str) -> impl Iterator<Item = &str> {
    let mut parts: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    parts.pop();
    parts.into_iter()
}

/// Evidence of one video file alone: positive = own footage.
fn video_evidence(f: &VideoFile<'_>) -> (f32, bool, Vec<&'static str>) {
    let mut score = 0.0;
    let mut reasons = Vec::new();
    let name = file_name(f.path).to_lowercase();
    let stem = stem(&name);
    let camera = camera_name(stem);
    if camera {
        score += 6.0;
        reasons.push("camera_name");
    }
    let camera_folder = folders(f.path).any(|d| {
        let d = d.to_lowercase();
        CAMERA_FOLDERS.contains(&d.trim())
            || d.starts_with("dcim")
            // `100MEDIA`, `101GOPRO`, `102_PANA`: camera card folders.
            || (d.len() == 8
                && d.as_bytes()[..3].iter().all(u8::is_ascii_digit)
                && d.is_ascii())
    });
    if camera_folder {
        score += 3.0;
        reasons.push("camera_folder");
    }
    let tagged = f
        .tags
        .iter()
        .any(|(k, v)| !v.trim().is_empty() && CAMERA_TAGS.contains(&k.to_lowercase().as_str()));
    if tagged {
        // Releases do not carry a phone's make and model.
        score += 7.0;
        reasons.push("camera_tags");
    }
    // What a release name says. A camera name's digits are no episode.
    let p = parse::parse(f.path);
    let best = &p.best().parsed;
    let mut titled = 0.0;
    if !camera {
        if best.season.is_some() || !best.episodes.is_empty() || !best.absolute.is_empty() {
            titled += 4.0;
            reasons.push("episode");
        }
        if best.year.is_some() {
            titled += 3.0;
            reasons.push("year");
        }
        if best.date.is_some() && best.title.is_some() {
            // A daily show: `Show 2024-06-12 Guest`.
            titled += 2.0;
            reasons.push("aired_date");
        }
        if best.resolution.is_some()
            || best.source.is_some()
            || best.video_codec.is_some()
            || best.release_group.is_some()
            || best.edition.is_some()
            || best.hdr.is_some()
        {
            titled += 3.0;
            reasons.push("release_tags");
        }
    }
    score -= titled;
    (score, titled > 0.0, reasons)
}

/// Decides the video files of one folder (or any set of files that belong
/// together). A file with strong evidence of its own keeps it; one without
/// follows the folder: in a folder of phone clips, `Birthday.mp4` is a clip
/// too, and in a folder of release names it is a movie.
#[must_use]
pub fn classify_videos(files: &[VideoFile<'_>]) -> Vec<Verdict> {
    let evidence: Vec<_> = files.iter().map(video_evidence).collect();
    let own = evidence.iter().filter(|(s, _, _)| *s >= THRESHOLD).count();
    let titled = evidence.iter().filter(|(_, t, _)| *t).count();
    // Most of the folder is footage and no file looks like a release.
    let folder_own = own * 2 > files.len() && own > titled;
    evidence
        .into_iter()
        .map(|(score, has_titled, mut reasons)| {
            let mut score = score;
            if folder_own && score.abs() < STRONG && !has_titled {
                score += THRESHOLD;
                reasons.push("folder_of_clips");
            }
            let kind = if score >= THRESHOLD {
                ContentKind::OwnVideo
            } else {
                ContentKind::Video
            };
            Verdict {
                kind,
                score,
                reasons,
            }
        })
        .collect()
}

// --- Audio ---------------------------------------------------------------

/// Words of an audiobook in the path or the tags (lowercase).
const AUDIOBOOK_WORDS: &[&str] = &[
    "аудиокниг",
    "аудио книг",
    "аудиоспектакл",
    "радиоспектакл",
    "аудиорассказ",
    "читает",
    "читают",
    "чтец",
    "читал ",
    "читала ",
    "в исполнении",
    "audiobook",
    "audio book",
    "audio-book",
    "hörbuch",
    "horbuch",
    "hoerbuch",
    "livre audio",
    "audiolibro",
    "audiolivro",
    "narrated by",
    "narrator",
    "read by",
    "unabridged",
    "abridged",
    "radio drama",
    "radio play",
];

/// Genres of spoken word (lowercase, compared after trimming).
const SPEECH_GENRES: &[&str] = &[
    "audiobook",
    "audiobooks",
    "audio book",
    "audiobook & spoken word",
    "speech",
    "spoken word",
    "spoken",
    "spoken & audio",
    "books & spoken",
    "audio drama",
    "hörbuch",
    "hörspiel",
    "аудиокнига",
    "аудиокниги",
    "литература",
    "радиоспектакль",
    "аудиоспектакль",
    "speech/spoken word",
];

/// Words of a music release in a folder or file name (lowercase, whole
/// words).
const MUSIC_WORDS: &[&str] = &[
    "discography",
    "дискография",
    "flac",
    "alac",
    "kbps",
    "320",
    "vinyl",
    "lossless",
    "soundtrack",
    "ost",
    "remaster",
    "remastered",
    "single",
    "ep",
    "lp",
    "feat",
    "ft",
    "remix",
    "mix",
    "24bit",
    "16bit",
    "hi",
    "cd1",
    "cd2",
    "disc",
    "альбом",
    "album",
    "deluxe",
    "live",
];

/// Whole words of `s` (lowercase): runs of letters and digits.
fn words(s: &str) -> impl Iterator<Item = &str> {
    s.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty())
}

/// «Глава 3», «Chapter 03», «Часть 2», «Kapitel 4»: numbered parts of a
/// book (lowercase text).
fn chapter_numbering(text: &str) -> bool {
    const WORDS: &[&str] = &["глава", "chapter", "kapitel", "chapitre", "capítulo", "розділ"];
    WORDS.iter().any(|w| {
        text.match_indices(w).any(|(i, _)| {
            let rest = text[i + w.len()..].trim_start_matches([' ', '_', '-', '.']);
            rest.starts_with(|c: char| c.is_ascii_digit() || "ivxlc".contains(c))
        })
    })
}

/// Evidence of one audio file alone: positive = audiobook.
fn audio_evidence(f: &AudioFile<'_>) -> (f32, Vec<&'static str>) {
    let mut score = 0.0;
    let mut reasons = Vec::new();
    let path = f.path.to_lowercase();
    let name = file_name(&path);
    if name.ends_with(".m4b") || name.ends_with(".aax") || name.ends_with(".aa") {
        score += 6.0;
        reasons.push("m4b");
    }
    let long = f.duration_ms.is_some_and(|d| d >= 25 * 60_000);
    if f.chapters >= 3 && long {
        score += 4.0;
        reasons.push("chapters");
    }
    let genres: Vec<String> = f.genres.iter().map(|g| g.trim().to_lowercase()).collect();
    if genres.iter().any(|g| SPEECH_GENRES.contains(&g.as_str())) {
        score += 6.0;
        reasons.push("speech_genre");
    } else if !genres.is_empty() && !genres.iter().all(|g| g == "other" || g == "unknown") {
        // A music genre: rock, jazz, classical…
        score -= 2.0;
        reasons.push("music_genre");
    }
    let texts: Vec<String> = f.texts.iter().map(|t| t.to_lowercase()).collect();
    let all = || std::iter::once(path.as_str()).chain(texts.iter().map(String::as_str));
    if all().any(|t| AUDIOBOOK_WORDS.iter().any(|w| t.contains(w))) {
        score += 5.0;
        reasons.push("audiobook_words");
    }
    if all().any(chapter_numbering) {
        score += 2.5;
        reasons.push("chapter_numbering");
    }
    if long {
        score += 1.5;
        reasons.push("long_track");
    } else if f.duration_ms.is_some_and(|d| d < 9 * 60_000) {
        score -= 1.0;
        reasons.push("song_length");
    }
    // Without the extension: a FLAC book is no FLAC album.
    if words(stem(&path)).any(|w| MUSIC_WORDS.contains(&w)) {
        score -= 1.5;
        reasons.push("music_words");
    }
    (score, reasons)
}

/// Decides the audio files of one folder (an album or a book). A book is
/// decided as a whole: long tracks across the folder and the evidence of
/// its files add up, so the 40-minute part 7 without a tag of its own is
/// a part of the book too. A file with strong evidence of its own (an
/// `.m4b` in a folder of songs) keeps it.
#[must_use]
pub fn classify_audio(files: &[AudioFile<'_>]) -> Vec<Verdict> {
    if files.is_empty() {
        return Vec::new();
    }
    let evidence: Vec<_> = files.iter().map(audio_evidence).collect();
    // The folder speaks for the files without strong evidence: most files
    // strongly a book make it a book; otherwise the mean of the weak files
    // and the track lengths decide, so one `.m4b` among songs does not
    // turn the songs into a book.
    let strong_books = evidence.iter().filter(|(s, _)| *s >= STRONG).count();
    let weak: Vec<f32> = evidence
        .iter()
        .map(|(s, _)| *s)
        .filter(|s| s.abs() < STRONG)
        .collect();
    #[allow(clippy::cast_precision_loss)]
    let mean = if weak.is_empty() {
        0.0
    } else {
        weak.iter().sum::<f32>() / weak.len() as f32
    };
    // Lengths of the files the folder speaks for.
    let mut durations: Vec<u64> = files
        .iter()
        .zip(&evidence)
        .filter(|(_, (s, _))| s.abs() < STRONG)
        .filter_map(|(f, _)| f.duration_ms)
        .collect();
    durations.sort_unstable();
    let median = durations.get(durations.len() / 2).copied();
    let mut folder = mean;
    let mut folder_reasons = Vec::new();
    if strong_books * 2 > files.len() {
        folder = folder.max(THRESHOLD);
        folder_reasons.push("folder_of_book");
    }
    if median.is_some_and(|d| d >= 20 * 60_000) {
        folder += 2.0;
        folder_reasons.push("long_folder");
    }
    // Many tracks of the same length band, all numbered: a ripped book
    // (`01.mp3` … `87.mp3`, 5–15 minutes each) is music-length per track,
    // but no album has 40 tracks of 10 minutes.
    if durations.len() >= 25
        && median.is_some_and(|d| d >= 4 * 60_000)
        && durations.iter().map(|d| *d / 60_000).sum::<u64>() >= 300
    {
        folder += 3.0;
        folder_reasons.push("many_long_parts");
    }
    evidence
        .into_iter()
        .map(|(own, mut reasons)| {
            let score = if own.abs() >= STRONG {
                own
            } else {
                reasons.extend(folder_reasons.iter().copied());
                reasons.push("folder");
                folder.max(own)
            };
            let kind = if score >= THRESHOLD {
                ContentKind::Audiobook
            } else {
                ContentKind::Music
            };
            Verdict {
                kind,
                score,
                reasons,
            }
        })
        .collect()
}

/// The parent folder of `path` (`""` for a file at the root): the unit
/// [`classify_videos`] and [`classify_audio`] decide together.
#[must_use]
pub fn folder_of(path: &str) -> &str {
    let path = path.trim_end_matches('/');
    path.rfind('/').map_or("", |i| &path[..i])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn video(path: &str) -> ContentKind {
        classify_videos(&[VideoFile {
            path,
            ..VideoFile::default()
        }])[0]
            .kind
    }

    #[test]
    fn camera_names_are_own_videos() {
        for p in [
            "DCIM/Camera/VID_20240612_183000.mp4",
            "Phone/IMG_1234.MOV",
            "GoPro/GOPR0042.MP4",
            "GoPro/GX010123.MP4",
            "Pixel/PXL_20240612_183000123.mp4",
            "Sony/DSC01234.MP4",
            "20240612_183000.mp4",
            "2024-06-12 18.30.00.mov",
            "WhatsApp Video/VID-20240612-WA0003.mp4",
            "Canon/MVI_0042.MOV",
            "DJI_0001.MP4",
            "Screen Recording 2024-06-12 at 18.30.00.mov",
        ] {
            assert_eq!(video(p), ContentKind::OwnVideo, "{p}");
        }
    }

    #[test]
    fn release_names_are_titled_videos() {
        for p in [
            "Movies/Inception (2010)/Inception.2010.1080p.BluRay.x264.mkv",
            "Shows/Dark/Season 1/Dark.S01E01.mkv",
            "Anime/[SubsPlease] Frieren - 12 (1080p) [ABCD1234].mkv",
            "Pi (1998).mkv",
            "Video 2 (2004).mkv",
            "Clip of the Day/Clip.of.the.Day.S02E03.720p.mkv",
            "Daily/The Daily Show 2024-06-12 Guest.mkv",
            "Movies/Mov 1 (2019).mkv",
        ] {
            assert_eq!(video(p), ContentKind::Video, "{p}");
        }
    }

    #[test]
    fn camera_tags_make_footage() {
        let v = classify_videos(&[VideoFile {
            path: "Trip/Beach.mp4",
            duration_ms: Some(45_000),
            tags: vec![("com.apple.quicktime.make", "Apple")],
        }]);
        assert_eq!(v[0].kind, ContentKind::OwnVideo);
        assert!(v[0].reasons.contains(&"camera_tags"));
    }

    #[test]
    fn a_folder_of_clips_takes_its_unnamed_files() {
        let files = [
            VideoFile {
                path: "Trip/VID_20240612_183000.mp4",
                ..VideoFile::default()
            },
            VideoFile {
                path: "Trip/VID_20240612_184500.mp4",
                ..VideoFile::default()
            },
            VideoFile {
                path: "Trip/Birthday.mp4",
                ..VideoFile::default()
            },
        ];
        let v = classify_videos(&files);
        assert!(v.iter().all(|v| v.kind == ContentKind::OwnVideo), "{v:?}");
    }

    #[test]
    fn a_movie_among_clips_stays_a_movie() {
        let files = [
            VideoFile {
                path: "Downloads/VID_20240612_183000.mp4",
                ..VideoFile::default()
            },
            VideoFile {
                path: "Downloads/IMG_0042.MOV",
                ..VideoFile::default()
            },
            VideoFile {
                path: "Downloads/Arrival.2016.1080p.WEB-DL.mkv",
                ..VideoFile::default()
            },
        ];
        let v = classify_videos(&files);
        assert_eq!(v[2].kind, ContentKind::Video);
        assert_eq!(v[0].kind, ContentKind::OwnVideo);
    }

    fn audio(path: &str, minutes: u64, genres: &[&'static str]) -> AudioFile<'static> {
        AudioFile {
            path: Box::leak(path.to_owned().into_boxed_str()),
            duration_ms: Some(minutes * 60_000),
            chapters: 0,
            genres: genres.to_vec(),
            texts: Vec::new(),
        }
    }

    #[test]
    fn audiobooks_by_their_signals() {
        let m4b = classify_audio(&[audio("Books/Dune.m4b", 1260, &[])]);
        assert_eq!(m4b[0].kind, ContentKind::Audiobook);
        let genre = classify_audio(&[audio("A/01.mp3", 30, &["Audiobook"])]);
        assert_eq!(genre[0].kind, ContentKind::Audiobook);
        let words = classify_audio(&[audio(
            "Стругацкие - Пикник на обочине (читает Иванов)/01.mp3",
            12,
            &[],
        )]);
        assert_eq!(words[0].kind, ContentKind::Audiobook);
        let mut ch = audio("Books/Novel.m4a", 600, &[]);
        ch.chapters = 24;
        assert_eq!(classify_audio(&[ch])[0].kind, ContentKind::Audiobook);
    }

    #[test]
    fn albums_stay_music() {
        let files: Vec<_> = (1..=10)
            .map(|i| {
                audio(
                    &format!("Music/Artist/2019 - Album/{i:02} - Song.flac"),
                    4,
                    &["Rock"],
                )
            })
            .collect();
        assert!(
            classify_audio(&files)
                .iter()
                .all(|v| v.kind == ContentKind::Music)
        );
        // A long prog-rock suite with "Part" in its name is still music.
        let suite = classify_audio(&[audio(
            "Music/Band/1973 - Album/01 - Suite, Part I.flac",
            23,
            &["Progressive Rock"],
        )]);
        assert_eq!(suite[0].kind, ContentKind::Music);
    }

    #[test]
    fn a_folder_of_long_parts_is_a_book() {
        let files: Vec<_> = (1..=30)
            .map(|i| audio(&format!("Books/Author - Title/{i:02}.mp3"), 14, &[]))
            .collect();
        assert!(
            classify_audio(&files)
                .iter()
                .all(|v| v.kind == ContentKind::Audiobook)
        );
    }

    #[test]
    fn chapter_words() {
        assert!(chapter_numbering("глава 12"));
        assert!(chapter_numbering("chapter_03"));
        assert!(chapter_numbering("chapter iv"));
        assert!(!chapter_numbering("chapterhouse"));
    }

    #[test]
    fn kinds_round_trip_and_flip() {
        for k in [
            ContentKind::Video,
            ContentKind::OwnVideo,
            ContentKind::Music,
            ContentKind::Audiobook,
        ] {
            assert_eq!(ContentKind::parse(k.as_str()), Some(k));
            assert_eq!(k.flipped().flipped(), k);
            assert_eq!(k.flipped().is_audio(), k.is_audio());
        }
        assert_eq!(folder_of("a/b/c.mp4"), "a/b");
        assert_eq!(folder_of("c.mp4"), "");
    }
}
