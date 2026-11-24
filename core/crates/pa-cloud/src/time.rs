// SPDX-License-Identifier: MPL-2.0
//! RFC 3339 timestamps of the cloud APIs (`2024-05-01T10:20:30.123Z`) to
//! Unix milliseconds, and the wall clock.

/// Unix milliseconds of an RFC 3339 timestamp; `None` if malformed.
pub(crate) fn rfc3339_ms(s: &str) -> Option<i64> {
    let s = s.trim();
    let b = s.as_bytes();
    if b.len() < 20 || b[4] != b'-' || b[7] != b'-' || !matches!(b[10], b'T' | b't' | b' ') {
        return None;
    }
    let num = |r: std::ops::Range<usize>| s.get(r)?.parse::<i64>().ok();
    let (y, mo, d) = (num(0..4)?, num(5..7)?, num(8..10)?);
    let (h, mi, sec) = (num(11..13)?, num(14..16)?, num(17..19)?);
    if !(1..=12).contains(&mo) || !(1..=31).contains(&d) || h > 23 || mi > 59 || sec > 60 {
        return None;
    }
    let mut rest = &s[19..];
    let mut ms = 0i64;
    if let Some(frac) = rest.strip_prefix('.') {
        let digits = frac.bytes().take_while(u8::is_ascii_digit).count();
        if digits == 0 {
            return None;
        }
        let mut f: String = frac[..digits].chars().take(3).collect();
        while f.len() < 3 {
            f.push('0');
        }
        ms = f.parse().ok()?;
        rest = &frac[digits..];
    }
    let offset_min = match rest {
        "Z" | "z" => 0,
        tz if tz.len() == 6 && (tz.starts_with('+') || tz.starts_with('-')) => {
            let oh: i64 = tz.get(1..3)?.parse().ok()?;
            let om: i64 = tz.get(4..6)?.parse().ok()?;
            let m = oh * 60 + om;
            if tz.starts_with('-') { -m } else { m }
        }
        _ => return None,
    };
    let days = days_from_civil(y, mo, d);
    let secs = days * 86_400 + h * 3600 + mi * 60 + sec - offset_min * 60;
    Some(secs * 1000 + ms)
}

/// Days since 1970-01-01 of a proleptic Gregorian date (H. Hinnant).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Wall clock, Unix seconds.
pub(crate) fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_timestamps() {
        assert_eq!(rfc3339_ms("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(rfc3339_ms("2000-03-01T00:00:00.5Z"), Some(951_868_800_500));
        assert_eq!(
            rfc3339_ms("2024-05-01T12:00:00+02:00"),
            rfc3339_ms("2024-05-01T10:00:00Z")
        );
        assert_eq!(
            rfc3339_ms("2024-05-01T10:00:00.1234567Z"),
            Some(rfc3339_ms("2024-05-01T10:00:00Z").unwrap() + 123)
        );
        assert_eq!(rfc3339_ms("yesterday"), None);
        assert_eq!(rfc3339_ms("2024-13-01T10:00:00Z"), None);
    }
}
