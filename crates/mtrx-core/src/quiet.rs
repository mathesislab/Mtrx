//! Pausing notifications and quiet hours (see `docs/quiet.md`), on all the
//! user's devices with Mtrx: global account data `org.mtrx.quiet`, decided
//! here on both paths, sync ([`crate::notifications`]) and push
//! ([`crate::push`]). Unread counts keep counting. Other Matrix apps aren't
//! paused, and the server still pushes.
//!
//! A pause ends at an instant, the same on every device; quiet hours are
//! wall-clock times, applied in each device's own time zone.

use std::time::{Duration, SystemTime};

use chrono::{
    DateTime, Datelike, Days, Local, NaiveDate, NaiveDateTime, NaiveTime, Weekday, WeekdaySet,
};
use matrix_sdk::ruma::events::macros::EventContent;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::{
    Live, Result, Session,
    account_data::AccountData,
    send_time::{describe_until, local_at, morning_words, tomorrow_morning},
};

/// Longest wait between looks at the clock: timers may not count time the
/// computer sleeps, and the clock or time zone may change.
const MAX_WAIT: Duration = Duration::from_secs(30);
const HOUR: Duration = Duration::from_secs(3600);
const DAY_NAMES: [(Weekday, &str); 7] = [
    (Weekday::Mon, "mon"),
    (Weekday::Tue, "tue"),
    (Weekday::Wed, "wed"),
    (Weekday::Thu, "thu"),
    (Weekday::Fri, "fri"),
    (Weekday::Sat, "sat"),
    (Weekday::Sun, "sun"),
];
const WEEKDAYS: WeekdaySet = WeekdaySet::from_array([
    Weekday::Mon,
    Weekday::Tue,
    Weekday::Wed,
    Weekday::Thu,
    Weekday::Fri,
]);

/// When notifications are held back, shared by the user's devices (global
/// account data `org.mtrx.quiet`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Quiet {
    pub pause: Option<Pause>,
    pub hours: QuietHours,
    /// Mentions and keywords notify during a pause and quiet hours.
    pub mentions_through: bool,
    /// The content's fields this version doesn't know, kept when it is
    /// saved again.
    pub(crate) unknown: Map<String, Value>,
}

/// Notifications paused until an instant, or until turned back on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pause {
    Until(SystemTime),
    UntilResumed,
}

/// Every chosen day, in each device's local time.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct QuietHours {
    pub on: bool,
    pub from: NaiveTime,
    /// `from == to` is the whole day.
    pub to: NaiveTime,
    /// The days a quiet period starts on.
    pub days: WeekdaySet,
}

impl Default for QuietHours {
    fn default() -> Self {
        Self {
            on: false,
            from: NaiveTime::from_hms_opt(22, 0, 0).unwrap(),
            to: NaiveTime::from_hms_opt(7, 0, 0).unwrap(),
            days: WeekdaySet::ALL,
        }
    }
}

/// What the pause menu offers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PauseFor {
    Hour,
    /// 08:00: today's before 05:00, else tomorrow's.
    TomorrowMorning,
    At(SystemTime),
    UntilResumed,
}

/// An entry of the pause menu: a pause, or "Until…" (the app asks for a
/// time, then pauses with [`PauseFor::At`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PauseEntry {
    Pause(PauseFor),
    PickTime,
}

/// The pause menu at `now`: "For 1 hour", "Until tomorrow, 08:00" ("Until
/// 08:00" before 05:00), "Until…", "Until I turn them back on".
pub fn pause_menu(now: SystemTime) -> Vec<(PauseEntry, String)> {
    vec![
        (PauseEntry::Pause(PauseFor::Hour), "For 1 hour".to_owned()),
        (
            PauseEntry::Pause(PauseFor::TomorrowMorning),
            format!("Until {}", morning_words(now)),
        ),
        (PauseEntry::PickTime, "Until…".to_owned()),
        (
            PauseEntry::Pause(PauseFor::UntilResumed),
            "Until I turn them back on".to_owned(),
        ),
    ]
}

impl PauseFor {
    /// The pause chosen at `now`.
    fn pause(self, now: SystemTime) -> Pause {
        match self {
            Self::Hour => Pause::Until(now + HOUR),
            Self::TomorrowMorning => Pause::Until(tomorrow_morning(now)),
            Self::At(at) => Pause::Until(at),
            Self::UntilResumed => Pause::UntilResumed,
        }
    }
}

/// Whether notifications are on, and if not, until when.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum QuietStatus {
    On,
    /// Until turned back on if `until` is none.
    Paused {
        until: Option<SystemTime>,
    },
    QuietHours {
        until: SystemTime,
    },
}

/// What the apps show.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QuietState {
    pub quiet: Quiet,
    pub status: QuietStatus,
    /// "Notifications on", "Paused until 08:00", "Paused until tomorrow
    /// 08:00", "Paused until you turn them back on", "Quiet hours until
    /// 07:00".
    pub line: String,
    /// "22:00–07:00 on weekdays", "All day on Sat, Sun", "Off".
    pub hours_line: String,
}

impl QuietState {
    fn new(quiet: Quiet, now: SystemTime) -> Self {
        let status = quiet.status(now);
        let line = match status {
            QuietStatus::On => "Notifications on".to_owned(),
            QuietStatus::Paused { until: None } => "Paused until you turn them back on".to_owned(),
            QuietStatus::Paused { until: Some(at) } => {
                format!("Paused until {}", describe_until(at, now))
            }
            QuietStatus::QuietHours { until } => {
                format!("Quiet hours until {}", describe_until(until, now))
            }
        };
        let hours_line = quiet.hours.words();
        Self {
            quiet,
            status,
            line,
            hours_line,
        }
    }
}

impl Quiet {
    /// Whether a notification is held back at `now`; `highlight`: it
    /// mentions us or a keyword.
    pub fn silences(&self, highlight: bool, now: SystemTime) -> bool {
        self.status(now) != QuietStatus::On && !(highlight && self.mentions_through)
    }

    pub fn status(&self, now: SystemTime) -> QuietStatus {
        match self.pause {
            Some(Pause::UntilResumed) => return QuietStatus::Paused { until: None },
            Some(Pause::Until(at)) if at > now => return QuietStatus::Paused { until: Some(at) },
            _ => {}
        }
        match self.hours.period_at(local(now)) {
            Some((_, end)) => QuietStatus::QuietHours {
                until: instant(end).unwrap_or(now + HOUR),
            },
            None => QuietStatus::On,
        }
    }

    /// When `status` may change next: the pause's end, quiet hours' next
    /// start or end.
    pub(crate) fn next_change(&self, now: SystemTime) -> Option<SystemTime> {
        let pause = match self.pause {
            Some(Pause::Until(at)) if at > now => Some(at),
            _ => None,
        };
        let at = local(now);
        let hours = match self.hours.period_at(at) {
            Some((_, end)) => Some(end),
            None => self.hours.next_start(at),
        };
        let hours = hours.and_then(instant);
        [pause, hours].into_iter().flatten().min()
    }
}

impl QuietHours {
    /// The quiet period `at` falls in, as local start and end, the end
    /// after any periods that follow without a break; none if not quiet.
    fn period_at(&self, at: NaiveDateTime) -> Option<(NaiveDateTime, NaiveDateTime)> {
        let today = at.date();
        let start = [today.pred_opt()?, today]
            .into_iter()
            .filter_map(|day| self.period(day))
            .find(|(start, end)| *start <= at && at < *end)?
            .0;
        let mut end = start;
        // A week of whole days at most.
        for _ in 0..7 {
            match self.period(end.date()).filter(|(start, _)| *start == end) {
                Some((_, next)) => end = next,
                None => break,
            }
        }
        Some((start, end))
    }

    /// The period starting on `day`, if it is one of the days.
    fn period(&self, day: NaiveDate) -> Option<(NaiveDateTime, NaiveDateTime)> {
        if !self.on || !self.days.contains(day.weekday()) {
            return None;
        }
        let start = day.and_time(self.from);
        let end_day = if self.from < self.to {
            day
        } else {
            day.succ_opt()?
        };
        Some((start, end_day.and_time(self.to)))
    }

    /// The first quiet period's start after `at`.
    fn next_start(&self, at: NaiveDateTime) -> Option<NaiveDateTime> {
        (0..=7)
            .filter_map(|days| at.date().checked_add_days(Days::new(days)))
            .filter_map(|day| self.period(day))
            .map(|(start, _)| start)
            .find(|start| *start > at)
    }

    /// "22:00–07:00 on weekdays", "All day on Sat, Sun", "22:00–07:00
    /// every day", "Off".
    pub fn words(&self) -> String {
        if !self.on || self.days.is_empty() {
            return "Off".to_owned();
        }
        let times = if self.from == self.to {
            "All day".to_owned()
        } else {
            format!("{}–{}", self.from.format("%H:%M"), self.to.format("%H:%M"))
        };
        if self.days == WeekdaySet::ALL {
            return format!("{times} every day");
        }
        let days = if self.days == WEEKDAYS {
            "weekdays".to_owned()
        } else {
            let names: Vec<String> = (self.days.iter(Weekday::Mon))
                .map(|day| day.to_string())
                .collect();
            names.join(", ")
        };
        format!("{times} on {days}")
    }
}

/// `now` on this device's wall clock.
fn local(now: SystemTime) -> NaiveDateTime {
    DateTime::<Local>::from(now).naive_local()
}

/// A wall-clock time here as an instant.
fn instant(at: NaiveDateTime) -> Option<SystemTime> {
    local_at(at.date(), at.time()).map(SystemTime::from)
}

/// The content of `org.mtrx.quiet`, read leniently: a damaged field reads
/// as its default.
#[derive(Clone, Debug, Default, Serialize, Deserialize, EventContent)]
#[ruma_event(type = "org.mtrx.quiet", kind = GlobalAccountData)]
pub(crate) struct QuietEventContent {
    #[serde(flatten)]
    fields: Map<String, Value>,
}

impl AccountData for Quiet {
    type Content = QuietEventContent;

    fn from_content(content: QuietEventContent) -> Self {
        let mut fields = content.fields;
        let pause = fields.remove("pause").and_then(|pause| {
            let pause = pause.as_object()?;
            Some(match pause.get("until") {
                Some(until) => {
                    Pause::Until(SystemTime::UNIX_EPOCH + Duration::from_millis(until.as_u64()?))
                }
                None => Pause::UntilResumed,
            })
        });
        let hours = fields
            .remove("hours")
            .map(|hours| read_hours(&hours))
            .unwrap_or_default();
        let mentions_through = fields
            .remove("mentions_through")
            .and_then(|on| on.as_bool())
            .unwrap_or(false);
        Self {
            pause,
            hours,
            mentions_through,
            unknown: fields,
        }
    }

    fn into_content(self) -> QuietEventContent {
        let mut fields = self.unknown;
        match self.pause {
            Some(Pause::Until(at)) => {
                let millis = at.duration_since(SystemTime::UNIX_EPOCH);
                let millis = millis.unwrap_or_default().as_millis() as u64;
                fields.insert("pause".into(), serde_json::json!({ "until": millis }));
            }
            Some(Pause::UntilResumed) => {
                fields.insert("pause".into(), serde_json::json!({}));
            }
            None => {}
        }
        let hours = self.hours;
        let days: Vec<&str> = (DAY_NAMES.iter())
            .filter(|(day, _)| hours.days.contains(*day))
            .map(|(_, name)| *name)
            .collect();
        fields.insert(
            "hours".into(),
            serde_json::json!({
                "on": hours.on,
                "from": hours.from.format("%H:%M").to_string(),
                "to": hours.to.format("%H:%M").to_string(),
                "days": days,
            }),
        );
        fields.insert("mentions_through".into(), self.mentions_through.into());
        QuietEventContent { fields }
    }
}

/// Quiet hours as stored; what can't be read is the default.
fn read_hours(json: &Value) -> QuietHours {
    let default = QuietHours::default();
    let time = |key: &str| {
        let time = json.get(key)?.as_str()?;
        NaiveTime::parse_from_str(time, "%H:%M").ok()
    };
    let days = json.get("days").and_then(Value::as_array).map(|days| {
        let mut set = WeekdaySet::EMPTY;
        for name in days.iter().filter_map(Value::as_str) {
            if let Some((day, _)) = DAY_NAMES.iter().find(|(_, n)| *n == name) {
                set.insert(*day);
            }
        }
        set
    });
    QuietHours {
        on: json
            .get("on")
            .and_then(Value::as_bool)
            .unwrap_or(default.on),
        from: time("from").unwrap_or(default.from),
        to: time("to").unwrap_or(default.to),
        days: days.unwrap_or(default.days),
    }
}

impl Session {
    /// When notifications are held back, now.
    pub fn quiet(&self) -> QuietState {
        QuietState::new(self.quiet.get(), SystemTime::now())
    }

    /// [`Session::quiet`], then every change: from our devices, and at each
    /// start and end, looked at again at least every 30 s (a changed clock,
    /// a new time zone, a computer waking from sleep).
    pub fn quiet_updates(&self) -> Live<QuietState> {
        let mut quiet = self.quiet.subscribe();
        let first = QuietState::new(quiet.borrow_and_update().clone(), SystemTime::now());
        Live::spawn(first, move |tx| async move {
            loop {
                let now = SystemTime::now();
                let next = quiet.borrow().next_change(now);
                let wait = next
                    .map(|at| at.duration_since(now).unwrap_or_default())
                    .map_or(MAX_WAIT, |wait| wait.min(MAX_WAIT));
                tokio::select! {
                    // A moment past the change, so it has happened.
                    () = tokio::time::sleep(wait + Duration::from_millis(10)) => {}
                    changed = quiet.changed() => {
                        if changed.is_err() {
                            return;
                        }
                    }
                }
                let state = QuietState::new(quiet.borrow_and_update().clone(), SystemTime::now());
                tx.send_if_modified(|shown| {
                    let changed = *shown != state;
                    if changed {
                        *shown = state;
                    }
                    changed
                });
            }
        })
    }

    /// Pause notifications on all our devices with Mtrx; shown at once.
    pub async fn pause_notifications(&self, pause: PauseFor) -> Result<()> {
        let pause = pause.pause(SystemTime::now());
        self.quiet.change(|quiet| quiet.pause = Some(pause)).await
    }

    pub async fn resume_notifications(&self) -> Result<()> {
        self.quiet.change(|quiet| quiet.pause = None).await
    }

    pub async fn set_quiet_hours(&self, hours: QuietHours) -> Result<()> {
        self.quiet.change(|quiet| quiet.hours = hours).await
    }

    /// Let mentions and keywords notify during a pause and quiet hours.
    pub async fn set_mentions_through(&self, on: bool) -> Result<()> {
        self.quiet.change(|quiet| quiet.mentions_through = on).await
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use chrono::TimeZone;
    use serde_json::json;

    use super::*;

    pub(crate) fn naive(s: &str) -> NaiveDateTime {
        NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M").unwrap()
    }

    fn at(s: &str) -> SystemTime {
        Local.from_local_datetime(&naive(s)).unwrap().into()
    }

    fn time(s: &str) -> NaiveTime {
        NaiveTime::parse_from_str(s, "%H:%M").unwrap()
    }

    fn hours(from: &str, to: &str, days: &[Weekday]) -> QuietHours {
        let mut set = WeekdaySet::EMPTY;
        for day in days {
            set.insert(*day);
        }
        QuietHours {
            on: true,
            from: time(from),
            to: time(to),
            days: set,
        }
    }

    const MON_FRI: [Weekday; 5] = [
        Weekday::Mon,
        Weekday::Tue,
        Weekday::Wed,
        Weekday::Thu,
        Weekday::Fri,
    ];

    // 2026-11-20 is a Friday.
    #[test]
    fn a_pause_holds_back_until_it_ends() {
        let now = at("2026-11-20 14:00");
        let quiet = Quiet::default();
        assert!(!quiet.silences(false, now));
        let paused = Quiet {
            pause: Some(Pause::Until(at("2026-11-20 15:00"))),
            ..Quiet::default()
        };
        assert!(paused.silences(false, now));
        assert!(paused.silences(true, now), "mentions too");
        assert!(!paused.silences(false, at("2026-11-20 15:00")), "ended");
        let resumed = Quiet {
            pause: Some(Pause::UntilResumed),
            ..Quiet::default()
        };
        assert!(resumed.silences(false, at("2027-01-01 00:00")));
        let through = Quiet {
            mentions_through: true,
            ..resumed
        };
        assert!(through.silences(false, now));
        assert!(!through.silences(true, now), "a mention gets through");
    }

    #[test]
    fn quiet_hours_from_their_start_to_their_end() {
        let night = hours("22:00", "07:00", &MON_FRI);
        let quiet = |at: &str| night.period_at(naive(at)).is_some();
        assert!(quiet("2026-11-18 23:00"), "Wednesday night");
        assert!(quiet("2026-11-19 06:59"));
        assert!(!quiet("2026-11-19 07:00"), "`to` isn't quiet");
        assert!(quiet("2026-11-19 22:00"), "`from` is");
        assert!(!quiet("2026-11-19 21:59"));
        // By the day a period starts: Friday night, not Sunday night.
        assert!(quiet("2026-11-20 23:30"));
        assert!(quiet("2026-11-21 06:00"), "Saturday morning");
        assert!(!quiet("2026-11-22 23:30"), "Sunday night");
        assert!(!quiet("2026-11-23 06:00"), "Monday morning");
        let off = QuietHours { on: false, ..night };
        assert!(off.period_at(naive("2026-11-18 23:00")).is_none());
        // The same day: 13:00–14:00.
        let lunch = hours("13:00", "14:00", &MON_FRI);
        assert!(lunch.period_at(naive("2026-11-18 13:30")).is_some());
        assert!(lunch.period_at(naive("2026-11-18 14:30")).is_none());
    }

    #[test]
    fn whole_days_follow_each_other() {
        let weekend = hours("00:00", "00:00", &[Weekday::Sat, Weekday::Sun]);
        assert_eq!(
            weekend.period_at(naive("2026-11-21 12:00")),
            Some((naive("2026-11-21 00:00"), naive("2026-11-23 00:00")))
        );
        assert!(weekend.period_at(naive("2026-11-20 23:59")).is_none());
        assert!(weekend.period_at(naive("2026-11-23 00:00")).is_none());
    }

    #[test]
    fn quiet_hours_are_wall_clock_times() {
        // The night clocks go back in Europe: still 22:00 to 07:00.
        let night = hours("22:00", "07:00", &[Weekday::Sat]);
        assert_eq!(
            night.period_at(naive("2026-10-25 02:30")),
            Some((naive("2026-10-24 22:00"), naive("2026-10-25 07:00")))
        );
        // And the night they go forward.
        assert_eq!(
            night.period_at(naive("2026-03-29 03:30")),
            Some((naive("2026-03-28 22:00"), naive("2026-03-29 07:00")))
        );
    }

    #[test]
    fn the_next_change() {
        let night = Quiet {
            hours: hours("22:00", "07:00", &MON_FRI),
            ..Quiet::default()
        };
        // Friday afternoon: Friday's start; then its end.
        assert_eq!(
            night.next_change(at("2026-11-20 14:00")),
            Some(at("2026-11-20 22:00"))
        );
        assert_eq!(
            night.next_change(at("2026-11-20 23:00")),
            Some(at("2026-11-21 07:00"))
        );
        // Saturday: not Saturday or Sunday night, Monday's.
        assert_eq!(
            night.next_change(at("2026-11-21 12:00")),
            Some(at("2026-11-23 22:00"))
        );
        // A pause's end, if sooner.
        let paused = Quiet {
            pause: Some(Pause::Until(at("2026-11-20 15:00"))),
            ..night.clone()
        };
        assert_eq!(
            paused.next_change(at("2026-11-20 14:00")),
            Some(at("2026-11-20 15:00"))
        );
        assert_eq!(Quiet::default().next_change(at("2026-11-20 14:00")), None);
    }

    #[test]
    fn the_status_in_words() {
        let now = at("2026-11-20 14:00");
        let line = |quiet: Quiet| QuietState::new(quiet, now).line;
        assert_eq!(line(Quiet::default()), "Notifications on");
        let until = |s| Quiet {
            pause: Some(Pause::Until(at(s))),
            ..Quiet::default()
        };
        assert_eq!(line(until("2026-11-20 18:00")), "Paused until 18:00");
        assert_eq!(
            line(until("2026-11-21 08:00")),
            "Paused until tomorrow 08:00"
        );
        let resumed = Quiet {
            pause: Some(Pause::UntilResumed),
            ..Quiet::default()
        };
        assert_eq!(line(resumed), "Paused until you turn them back on");
        let afternoon = Quiet {
            hours: hours("13:00", "17:00", &MON_FRI),
            ..Quiet::default()
        };
        assert_eq!(line(afternoon), "Quiet hours until 17:00");
        assert_eq!(
            hours("22:00", "07:00", &MON_FRI).words(),
            "22:00–07:00 on weekdays"
        );
        assert_eq!(
            hours("00:00", "00:00", &[Weekday::Sat, Weekday::Sun]).words(),
            "All day on Sat, Sun"
        );
        assert_eq!(QuietHours::default().words(), "Off");
        let every_day = QuietHours {
            on: true,
            ..QuietHours::default()
        };
        assert_eq!(every_day.words(), "22:00–07:00 every day");
    }

    #[test]
    fn the_pause_menu() {
        let titles: Vec<String> = (pause_menu(at("2026-11-20 14:00")).into_iter())
            .map(|(_, title)| title)
            .collect();
        assert_eq!(
            titles,
            [
                "For 1 hour",
                "Until tomorrow, 08:00",
                "Until…",
                "Until I turn them back on"
            ]
        );
        let early = pause_menu(at("2026-11-20 04:30"));
        assert_eq!(early[1].1, "Until 08:00");
        assert_eq!(early[2].0, PauseEntry::PickTime);
    }

    #[test]
    fn the_content_survives_a_round_trip() {
        let quiet = Quiet {
            pause: Some(Pause::Until(
                SystemTime::UNIX_EPOCH + Duration::from_millis(1_763_539_200_000),
            )),
            hours: hours("22:00", "07:00", &MON_FRI),
            mentions_through: true,
            unknown: Map::new(),
        };
        let json = serde_json::to_value(quiet.clone().into_content()).unwrap();
        assert_eq!(
            json,
            json!({
                "pause": { "until": 1_763_539_200_000u64 },
                "hours": {
                    "on": true, "from": "22:00", "to": "07:00",
                    "days": ["mon", "tue", "wed", "thu", "fri"],
                },
                "mentions_through": true,
            })
        );
        let back: QuietEventContent = serde_json::from_value(json).unwrap();
        assert_eq!(Quiet::from_content(back), quiet);
        let resumed = Quiet {
            pause: Some(Pause::UntilResumed),
            ..Quiet::default()
        };
        let json = serde_json::to_value(resumed.clone().into_content()).unwrap();
        assert_eq!(json["pause"], json!({}));
        let back: QuietEventContent = serde_json::from_value(json).unwrap();
        assert_eq!(Quiet::from_content(back), resumed);
    }

    #[test]
    fn unknown_fields_are_kept() {
        let content: QuietEventContent =
            serde_json::from_value(json!({ "mentions_through": true, "dms_through": true }))
                .unwrap();
        let mut quiet = Quiet::from_content(content);
        assert!(quiet.mentions_through);
        quiet.mentions_through = false;
        let json = serde_json::to_value(quiet.into_content()).unwrap();
        assert_eq!(json["dms_through"], true);
        assert_eq!(json["mentions_through"], false);
    }

    #[test]
    fn damaged_content_reads_as_not_quiet() {
        let content: QuietEventContent = serde_json::from_value(json!({
            "pause": "soon",
            "hours": { "on": "yes", "from": "late", "days": "all" },
            "mentions_through": 1,
        }))
        .unwrap();
        let quiet = Quiet::from_content(content);
        assert_eq!(quiet.pause, None);
        assert_eq!(quiet.hours, QuietHours::default());
        assert!(!quiet.mentions_through);
        assert!(!quiet.silences(false, SystemTime::now()));
    }
}
