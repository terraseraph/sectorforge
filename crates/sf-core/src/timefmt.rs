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
        assert_eq!(filetime_to_rfc3339(0), None);
        assert_eq!(filetime_to_rfc3339(u64::MAX), None);
    }
}
