//! Changes made here, shown at once in the room lists and the timelines'
//! rows: the server answers before sync brings them back.

use std::{
    collections::{BTreeSet, HashMap},
    time::{Duration, Instant},
};

use matrix_sdk::ruma::{OwnedEventId, OwnedRoomId, RoomId};
use tokio::sync::watch;

use crate::{Membership, RoomSummary, RoomTag};

/// How long a change the server accepted may still differ from what sync
/// brought (sync lags behind the server's answer).
pub const ECHO_TIMEOUT: Duration = Duration::from_secs(10);

/// A value set here, until sync brings it.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Pending<T> {
    value: T,
    /// When the server accepted it.
    since: Option<Instant>,
}

impl<T: PartialEq> Pending<T> {
    fn new(value: T) -> Self {
        Self { value, since: None }
    }

    /// Whether it still shows over `synced`, the room's value as synced
    /// (none once the room is gone): until synced, or timed out.
    fn shows_over(&self, synced: Option<&T>, now: Instant) -> bool {
        let timed_out = self.timeout().is_some_and(|at| now >= at);
        synced.is_some_and(|synced| *synced != self.value) && !timed_out
    }

    /// [`ECHO_TIMEOUT`] after the server accepted it.
    fn timeout(&self) -> Option<Instant> {
        Some(self.since? + ECHO_TIMEOUT)
    }
}

/// The values of one kind set here, by room.
type Pendings<T> = HashMap<OwnedRoomId, Pending<T>>;

/// The server answered setting `room`'s value to `value`: on failure it
/// goes. Returns whether that changed anything.
fn answer<T: PartialEq<U>, U: ?Sized>(
    pendings: &mut Pendings<T>,
    room: &RoomId,
    value: &U,
    ok: bool,
    now: Instant,
) -> bool {
    match pendings.get_mut(room) {
        // A later change is under way.
        Some(pending) if pending.value != *value => false,
        Some(pending) if ok => {
            pending.since = Some(now);
            true
        }
        Some(_) => pendings.remove(room).is_some(),
        None => false,
    }
}

/// Show the value set here for `room`, if any, as `shown`.
fn lay<T: Clone>(pendings: &Pendings<T>, room: &RoomId, shown: &mut T) {
    if let Some(pending) = pendings.get(room) {
        *shown = pending.value.clone();
    }
}

/// The changes made here that sync has not brought yet.
#[derive(Clone, Debug, Default)]
pub(crate) struct Changes {
    tags: Pendings<RoomTag>,
    /// Without a topic: none.
    topics: Pendings<Option<String>>,
    pinned: Pendings<Vec<OwnedEventId>>,
    /// Rooms joined or made here that the room list does not list as
    /// joined yet.
    joining: BTreeSet<OwnedRoomId>,
    /// Rooms asked to join here that the room list does not list yet.
    knocking: BTreeSet<OwnedRoomId>,
}

impl Changes {
    fn count(&self) -> usize {
        let rooms = self.joining.len() + self.knocking.len();
        self.tags.len() + self.topics.len() + self.pinned.len() + rooms
    }

    /// When the next of the changes the server accepted times out.
    fn timeout(&self) -> Option<Instant> {
        let tags = self.tags.values().filter_map(Pending::timeout);
        let topics = self.topics.values().filter_map(Pending::timeout);
        let pinned = self.pinned.values().filter_map(Pending::timeout);
        tags.chain(topics).chain(pinned).min()
    }
}

/// Changes made here, until the room list and the timelines show them.
pub(crate) struct Echoes {
    changes: watch::Sender<Changes>,
}

impl Default for Echoes {
    fn default() -> Self {
        Self {
            changes: watch::Sender::new(Changes::default()),
        }
    }
}

impl Echoes {
    /// `room` is being tagged `tag`.
    pub(crate) fn set_tag(&self, room: &RoomId, tag: RoomTag) {
        self.changes.send_modify(|changes| {
            changes.tags.insert(room.to_owned(), Pending::new(tag));
        });
    }

    /// The server answered tagging `room` `tag`: on failure the tag goes.
    pub(crate) fn tag_answered(&self, room: &RoomId, tag: RoomTag, ok: bool, now: Instant) {
        (self.changes).send_if_modified(|changes| answer(&mut changes.tags, room, &tag, ok, now));
    }

    /// `room`'s topic is being set to `topic` (none to remove it).
    pub(crate) fn set_topic(&self, room: &RoomId, topic: Option<String>) {
        self.changes.send_modify(|changes| {
            changes.topics.insert(room.to_owned(), Pending::new(topic));
        });
    }

    /// The server answered setting `room`'s topic: on failure it goes.
    pub(crate) fn topic_answered(
        &self,
        room: &RoomId,
        topic: &Option<String>,
        ok: bool,
        now: Instant,
    ) {
        (self.changes)
            .send_if_modified(|changes| answer(&mut changes.topics, room, topic, ok, now));
    }

    /// `room`'s pinned messages being set here, if any: what the next
    /// change starts from.
    pub(crate) fn pending_pinned(&self, room: &RoomId) -> Option<Vec<OwnedEventId>> {
        let changes = self.changes.borrow();
        changes
            .pinned
            .get(room)
            .map(|pending| pending.value.clone())
    }

    /// `room`'s pinned messages are being set to `pinned`.
    pub(crate) fn set_pinned(&self, room: &RoomId, pinned: Vec<OwnedEventId>) {
        self.changes.send_modify(|changes| {
            changes.pinned.insert(room.to_owned(), Pending::new(pinned));
        });
    }

    /// The server answered setting `room`'s pinned messages: on failure
    /// they go.
    pub(crate) fn pinned_answered(
        &self,
        room: &RoomId,
        pinned: &[OwnedEventId],
        ok: bool,
        now: Instant,
    ) {
        (self.changes)
            .send_if_modified(|changes| answer(&mut changes.pinned, room, pinned, ok, now));
    }

    /// `room` is being joined or was made: coming until the room list
    /// lists it as joined.
    pub(crate) fn set_joining(&self, room: &RoomId) {
        self.changes
            .send_if_modified(|changes| changes.joining.insert(room.to_owned()));
    }

    /// Joining `room` failed, or we left it: it is not coming.
    pub(crate) fn drop_joining(&self, room: &RoomId) {
        self.changes
            .send_if_modified(|changes| changes.joining.remove(room));
    }

    /// `room` is being asked to join: asked until the room list lists it.
    pub(crate) fn set_knocking(&self, room: &RoomId) {
        self.changes
            .send_if_modified(|changes| changes.knocking.insert(room.to_owned()));
    }

    /// Asking to join `room` failed.
    pub(crate) fn drop_knocking(&self, room: &RoomId) {
        self.changes
            .send_if_modified(|changes| changes.knocking.remove(room));
    }

    /// Changes after every change.
    pub(crate) fn subscribe(&self) -> watch::Receiver<Changes> {
        self.changes.subscribe()
    }

    /// Lay the changes over `rooms`, first dropping those the rooms show,
    /// those of rooms gone, those the server accepted [`ECHO_TIMEOUT`] ago,
    /// the rooms coming that are listed as joined and those asked to join
    /// that are listed; returns when the next of those left times out.
    pub(crate) fn lay_over(&self, rooms: &mut [RoomSummary], now: Instant) -> Option<Instant> {
        self.changes.send_if_modified(|changes| {
            let synced = |id: &OwnedRoomId| rooms.iter().find(|r| &r.id == id);
            let before = changes.count();
            (changes.tags).retain(|id, p| p.shows_over(synced(id).map(|r| &r.tag), now));
            (changes.topics).retain(|id, p| p.shows_over(synced(id).map(|r| &r.topic), now));
            (changes.pinned).retain(|id, p| p.shows_over(synced(id).map(|r| &r.pinned), now));
            let coming = |r: &RoomSummary| r.membership != Membership::Joined;
            (changes.joining).retain(|id| synced(id).is_none_or(coming));
            (changes.knocking).retain(|id| synced(id).is_none());
            changes.count() != before
        });
        let changes = self.changes.borrow();
        for room in rooms.iter_mut() {
            lay(&changes.tags, &room.id, &mut room.tag);
            lay(&changes.topics, &room.id, &mut room.topic);
            lay(&changes.pinned, &room.id, &mut room.pinned);
        }
        changes.timeout()
    }

    /// The rooms joined or made here that the room list did not list as
    /// joined when last laid over.
    pub(crate) fn joining(&self) -> BTreeSet<OwnedRoomId> {
        self.changes.borrow().joining.clone()
    }

    /// The rooms asked to join here that the room list did not list when
    /// last laid over.
    pub(crate) fn knocking(&self) -> BTreeSet<OwnedRoomId> {
        self.changes.borrow().knocking.clone()
    }

    /// `room`'s pinned messages as shown: those set here until `synced`
    /// has them or they time out (then dropped), else `synced`; and when
    /// those set here time out.
    pub(crate) fn pinned(
        &self,
        room: &RoomId,
        synced: Vec<OwnedEventId>,
        now: Instant,
    ) -> (Vec<OwnedEventId>, Option<Instant>) {
        self.changes.send_if_modified(|changes| {
            let shows = (changes.pinned.get(room)).is_none_or(|p| p.shows_over(Some(&synced), now));
            !shows && changes.pinned.remove(room).is_some()
        });
        match self.changes.borrow().pinned.get(room) {
            Some(pending) => (pending.value.clone(), pending.timeout()),
            None => (synced, None),
        }
    }
}

#[cfg(test)]
mod tests {
    use matrix_sdk::ruma::{owned_event_id, owned_room_id};

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
        let plain = |id: &OwnedRoomId| room(id.as_str(), RoomKind::Room);
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
        assert!(!echoes.changes.borrow().tags.contains_key(&c));
        let mut shown = rooms.clone();
        assert_eq!(echoes.lay_over(&mut shown, now + ECHO_TIMEOUT), None);
        assert_eq!(shown[0].tag, RoomTag::None);
        assert!(echoes.changes.borrow().tags.is_empty());
    }

    #[test]
    fn a_room_gone_drops_its_changes() {
        let echoes = Echoes::default();
        let a = owned_room_id!("!a:x");
        echoes.set_tag(&a, RoomTag::Favourite);
        echoes.set_topic(&a, Some("About.".into()));
        echoes.set_pinned(&a, vec![owned_event_id!("$1")]);
        echoes.lay_over(&mut [], Instant::now());
        assert_eq!(echoes.changes.borrow().count(), 0);
    }

    #[test]
    fn a_topic_shows_until_synced_or_failed() {
        let echoes = Echoes::default();
        let (a, b) = (owned_room_id!("!a:x"), owned_room_id!("!b:x"));
        let mut rooms = vec![
            room(a.as_str(), RoomKind::Room),
            room(b.as_str(), RoomKind::Room),
        ];
        rooms[1].topic = Some("Old".into());
        let now = Instant::now();
        let new = Some("New".into());
        echoes.set_topic(&a, new.clone());
        echoes.set_topic(&b, None);
        let mut shown = rooms.clone();
        echoes.lay_over(&mut shown, now);
        assert_eq!(shown[0].topic, new);
        assert_eq!(shown[1].topic, None);

        echoes.topic_answered(&a, &new, true, now);
        echoes.topic_answered(&b, &None, false, now);
        let mut shown = rooms.clone();
        assert_eq!(echoes.lay_over(&mut shown, now), Some(now + ECHO_TIMEOUT));
        assert_eq!(shown[0].topic, new);
        assert_eq!(shown[1].topic.as_deref(), Some("Old"));

        rooms[0].topic = new.clone();
        echoes.lay_over(&mut rooms.clone(), now);
        assert!(echoes.changes.borrow().topics.is_empty());
    }

    #[test]
    fn pinned_messages_show_in_the_list_and_the_rows_until_synced() {
        let echoes = Echoes::default();
        let a = owned_room_id!("!a:x");
        let (one, two) = (owned_event_id!("$1"), owned_event_id!("$2"));
        let mut rooms = vec![room(a.as_str(), RoomKind::Room)];
        rooms[0].pinned = vec![one.clone()];
        let now = Instant::now();
        assert_eq!(echoes.pending_pinned(&a), None);
        let pinned = vec![one.clone(), two.clone()];
        echoes.set_pinned(&a, pinned.clone());
        assert_eq!(echoes.pending_pinned(&a), Some(pinned.clone()));
        let mut shown = rooms.clone();
        echoes.lay_over(&mut shown, now);
        assert_eq!(shown[0].pinned, pinned);
        assert_eq!(
            echoes.pinned(&a, vec![one.clone()], now),
            (pinned.clone(), None)
        );

        // Accepted: until synced, or timed out.
        echoes.pinned_answered(&a, &pinned, true, now);
        assert_eq!(
            echoes.pinned(&a, vec![one.clone()], now),
            (pinned.clone(), Some(now + ECHO_TIMEOUT))
        );
        assert_eq!(
            echoes.pinned(&a, vec![one.clone()], now + ECHO_TIMEOUT),
            (vec![one.clone()], None)
        );
        assert_eq!(echoes.pending_pinned(&a), None);
        echoes.set_pinned(&a, vec![]);
        assert_eq!(echoes.pinned(&a, vec![], now), (vec![], None));
        assert_eq!(echoes.pending_pinned(&a), None);
    }

    #[test]
    fn rooms_joined_are_coming_until_listed_as_joined() {
        let echoes = Echoes::default();
        let (made, invite, failed, left) = (
            owned_room_id!("!made:x"),
            owned_room_id!("!invite:x"),
            owned_room_id!("!failed:x"),
            owned_room_id!("!left:x"),
        );
        for id in [&made, &invite, &failed, &left] {
            echoes.set_joining(id);
        }
        echoes.drop_joining(&failed);
        echoes.drop_joining(&left);
        let mut rooms = vec![room(invite.as_str(), RoomKind::Room)];
        rooms[0].membership = Membership::Invited;
        echoes.lay_over(&mut rooms, Instant::now());
        assert_eq!(
            echoes.joining(),
            BTreeSet::from([invite.clone(), made.clone()])
        );

        // Listed as joined: here, no longer coming.
        rooms[0].membership = Membership::Joined;
        rooms.push(room(made.as_str(), RoomKind::Space));
        echoes.lay_over(&mut rooms, Instant::now());
        assert!(echoes.joining().is_empty());
    }

    #[test]
    fn rooms_asked_to_join_are_asked_until_listed() {
        let echoes = Echoes::default();
        let (asked, failed) = (owned_room_id!("!asked:x"), owned_room_id!("!failed:x"));
        echoes.set_knocking(&asked);
        echoes.set_knocking(&failed);
        echoes.drop_knocking(&failed);
        echoes.lay_over(&mut [], Instant::now());
        assert_eq!(echoes.knocking(), BTreeSet::from([asked.clone()]));

        // Listed, as asked or let in: the list has it.
        let mut rooms = vec![room(asked.as_str(), RoomKind::Room)];
        rooms[0].membership = Membership::Asked;
        echoes.lay_over(&mut rooms, Instant::now());
        assert!(echoes.knocking().is_empty());
    }
}
