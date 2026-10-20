//! Times as text for file names.

use fusion_domain::Timestamp;

/// A UTC time as `YYYYMMDDTHHMMSSZ`, which sorts in time order.
pub fn utc_stamp(time: Timestamp) -> String {
    let seconds = time.unix_nanos().div_euclid(1_000_000_000);
    let days = seconds.div_euclid(86_400);
    let of_day = seconds.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    format!(
        "{year:04}{month:02}{day:02}T{:02}{:02}{:02}Z",
        of_day / 3600,
        of_day % 3600 / 60,
        of_day % 60
    )
}

/// The calendar date of a day number, counting from 1970-01-01 (Howard
/// Hinnant's `civil_from_days`).
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(seconds: i64) -> Timestamp {
        Timestamp::from_unix_nanos(seconds * 1_000_000_000)
    }

    #[test]
    fn times_print_as_utc() {
        assert_eq!(utc_stamp(Timestamp::UNIX_EPOCH), "19700101T000000Z");
        // 2026-10-20T16:43:18Z
        assert_eq!(utc_stamp(at(1_792_514_598)), "20261020T164318Z");
        // A leap day and the last second of a year.
        assert_eq!(utc_stamp(at(951_782_400)), "20000229T000000Z");
        assert_eq!(utc_stamp(at(1_735_689_599)), "20241231T235959Z");
        // Before 1970.
        assert_eq!(utc_stamp(at(-1)), "19691231T235959Z");
        // March 1st, where the calendar's year starts in this algorithm.
        assert_eq!(utc_stamp(at(1_709_251_200)), "20240301T000000Z");
    }

    fn is_leap(year: i64) -> bool {
        year % 4 == 0 && (year % 100 != 0 || year % 400 == 0)
    }

    /// Every day from 1960 to 2130, against plain day counting.
    #[test]
    fn every_day_matches_a_plain_count() {
        let month_days = |year, month| match month {
            2 if is_leap(year) => 29,
            2 => 28,
            4 | 6 | 9 | 11 => 30,
            _ => 31,
        };
        // 1960-01-01 is 3653 days before 1970-01-01.
        let mut days = -3653i64;
        for year in 1960..2130 {
            for month in 1..=12 {
                for day in 1..=month_days(year, month) {
                    assert_eq!(civil_from_days(days), (year, month, day), "day {days}");
                    days += 1;
                }
            }
        }
    }
}
