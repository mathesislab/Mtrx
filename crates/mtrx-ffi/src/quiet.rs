//! Pausing notifications and quiet hours (see `docs/quiet.md`), on all our
//! devices with Mtrx: the library decides, for pushes too
//! (`Mtrx.notification` gives nothing while quiet). Other Matrix apps
//! aren't paused.

use std::{sync::Arc, time::SystemTime};

use mtrx_core::{self as core, NaiveTime};

use crate::{
    MtrxError, Session, TaskHandle,
    runtime::{follow, run},
};

/// A day of the week.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, uniffi::Enum)]
pub enum Weekday {
    Mon,
    Tue,
    Wed,
    Thu,
    Fri,
    Sat,
    Sun,
}

const WEEK: [(Weekday, core::Weekday); 7] = [
    (Weekday::Mon, core::Weekday::Mon),
    (Weekday::Tue, core::Weekday::Tue),
    (Weekday::Wed, core::Weekday::Wed),
    (Weekday::Thu, core::Weekday::Thu),
    (Weekday::Fri, core::Weekday::Fri),
    (Weekday::Sat, core::Weekday::Sat),
    (Weekday::Sun, core::Weekday::Sun),
];

/// Every chosen day, in each device's local time. Times are minutes after
/// midnight; `from == to` is the whole day.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct QuietHours {
    pub on: bool,
    pub from_minute: u16,
    pub to_minute: u16,
    /// The days a quiet period starts on, Monday first.
    pub days: Vec<Weekday>,
}

impl From<core::QuietHours> for QuietHours {
    fn from(hours: core::QuietHours) -> Self {
        let minute =
            |time: NaiveTime| time.signed_duration_since(NaiveTime::MIN).num_minutes() as u16;
        Self {
            on: hours.on,
            from_minute: minute(hours.from),
            to_minute: minute(hours.to),
            days: (WEEK.iter())
                .filter(|(_, day)| hours.days.contains(*day))
                .map(|(day, _)| *day)
                .collect(),
        }
    }
}

impl QuietHours {
    fn core(&self) -> Result<core::QuietHours, MtrxError> {
        let time = |minute: u16| {
            let minute = u32::from(minute);
            NaiveTime::from_hms_opt(minute / 60, minute % 60, 0)
                .ok_or_else(|| MtrxError::invalid(format!("{minute} minutes is no time of day")))
        };
        let mut days = core::WeekdaySet::EMPTY;
        for (day, weekday) in WEEK {
            if self.days.contains(&day) {
                days.insert(weekday);
            }
        }
        Ok(core::QuietHours {
            on: self.on,
            from: time(self.from_minute)?,
            to: time(self.to_minute)?,
            days,
        })
    }
}

/// How long to pause notifications.
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum PauseFor {
    Hour,
    /// 08:00: today's before 05:00, else tomorrow's.
    TomorrowMorning,
    /// Until a time picked.
    At {
        at: SystemTime,
    },
    UntilResumed,
}

impl From<core::PauseFor> for PauseFor {
    fn from(pause: core::PauseFor) -> Self {
        match pause {
            core::PauseFor::Hour => Self::Hour,
            core::PauseFor::TomorrowMorning => Self::TomorrowMorning,
            core::PauseFor::At(at) => Self::At { at },
            core::PauseFor::UntilResumed => Self::UntilResumed,
        }
    }
}

impl From<PauseFor> for core::PauseFor {
    fn from(pause: PauseFor) -> Self {
        match pause {
            PauseFor::Hour => Self::Hour,
            PauseFor::TomorrowMorning => Self::TomorrowMorning,
            PauseFor::At { at } => Self::At(at),
            PauseFor::UntilResumed => Self::UntilResumed,
        }
    }
}

/// An entry of the pause menu: a pause, or "Until…": ask for a date and
/// time, then pause with `PauseFor.At`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum PauseEntry {
    Pause { pause: PauseFor },
    PickTime,
}

/// An entry of the pause menu, with its title: "For 1 hour", "Until
/// tomorrow, 08:00", "Until…", "Until I turn them back on".
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct PauseMenuItem {
    pub entry: PauseEntry,
    pub title: String,
}

/// Whether notifications are held back, and the settings.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct QuietState {
    /// Held back now: paused or in quiet hours.
    pub quiet: bool,
    pub paused: bool,
    /// When the pause or quiet hours end; none while not quiet, or paused
    /// until turned back on.
    pub until: Option<SystemTime>,
    /// "Notifications on", "Paused until 08:00", "Paused until tomorrow
    /// 08:00", "Paused until you turn them back on", "Quiet hours until
    /// 07:00".
    pub line: String,
    pub hours: QuietHours,
    /// "22:00–07:00 on weekdays", "All day on Sat, Sun", "Off".
    pub hours_line: String,
    /// Mentions and keywords notify during a pause and quiet hours.
    pub mentions_through: bool,
}

impl From<core::QuietState> for QuietState {
    fn from(state: core::QuietState) -> Self {
        let (quiet, paused, until) = match state.status {
            core::QuietStatus::On => (false, false, None),
            core::QuietStatus::Paused { until } => (true, true, until),
            core::QuietStatus::QuietHours { until } => (true, false, Some(until)),
        };
        Self {
            quiet,
            paused,
            until,
            line: state.line,
            hours: state.quiet.hours.into(),
            hours_line: state.hours_line,
            mentions_through: state.quiet.mentions_through,
        }
    }
}

/// Receives whether notifications are held back, now and after every
/// change: from our devices, and when a pause or quiet hours start or end.
#[uniffi::export(with_foreign)]
pub trait QuietListener: Send + Sync {
    fn on_update(&self, state: QuietState);
}

#[uniffi::export]
impl Session {
    pub fn quiet(&self) -> QuietState {
        self.core().quiet().into()
    }

    /// Call `listener` with whether notifications are held back, now and
    /// after every change.
    pub fn subscribe_quiet(&self, listener: Arc<dyn QuietListener>) -> Arc<TaskHandle> {
        let updates = {
            let _runtime = crate::runtime::enter();
            self.core().quiet_updates()
        };
        follow(updates, QuietState::from, move |state| {
            listener.on_update(state)
        })
    }

    /// The pause menu, its titles as of now: call it when the menu opens.
    pub fn pause_menu(&self) -> Vec<PauseMenuItem> {
        (core::pause_menu(SystemTime::now()).into_iter())
            .map(|(entry, title)| PauseMenuItem {
                entry: match entry {
                    core::PauseEntry::Pause(pause) => PauseEntry::Pause {
                        pause: pause.into(),
                    },
                    core::PauseEntry::PickTime => PauseEntry::PickTime,
                },
                title,
            })
            .collect()
    }

    /// Pause notifications on all our devices with Mtrx. Shows at once; if
    /// saving fails, the saved state shows again.
    pub async fn pause_notifications(&self, pause: PauseFor) -> Result<(), MtrxError> {
        let session = self.core();
        run(async move { Ok(session.pause_notifications(pause.into()).await?) }).await
    }

    pub async fn resume_notifications(&self) -> Result<(), MtrxError> {
        let session = self.core();
        run(async move { Ok(session.resume_notifications().await?) }).await
    }

    pub async fn set_quiet_hours(&self, hours: QuietHours) -> Result<(), MtrxError> {
        let hours = hours.core()?;
        let session = self.core();
        run(async move { Ok(session.set_quiet_hours(hours).await?) }).await
    }

    /// Let mentions and keywords notify during a pause and quiet hours.
    pub async fn set_mentions_through(&self, on: bool) -> Result<(), MtrxError> {
        let session = self.core();
        run(async move { Ok(session.set_mentions_through(on).await?) }).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quiet_hours_in_minutes_and_back() {
        let hours = QuietHours {
            on: true,
            from_minute: 22 * 60,
            to_minute: 7 * 60 + 30,
            days: vec![Weekday::Mon, Weekday::Fri],
        };
        let core = hours.core().unwrap();
        assert_eq!(core.from, NaiveTime::from_hms_opt(22, 0, 0).unwrap());
        assert!(core.days.contains(core::Weekday::Fri));
        assert!(!core.days.contains(core::Weekday::Sun));
        assert_eq!(QuietHours::from(core), hours);
        let late = QuietHours {
            to_minute: 24 * 60,
            ..hours
        };
        assert!(late.core().is_err());
    }
}
