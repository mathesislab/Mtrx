//! Reading and describing times: those messages are scheduled for, and
//! the ends of mutes.

use std::time::SystemTime;

use chrono::{DateTime, Datelike, Days, Duration, Local, NaiveDate, NaiveTime, TimeZone, Weekday};

/// Time of day used when only a day is given ("tomorrow", "monday").
const DEFAULT_TIME: NaiveTime = NaiveTime::from_hms_opt(9, 0, 0).unwrap();

/// "Tomorrow morning", for a room muted or notifications paused until
/// then: 08:00 local.
pub const MORNING: NaiveTime = NaiveTime::from_hms_opt(8, 0, 0).unwrap();
/// Before this, "tomorrow morning" is today's: the night is not over.
const NIGHT_ENDS: NaiveTime = NaiveTime::from_hms_opt(5, 0, 0).unwrap();

/// Read a send time typed by the user, relative to `now`:
///
/// - `in 30m`, `2h`, `1h30m`, `3d`: from now
/// - `17:30`: today, or tomorrow if that has passed
/// - `tomorrow`, `tomorrow 8:00`, `mon`, `friday 14:00`: that day (09:00 by default)
/// - `2026-11-02`, `2026-11-02 10:00`
pub fn parse_send_time(input: &str, now: SystemTime) -> Option<SystemTime> {
    let now = DateTime::<Local>::from(now);
    let input = input.trim().to_lowercase();
    let input = input.strip_prefix("in ").unwrap_or(&input).trim();
    if let Some(offset) = parse_offset(input) {
        return Some((now + offset).into());
    }

    let (day, time) = match input.split_once(' ') {
        Some((day, time)) => (day, Some(parse_clock(time.trim())?)),
        None => match parse_clock(input) {
            Some(time) => {
                let today = local(now.date_naive(), time)?;
                let at = if today > now {
                    today
                } else {
                    local(now.date_naive().succ_opt()?, time)?
                };
                return Some(at.into());
            }
            None => (input, None),
        },
    };
    let date = parse_day(day, now)?;
    Some(local(date, time.unwrap_or(DEFAULT_TIME))?.into())
}

/// A short description of `at` for lists, e.g. "today 17:30",
/// "tomorrow 09:00", "Mon 3 Nov 09:00".
pub fn describe_send_time(at: SystemTime, now: SystemTime) -> String {
    let (at, now) = (DateTime::<Local>::from(at), DateTime::<Local>::from(now));
    let time = at.format("%H:%M");
    let days = (at.date_naive() - now.date_naive()).num_days();
    match days {
        0 => format!("today {time}"),
        1 => format!("tomorrow {time}"),
        -1 => format!("yesterday {time}"),
        2..7 => format!("{} {time}", at.format("%a")),
        _ if at.year() == now.year() => at.format("%a %-d %b %H:%M").to_string(),
        _ => at.format("%a %-d %b %Y %H:%M").to_string(),
    }
}

/// [`MORNING`] local: today's before 05:00, else tomorrow's.
pub(crate) fn tomorrow_morning(now: SystemTime) -> SystemTime {
    let now = DateTime::<Local>::from(now);
    let today = now.date_naive();
    let date = if now.time() < NIGHT_ENDS {
        today
    } else {
        today.succ_opt().unwrap_or(today)
    };
    local_at(date, MORNING).map_or_else(|| (now + Duration::hours(12)).into(), Into::into)
}

/// "Tomorrow morning" as a menu says it at `now`: "tomorrow, 08:00", or
/// "08:00" before 05:00.
pub(crate) fn morning_words(now: SystemTime) -> String {
    let at = tomorrow_morning(now);
    let (at, now) = (DateTime::<Local>::from(at), DateTime::<Local>::from(now));
    let time = at.format("%H:%M");
    if at.date_naive() == now.date_naive() {
        time.to_string()
    } else {
        format!("tomorrow, {time}")
    }
}

/// When something ends, in words, e.g. "14:00" (today), "tomorrow 08:00",
/// "Fri 08:00", "Mon 3 Nov 08:00".
pub(crate) fn describe_until(at: SystemTime, now: SystemTime) -> String {
    let (at, now) = (DateTime::<Local>::from(at), DateTime::<Local>::from(now));
    let time = at.format("%H:%M");
    match (at.date_naive() - now.date_naive()).num_days() {
        ..=0 => time.to_string(),
        1 => format!("tomorrow {time}"),
        2..7 => format!("{} {time}", at.format("%a")),
        _ if at.year() == now.year() => at.format("%a %-d %b %H:%M").to_string(),
        _ => at.format("%a %-d %b %Y %H:%M").to_string(),
    }
}

/// `time` on `date`, local: the earlier when clocks go back, the first
/// time that exists when they go forward.
pub(crate) fn local_at(date: NaiveDate, time: NaiveTime) -> Option<DateTime<Local>> {
    let at = date.and_time(time);
    (0..=24 * 60)
        .map(|minutes| at + Duration::minutes(minutes))
        .find_map(|at| Local.from_local_datetime(&at).earliest())
}

/// `30m`, `2h`, `1h30m`, `3d`.
fn parse_offset(input: &str) -> Option<Duration> {
    let mut total = Duration::zero();
    let mut number = String::new();
    for c in input.chars() {
        if c.is_ascii_digit() {
            number.push(c);
            continue;
        }
        let n: i64 = number.parse().ok()?;
        number.clear();
        total += match c {
            'm' => Duration::minutes(n),
            'h' => Duration::hours(n),
            'd' => Duration::days(n),
            _ => return None,
        };
    }
    (number.is_empty() && total > Duration::zero()).then_some(total)
}

/// `9:00`, `17:30`.
fn parse_clock(input: &str) -> Option<NaiveTime> {
    NaiveTime::parse_from_str(input, "%H:%M").ok()
}

fn parse_day(input: &str, now: DateTime<Local>) -> Option<NaiveDate> {
    let today = now.date_naive();
    match input {
        "today" => Some(today),
        "tomorrow" => today.succ_opt(),
        _ => match input.parse::<Weekday>() {
            // The next such day, a week ahead if it is today.
            Ok(weekday) => {
                let ahead = match weekday.days_since(today.weekday()) {
                    0 => 7,
                    days => days,
                };
                today.checked_add_days(Days::new(ahead.into()))
            }
            Err(_) => NaiveDate::parse_from_str(input, "%Y-%m-%d").ok(),
        },
    }
}

fn local(date: NaiveDate, time: NaiveTime) -> Option<DateTime<Local>> {
    // `earliest` resolves times that occur twice when clocks go back.
    Local.from_local_datetime(&date.and_time(time)).earliest()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(s: &str) -> SystemTime {
        let naive = chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M").unwrap();
        Local.from_local_datetime(&naive).unwrap().into()
    }

    // Wednesday.
    const NOW: &str = "2026-10-21 14:00";

    #[test]
    fn parses_send_times() {
        let now = at(NOW);
        let cases = [
            ("in 30m", "2026-10-21 14:30"),
            ("2h", "2026-10-21 16:00"),
            ("1h30m", "2026-10-21 15:30"),
            ("in 2d", "2026-10-23 14:00"),
            ("17:30", "2026-10-21 17:30"),
            ("9:15", "2026-10-22 09:15"),
            ("tomorrow", "2026-10-22 09:00"),
            ("Tomorrow 8:00", "2026-10-22 08:00"),
            ("fri", "2026-10-23 09:00"),
            ("monday 10:30", "2026-10-26 10:30"),
            ("wed", "2026-10-28 09:00"),
            ("2026-11-02", "2026-11-02 09:00"),
            ("2026-11-02 10:00", "2026-11-02 10:00"),
        ];
        for (input, expected) in cases {
            assert_eq!(parse_send_time(input, now), Some(at(expected)), "{input}");
        }
        for input in ["", "soon", "in", "5x", "25:00", "tomorrow noon", "0m"] {
            assert_eq!(parse_send_time(input, now), None, "{input}");
        }
    }

    #[test]
    fn tomorrow_morning_is_todays_before_five() {
        assert_eq!(
            tomorrow_morning(at("2026-10-21 04:59")),
            at("2026-10-21 08:00")
        );
        assert_eq!(
            tomorrow_morning(at("2026-10-21 05:00")),
            at("2026-10-22 08:00")
        );
        assert_eq!(
            tomorrow_morning(at("2026-10-21 23:30")),
            at("2026-10-22 08:00")
        );
        assert_eq!(morning_words(at("2026-10-21 04:59")), "08:00");
        assert_eq!(morning_words(at(NOW)), "tomorrow, 08:00");
    }

    #[test]
    fn describes_ends() {
        let now = at(NOW);
        let cases = [
            ("2026-10-21 17:30", "17:30"),
            ("2026-10-22 08:00", "tomorrow 08:00"),
            ("2026-10-24 09:00", "Sat 09:00"),
            ("2026-11-02 10:00", "Mon 2 Nov 10:00"),
            ("2027-01-04 10:00", "Mon 4 Jan 2027 10:00"),
            // Passed: the time.
            ("2026-10-20 09:00", "09:00"),
        ];
        for (time, expected) in cases {
            assert_eq!(describe_until(at(time), now), expected);
        }
    }

    #[test]
    fn describes_send_times() {
        let now = at(NOW);
        let cases = [
            ("2026-10-21 17:30", "today 17:30"),
            ("2026-10-22 09:00", "tomorrow 09:00"),
            ("2026-10-20 09:00", "yesterday 09:00"),
            ("2026-10-24 09:00", "Sat 09:00"),
            ("2026-11-02 10:00", "Mon 2 Nov 10:00"),
            ("2027-01-04 10:00", "Mon 4 Jan 2027 10:00"),
        ];
        for (time, expected) in cases {
            assert_eq!(describe_send_time(at(time), now), expected);
        }
    }
}
