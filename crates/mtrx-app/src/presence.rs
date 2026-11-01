//! Presence: a dot on people's pictures and a line under their name, from
//! one watcher for the people shown; and ours, from the window's focus and
//! input (`docs/mobile.md` section 11 has the rules the apps share).

use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
    time::{Duration, Instant, SystemTime},
};

use futures_util::StreamExt;
use iced::{
    Border, Color, Element, Subscription, Task, Theme,
    widget::{container, space, stack},
};
use mtrx_core::{OwnedUserId, Presence, PresenceState, Session, UserId, describe_last_active};

use crate::app::Shared;

/// Without input this long (and the window not focused), we are idle.
pub const IDLE_AFTER: Duration = Duration::from_secs(5 * 60);
/// How often idleness is checked while it can begin.
pub const IDLE_CHECK: Duration = Duration::from_secs(15);
/// The people shown are followed once they stayed the same this long, so
/// that switching rooms or spaces quickly starts one watcher, not many.
pub const SETTLE: Duration = Duration::from_secs(1);
/// How long quitting or signing out waits for the server to hear that we
/// are offline.
pub const OFFLINE_WAIT: Duration = Duration::from_secs(2);

const ONLINE: Color = iced::color!(0x2bb673);
const IDLE: Color = iced::color!(0xf2a51a);

#[derive(Debug, Clone)]
pub enum Message {
    /// Someone's presence, from the watcher or a lookup.
    Changed(OwnedUserId, Presence),
    /// The server does not share presence.
    Unavailable,
    /// The people shown may have settled: follow them if this is still the
    /// latest change.
    Settle(u64),
    /// Keyboard or mouse input in the window.
    Input,
    /// Check whether we became idle.
    CheckIdle,
}

/// Others' presence, as far as known, and who is followed.
#[derive(Default)]
pub struct Presences {
    known: HashMap<OwnedUserId, Presence>,
    /// The server does not share presence: nothing is shown or followed
    /// for the rest of the session.
    unavailable: bool,
    /// Followed by the watcher, in the order they are shown.
    watched: Vec<OwnedUserId>,
    /// Shown now; followed once settled.
    wanted: Vec<OwnedUserId>,
    /// Counts changes of `wanted`.
    generation: u64,
}

impl Presences {
    /// `user`'s presence, unless unknown or the server does not share it.
    pub fn get(&self, user: &UserId) -> Option<&Presence> {
        self.known.get(user)
    }

    pub fn unavailable(&self) -> bool {
        self.unavailable
    }

    /// Someone's presence is followed, so lines saying how long ago need
    /// refreshing.
    pub fn following(&self) -> bool {
        !self.watched.is_empty()
    }

    /// The people shown are now `people`, in the order shown; a changed set
    /// is followed after [`SETTLE`]: the generation to settle then.
    pub fn show(&mut self, people: Vec<OwnedUserId>) -> Option<u64> {
        if self.unavailable || same_people(&people, &self.wanted) {
            return None;
        }
        self.wanted = people;
        self.generation += 1;
        Some(self.generation)
    }

    /// Others' presence and who is followed; our own is the app's.
    pub fn update(&mut self, message: Message) {
        match message {
            Message::Changed(user, presence) => {
                if !self.unavailable {
                    self.known.insert(user, presence);
                }
            }
            Message::Unavailable => {
                *self = Self {
                    unavailable: true,
                    ..Self::default()
                };
            }
            Message::Settle(generation) => {
                if generation == self.generation && !same_people(&self.wanted, &self.watched) {
                    self.watched = self.wanted.clone();
                }
            }
            Message::Input | Message::CheckIdle => {}
        }
    }

    /// One watcher for the people followed; it ends, and reports
    /// [`Message::Unavailable`], if the server does not share presence.
    pub fn subscription(&self, session: &Arc<Session>) -> Option<Subscription<Message>> {
        if self.unavailable || self.watched.is_empty() {
            return None;
        }
        Some(Subscription::run_with(
            (Shared(session.clone()), self.watched.clone()),
            |(session, users)| {
                let watcher = session.0.watch_presence(users.iter().cloned());
                watcher.updates().filter_map(move |item| {
                    // The stream keeps the watcher, which stops when dropped.
                    let _ = &watcher;
                    std::future::ready(match item {
                        Ok((user, presence)) => Some(Message::Changed(user, presence)),
                        Err(mtrx_core::Error::PresenceUnavailable(_)) => Some(Message::Unavailable),
                        Err(_) => None,
                    })
                })
            },
        ))
    }
}

/// Whether `a` and `b` list the same people, in any order.
fn same_people(a: &[OwnedUserId], b: &[OwnedUserId]) -> bool {
    a.len() == b.len() && a.iter().collect::<HashSet<_>>() == b.iter().collect::<HashSet<_>>()
}

/// Read `user`'s presence once, e.g. for their card.
pub fn look_up(session: &Arc<Session>, user: OwnedUserId) -> Task<Message> {
    let session = session.clone();
    Task::future(async move {
        match session.presence(&user).await {
            Ok(presence) => Some(Message::Changed(user, presence)),
            Err(mtrx_core::Error::PresenceUnavailable(_)) => Some(Message::Unavailable),
            // Hidden from us, or the server is unreachable: shown as before.
            Err(_) => None,
        }
    })
    .and_then(Task::done)
}

/// Set our presence; failures are logged, except that the server does not
/// share presence.
pub fn set_own(session: &Arc<Session>, state: PresenceState) -> Task<Message> {
    let session = session.clone();
    Task::future(async move {
        match session.set_presence(state, None).await {
            Ok(()) => None,
            Err(mtrx_core::Error::PresenceUnavailable(_)) => Some(Message::Unavailable),
            Err(e) => {
                tracing::warn!("could not set our presence: {e}");
                None
            }
        }
    })
    .and_then(Task::done)
}

/// Set us offline, waiting at most [`OFFLINE_WAIT`] for the server.
pub async fn go_offline(session: &Session) {
    match tokio::time::timeout(
        OFFLINE_WAIT,
        session.set_presence(PresenceState::Offline, None),
    )
    .await
    {
        Ok(Ok(())) => {}
        Ok(Err(e)) => tracing::debug!("could not set our presence offline: {e}"),
        Err(_) => tracing::debug!("setting our presence offline took too long"),
    }
}

/// Our presence as the window's focus and input say: online while the
/// window is focused or had input (or focus) in the last [`IDLE_AFTER`],
/// else idle.
pub struct Activity {
    focused: bool,
    last_input: Instant,
    /// What we set last.
    set: PresenceState,
}

impl Activity {
    /// Starting: online.
    pub fn new(focused: bool, now: Instant) -> Self {
        Self {
            focused,
            last_input: now,
            set: PresenceState::Online,
        }
    }

    /// What we set last.
    pub fn state(&self) -> PresenceState {
        self.set
    }

    /// Input in the window: the presence to set, if it changes.
    pub fn input(&mut self, now: Instant) -> Option<PresenceState> {
        self.last_input = now;
        self.change(PresenceState::Online)
    }

    /// The window gained or lost the focus; until then it was in use.
    pub fn focus(&mut self, focused: bool, now: Instant) -> Option<PresenceState> {
        self.focused = focused;
        self.input(now)
    }

    /// The clock: the presence to set, if we became idle.
    pub fn check(&mut self, now: Instant) -> Option<PresenceState> {
        let idle = !self.focused && now.duration_since(self.last_input) >= IDLE_AFTER;
        if idle {
            self.change(PresenceState::Unavailable)
        } else {
            None
        }
    }

    /// Whether [`Activity::check`] can change anything now.
    pub fn may_idle(&self) -> bool {
        !self.focused && self.set == PresenceState::Online
    }

    /// Sharing was turned on: online again, from now.
    pub fn restart(&mut self, now: Instant) -> PresenceState {
        self.last_input = now;
        self.set = PresenceState::Online;
        self.set
    }

    fn change(&mut self, to: PresenceState) -> Option<PresenceState> {
        (self.set != to).then(|| {
            self.set = to;
            to
        })
    }
}

/// The dot's colour: green online, amber idle, none offline.
pub fn dot_color(state: PresenceState) -> Option<Color> {
    match state {
        PresenceState::Online => Some(ONLINE),
        PresenceState::Unavailable => Some(IDLE),
        PresenceState::Offline => None,
    }
}

/// The dot's diameter on a picture of `avatar` size, its ring included.
pub fn dot_size(avatar: f32) -> f32 {
    (avatar * 0.28 + 2.0).round()
}

/// The line under someone's name: their status message, else "Active
/// now", "Idle" or when they were last active; nothing if offline with
/// nothing more known.
pub fn line(presence: &Presence, now: SystemTime) -> Option<String> {
    if let Some(message) = &presence.status_message {
        return Some(message.clone());
    }
    match presence.state {
        PresenceState::Online => Some("Active now".into()),
        PresenceState::Unavailable => Some("Idle".into()),
        PresenceState::Offline => presence
            .last_active
            .map(|at| format!("Last active {}", describe_last_active(at, now))),
    }
}

/// `avatar` (`size` wide) with `presence`'s dot on its bottom right, in a
/// ring of the background, `ring`.
pub fn with_dot<'a, M: 'a>(
    avatar: Element<'a, M>,
    presence: Option<&Presence>,
    size: f32,
    ring: impl Fn(&Theme) -> Color + 'a,
) -> Element<'a, M> {
    let Some(color) = presence.and_then(|p| dot_color(p.state)) else {
        return avatar;
    };
    let diameter = dot_size(size);
    let dot = container(space())
        .width(diameter)
        .height(diameter)
        .style(move |theme| container::Style {
            background: Some(color.into()),
            border: Border::default()
                .rounded(diameter / 2.0)
                .width(2)
                .color(ring(theme)),
            ..Default::default()
        });
    stack![avatar, container(dot).align_right(size).align_bottom(size)].into()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn presence(state: PresenceState) -> Presence {
        Presence {
            state,
            last_active: None,
            currently_active: false,
            status_message: None,
        }
    }

    fn user(id: &str) -> OwnedUserId {
        id.try_into().unwrap()
    }

    #[test]
    fn dot_per_state() {
        assert_eq!(dot_color(PresenceState::Online), Some(ONLINE));
        assert_eq!(dot_color(PresenceState::Unavailable), Some(IDLE));
        assert_eq!(dot_color(PresenceState::Offline), None);
        assert_eq!(dot_size(24.0), 9.0);
        assert_eq!(dot_size(32.0), 11.0);
        assert_eq!(dot_size(64.0), 20.0);
    }

    #[test]
    fn line_per_state_and_age() {
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000);
        let ago = |seconds| Some(now - Duration::from_secs(seconds));
        assert_eq!(
            line(&presence(PresenceState::Online), now).as_deref(),
            Some("Active now")
        );
        assert_eq!(
            line(&presence(PresenceState::Unavailable), now).as_deref(),
            Some("Idle")
        );
        // Synapse with presence off: offline, nothing else.
        assert_eq!(line(&presence(PresenceState::Offline), now), None);
        let offline = |last_active| Presence {
            last_active,
            ..presence(PresenceState::Offline)
        };
        assert_eq!(
            line(&offline(ago(20)), now).as_deref(),
            Some("Last active just now")
        );
        assert_eq!(
            line(&offline(ago(5 * 60)), now).as_deref(),
            Some("Last active 5 min ago")
        );
        assert_eq!(
            line(&offline(ago(2 * 3600 + 59)), now).as_deref(),
            Some("Last active 2 hr ago")
        );
        assert_eq!(
            line(&offline(ago(3 * 86400)), now).as_deref(),
            Some("Last active 3 days ago")
        );
        // A status message comes first, whatever the state.
        for state in [
            PresenceState::Online,
            PresenceState::Unavailable,
            PresenceState::Offline,
        ] {
            let busy = Presence {
                status_message: Some("In a meeting".into()),
                last_active: ago(600),
                ..presence(state)
            };
            assert_eq!(line(&busy, now).as_deref(), Some("In a meeting"));
        }
    }

    #[test]
    fn idle_after_five_minutes_without_input_unless_focused() {
        let start = Instant::now();
        let at = |seconds| start + Duration::from_secs(seconds);
        let mut activity = Activity::new(true, start);
        assert_eq!(activity.state(), PresenceState::Online);
        // Focused: online however long without input.
        assert!(!activity.may_idle());
        assert_eq!(activity.check(at(3600)), None);

        // Unfocused: the focus counts as use until it is lost.
        assert_eq!(activity.focus(false, at(3600)), None);
        assert!(activity.may_idle());
        assert_eq!(activity.check(at(3600 + 299)), None);
        // Input over the unfocused window starts the 5 minutes anew.
        assert_eq!(activity.input(at(3600 + 299)), None);
        assert_eq!(activity.check(at(3600 + 598)), None);
        assert_eq!(
            activity.check(at(3600 + 599)),
            Some(PresenceState::Unavailable)
        );
        // Set once, not on every check.
        assert_eq!(activity.check(at(3600 + 600)), None);
        assert!(!activity.may_idle());
        assert_eq!(activity.state(), PresenceState::Unavailable);

        // The next input sets online again, once.
        assert_eq!(activity.input(at(4500)), Some(PresenceState::Online));
        assert_eq!(activity.input(at(4501)), None);
        assert_eq!(activity.check(at(4800)), None);
        assert_eq!(activity.check(at(4801)), Some(PresenceState::Unavailable));
        // So does the focus.
        assert_eq!(activity.focus(true, at(5000)), Some(PresenceState::Online));
        assert_eq!(activity.check(at(9000)), None);
    }

    #[test]
    fn sharing_again_is_online_from_then() {
        let start = Instant::now();
        let mut activity = Activity::new(false, start);
        let later = start + IDLE_AFTER;
        assert_eq!(activity.check(later), Some(PresenceState::Unavailable));
        assert_eq!(activity.restart(later), PresenceState::Online);
        assert_eq!(activity.check(later + IDLE_AFTER / 2), None);
    }

    #[test]
    fn follows_the_people_shown_once_settled() {
        let (bob, carol, dan) = (user("@bob:x"), user("@carol:x"), user("@dan:x"));
        let mut presences = Presences::default();
        assert!(!presences.following());
        let first = presences.show(vec![bob.clone(), carol.clone()]).unwrap();
        // Shown again in another order: no change.
        assert_eq!(presences.show(vec![carol.clone(), bob.clone()]), None);
        assert!(!presences.following(), "not before settling");
        presences.update(Message::Settle(first));
        assert_eq!(presences.watched, [bob.clone(), carol.clone()]);

        // Changes quickly one after the other: only the last is followed.
        let second = presences.show(vec![bob.clone()]).unwrap();
        let third = presences
            .show(vec![bob.clone(), carol.clone(), dan.clone()])
            .unwrap();
        presences.update(Message::Settle(second));
        assert_eq!(presences.watched, [bob.clone(), carol.clone()]);
        presences.update(Message::Settle(third));
        assert_eq!(presences.watched, [bob.clone(), carol.clone(), dan.clone()]);

        // Back to the same people before settling: the watcher stays.
        let fourth = presences.show(vec![bob.clone()]).unwrap();
        let fifth = presences
            .show(vec![dan.clone(), carol.clone(), bob.clone()])
            .unwrap();
        presences.update(Message::Settle(fourth));
        presences.update(Message::Settle(fifth));
        assert_eq!(presences.watched, [bob.clone(), carol.clone(), dan.clone()]);
    }

    #[test]
    fn hides_everything_once_unavailable() {
        let bob = user("@bob:x");
        let mut presences = Presences::default();
        let shown = presences.show(vec![bob.clone()]).unwrap();
        presences.update(Message::Settle(shown));
        presences.update(Message::Changed(
            bob.clone(),
            presence(PresenceState::Online),
        ));
        assert!(presences.get(&bob).is_some());

        presences.update(Message::Unavailable);
        assert!(presences.unavailable());
        assert!(presences.get(&bob).is_none());
        assert!(!presences.following());
        // For the rest of the session.
        assert_eq!(presences.show(vec![user("@carol:x")]), None);
        presences.update(Message::Changed(
            bob.clone(),
            presence(PresenceState::Online),
        ));
        assert!(presences.get(&bob).is_none());
    }
}
