//! Disappearing messages (see `docs/disappearing.md`): ours deleted for
//! everyone a set time after every other member has read them, at the
//! latest a set time after sending (the limit, in Settings). Sent with
//! `Timeline.send`; a room's mode is the composer's default there. The
//! library deletes them while sync runs (`Session.startSync`), as the
//! desktop app does while it runs.

use std::{sync::Arc, time::Duration};

use mtrx_core as core;

use crate::{
    MtrxError, Room, Session, TaskHandle,
    runtime::{self, follow, run},
};

/// How a message disappears, as its sender marked it.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct Disappearance {
    /// After every other member has read it.
    pub after_read: Duration,
    /// At the latest after it was sent.
    pub at_most: Duration,
    /// In words, for its menu: ours when it goes ("Disappears 5 minutes
    /// after everyone has read it, at the latest 1 day after sending"),
    /// others' only that it does.
    pub words: String,
}

impl Disappearance {
    pub(crate) fn new(disappear: core::Disappear, own: bool) -> Self {
        Self {
            after_read: disappear.after_read,
            at_most: disappear.at_most,
            words: disappear.words(own),
        }
    }
}

/// How long after everyone has read them messages disappear, or not
/// (`after_read` `None`).
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct DisappearChoice {
    pub after_read: Option<Duration>,
    /// For the composer's menu: "Off", "5 minutes".
    pub label: String,
    /// For a room's settings: "Off", "5 minutes after read".
    pub mode_label: String,
}

/// A limit Settings offers, with its words ("1 day").
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct LimitChoice {
    pub at_most: Duration,
    pub label: String,
}

/// Off, then each time after read, in order: for the composer's menu
/// (headed "Disappear after everyone read it") and a room's mode.
#[uniffi::export]
pub fn disappear_choices() -> Vec<DisappearChoice> {
    let choice = |after_read: Option<Duration>| DisappearChoice {
        after_read,
        label: after_read.map_or_else(|| "Off".to_owned(), core::duration_words),
        mode_label: core::disappearing_mode_words(after_read),
    };
    std::iter::once(choice(None))
        .chain(core::AFTER_READ_CHOICES.map(|after| choice(Some(after))))
        .collect()
}

/// What the composer sends, for its hourglass's label: "Disappears 5
/// minutes after everyone has read it", "Disappearing messages: off".
#[uniffi::export]
pub fn disappear_words(after_read: Option<Duration>) -> String {
    core::disappear_words(after_read)
}

/// The limits Settings offers, in order.
#[uniffi::export]
pub fn disappearing_limit_choices() -> Vec<LimitChoice> {
    (core::AT_MOST_CHOICES.into_iter())
        .map(|at_most| LimitChoice {
            at_most,
            label: core::duration_words(at_most),
        })
        .collect()
}

/// Receives a room's mode, now and after every change.
#[uniffi::export(with_foreign)]
pub trait DisappearingModeListener: Send + Sync {
    /// Our messages here disappear this long after read; `None` for off.
    fn on_update(&self, after_read: Option<Duration>);
}

/// Receives the limit, now and after every change.
#[uniffi::export(with_foreign)]
pub trait DisappearingLimitListener: Send + Sync {
    fn on_update(&self, at_most: Duration);
}

#[uniffi::export]
impl Room {
    /// Call `listener` with the room's mode, now and after every change,
    /// made here or on our other devices.
    pub fn subscribe_disappearing_mode(
        &self,
        listener: Arc<dyn DisappearingModeListener>,
    ) -> Arc<TaskHandle> {
        let modes = {
            let _runtime = runtime::enter();
            self.core().disappearing_mode_updates()
        };
        follow(modes, std::convert::identity, move |after_read| {
            listener.on_update(after_read)
        })
    }

    /// Set the room's mode (`None` for off): ours alone, on all our
    /// devices; others' messages here don't change. Shows at once; if
    /// saving fails, the saved mode shows again.
    pub async fn set_disappearing_mode(
        &self,
        after_read: Option<Duration>,
    ) -> Result<(), MtrxError> {
        let room = self.core();
        run(async move { Ok(room.set_disappearing_mode(after_read).await?) }).await
    }
}

#[uniffi::export]
impl Session {
    /// How long after sending our disappearing messages go at the latest,
    /// shared by our devices; 1 day unless changed.
    pub fn disappearing_limit(&self) -> Duration {
        self.core().disappearing_limit().at_most
    }

    /// Call `listener` with the limit, now and after every change, made
    /// here or on our other devices.
    pub fn subscribe_disappearing_limit(
        &self,
        listener: Arc<dyn DisappearingLimitListener>,
    ) -> Arc<TaskHandle> {
        let limits = {
            let _runtime = runtime::enter();
            self.core().disappearing_limit_updates()
        };
        follow(
            limits,
            |limit| limit.at_most,
            move |at_most| listener.on_update(at_most),
        )
    }

    /// Change the limit for messages sent from now on. Shows at once; if
    /// saving fails, the saved one shows again.
    pub async fn set_disappearing_limit(&self, at_most: Duration) -> Result<(), MtrxError> {
        let session = self.core();
        run(async move { Ok(session.set_disappearing_limit(at_most).await?) }).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn choices_read_as_menu_items() {
        let labels: Vec<(String, String)> = disappear_choices()
            .into_iter()
            .map(|c| (c.label, c.mode_label))
            .collect();
        assert_eq!(labels[0], ("Off".to_owned(), "Off".to_owned()));
        assert_eq!(
            labels[2],
            ("5 minutes".to_owned(), "5 minutes after read".to_owned())
        );
        assert_eq!(labels.len(), 5);
        let limits: Vec<String> = (disappearing_limit_choices().into_iter())
            .map(|c| c.label)
            .collect();
        assert_eq!(limits, ["1 hour", "6 hours", "1 day", "7 days"]);
    }
}
