//! Changes made here, shown at once in the room lists, the timelines' rows
//! and the rooms' settings: the server answers before sync brings them
//! back. Rooms read here show as read while the receipt waits for the
//! server.

use std::{
    borrow::Borrow,
    collections::{BTreeSet, HashMap},
    hash::Hash,
    time::{Duration, Instant},
};

use matrix_sdk::{
    Room,
    ruma::{OwnedEventId, OwnedRoomAliasId, OwnedRoomId, OwnedUserId, RoomId},
};
use tokio::sync::watch;

use crate::{
    History, JoinRule, Membership, NotifyMode, Permissions, RoomSettings, RoomSummary, RoomTag,
    SpaceLink, Topic,
};

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
        synced.is_some_and(|synced| *synced != self.value) && !self.timed_out(now)
    }

    /// [`ECHO_TIMEOUT`] after the server accepted it.
    fn timeout(&self) -> Option<Instant> {
        Some(self.since? + ECHO_TIMEOUT)
    }

    fn timed_out(&self, now: Instant) -> bool {
        self.timeout().is_some_and(|at| now >= at)
    }
}

/// The values of one kind set here, by room (or by what else `K` says).
type Pendings<T, K = OwnedRoomId> = HashMap<K, Pending<T>>;

/// The server answered setting the value under `key` to `value`: on
/// failure it goes. Returns whether that changed anything.
fn answer<K, Q, T, U>(
    pendings: &mut Pendings<T, K>,
    key: &Q,
    value: &U,
    ok: bool,
    now: Instant,
) -> bool
where
    K: Borrow<Q> + Hash + Eq,
    Q: Hash + Eq + ?Sized,
    T: PartialEq<U>,
    U: ?Sized,
{
    match pendings.get_mut(key) {
        // A later change is under way.
        Some(pending) if pending.value != *value => false,
        Some(pending) if ok => {
            pending.since = Some(now);
            true
        }
        Some(_) => pendings.remove(key).is_some(),
        None => false,
    }
}

/// A room's setting changed here, with its new value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Setting {
    JoinRule(JoinRule),
    History(History),
    Address(Option<OwnedRoomAliasId>),
    Listed(bool),
    Permissions(Permissions),
    Notify(NotifyMode),
    Encrypted(bool),
    /// Listed in the space, suggested or not; or not (none).
    Link(OwnedRoomId, Option<bool>),
    /// Someone banned, or not.
    Banned(OwnedUserId, bool),
}

/// Which setting a [`Setting`] is.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) enum SettingKey {
    JoinRule,
    History,
    Address,
    Listed,
    Permissions,
    Notify,
    Encrypted,
    Link(OwnedRoomId),
    Banned(OwnedUserId),
}

impl Setting {
    fn key(&self) -> SettingKey {
        match self {
            Self::JoinRule(_) => SettingKey::JoinRule,
            Self::History(_) => SettingKey::History,
            Self::Address(_) => SettingKey::Address,
            Self::Listed(_) => SettingKey::Listed,
            Self::Permissions(_) => SettingKey::Permissions,
            Self::Notify(_) => SettingKey::Notify,
            Self::Encrypted(_) => SettingKey::Encrypted,
            Self::Link(space, _) => SettingKey::Link(space.clone()),
            Self::Banned(user, _) => SettingKey::Banned(user.clone()),
        }
    }

    /// Show it in `settings`.
    fn lay(&self, settings: &mut RoomSettings) {
        match self {
            Self::JoinRule(rule) => settings.join_rule = *rule,
            Self::History(history) => settings.history = *history,
            Self::Address(address) => settings.address = address.clone(),
            Self::Listed(listed) => settings.listed = Some(*listed),
            Self::Permissions(permissions) => settings.permissions = *permissions,
            Self::Notify(mode) => settings.notify = *mode,
            Self::Encrypted(encrypted) => settings.encrypted = *encrypted,
            Self::Link(space, listed) => {
                let at = settings.spaces.iter().position(|l| l.space == *space);
                match (listed, at) {
                    (Some(suggested), Some(at)) => settings.spaces[at].suggested = *suggested,
                    (Some(suggested), None) => {
                        settings.can_add_to.retain(|id| id != space);
                        settings.spaces.push(SpaceLink {
                            space: space.clone(),
                            suggested: *suggested,
                            // We just did.
                            can_change: true,
                        });
                    }
                    (None, Some(at)) => {
                        let link = settings.spaces.remove(at);
                        if link.can_change && !settings.can_add_to.contains(space) {
                            settings.can_add_to.push(link.space);
                        }
                    }
                    (None, None) => {}
                }
            }
            Self::Banned(user, false) => settings.banned.retain(|b| b.user.id != *user),
            // Only unbanning is changed here.
            Self::Banned(_, true) => {}
        }
    }
}

impl SettingKey {
    /// Its value in `settings`; none where they don't say (whether the
    /// room is listed).
    fn read(&self, settings: &RoomSettings) -> Option<Setting> {
        Some(match self {
            Self::JoinRule => Setting::JoinRule(settings.join_rule),
            Self::History => Setting::History(settings.history),
            Self::Address => Setting::Address(settings.address.clone()),
            Self::Listed => Setting::Listed(settings.listed?),
            Self::Permissions => Setting::Permissions(settings.permissions),
            Self::Notify => Setting::Notify(settings.notify),
            Self::Encrypted => Setting::Encrypted(settings.encrypted),
            Self::Link(space) => {
                let link = settings.spaces.iter().find(|l| l.space == *space);
                Setting::Link(space.clone(), link.map(|l| l.suggested))
            }
            Self::Banned(user) => {
                let banned = settings.banned.iter().any(|b| b.user.id == *user);
                Setting::Banned(user.clone(), banned)
            }
        })
    }
}

/// Show the value set here for `room`, if any, as `shown`.
fn lay<T: Clone>(pendings: &Pendings<T>, room: &RoomId, shown: &mut T) {
    if let Some(pending) = pendings.get(room) {
        *shown = pending.value.clone();
    }
}

/// A room's unread messages, mentions and notifications.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Unread {
    messages: u64,
    mentions: u64,
    notifications: u64,
}

impl Unread {
    /// As the SDK counts them in `room`: from our read receipt as synced.
    pub(crate) fn of(room: &Room) -> Self {
        Self {
            messages: room.num_unread_messages(),
            mentions: room.num_unread_mentions(),
            notifications: room.num_unread_notifications(),
        }
    }

    /// As the room list has them.
    fn listed(room: &RoomSummary) -> Self {
        Self {
            messages: room.unread_messages,
            mentions: room.unread_mentions,
            notifications: room.unread_notifications,
        }
    }

    fn is_zero(self) -> bool {
        self == Self::default()
    }

    /// Those counted since `then`.
    fn since(self, then: Self) -> Self {
        Self {
            messages: self.messages.saturating_sub(then.messages),
            mentions: self.mentions.saturating_sub(then.mentions),
            notifications: self.notifications.saturating_sub(then.notifications),
        }
    }

    fn show(self, room: &mut RoomSummary) {
        room.unread_messages = self.messages;
        room.unread_mentions = self.mentions;
        room.unread_notifications = self.notifications;
    }
}

/// A room read here up to an event, until sync brings our receipt.
///
/// The SDK counts a room's unread messages from our read receipt as synced:
/// while that stays where it was when we read, what it counted then was
/// read, and what it counts beyond that came since.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct PendingRead {
    /// The newest event read: where the receipt goes.
    pub(crate) up_to: OwnedEventId,
    /// Where our read receipt was as synced when we read (an event we sent
    /// counts), if known.
    pub(crate) receipt: Option<OwnedEventId>,
    /// The SDK's counts then.
    pub(crate) counted: Unread,
}

/// The changes made here that sync has not brought yet.
#[derive(Clone, Debug, Default)]
pub(crate) struct Changes {
    tags: Pendings<RoomTag>,
    /// Without a topic: empty.
    topics: Pendings<Topic>,
    /// Only names that are not blank: a room without one is named after
    /// its members.
    names: Pendings<String>,
    pinned: Pendings<Vec<OwnedEventId>>,
    /// Rooms' settings, by room and setting.
    settings: Pendings<Setting, (OwnedRoomId, SettingKey)>,
    /// Rooms joined or made here that the room list does not list as
    /// joined yet.
    joining: BTreeSet<OwnedRoomId>,
    /// Rooms asked to join here that the room list does not list yet.
    knocking: BTreeSet<OwnedRoomId>,
    /// Rooms read here, until sync moves our read receipt. No timeout: the
    /// receipt is sent again once the server is reachable.
    reads: HashMap<OwnedRoomId, PendingRead>,
}

impl Changes {
    fn count(&self) -> usize {
        let rooms = self.joining.len() + self.knocking.len();
        let listed = self.tags.len() + self.topics.len() + self.names.len() + self.pinned.len();
        listed + self.settings.len() + rooms
    }

    /// When the next of the changes the room list shows that the server
    /// accepted times out.
    fn timeout(&self) -> Option<Instant> {
        let tags = self.tags.values().filter_map(Pending::timeout);
        let topics = self.topics.values().filter_map(Pending::timeout);
        let names = self.names.values().filter_map(Pending::timeout);
        let pinned = self.pinned.values().filter_map(Pending::timeout);
        tags.chain(topics).chain(names).chain(pinned).min()
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

    /// `room`'s topic is being set to `topic` (empty to remove it).
    pub(crate) fn set_topic(&self, room: &RoomId, topic: Topic) {
        self.changes.send_modify(|changes| {
            changes.topics.insert(room.to_owned(), Pending::new(topic));
        });
    }

    /// The server answered setting `room`'s topic: on failure it goes.
    pub(crate) fn topic_answered(&self, room: &RoomId, topic: &Topic, ok: bool, now: Instant) {
        (self.changes)
            .send_if_modified(|changes| answer(&mut changes.topics, room, topic, ok, now));
    }

    /// `room` is being renamed `name` (not blank).
    pub(crate) fn set_name(&self, room: &RoomId, name: String) {
        self.changes.send_modify(|changes| {
            changes.names.insert(room.to_owned(), Pending::new(name));
        });
    }

    /// The server answered renaming `room`: on failure the name goes.
    pub(crate) fn name_answered(&self, room: &RoomId, name: &str, ok: bool, now: Instant) {
        (self.changes).send_if_modified(|changes| answer(&mut changes.names, room, name, ok, now));
    }

    /// `room`'s `setting` is being changed.
    pub(crate) fn set_setting(&self, room: &RoomId, setting: Setting) {
        self.changes.send_modify(|changes| {
            let key = (room.to_owned(), setting.key());
            changes.settings.insert(key, Pending::new(setting));
        });
    }

    /// The server answered changing `room`'s `setting`: on failure the
    /// change goes.
    pub(crate) fn setting_answered(
        &self,
        room: &RoomId,
        setting: &Setting,
        ok: bool,
        now: Instant,
    ) {
        let key = (room.to_owned(), setting.key());
        (self.changes)
            .send_if_modified(|changes| answer(&mut changes.settings, &key, setting, ok, now));
    }

    /// When the server accepted the change to `room`'s setting `key` that
    /// is still shown, if it has.
    pub(crate) fn accepted(&self, room: &RoomId, key: SettingKey) -> Option<Instant> {
        let key = (room.to_owned(), key);
        self.changes.borrow().settings.get(&key)?.since
    }

    /// `room`'s setting `key` as changed here, while still shown.
    pub(crate) fn setting(&self, room: &RoomId, key: SettingKey) -> Option<Setting> {
        let key = (room.to_owned(), key);
        Some(self.changes.borrow().settings.get(&key)?.value.clone())
    }

    /// Lay the changes to `room`'s settings over `settings` (as synced),
    /// first dropping those they show and those the server accepted
    /// [`ECHO_TIMEOUT`] ago; returns when the next of those left times out.
    pub(crate) fn lay_settings(
        &self,
        room: &RoomId,
        settings: &mut RoomSettings,
        now: Instant,
    ) -> Option<Instant> {
        self.changes.send_if_modified(|changes| {
            let before = changes.settings.len();
            (changes.settings).retain(|(id, key), pending| {
                **id != *room || pending.shows_over(key.read(settings).as_ref(), now)
            });
            changes.settings.len() != before
        });
        let changes = self.changes.borrow();
        let mine = (changes.settings.iter()).filter(|((id, _), _)| **id == *room);
        let mut timeouts = Vec::new();
        for (_, pending) in mine {
            pending.value.lay(settings);
            timeouts.extend(pending.timeout());
        }
        // Space members only for a room in a space, as shown now.
        let in_space = !settings.spaces.is_empty();
        settings.join_rule_choices = JoinRule::choices(settings.join_rule, in_space);
        timeouts.into_iter().min()
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

    /// `room` was read here, `read`; shown as read from now.
    pub(crate) fn set_read(&self, room: &RoomId, read: PendingRead) {
        self.changes.send_modify(|changes| {
            changes.reads.insert(room.to_owned(), read);
        });
    }

    /// The server has our receipt in `room` at or after `up_to` already:
    /// the read up to there goes, unless the room was read further since.
    pub(crate) fn drop_read(&self, room: &RoomId, up_to: &OwnedEventId) {
        self.changes.send_if_modified(|changes| {
            let ours = changes.reads.get(room).is_some_and(|r| r.up_to == *up_to);
            ours && changes.reads.remove(room).is_some()
        });
    }

    /// The rooms read here that sync has not shown read, with the event
    /// read up to: their receipts, to send again.
    pub(crate) fn reads(&self) -> Vec<(OwnedRoomId, OwnedEventId)> {
        let changes = self.changes.borrow();
        (changes.reads.iter())
            .map(|(room, read)| (room.clone(), read.up_to.clone()))
            .collect()
    }

    /// Show the rooms read here as read in `rooms`, but for what came
    /// since; first dropping the reads of rooms gone, of those whose read
    /// receipt sync moved (`receipt`: where it is now, see
    /// [`PendingRead::receipt`]) and of those the SDK counts nothing unread
    /// in.
    pub(crate) fn lay_reads(
        &self,
        rooms: &mut [RoomSummary],
        receipt: impl Fn(&RoomId) -> Option<OwnedEventId>,
    ) {
        self.changes.send_if_modified(|changes| {
            let before = changes.reads.len();
            changes.reads.retain(|id, read| {
                rooms.iter().find(|r| r.id == *id).is_some_and(|room| {
                    receipt(id) == read.receipt && !Unread::listed(room).is_zero()
                })
            });
            changes.reads.len() != before
        });
        let changes = self.changes.borrow();
        for room in rooms.iter_mut() {
            if let Some(read) = changes.reads.get(&room.id) {
                Unread::listed(room).since(read.counted).show(room);
            }
        }
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
            (changes.names).retain(|id, p| p.shows_over(synced(id).map(|r| &r.name), now));
            (changes.pinned).retain(|id, p| p.shows_over(synced(id).map(|r| &r.pinned), now));
            // The settings drop those they show; here go those of rooms
            // gone and those timed out, whether shown or not.
            (changes.settings).retain(|(id, _), p| synced(id).is_some() && !p.timed_out(now));
            let coming = |r: &RoomSummary| r.membership != Membership::Joined;
            (changes.joining).retain(|id| synced(id).is_none_or(coming));
            (changes.knocking).retain(|id| synced(id).is_none());
            changes.count() != before
        });
        let changes = self.changes.borrow();
        for room in rooms.iter_mut() {
            lay(&changes.tags, &room.id, &mut room.tag);
            lay(&changes.topics, &room.id, &mut room.topic);
            lay(&changes.names, &room.id, &mut room.name);
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
    use matrix_sdk::ruma::{owned_event_id, owned_room_alias_id, owned_room_id, owned_user_id};

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
        echoes.set_topic(&a, Topic::parse("About."));
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
        rooms[1].topic = Topic::parse("Old");
        let now = Instant::now();
        let new = Topic::parse("New\n[call] https://zoom.us/j/1");
        echoes.set_topic(&a, new.clone());
        echoes.set_topic(&b, Topic::default());
        let mut shown = rooms.clone();
        echoes.lay_over(&mut shown, now);
        assert_eq!(shown[0].topic, new);
        assert!(shown[1].topic.is_empty());

        echoes.topic_answered(&a, &new, true, now);
        echoes.topic_answered(&b, &Topic::default(), false, now);
        let mut shown = rooms.clone();
        assert_eq!(echoes.lay_over(&mut shown, now), Some(now + ECHO_TIMEOUT));
        assert_eq!(shown[0].topic, new);
        assert_eq!(shown[1].topic.text, "Old");

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

    #[test]
    fn a_name_shows_in_the_list_until_synced_or_failed() {
        let echoes = Echoes::default();
        let (a, b) = (owned_room_id!("!a:x"), owned_room_id!("!b:x"));
        let rooms = vec![
            room(a.as_str(), RoomKind::Room),
            room(b.as_str(), RoomKind::Room),
        ];
        let now = Instant::now();
        echoes.set_name(&a, "Ice".into());
        echoes.set_name(&b, "Sheets".into());
        echoes.name_answered(&a, "Ice", true, now);
        echoes.name_answered(&b, "Sheets", false, now);
        let mut shown = rooms.clone();
        assert_eq!(echoes.lay_over(&mut shown, now), Some(now + ECHO_TIMEOUT));
        assert_eq!(
            (shown[0].name.as_str(), shown[1].name.as_str()),
            ("Ice", "!b:x")
        );

        let mut synced = rooms.clone();
        synced[0].name = "Ice".into();
        echoes.lay_over(&mut synced, now);
        assert!(echoes.changes.borrow().names.is_empty());
    }

    /// A room's settings as synced: invite only, in the space Team, which
    /// suggests it, Bob banned; we may add it to Other.
    fn settings() -> RoomSettings {
        RoomSettings {
            rights: Default::default(),
            notify: NotifyMode::All,
            join_rule: JoinRule::Invite,
            join_rule_choices: JoinRule::choices(JoinRule::Invite, true),
            address: None,
            listed: Some(false),
            history: History::Members,
            permissions: Permissions::Everyone,
            encrypted: false,
            version: Some("12".into()),
            spaces: vec![SpaceLink {
                space: owned_room_id!("!team:x"),
                suggested: true,
                can_change: true,
            }],
            can_add_to: vec![owned_room_id!("!other:x")],
            banned: vec![crate::Banned {
                user: crate::User {
                    id: owned_user_id!("@bob:x"),
                    name: None,
                    avatar: None,
                },
                reason: None,
            }],
        }
    }

    #[test]
    fn settings_show_until_synced_failed_or_timed_out() {
        let echoes = Echoes::default();
        let (a, b) = (owned_room_id!("!a:x"), owned_room_id!("!b:x"));
        let now = Instant::now();
        let public = Setting::JoinRule(JoinRule::Public);
        let joined = Setting::History(History::Joined);
        let address = Setting::Address(Some(owned_room_alias_id!("#ice:x")));
        echoes.set_setting(&a, public.clone());
        echoes.set_setting(&a, joined.clone());
        echoes.set_setting(&a, address.clone());
        echoes.set_setting(&a, Setting::Notify(NotifyMode::Mute));
        // Another room's: not shown here.
        echoes.set_setting(&b, Setting::Encrypted(true));
        let mut shown = settings();
        assert_eq!(echoes.lay_settings(&a, &mut shown, now), None);
        assert_eq!(shown.join_rule, JoinRule::Public);
        assert_eq!(shown.history, History::Joined);
        assert_eq!(shown.address, Some(owned_room_alias_id!("#ice:x")));
        assert_eq!(shown.notify, NotifyMode::Mute);
        assert!(!shown.encrypted);

        // Failed: gone. Accepted: until synced or timed out. An answer to
        // an earlier change leaves a later one.
        echoes.setting_answered(&a, &address, false, now);
        echoes.setting_answered(&a, &public, true, now);
        echoes.setting_answered(&a, &joined, true, now);
        echoes.setting_answered(&a, &Setting::Notify(NotifyMode::All), true, now);
        let mut shown = settings();
        assert_eq!(
            echoes.lay_settings(&a, &mut shown, now),
            Some(now + ECHO_TIMEOUT)
        );
        assert_eq!(shown.address, None);
        assert_eq!(shown.notify, NotifyMode::Mute);
        let mut synced = settings();
        synced.join_rule = JoinRule::Public;
        echoes.lay_settings(&a, &mut synced, now);
        assert_eq!(synced.history, History::Joined);
        assert!(
            !(echoes.changes.borrow().settings).contains_key(&(a.clone(), SettingKey::JoinRule))
        );
        let mut shown = settings();
        echoes.lay_settings(&a, &mut shown, now + ECHO_TIMEOUT);
        assert_eq!(shown.history, History::Members);
        // The mode's request runs on; the other room's stays.
        let left: Vec<_> = echoes.changes.borrow().settings.keys().cloned().collect();
        assert_eq!(left.len(), 2);
        assert!(left.contains(&(a.clone(), SettingKey::Notify)));
    }

    #[test]
    fn space_links_and_unbans_show_in_the_settings() {
        let echoes = Echoes::default();
        let a = owned_room_id!("!a:x");
        let (team, other) = (owned_room_id!("!team:x"), owned_room_id!("!other:x"));
        let now = Instant::now();
        // Moved from Team to Other, not suggested there; Bob unbanned.
        echoes.set_setting(&a, Setting::Link(team.clone(), None));
        echoes.set_setting(&a, Setting::Link(other.clone(), Some(false)));
        echoes.set_setting(&a, Setting::Banned(owned_user_id!("@bob:x"), false));
        let mut shown = settings();
        echoes.lay_settings(&a, &mut shown, now);
        let spaces: Vec<_> = (shown.spaces.iter())
            .map(|l| (&l.space, l.suggested))
            .collect();
        assert_eq!(spaces, [(&other, false)]);
        assert_eq!(shown.can_add_to, std::slice::from_ref(&team));
        assert!(shown.banned.is_empty());

        // Out of every space: Space members no longer offered.
        let mut shown = settings();
        shown.can_add_to.clear();
        let not_listed = Setting::Link(other.clone(), None);
        echoes.set_setting(&a, not_listed);
        echoes.lay_settings(&a, &mut shown, now);
        assert!(shown.spaces.is_empty());
        assert_eq!(
            shown.join_rule_choices,
            JoinRule::choices(JoinRule::Invite, false)
        );

        // Synced: Bob no longer banned, out of Team.
        let mut synced = settings();
        synced.banned.clear();
        synced.spaces.clear();
        echoes.lay_settings(&a, &mut synced, now);
        let keys: Vec<_> = echoes.changes.borrow().settings.keys().cloned().collect();
        assert!(keys.is_empty(), "{keys:?}");
    }

    #[test]
    fn a_listed_change_says_when_it_was_accepted() {
        let echoes = Echoes::default();
        let a = owned_room_id!("!a:x");
        let now = Instant::now();
        let listed = Setting::Listed(true);
        echoes.set_setting(&a, listed.clone());
        assert_eq!(echoes.accepted(&a, SettingKey::Listed), None);
        assert_eq!(echoes.setting(&a, SettingKey::Listed), Some(listed.clone()));
        echoes.setting_answered(&a, &listed, true, now);
        assert_eq!(echoes.accepted(&a, SettingKey::Listed), Some(now));
        // Read again: listed.
        let mut synced = settings();
        synced.listed = Some(true);
        echoes.lay_settings(&a, &mut synced, now);
        assert_eq!(echoes.accepted(&a, SettingKey::Listed), None);
    }

    /// Unread counts: messages, mentions, notifications.
    fn unread(messages: u64, mentions: u64, notifications: u64) -> Unread {
        Unread {
            messages,
            mentions,
            notifications,
        }
    }

    #[test]
    fn a_room_read_here_shows_read_but_for_what_came_since() {
        let echoes = Echoes::default();
        let (a, b) = (owned_room_id!("!a:x"), owned_room_id!("!b:x"));
        let old = owned_event_id!("$old");
        let mut rooms = vec![
            room(a.as_str(), RoomKind::Room),
            room(b.as_str(), RoomKind::Room),
        ];
        unread(3, 1, 2).show(&mut rooms[0]);
        unread(4, 0, 4).show(&mut rooms[1]);
        echoes.set_read(
            &a,
            PendingRead {
                up_to: owned_event_id!("$3"),
                receipt: Some(old.clone()),
                counted: unread(3, 1, 2),
            },
        );
        let synced = |_: &RoomId| Some(old.clone());
        let mut shown = rooms.clone();
        echoes.lay_reads(&mut shown, synced);
        assert_eq!(Unread::listed(&shown[0]), Unread::default());
        // Another room's counts stay.
        assert_eq!(Unread::listed(&shown[1]), unread(4, 0, 4));
        assert_eq!(echoes.reads(), [(a.clone(), owned_event_id!("$3"))]);

        // A mention came since, counted from the same receipt.
        unread(4, 2, 3).show(&mut rooms[0]);
        let mut shown = rooms.clone();
        echoes.lay_reads(&mut shown, synced);
        assert_eq!(Unread::listed(&shown[0]), unread(1, 1, 1));
        // Read again, further: counted from there.
        echoes.set_read(
            &a,
            PendingRead {
                up_to: owned_event_id!("$4"),
                receipt: Some(old.clone()),
                counted: unread(4, 2, 3),
            },
        );
        // The server had the earlier one already: the later read stays.
        echoes.drop_read(&a, &owned_event_id!("$3"));
        let mut shown = rooms.clone();
        echoes.lay_reads(&mut shown, synced);
        assert_eq!(Unread::listed(&shown[0]), Unread::default());

        // Sync moved the receipt: its counts again, the read gone.
        let mut shown = rooms.clone();
        echoes.lay_reads(&mut shown, |_| Some(owned_event_id!("$4")));
        assert_eq!(Unread::listed(&shown[0]), unread(4, 2, 3));
        assert!(echoes.reads().is_empty());
    }

    #[test]
    fn reads_go_when_nothing_is_unread_the_room_is_gone_or_the_server_had_them() {
        let echoes = Echoes::default();
        let (a, b, c) = (
            owned_room_id!("!a:x"),
            owned_room_id!("!b:x"),
            owned_room_id!("!c:x"),
        );
        let read = |up_to| PendingRead {
            up_to,
            receipt: None,
            counted: unread(2, 0, 2),
        };
        echoes.set_read(&a, read(owned_event_id!("$a")));
        echoes.set_read(&b, read(owned_event_id!("$b")));
        echoes.set_read(&c, read(owned_event_id!("$c")));
        echoes.drop_read(&c, &owned_event_id!("$c"));
        // Nothing unread in a; b not listed.
        let mut rooms = vec![room(a.as_str(), RoomKind::Room)];
        echoes.lay_reads(&mut rooms, |_| None);
        assert!(echoes.reads().is_empty());
        assert_eq!(Unread::listed(&rooms[0]), Unread::default());
    }

    #[test]
    fn settings_of_rooms_gone_go_with_the_room_list() {
        let echoes = Echoes::default();
        let (a, b) = (owned_room_id!("!a:x"), owned_room_id!("!b:x"));
        let now = Instant::now();
        let mute = Setting::Notify(NotifyMode::Mute);
        echoes.set_setting(&a, mute.clone());
        echoes.set_setting(&b, Setting::Encrypted(true));
        echoes.setting_answered(&a, &mute, true, now);
        // Room list timeouts are its own.
        let mut rooms = vec![room(a.as_str(), RoomKind::Room)];
        assert_eq!(echoes.lay_over(&mut rooms, now), None);
        assert_eq!(echoes.changes.borrow().settings.len(), 1);
        echoes.lay_over(&mut rooms, now + ECHO_TIMEOUT);
        assert!(echoes.changes.borrow().settings.is_empty());
    }
}
