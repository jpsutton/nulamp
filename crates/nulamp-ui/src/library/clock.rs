//! Local date and time of a unix timestamp, for the "Played Last" column.

/// `YYYY-MM-DD HH:MM` in the local time zone (UTC where the platform offers no zone
/// lookup).
pub fn format_local(secs: i64) -> String {
    let (y, mo, d, h, mi) = local_parts(secs);
    format!("{y:04}-{mo:02}-{d:02} {h:02}:{mi:02}")
}

#[cfg(unix)]
fn local_parts(secs: i64) -> (i64, u32, u32, u32, u32) {
    // SAFETY: `localtime_r` writes only to the `tm` we pass, and `t` outlives the call.
    unsafe {
        let t: libc::time_t = secs as libc::time_t;
        let mut tm: libc::tm = std::mem::zeroed();
        if !libc::localtime_r(&t, &mut tm).is_null() {
            return (
                i64::from(tm.tm_year) + 1900,
                (tm.tm_mon + 1) as u32,
                tm.tm_mday as u32,
                tm.tm_hour as u32,
                tm.tm_min as u32,
            );
        }
    }
    utc_parts(secs)
}

#[cfg(not(unix))]
fn local_parts(secs: i64) -> (i64, u32, u32, u32, u32) {
    utc_parts(secs)
}

/// Civil date and time of a unix timestamp in UTC.
fn utc_parts(secs: i64) -> (i64, u32, u32, u32, u32) {
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    // Days since 0000-03-01 (Howard Hinnant's civil-from-days).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y, m, d, (rem / 3600) as u32, (rem % 3600 / 60) as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utc_dates() {
        assert_eq!(utc_parts(0), (1970, 1, 1, 0, 0));
        assert_eq!(
            utc_parts(951_782_400 + 3600 * 13 + 60 * 5),
            (2000, 2, 29, 13, 5)
        );
        assert_eq!(utc_parts(-86_400), (1969, 12, 31, 0, 0));
        assert_eq!(utc_parts(1_760_227_200), (2025, 10, 12, 0, 0));
    }

    #[test]
    fn local_format_has_fixed_shape() {
        let s = format_local(1_760_227_200);
        assert_eq!(s.len(), 16);
        assert!(s.starts_with("2025-10-1"));
    }
}
