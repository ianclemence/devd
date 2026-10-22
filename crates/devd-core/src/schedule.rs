//! When a schedule is next due.
//!
//! Three shapes, written the way a person says them:
//!
//! * `every 15m`, `every 6h`, `every 2d`: a fixed interval after the last run.
//! * `daily 09:30`: every day at a local time.
//! * `weekdays 09:30`, `weekends 10:00`, `mon,wed,fri 18:00`: on those days.
//!
//! Times are the machine's local time, because "every morning at nine" means the
//! person's morning. No cron syntax: five numeric fields are easy to get wrong and
//! nobody reads them back.

use chrono::{DateTime, Datelike, Duration, Local, NaiveTime, TimeZone, Weekday};

use crate::error::{DevdError, Result};

/// A parsed schedule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Spec {
    /// A fixed interval.
    Every(Duration),
    /// A local time on certain days.
    At {
        /// The days it is due on.
        days: Vec<Weekday>,
        /// The local time.
        time: NaiveTime,
    },
}

/// The shortest interval a schedule may have. A run of the team costs real money
/// in model calls; nothing sensible runs more often than this.
pub const MIN_INTERVAL_MINUTES: i64 = 5;

fn day(name: &str) -> Option<Weekday> {
    Some(match name {
        "mon" | "monday" => Weekday::Mon,
        "tue" | "tuesday" => Weekday::Tue,
        "wed" | "wednesday" => Weekday::Wed,
        "thu" | "thursday" => Weekday::Thu,
        "fri" | "friday" => Weekday::Fri,
        "sat" | "saturday" => Weekday::Sat,
        "sun" | "sunday" => Weekday::Sun,
        _ => return None,
    })
}

/// Read a schedule written by a person.
pub fn parse(text: &str) -> Result<Spec> {
    let bad = |why: &str| {
        DevdError::invalid(format!(
            "`{text}` is not a schedule: {why}. Use `every 30m`, `every 6h`, `daily 09:30`, `weekdays 09:30` or `mon,wed 18:00`"
        ))
    };
    let lower = text.trim().to_ascii_lowercase();
    let mut words = lower.split_whitespace();
    let (Some(first), Some(second), None) = (words.next(), words.next(), words.next()) else {
        return Err(bad("it needs two words"));
    };
    if first == "every" {
        let split = second.char_indices().last().map(|(i, _)| i).unwrap_or(0);
        let (digits, unit) = second.split_at(split);
        let n: i64 = digits.parse().map_err(|_| bad("the interval needs a number, like 30m"))?;
        if n <= 0 {
            return Err(bad("the interval must be more than zero"));
        }
        let interval = match unit {
            "m" => Duration::minutes(n),
            "h" => Duration::hours(n),
            "d" => Duration::days(n),
            _ => return Err(bad("the interval ends in m, h or d")),
        };
        if interval < Duration::minutes(MIN_INTERVAL_MINUTES) {
            return Err(bad(&format!("the shortest interval is {MIN_INTERVAL_MINUTES} minutes")));
        }
        return Ok(Spec::Every(interval));
    }
    let days: Vec<Weekday> = match first {
        "daily" => vec![Weekday::Mon, Weekday::Tue, Weekday::Wed, Weekday::Thu, Weekday::Fri, Weekday::Sat, Weekday::Sun],
        "weekdays" => vec![Weekday::Mon, Weekday::Tue, Weekday::Wed, Weekday::Thu, Weekday::Fri],
        "weekends" => vec![Weekday::Sat, Weekday::Sun],
        list => list.split(',').map(|d| day(d).ok_or_else(|| bad(&format!("`{d}` is not a day")))).collect::<Result<Vec<_>>>()?,
    };
    let time = NaiveTime::parse_from_str(second, "%H:%M").map_err(|_| bad("the time is HH:MM, like 09:30"))?;
    Ok(Spec::At { days, time })
}

/// The first moment after `from` that the schedule is due. For an interval that is
/// `from` plus the interval, so a run that took an hour does not make the next one
/// come early.
pub fn next_after(spec: &Spec, from: DateTime<Local>) -> Option<DateTime<Local>> {
    match spec {
        Spec::Every(interval) => Some(from + *interval),
        Spec::At { days, time } => {
            for offset in 0..=8 {
                let date = (from + Duration::days(offset)).date_naive();
                if !days.contains(&date.weekday()) {
                    continue;
                }
                // A local time that does not exist (a clock change) is skipped to the next day.
                let Some(candidate) = Local.from_local_datetime(&date.and_time(*time)).earliest() else { continue };
                if candidate > from {
                    return Some(candidate);
                }
            }
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(y: i32, m: u32, d: u32, h: u32, min: u32) -> DateTime<Local> {
        Local.with_ymd_and_hms(y, m, d, h, min, 0).earliest().unwrap()
    }

    #[test]
    fn schedules_are_read_the_way_a_person_writes_them() {
        assert_eq!(parse("every 30m").unwrap(), Spec::Every(Duration::minutes(30)));
        assert_eq!(parse("Every 6H").unwrap(), Spec::Every(Duration::hours(6)));
        assert_eq!(parse("every 2d").unwrap(), Spec::Every(Duration::days(2)));
        match parse("weekdays 09:30").unwrap() {
            Spec::At { days, time } => {
                assert_eq!(days.len(), 5);
                assert_eq!(time, NaiveTime::from_hms_opt(9, 30, 0).unwrap());
            }
            other => panic!("{other:?}"),
        }
        match parse("mon,wed,fri 18:00").unwrap() {
            Spec::At { days, .. } => assert_eq!(days, vec![Weekday::Mon, Weekday::Wed, Weekday::Fri]),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn nonsense_and_runaway_schedules_are_refused_with_the_forms_that_work() {
        for bad in ["", "every", "every 0m", "every 1m", "every 4m", "every ten minutes", "every 5x", "daily 25:00", "daily 9", "funday 09:00", "mon,xyz 09:00", "daily 09:00 now"] {
            let e = parse(bad).unwrap_err().to_string();
            assert!(e.contains("weekdays 09:30"), "{bad}: {e}");
        }
        assert!(parse("every 5m").is_ok());
    }

    #[test]
    fn an_interval_counts_from_the_run_not_from_the_clock() {
        let from = at(2026, 10, 22, 10, 17);
        assert_eq!(next_after(&Spec::Every(Duration::hours(2)), from), Some(at(2026, 10, 22, 12, 17)));
    }

    #[test]
    fn a_time_of_day_is_the_next_one_on_a_listed_day() {
        let spec = parse("weekdays 09:30").unwrap();
        // 2026-10-22 is a Thursday.
        assert_eq!(next_after(&spec, at(2026, 10, 22, 8, 0)), Some(at(2026, 10, 22, 9, 30)));
        assert_eq!(next_after(&spec, at(2026, 10, 22, 9, 30)), Some(at(2026, 10, 23, 9, 30)), "not the same minute again");
        // Friday evening -> Monday morning.
        assert_eq!(next_after(&spec, at(2026, 10, 23, 18, 0)), Some(at(2026, 10, 26, 9, 30)));
        let weekend = parse("weekends 10:00").unwrap();
        assert_eq!(next_after(&weekend, at(2026, 10, 22, 12, 0)), Some(at(2026, 10, 24, 10, 0)));
    }
}
