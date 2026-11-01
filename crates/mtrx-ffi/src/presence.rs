//! Presence: whether people are online, idle or away; and setting ours.

use std::{collections::HashMap, sync::Arc, time::SystemTime};

use futures_util::{FutureExt, Stream, StreamExt};
use mtrx_core::{self as core, OwnedUserId};

use crate::{MtrxError, Session, TaskHandle, runtime::run};

#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum PresenceState {
    /// Green dot.
    Online,
    /// Idle or away: amber dot.
    Unavailable,
    /// No dot.
    Offline,
}

impl From<core::PresenceState> for PresenceState {
    fn from(state: core::PresenceState) -> Self {
        match state {
            core::PresenceState::Online => Self::Online,
            core::PresenceState::Unavailable => Self::Unavailable,
            core::PresenceState::Offline => Self::Offline,
        }
    }
}

impl From<PresenceState> for core::PresenceState {
    fn from(state: PresenceState) -> Self {
        match state {
            PresenceState::Online => Self::Online,
            PresenceState::Unavailable => Self::Unavailable,
            PresenceState::Offline => Self::Offline,
        }
    }
}

/// The dot on someone's picture.
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum PresenceDot {
    /// Green.
    Online,
    /// Amber.
    Idle,
}

impl From<core::PresenceDot> for PresenceDot {
    fn from(dot: core::PresenceDot) -> Self {
        match dot {
            core::PresenceDot::Online => Self::Online,
            core::PresenceDot::Idle => Self::Idle,
        }
    }
}

impl From<PresenceDot> for core::PresenceDot {
    fn from(dot: PresenceDot) -> Self {
        match dot {
            PresenceDot::Online => Self::Online,
            PresenceDot::Idle => Self::Idle,
        }
    }
}

/// Someone's presence, as their server knows it.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct Presence {
    pub state: PresenceState,
    /// When they last did something, if the server says.
    pub last_active: Option<SystemTime>,
    /// Online and active within about the last minute.
    pub currently_active: bool,
    pub status_message: Option<String>,
    /// The dot on their picture; none offline.
    #[uniffi(default)]
    pub dot: Option<PresenceDot>,
    /// The line under their name, as of when it was sent (see
    /// [`presence_line`] for later); none offline with nothing more known.
    #[uniffi(default)]
    pub line: Option<String>,
}

impl Presence {
    /// `presence`, its line as of `now`.
    fn at(presence: core::Presence, now: SystemTime) -> Self {
        Self {
            dot: presence.dot().map(Into::into),
            line: presence.line(now),
            state: presence.state.into(),
            last_active: presence.last_active,
            currently_active: presence.currently_active,
            status_message: presence.status_message,
        }
    }
}

impl From<core::Presence> for Presence {
    fn from(presence: core::Presence) -> Self {
        Self::at(presence, SystemTime::now())
    }
}

impl From<Presence> for core::Presence {
    fn from(presence: Presence) -> Self {
        Self {
            state: presence.state.into(),
            last_active: presence.last_active,
            currently_active: presence.currently_active,
            status_message: presence.status_message,
        }
    }
}

/// The line under someone's name as of `now`: their status message, else
/// "Active now", "Idle" or when they were last active ("Last active 2 hr
/// ago"); none offline with nothing more known.
#[uniffi::export]
pub fn presence_line(presence: Presence, now: SystemTime) -> Option<String> {
    core::Presence::from(presence).line(now)
}

/// What `dot` says, for accessibility: "Online" or "Idle".
#[uniffi::export]
pub fn presence_dot_label(dot: PresenceDot) -> String {
    core::PresenceDot::from(dot).label().to_owned()
}

/// What the app is doing, which sets our presence.
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum PresenceActivity {
    /// The app comes to the foreground; also at start and sign-in.
    Foreground,
    /// The app goes to the background.
    Background,
    /// A touch or key press.
    Input,
    /// Signing out: offline, as in the background (until the next
    /// foreground, should signing out fail).
    SignOut,
}

impl From<PresenceActivity> for core::Activity {
    fn from(activity: PresenceActivity) -> Self {
        match activity {
            PresenceActivity::Foreground => Self::Foreground,
            PresenceActivity::Background => Self::Background,
            PresenceActivity::Input => Self::Input,
            PresenceActivity::SignOut => Self::SignOut,
        }
    }
}

/// Receives the presence of the users subscribed to.
#[uniffi::export(with_foreign)]
pub trait PresenceListener: Send + Sync {
    /// All presences known, by user ID, after every change. Users whose
    /// presence could not be read yet are missing.
    fn on_update(&self, presences: HashMap<String, Presence>);
    /// The server does not share presence: hide presence for the session.
    /// The last call.
    fn on_unavailable(&self);
}

#[uniffi::export]
impl Session {
    /// `user_id`'s presence. Fails with `PresenceUnavailable` when the
    /// server does not share presence; hide presence for the session then.
    /// Other failures concern this user (or the network) only.
    pub async fn presence(&self, user_id: String) -> Result<Presence, MtrxError> {
        let user = OwnedUserId::try_from(user_id.as_str()).map_err(MtrxError::invalid)?;
        let session = self.core();
        run(async move { Ok(session.presence(&user).await?.into()) }).await
    }

    /// Set our presence, and keep it until it is set again, set offline or
    /// the session ends. `status_message` nil clears it.
    pub async fn set_presence(
        &self,
        state: PresenceState,
        status_message: Option<String>,
    ) -> Result<(), MtrxError> {
        let session = self.core();
        run(async move {
            Ok(session
                .set_presence(state.into(), status_message.as_deref())
                .await?)
        })
        .await
    }

    /// What the app is doing, which sets our presence while sharing it:
    /// online in the foreground, idle after 5 minutes there without input,
    /// offline in the background and when signing out. Counts from the
    /// call, and returns once a change is set. Wait for it before
    /// `start_sync` (foreground), `stop_sync` (background) and `logout`
    /// (sign-out). Fails with `PresenceUnavailable` when the server does not
    /// share presence; other failures are tried again within 20 s.
    pub async fn presence_activity(&self, activity: PresenceActivity) -> Result<(), MtrxError> {
        let set = self.core().own_presence().activity(activity.into());
        run(set).await.map_err(Into::into)
    }

    /// "Share my presence", the app's device setting, as of the call; pass
    /// it before the first foreground activity. Returns once we are set
    /// offline, or online if in the foreground; fails as
    /// [`Session::presence_activity`].
    pub async fn set_presence_sharing(&self, on: bool) -> Result<(), MtrxError> {
        let set = self.core().own_presence().set_sharing(on);
        run(set).await.map_err(Into::into)
    }

    /// Call `listener` with the presence of `user_ids` as it changes: each
    /// is read about once a minute, spread over the minute.
    pub fn subscribe_presence(
        &self,
        user_ids: Vec<String>,
        listener: Arc<dyn PresenceListener>,
    ) -> Result<Arc<TaskHandle>, MtrxError> {
        let users = user_ids
            .iter()
            .map(|id| OwnedUserId::try_from(id.as_str()))
            .collect::<Result<Vec<_>, _>>()
            .map_err(MtrxError::invalid)?;
        let session = self.core();
        Ok(TaskHandle::spawn(async move {
            // Stops when the task is cancelled.
            let watcher = session.watch_presence(users);
            follow(watcher.updates(), listener).await;
        }))
    }
}

/// Call `listener` with all presences after each batch of updates.
async fn follow(
    updates: impl Stream<Item = core::Result<(OwnedUserId, core::Presence)>>,
    listener: Arc<dyn PresenceListener>,
) {
    let mut updates = std::pin::pin!(updates);
    let mut presences = HashMap::new();
    while let Some(mut update) = updates.next().await {
        loop {
            match update {
                Ok((user, presence)) => {
                    presences.insert(user.to_string(), Presence::from(presence));
                }
                Err(_) => {
                    listener.on_unavailable();
                    return;
                }
            }
            // Those ready now go in the same call.
            match updates.next().now_or_never() {
                Some(Some(next)) => update = next,
                _ => break,
            }
        }
        listener.on_update(presences.clone());
    }
}

#[cfg(test)]
mod tests {
    use std::{sync::Mutex, time::Duration};

    use super::*;

    #[derive(Default)]
    struct Calls(Mutex<Vec<Option<HashMap<String, Presence>>>>);

    impl PresenceListener for Calls {
        fn on_update(&self, presences: HashMap<String, Presence>) {
            self.0.lock().unwrap().push(Some(presences));
        }

        fn on_unavailable(&self) {
            self.0.lock().unwrap().push(None);
        }
    }

    fn core_presence(state: core::PresenceState) -> core::Presence {
        core::Presence {
            state,
            last_active: Some(SystemTime::UNIX_EPOCH + Duration::from_secs(5)),
            currently_active: state == core::PresenceState::Online,
            status_message: Some("Lunch".into()),
        }
    }

    fn user(name: &str) -> OwnedUserId {
        format!("@{name}:example.org").try_into().unwrap()
    }

    #[test]
    fn presences_keep_what_the_server_says() {
        let presence = Presence::from(core_presence(core::PresenceState::Unavailable));
        assert_eq!(
            presence,
            Presence {
                state: PresenceState::Unavailable,
                last_active: Some(SystemTime::UNIX_EPOCH + Duration::from_secs(5)),
                currently_active: false,
                status_message: Some("Lunch".into()),
                dot: Some(PresenceDot::Idle),
                line: Some("Lunch".into()),
            }
        );
        assert_eq!(
            core::Presence::from(presence),
            core_presence(core::PresenceState::Unavailable)
        );
        for state in [
            PresenceState::Online,
            PresenceState::Unavailable,
            PresenceState::Offline,
        ] {
            assert_eq!(PresenceState::from(core::PresenceState::from(state)), state);
        }
    }

    #[test]
    fn lines_go_stale_and_are_read_again() {
        let sent = SystemTime::UNIX_EPOCH + Duration::from_secs(1000);
        let offline = core::Presence {
            status_message: None,
            ..core_presence(core::PresenceState::Offline)
        };
        let presence = Presence::at(offline, sent);
        assert_eq!(presence.dot, None);
        assert_eq!(presence.line.as_deref(), Some("Last active 16 min ago"));
        let later = sent + Duration::from_secs(3 * 3600);
        assert_eq!(
            presence_line(presence, later).as_deref(),
            Some("Last active 3 hr ago")
        );
        assert_eq!(presence_dot_label(PresenceDot::Online), "Online");
        assert_eq!(presence_dot_label(PresenceDot::Idle), "Idle");
    }

    #[test]
    fn listeners_get_the_whole_map_per_batch_then_unavailable() {
        let calls = Arc::new(Calls::default());
        let online = core_presence(core::PresenceState::Online);
        let idle = core_presence(core::PresenceState::Unavailable);
        // Two ready at once, then one later, then the end.
        let batches = futures_util::stream::iter([
            Ok((user("a"), online.clone())),
            Ok((user("b"), online.clone())),
        ])
        .chain(
            futures_util::stream::once(async {
                tokio::task::yield_now().await;
                Ok((user("a"), idle.clone()))
            })
            .chain(futures_util::stream::once(async {
                tokio::task::yield_now().await;
                Err(core::Error::PresenceUnavailable("M_FORBIDDEN".into()))
            })),
        );
        tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap()
            .block_on(follow(batches, calls.clone()));
        let calls = calls.0.lock().unwrap();
        let map = |entries: &[(&str, &core::Presence)]| {
            Some(
                entries
                    .iter()
                    .map(|(name, p)| (user(name).to_string(), Presence::from((*p).clone())))
                    .collect::<HashMap<_, _>>(),
            )
        };
        assert_eq!(
            *calls,
            [
                map(&[("a", &online), ("b", &online)]),
                map(&[("a", &idle), ("b", &online)]),
                None,
            ]
        );
    }
}
