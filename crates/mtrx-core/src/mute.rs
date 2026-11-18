//! Muting a room for a time (see `docs/quiet.md`): the room's push rule
//! mutes it in every Matrix app, and a record in its account data
//! (`org.mtrx.mute`) says until when, and which mode it had before. Matrix
//! has no end for a mute, so a timer here ends it: while the desktop app,
//! `mtrx watch` or a phone app in front runs, as for disappearing
//! messages; one that fell due while none ran ends once one starts.

use std::{
    collections::BTreeMap,
    sync::Arc,
    time::{Duration, SystemTime},
};

use matrix_sdk::{
    Client, Room, RoomState,
    ruma::{OwnedRoomId, events::macros::EventContent},
};
use matrix_sdk_ui::{room_list_service::State, sync_service::SyncService};
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast::error::RecvError;
use tracing::warn;

use crate::{
    AbortOnDrop, Live, NotifyMode, Result, RoomHandle, Session,
    account_data::{RoomAccountData, RoomSynced},
    echo::Setting,
    room_settings::RoomModes,
    send_time::{morning_words, tomorrow_morning},
};

/// Longest wait between checks, as for disappearing messages: timers may
/// not count time the computer sleeps.
const MAX_WAIT: Duration = Duration::from_secs(30);
const HOUR: Duration = Duration::from_secs(3600);

/// How long a room stays muted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MuteFor {
    Hour,
    EightHours,
    /// 08:00: today's before 05:00, else tomorrow's.
    TomorrowMorning,
    UntilUnmuted,
}

impl MuteFor {
    /// In the menu's order.
    pub const ALL: [Self; 4] = [
        Self::Hour,
        Self::EightHours,
        Self::TomorrowMorning,
        Self::UntilUnmuted,
    ];

    /// When a mute chosen at `now` ends; none until unmuted.
    pub fn until(self, now: SystemTime) -> Option<SystemTime> {
        match self {
            Self::Hour => Some(now + HOUR),
            Self::EightHours => Some(now + 8 * HOUR),
            Self::TomorrowMorning => Some(tomorrow_morning(now)),
            Self::UntilUnmuted => None,
        }
    }

    /// As the menu names it at `now`: "Mute for 1 hour", "Mute for 8
    /// hours", "Mute until tomorrow, 08:00" ("Mute until 08:00" before
    /// 05:00), "Mute until I unmute".
    pub fn title(self, now: SystemTime) -> String {
        match self {
            Self::Hour => "Mute for 1 hour".to_owned(),
            Self::EightHours => "Mute for 8 hours".to_owned(),
            Self::TomorrowMorning => format!("Mute until {}", morning_words(now)),
            Self::UntilUnmuted => "Mute until I unmute".to_owned(),
        }
    }
}

/// A room muted for a time (room account data `org.mtrx.mute`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TimedMute {
    pub until: SystemTime,
    /// The room's own mode before; `None`: it followed the default.
    pub before: Option<NotifyMode>,
}

/// A room's timed mute: `{}` for none, as account data can't be deleted.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, EventContent)]
#[ruma_event(type = "org.mtrx.mute", kind = RoomAccountData)]
pub(crate) struct MuteEventContent {
    /// Milliseconds since the Unix epoch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    until: Option<u64>,
    /// "all" or "mentions"; none if it followed the default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    before: Option<String>,
}

impl RoomAccountData for TimedMute {
    type Value = Self;
    type Content = MuteEventContent;

    fn from_content(content: MuteEventContent) -> Option<Self> {
        let until = SystemTime::UNIX_EPOCH + Duration::from_millis(content.until?);
        let before = match content.before.as_deref() {
            Some("all") => Some(NotifyMode::All),
            Some("mentions") => Some(NotifyMode::Mentions),
            _ => None,
        };
        Some(Self { until, before })
    }

    fn into_content(mute: Option<Self>) -> MuteEventContent {
        let Some(mute) = mute else {
            return MuteEventContent::default();
        };
        let millis = mute.until.duration_since(SystemTime::UNIX_EPOCH);
        MuteEventContent {
            until: Some(millis.unwrap_or_default().as_millis() as u64),
            before: match mute.before {
                Some(NotifyMode::All) => Some("all".to_owned()),
                Some(NotifyMode::Mentions) => Some("mentions".to_owned()),
                Some(NotifyMode::Mute) | None => None,
            },
        }
    }
}

/// `time` to the millisecond, as records keep it.
fn millis(time: SystemTime) -> SystemTime {
    let since = time.duration_since(SystemTime::UNIX_EPOCH);
    SystemTime::UNIX_EPOCH + Duration::from_millis(since.unwrap_or_default().as_millis() as u64)
}

/// What the timer does with a timed mute.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Step {
    /// Not due.
    Wait,
    /// Due and the room still muted: give it back its mode, or make it
    /// follow the default (`None`); then clear the record.
    Restore(Option<NotifyMode>),
    /// Due, and the room no longer muted (changed elsewhere): clear the
    /// record.
    Clear,
}

/// What to do with `mute` at `now`, the room's own mode being `own_mode`.
pub(crate) fn unmute_step(mute: &TimedMute, own_mode: Option<NotifyMode>, now: SystemTime) -> Step {
    if mute.until > now {
        Step::Wait
    } else if own_mode == Some(NotifyMode::Mute) {
        Step::Restore(mute.before)
    } else {
        Step::Clear
    }
}

/// Ends timed mutes when due, while started.
pub(crate) struct Unmuter {
    pub(crate) client: Client,
    pub(crate) sync: Arc<SyncService>,
    pub(crate) modes: RoomModes,
    pub(crate) mutes: Arc<RoomSynced<TimedMute>>,
}

impl Unmuter {
    /// After the first sync from now, end the mutes due, and each when
    /// it falls due or a record changes; until dropped.
    pub(crate) fn start(self) -> AbortOnDrop {
        let mut states = self.sync.room_list_service().state();
        AbortOnDrop(tokio::spawn(async move {
            // The rules and records as they are now, not as stored before:
            // another device may have changed them meanwhile. The room
            // list's state is one of these once it synced since sync
            // (re)started.
            let synced = |state: &State| {
                matches!(state, State::SettingUp | State::Recovering | State::Running)
            };
            if !synced(&states.get()) {
                while let Some(state) = states.next().await {
                    if synced(&state) {
                        break;
                    }
                }
            }
            let mut changes = self.mutes.subscribe();
            loop {
                changes.mark_unchanged();
                let now = SystemTime::now();
                let (next, failed) = self.pass(now).await;
                if failed {
                    // Not again at once for a record restored after it
                    // could not be cleared.
                    changes.mark_unchanged();
                }
                let wait = next
                    .map(|at| at.duration_since(now).unwrap_or_default())
                    .map_or(MAX_WAIT, |wait| wait.min(MAX_WAIT));
                tokio::select! {
                    _ = tokio::time::sleep(wait) => {}
                    _ = changes.changed() => {}
                }
            }
        }))
    }

    /// End the mutes due at `now`; returns when the next one is due, and
    /// whether any could not be ended: they are tried again at the next
    /// pass.
    async fn pass(&self, now: SystemTime) -> (Option<SystemTime>, bool) {
        let mutes: BTreeMap<OwnedRoomId, TimedMute> = self.mutes.subscribe().borrow().clone();
        let (mut next, mut failed): (Option<SystemTime>, bool) = (None, false);
        for (id, mute) in mutes {
            if mute.until > now {
                next = Some(next.map_or(mute.until, |next| next.min(mute.until)));
                continue;
            }
            // Rooms left keep their records, harmless.
            let Some(room) = self.client.get_room(&id) else {
                continue;
            };
            if room.state() != RoomState::Joined {
                continue;
            }
            if let Err(e) = self.unmute(&room, &mute, now).await {
                warn!(room = %id, "could not end a timed mute: {e}");
                failed = true;
            }
        }
        (next, failed)
    }

    /// In the room's turn, give it back its mode if still muted, then
    /// clear the record if it is still the one acted on.
    async fn unmute(&self, room: &Room, mute: &TimedMute, now: SystemTime) -> Result<()> {
        let id = room.room_id();
        let turn = self.modes.turn(id).await;
        match unmute_step(mute, self.modes.own(id).await, now) {
            Step::Wait => return Ok(()),
            Step::Restore(before) => self.modes.send(turn, id, before).await?,
            Step::Clear => drop(turn),
        }
        // Another device may have muted it anew meanwhile.
        (self.mutes)
            .change(room, |current| {
                if current.as_ref() == Some(mute) {
                    *current = None;
                }
            })
            .await
    }
}

impl RoomHandle {
    /// Mute the room; for a time, it gets its mode back afterwards (while
    /// Mtrx runs on one of our devices).
    pub async fn mute(&self, for_: MuteFor) -> Result<()> {
        match for_.until(SystemTime::now()) {
            Some(until) => self.mute_until(until).await,
            None => self.set_notify(NotifyMode::Mute).await,
        }
    }

    /// [`Self::mute`] until `until`, for tests.
    #[cfg(feature = "test-support")]
    pub async fn mute_until_for_test(&self, until: SystemTime) -> Result<()> {
        self.mute_until(until).await
    }

    /// The room's own mode and its timed mute's record, for tests.
    #[cfg(feature = "test-support")]
    pub async fn own_notify_for_test(&self) -> (Option<NotifyMode>, Option<TimedMute>) {
        let session = &self.session;
        let own = session.room_modes.own(self.id()).await;
        (own, session.mutes.get(self.id()))
    }

    /// Mute the room until `until`, shown at once. In its turn: the record
    /// first, then the rule, so a room is never left muted with no end.
    pub(crate) async fn mute_until(&self, until: SystemTime) -> Result<()> {
        let (session, id) = (&self.session, self.id());
        let request = async {
            let turn = session.room_modes.turn(id).await;
            let own = session.room_modes.own(id).await;
            // Muted already for a time: a new end keeps the mode before.
            let before = match (session.mutes.get(id), own) {
                (Some(timed), Some(NotifyMode::Mute)) => timed.before,
                (_, own) => own.filter(|mode| *mode != NotifyMode::Mute),
            };
            // As the record keeps it, so a synced one is the same.
            let until = millis(until);
            let mute = TimedMute { until, before };
            session.mutes.set(&self.room, Some(mute)).await?;
            let muted = (session.room_modes)
                .send(turn, id, Some(NotifyMode::Mute))
                .await;
            if muted.is_err()
                && let Err(e) = session.mutes.set(&self.room, None).await
            {
                warn!(room = %id, "could not take back a timed mute: {e}");
            }
            muted
        };
        (session)
            .echoed(id, Setting::Notify(NotifyMode::Mute), request)
            .await
    }
}

impl Session {
    /// The muted rooms, with the end of a timed mute, then every change:
    /// the rooms whose own mode is Mute, as the push rules say, with their
    /// records' ends.
    pub(crate) fn muted_rooms_updates(&self) -> Live<BTreeMap<OwnedRoomId, Option<SystemTime>>> {
        let settings = self.notification_settings.clone();
        let mut rules = settings.subscribe_to_changes();
        let mut records = self.mutes.subscribe();
        Live::spawn(BTreeMap::new(), move |tx| async move {
            loop {
                let ends = records.borrow_and_update().clone();
                let mut muted = BTreeMap::new();
                for id in settings.get_rooms_with_user_defined_rules(Some(true)).await {
                    let Ok(id) = OwnedRoomId::try_from(id) else {
                        continue;
                    };
                    let mode = settings.get_user_defined_room_notification_mode(&id).await;
                    if mode.map(NotifyMode::from) == Some(NotifyMode::Mute) {
                        let end = ends.get(&id).map(|mute| mute.until);
                        muted.insert(id, end);
                    }
                }
                tx.send_if_modified(|shown| {
                    let changed = *shown != muted;
                    if changed {
                        *shown = muted;
                    }
                    changed
                });
                tokio::select! {
                    changed = rules.recv() => {
                        // A change missed while lagging still left its rules.
                        if let Err(RecvError::Closed) = changed {
                            return;
                        }
                    }
                    changed = records.changed() => {
                        if changed.is_err() {
                            return;
                        }
                    }
                }
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(secs: u64) -> SystemTime {
        SystemTime::UNIX_EPOCH + Duration::from_secs(secs)
    }

    #[test]
    fn mutes_end_when_due_with_the_mode_before() {
        let mute = TimedMute {
            until: at(100),
            before: Some(NotifyMode::Mentions),
        };
        let muted = Some(NotifyMode::Mute);
        assert_eq!(unmute_step(&mute, muted, at(99)), Step::Wait);
        assert_eq!(
            unmute_step(&mute, muted, at(100)),
            Step::Restore(Some(NotifyMode::Mentions))
        );
        // Unmuted elsewhere meanwhile: only the record goes.
        let all = Some(NotifyMode::All);
        assert_eq!(unmute_step(&mute, all, at(200)), Step::Clear);
        assert_eq!(unmute_step(&mute, None, at(200)), Step::Clear);
        // It followed the default: its rules are deleted.
        let default = TimedMute {
            before: None,
            ..mute
        };
        assert_eq!(unmute_step(&default, muted, at(100)), Step::Restore(None));
    }

    #[test]
    fn records_survive_a_round_trip() {
        let mute = TimedMute {
            until: at(1_763_485_200),
            before: Some(NotifyMode::Mentions),
        };
        let content = TimedMute::into_content(Some(mute));
        let json = serde_json::to_value(&content).unwrap();
        assert_eq!(
            json,
            serde_json::json!({ "until": 1_763_485_200_000u64, "before": "mentions" })
        );
        let back: MuteEventContent = serde_json::from_value(json).unwrap();
        assert_eq!(TimedMute::from_content(back), Some(mute));
        let all = TimedMute {
            before: Some(NotifyMode::All),
            ..mute
        };
        assert_eq!(
            TimedMute::from_content(TimedMute::into_content(Some(all))),
            Some(all)
        );
        // Followed the default: no "before".
        let default = TimedMute {
            before: None,
            ..mute
        };
        let json = serde_json::to_value(TimedMute::into_content(Some(default))).unwrap();
        assert_eq!(json, serde_json::json!({ "until": 1_763_485_200_000u64 }));
    }

    #[test]
    fn an_empty_record_is_none() {
        let none = serde_json::to_value(TimedMute::into_content(None)).unwrap();
        assert_eq!(none, serde_json::json!({}));
        let empty: MuteEventContent = serde_json::from_value(serde_json::json!({})).unwrap();
        assert_eq!(TimedMute::from_content(empty), None);
    }

    #[test]
    fn mutes_end_after_their_time() {
        let now = at(1_000_000);
        assert_eq!(MuteFor::Hour.until(now), Some(at(1_003_600)));
        assert_eq!(MuteFor::EightHours.until(now), Some(at(1_028_800)));
        assert!(MuteFor::TomorrowMorning.until(now).is_some_and(|t| t > now));
        assert_eq!(MuteFor::UntilUnmuted.until(now), None);
        assert_eq!(MuteFor::Hour.title(now), "Mute for 1 hour");
        assert_eq!(MuteFor::UntilUnmuted.title(now), "Mute until I unmute");
    }
}
