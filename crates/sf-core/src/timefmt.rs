//! Windows FILETIME → RFC 3339 UTC, without pulling in a date crate.

const FILETIME_UNIX_EPOCH: u64 = 116_444_736_000_000_000;

/// `None` for zero or implausible values (before 1980 or after 2200), which
/// on a damaged disk mean "garbage", not a real date.
pub fn filetime_to_rfc3339(ft: u64) -> Option<String> {
    let secs_since_unix = (ft.checked_sub(FILETIME_UNIX_EPOCH)? / 10_000_000) as i64;
    if !(315_532_800..7_258_118_400).contains(&secs_since_unix) {
        return None;
    }
    let days = secs_since_unix.div_euclid(86_400);
    let rem = secs_since_unix.rem_euclid(86_400);
    let (y, m, d) = civil_from_days(days);
    Some(format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        rem / 3600,
        (rem % 3600) / 60,
        rem % 60
    ))
}

/// Unix seconds → FILETIME (0 for times before 1601).
pub fn unix_to_filetime(secs: i64) -> u64 {
    (secs.saturating_mul(10_000_000)).saturating_add(FILETIME_UNIX_EPOCH as i64).max(0) as u64
}

/// A FAT/exFAT date and time (`centis` = hundredths of a second added to
/// the 2-second resolution) as FILETIME, shifted back by `utc_offset_min`.
/// FAT stores local time with no zone, so callers pass 0 there and the
/// result is local time labelled as UTC. 0 for an invalid date.
pub fn dos_to_filetime(date: u16, time: u16, centis: u8, utc_offset_min: i32) -> u64 {
    let (y, m, d) = (1980 + (date >> 9) as i64, ((date >> 5) & 15) as u32, (date & 31) as u32);
    let (h, mi, s) = ((time >> 11) as i64, ((time >> 5) & 63) as i64, ((time & 31) * 2) as i64);
    if !(1..=12).contains(&m) || d == 0 || h > 23 || mi > 59 || s > 59 {
        return 0;
    }
    let secs = days_from_civil(y, m, d) * 86_400 + h * 3600 + mi * 60 + s - utc_offset_min as i64 * 60;
    unix_to_filetime(secs) + centis.min(199) as u64 * 100_000
}

/// Howard Hinnant's (year, month, day) → days-since-1970.
fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let mp = (m as i64 + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Howard Hinnant's days-since-1970 → (year, month, day).
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = yoe + era * 400 + if m <= 2 { 1 } else { 0 };
    (y, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_known_filetimes() {
        // 2024-03-11T09:12:00Z
        let unix = 1_710_148_320u64;
        assert_eq!(
            filetime_to_rfc3339(unix * 10_000_000 + FILETIME_UNIX_EPOCH).as_deref(),
            Some("2024-03-11T09:12:00Z")
        );
        // 2000-02-29T23:59:59Z (leap day)
        let unix = 951_868_799u64;
        assert_eq!(
            filetime_to_rfc3339(unix * 10_000_000 + FILETIME_UNIX_EPOCH).as_deref(),
            Some("2000-02-29T23:59:59Z")
        );
        // FAT: 2024-03-11 09:12:00, 2024-01-31 23:59:58 at UTC+10.
        let date = (2024 - 1980) << 9 | 3 << 5 | 11;
        assert_eq!(filetime_to_rfc3339(dos_to_filetime(date, 9 << 11 | 12 << 5, 0, 0)).as_deref(), Some("2024-03-11T09:12:00Z"));
        let date = (2024 - 1980) << 9 | 1 << 5 | 31;
        let ft = dos_to_filetime(date, 23 << 11 | 59 << 5 | 29, 0, 600);
        assert_eq!(filetime_to_rfc3339(ft).as_deref(), Some("2024-01-31T13:59:58Z"));
        assert_eq!(dos_to_filetime(0, 0, 0, 0), 0);
        assert_eq!(filetime_to_rfc3339(unix_to_filetime(1_710_148_320)).as_deref(), Some("2024-03-11T09:12:00Z"));
        assert_eq!(filetime_to_rfc3339(0), None);
        assert_eq!(filetime_to_rfc3339(u64::MAX), None);
    }
}
