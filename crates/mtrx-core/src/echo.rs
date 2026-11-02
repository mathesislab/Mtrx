//! Changes made here, shown in the room list at once: the server answers
//! before sync brings them back.

use std::{
    collections::HashMap,
    time::{Duration, Instant},
};

use matrix_sdk::ruma::{OwnedRoomId, RoomId};
use tokio::sync::watch;

use crate::{RoomSummary, RoomTag};

/// How long a change the server accepted may still differ from the room
/// list (sync lags behind the server's answer).
pub const ECHO_TIMEOUT: Duration = Duration::from_secs(10);

/// A value set here, until the room list has it.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Pending<T> {
    value: T,
    /// When the server accepted it.
    since: Option<Instant>,
}

/// The tags set here that the room list may not show yet.
pub(crate) struct Echoes {
    tags: watch::Sender<HashMap<OwnedRoomId, Pending<RoomTag>>>,
}

impl Default for Echoes {
    fn default() -> Self {
        Self {
            tags: watch::Sender::new(HashMap::new()),
        }
    }
}

impl Echoes {
    /// `room` is being tagged `tag`.
    pub(crate) fn set_tag(&self, room: &RoomId, tag: RoomTag) {
        let pending = Pending {
            value: tag,
            since: None,
        };
        self.tags.send_modify(|tags| {
            tags.insert(room.to_owned(), pending);
        });
    }

    /// The server answered tagging `room` `tag`: on failure the tag goes.
    pub(crate) fn tag_answered(&self, room: &RoomId, tag: RoomTag, ok: bool, now: Instant) {
        self.tags.send_if_modified(|tags| match tags.get_mut(room) {
            // A later change is under way.
            Some(pending) if pending.value != tag => false,
            Some(pending) if ok => {
                pending.since = Some(now);
                true
            }
            Some(_) => tags.remove(room).is_some(),
            None => false,
        });
    }

    /// Changes after every change.
    pub(crate) fn subscribe(&self) -> watch::Receiver<HashMap<OwnedRoomId, Pending<RoomTag>>> {
        self.tags.subscribe()
    }

    /// Lay the tags over `rooms`, first dropping those the rooms show, those
    /// of rooms gone and those the server accepted [`ECHO_TIMEOUT`] ago;
    /// returns when the next of those left times out.
    pub(crate) fn lay_over(&self, rooms: &mut [RoomSummary], now: Instant) -> Option<Instant> {
        self.tags.send_if_modified(|tags| {
            let before = tags.len();
            tags.retain(|id, pending| {
                let synced = rooms.iter().find(|r| &r.id == id).map(|r| r.tag);
                let timed_out = pending.since.is_some_and(|at| now >= at + ECHO_TIMEOUT);
                synced.is_some_and(|tag| tag != pending.value) && !timed_out
            });
            tags.len() != before
        });
        let tags = self.tags.borrow();
        for room in rooms.iter_mut() {
            if let Some(pending) = tags.get(&room.id) {
                room.tag = pending.value;
            }
        }
        tags.values()
            .filter_map(|pending| Some(pending.since? + ECHO_TIMEOUT))
            .min()
    }
}

#[cfg(test)]
mod tests {
    use matrix_sdk::ruma::owned_room_id;

    use super::*;
    use crate::{RoomKind, spaces::tests::room};

    #[test]
    fn a_tag_shows_until_synced_failed_or_timed_out() {
        let echoes = Echoes::default();
        let (a, b, c) = (
            owned_room_id!("!a:x"),
            owned_room_id!("!b:x"),
            owned_room_id!("!c:x"),
        );
        let plain = |id: &OwnedRoomId| room(id.as_str(), RoomKind::Room, &[]);
        let mut rooms = vec![plain(&a), plain(&b), plain(&c)];
        let now = Instant::now();
        echoes.set_tag(&a, RoomTag::Favourite);
        echoes.set_tag(&b, RoomTag::LowPriority);
        echoes.set_tag(&c, RoomTag::Favourite);
        // Shown while the requests run, with no time out.
        let mut shown = rooms.clone();
        assert_eq!(echoes.lay_over(&mut shown, now), None);
        let tags: Vec<_> = shown.iter().map(|r| r.tag).collect();
        assert_eq!(
            tags,
            [RoomTag::Favourite, RoomTag::LowPriority, RoomTag::Favourite]
        );

        // Failed: gone. Accepted: until synced or timed out. An answer to
        // an earlier change leaves a later one.
        echoes.tag_answered(&b, RoomTag::LowPriority, false, now);
        echoes.tag_answered(&a, RoomTag::Favourite, true, now);
        echoes.tag_answered(&c, RoomTag::LowPriority, true, now);
        let mut shown = rooms.clone();
        assert_eq!(echoes.lay_over(&mut shown, now), Some(now + ECHO_TIMEOUT));
        let tags: Vec<_> = shown.iter().map(|r| r.tag).collect();
        assert_eq!(
            tags,
            [RoomTag::Favourite, RoomTag::None, RoomTag::Favourite]
        );

        rooms[2].tag = RoomTag::Favourite;
        let mut shown = rooms.clone();
        echoes.lay_over(&mut shown, now);
        assert!(!echoes.tags.borrow().contains_key(&c));
        let mut shown = rooms.clone();
        assert_eq!(echoes.lay_over(&mut shown, now + ECHO_TIMEOUT), None);
        assert_eq!(shown[0].tag, RoomTag::None);
        assert!(echoes.tags.borrow().is_empty());
    }

    #[test]
    fn a_room_gone_drops_its_tag() {
        let echoes = Echoes::default();
        echoes.set_tag(&owned_room_id!("!a:x"), RoomTag::Favourite);
        echoes.lay_over(&mut [], Instant::now());
        assert!(echoes.tags.borrow().is_empty());
    }
}
