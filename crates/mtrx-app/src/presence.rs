//! Presence: a dot on people's pictures and a line under their name, from
//! one watcher for the people shown; and ours, which the core sets from the
//! window's input and from quitting (`docs/mobile.md` section 11 has the
//! rules the apps share).

use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
    time::Duration,
};

use futures_util::StreamExt;
use iced::{
    Border, Color, Element, Subscription, Task, Theme,
    widget::{container, space, stack},
};
use mtrx_core::{Activity, OwnedUserId, Presence, PresenceDot, Session, UserId};

use crate::app::Shared;

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
    /// Keyboard or mouse input in the window, or its focus changed.
    Input,
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
            Message::Input => {}
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

/// What the app is doing, which sets our presence (see
/// [`mtrx_core::OwnPresence`]).
pub fn activity(session: &Arc<Session>, activity: Activity) -> Task<Message> {
    reported(session.own_presence().activity(activity))
}

/// "Share my presence" turned on or off.
pub fn sharing(session: &Arc<Session>, on: bool) -> Task<Message> {
    reported(session.own_presence().set_sharing(on))
}

/// Pass "Share my presence" (`share`), then come to the foreground. Not
/// sharing, sync starts after, as its requests would say online.
pub fn start(session: &Arc<Session>, share: bool) -> Task<Message> {
    let own = session.own_presence();
    let (sharing, foreground) = (own.set_sharing(share), own.activity(Activity::Foreground));
    reported(async move {
        sharing.await?;
        foreground.await
    })
}

/// Setting ours; failures are logged, except that the server does not share
/// presence.
fn reported(set: impl Future<Output = mtrx_core::Result<()>> + Send + 'static) -> Task<Message> {
    Task::future(async move {
        match set.await {
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

/// Set us offline for `activity` (the window closes, or signing out),
/// waiting at most [`OFFLINE_WAIT`] for the server.
pub async fn go_offline(session: &Session, activity: Activity) {
    match tokio::time::timeout(OFFLINE_WAIT, session.own_presence().activity(activity)).await {
        Ok(Ok(())) => {}
        Ok(Err(e)) => tracing::debug!("could not set our presence offline: {e}"),
        Err(_) => tracing::debug!("setting our presence offline took too long"),
    }
}

/// The dot's colour: green online, amber idle.
pub fn dot_color(dot: PresenceDot) -> Color {
    match dot {
        PresenceDot::Online => ONLINE,
        PresenceDot::Idle => IDLE,
    }
}

/// The dot's diameter on a picture of `avatar` size, its ring included.
pub fn dot_size(avatar: f32) -> f32 {
    (avatar * 0.28 + 2.0).round()
}

/// `avatar` (`size` wide) with `presence`'s dot on its bottom right, in a
/// ring of the background, `ring`.
pub fn with_dot<'a, M: 'a>(
    avatar: Element<'a, M>,
    presence: Option<&Presence>,
    size: f32,
    ring: impl Fn(&Theme) -> Color + 'a,
) -> Element<'a, M> {
    let Some(color) = presence.and_then(Presence::dot).map(dot_color) else {
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
    use mtrx_core::PresenceState;

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
    fn dot_colours_and_sizes() {
        assert_eq!(dot_color(PresenceDot::Online), ONLINE);
        assert_eq!(dot_color(PresenceDot::Idle), IDLE);
        assert_eq!(dot_size(24.0), 9.0);
        assert_eq!(dot_size(32.0), 11.0);
        assert_eq!(dot_size(64.0), 20.0);
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
