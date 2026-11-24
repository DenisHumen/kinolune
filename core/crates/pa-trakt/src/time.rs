// SPDX-License-Identifier: MPL-2.0
//! Trakt timestamps (`2014-09-01T09:10:11.000Z`, UTC) to and from Unix
//! milliseconds, without a date library.

/// Days since 1970-01-01 of a civil date (H. Hinnant's algorithm).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Civil date of days since 1970-01-01.
fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// Unix milliseconds of an RFC 3339 UTC time as Trakt writes it
/// (`2014-09-01T09:10:11.000Z`, fractions and `+00:00` accepted). `None`
/// if it does not parse.
pub fn parse_ms(text: &str) -> Option<i64> {
    let t = text.trim();
    let num = |s: &str| s.parse::<i64>().ok();
    let (date, rest) = t.split_once('T')?;
    let mut d = date.split('-');
    let (y, m, day) = (num(d.next()?)?, num(d.next()?)?, num(d.next()?)?);
    let rest = rest
        .strip_suffix('Z')
        .or_else(|| rest.strip_suffix("+00:00"))?;
    let (hms, frac) = rest.split_once('.').unwrap_or((rest, ""));
    let mut p = hms.split(':');
    let (h, mi, s) = (num(p.next()?)?, num(p.next()?)?, num(p.next()?)?);
    if !(1..=12).contains(&m) || !(1..=31).contains(&day) || h > 23 || mi > 59 || s > 60 {
        return None;
    }
    let ms = if frac.is_empty() {
        0
    } else {
        let f: String = frac.chars().chain("000".chars()).take(3).collect();
        num(&f)?
    };
    Some(((days_from_civil(y, m, day) * 24 + h) * 60 + mi) * 60_000 + s * 1000 + ms)
}

/// `2014-09-01T09:10:11.000Z` of Unix milliseconds.
pub fn format_ms(ms: i64) -> String {
    let days = ms.div_euclid(86_400_000);
    let rest = ms.rem_euclid(86_400_000);
    let (y, m, d) = civil_from_days(days);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}.{:03}Z",
        rest / 3_600_000,
        rest / 60_000 % 60,
        rest / 1000 % 60,
        rest % 1000
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let ms = parse_ms("2014-09-01T09:10:11.000Z").unwrap();
        assert_eq!(ms, 1_409_562_611_000);
        assert_eq!(format_ms(ms), "2014-09-01T09:10:11.000Z");
        assert_eq!(parse_ms("2024-02-29T23:59:59.5Z"), Some(1_709_251_199_500));
        assert_eq!(format_ms(0), "1970-01-01T00:00:00.000Z");
        assert_eq!(
            parse_ms("2020-01-01T00:00:00+00:00"),
            Some(1_577_836_800_000)
        );
        assert_eq!(parse_ms("yesterday"), None);
        assert_eq!(parse_ms("2020-13-01T00:00:00Z"), None);
    }
}
