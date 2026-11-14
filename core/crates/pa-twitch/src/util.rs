// SPDX-License-Identifier: MPL-2.0
//! Small helpers: URL encoding, image templates, VOD durations.

use std::time::Duration;

/// Percent-encodes a query component (RFC 3986 unreserved set kept as is).
pub(crate) fn encode_query_component(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char)
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

/// Fills a Helix image template: `{width}`/`{height}` (stream and VOD
/// thumbnails, box art) and `%{width}`/`%{height}` (VOD thumbnails use this
/// form).
///
/// ```
/// let url = pa_twitch::fill_image_template(
///     "https://static-cdn.jtvnw.net/ttv-boxart/33214-{width}x{height}.jpg",
///     285,
///     380,
/// );
/// assert_eq!(url, "https://static-cdn.jtvnw.net/ttv-boxart/33214-285x380.jpg");
/// ```
pub fn fill_image_template(template: &str, width: u32, height: u32) -> String {
    let (w, h) = (width.to_string(), height.to_string());
    template
        .replace("%{width}", &w)
        .replace("%{height}", &h)
        .replace("{width}", &w)
        .replace("{height}", &h)
}

/// Parses a Helix video duration such as `3h8m33s`, `52m4s` or `21s`.
///
/// ```
/// use std::time::Duration;
/// assert_eq!(
///     pa_twitch::parse_video_duration("1h2m3s"),
///     Some(Duration::from_secs(3723))
/// );
/// assert_eq!(pa_twitch::parse_video_duration("oops"), None);
/// ```
pub fn parse_video_duration(text: &str) -> Option<Duration> {
    let mut total: u64 = 0;
    let mut number: Option<u64> = None;
    let mut seen_unit = false;
    for ch in text.trim().chars() {
        if let Some(digit) = ch.to_digit(10) {
            number = Some(
                number
                    .unwrap_or(0)
                    .checked_mul(10)?
                    .checked_add(digit as u64)?,
            );
            continue;
        }
        let factor = match ch {
            'h' => 3600,
            'm' => 60,
            's' => 1,
            _ => return None,
        };
        total = total.checked_add(number.take()?.checked_mul(factor)?)?;
        seen_unit = true;
    }
    if number.is_some() || !seen_unit {
        return None;
    }
    Some(Duration::from_secs(total))
}

/// Parses an RFC 3339 timestamp as Helix and EventSub send it
/// (`2026-11-10T18:00:00Z`, optional fraction, `Z` or `±hh:mm`) into Unix
/// milliseconds.
///
/// ```
/// assert_eq!(
///     pa_twitch::parse_rfc3339_ms("1970-01-02T00:00:01.5Z"),
///     Some(86_401_500)
/// );
/// assert_eq!(
///     pa_twitch::parse_rfc3339_ms("2026-11-10T20:00:00+02:00"),
///     pa_twitch::parse_rfc3339_ms("2026-11-10T18:00:00Z")
/// );
/// assert_eq!(pa_twitch::parse_rfc3339_ms("yesterday"), None);
/// ```
pub fn parse_rfc3339_ms(text: &str) -> Option<i64> {
    let b = text.trim().as_bytes();
    if b.len() < 20
        || b[4] != b'-'
        || b[7] != b'-'
        || !matches!(b[10], b'T' | b't' | b' ')
        || b[13] != b':'
        || b[16] != b':'
    {
        return None;
    }
    let num = |from: usize, to: usize| -> Option<i64> {
        let part = b.get(from..to)?;
        if !part.iter().all(u8::is_ascii_digit) {
            return None;
        }
        std::str::from_utf8(part).ok()?.parse().ok()
    };
    let (year, month, day) = (num(0, 4)?, num(5, 7)?, num(8, 10)?);
    let (hour, minute, second) = (num(11, 13)?, num(14, 16)?, num(17, 19)?);
    if !(1..=12).contains(&month)
        || !(1..=31).contains(&day)
        || hour > 23
        || minute > 59
        || second > 60
    {
        return None;
    }
    let mut i = 19;
    let mut millis = 0i64;
    if b.get(i) == Some(&b'.') {
        i += 1;
        let start = i;
        while i < b.len() && b[i].is_ascii_digit() {
            if i - start < 3 {
                millis = millis * 10 + i64::from(b[i] - b'0');
            }
            i += 1;
        }
        if i == start {
            return None;
        }
        for _ in (i - start)..3 {
            millis *= 10;
        }
    }
    let offset_min = match b.get(i)? {
        b'Z' | b'z' if i + 1 == b.len() => 0,
        sign @ (b'+' | b'-') if i + 6 == b.len() && b[i + 3] == b':' => {
            let minutes = num(i + 1, i + 3)? * 60 + num(i + 4, i + 6)?;
            if *sign == b'+' { minutes } else { -minutes }
        }
        _ => return None,
    };
    // Days from the civil date (H. Hinnant's `days_from_civil`).
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (month + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    let secs = days * 86_400 + hour * 3600 + minute * 60 + second - offset_min * 60;
    Some(secs * 1000 + millis)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_reserved_characters() {
        assert_eq!(
            encode_query_component("a b&c=d/é"),
            "a%20b%26c%3Dd%2F%C3%A9"
        );
        assert_eq!(
            encode_query_component("Just_Chatting-1.~"),
            "Just_Chatting-1.~"
        );
    }

    #[test]
    fn durations() {
        assert_eq!(
            parse_video_duration("3h8m33s"),
            Some(Duration::from_secs(11313))
        );
        assert_eq!(
            parse_video_duration("52m4s"),
            Some(Duration::from_secs(3124))
        );
        assert_eq!(parse_video_duration("21s"), Some(Duration::from_secs(21)));
        assert_eq!(parse_video_duration(""), None);
        assert_eq!(parse_video_duration("12"), None);
        assert_eq!(parse_video_duration("1h2"), None);
        assert_eq!(parse_video_duration("h"), None);
    }

    #[test]
    fn rfc3339() {
        assert_eq!(parse_rfc3339_ms("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(
            parse_rfc3339_ms("2000-03-01T00:00:00Z"),
            Some(951_868_800_000)
        );
        assert_eq!(
            parse_rfc3339_ms("2026-11-10T18:00:00.123456Z"),
            Some(parse_rfc3339_ms("2026-11-10T18:00:00Z").unwrap() + 123)
        );
        assert_eq!(
            parse_rfc3339_ms("2026-11-10T13:00:00-05:00"),
            parse_rfc3339_ms("2026-11-10T18:00:00Z")
        );
        assert_eq!(parse_rfc3339_ms(""), None);
        assert_eq!(parse_rfc3339_ms("2026-11-10T18:00:00"), None);
        assert_eq!(parse_rfc3339_ms("2026-13-10T18:00:00Z"), None);
        assert_eq!(parse_rfc3339_ms("2026-11-10T18:00:00.Z"), None);
    }

    #[test]
    fn vod_thumbnail_template() {
        assert_eq!(
            fill_image_template("https://x/thumb/%{width}x%{height}.jpg", 320, 180),
            "https://x/thumb/320x180.jpg"
        );
    }
}
