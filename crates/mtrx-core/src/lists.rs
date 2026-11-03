//! The room lists the apps show, from the room list, the user's categories,
//! the members of the spaces, the drafts and the Notes to self: the chats,
//! the spaces, a space's sections, the Rooms tab's groups, the search and
//! the unread counts.

use std::{
    cmp::Ordering,
    collections::{BTreeSet, HashMap, HashSet},
    sync::Arc,
    time::Instant,
};

use futures_util::{Stream, StreamExt};
use matrix_sdk::{
    Client, RoomMemberships,
    ruma::{OwnedRoomId, OwnedUserId, RoomId, UserId},
};
use tokio::{sync::watch, task::JoinSet};
use tokio_stream::wrappers::WatchStream;
use tracing::warn;

use crate::{
    Grouping, Layout, Live, Result, RoomCategories, RoomKind, RoomListState, RoomSummary, Section,
    Session,
    spaces::{SpaceMembers, home_rooms, space_rooms},
    top_level_spaces,
};

/// What a room list lists.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum SpaceKey {
    /// A space, top-level or nested.
    Space(OwnedRoomId),
    /// The desktop's Home: direct chats, invites, rooms in no space, rooms
    /// put into Home's categories, and what [`crate::AtHome`] adds.
    Home,
    /// The phones' Home: rooms and invites no top-level space lists; no
    /// direct chats.
    Outside,
}

/// An entry of the phones' spaces page: a top-level space or
/// [`SpaceKey::Outside`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SpaceEntry {
    pub key: SpaceKey,
    /// What it lists, in the room list's order.
    pub rooms: Vec<OwnedRoomId>,
    /// Any of its rooms has unread messages; a space's direct chats do not
    /// count (#45).
    pub unread: bool,
}

/// Unread messages for the phones' tabs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Badges {
    /// In the direct chats, the Notes to self and invites too.
    pub chats: u64,
    /// In the joined rooms.
    pub rooms: u64,
    /// In the joined rooms the top-level spaces list.
    pub spaces: u64,
}

/// The order of the rooms in each section, chosen on a device.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RoomSort {
    /// The room list's: most recently active first.
    #[default]
    ListOrder,
    AToZ,
    ZToA,
    /// By the last activity, newest first; rooms without one last.
    MostRecent,
}

/// How a device shows a list's sections, by [`Section::key`]: their order
/// once reordered there, those hidden and those folded. A section new to
/// the device takes its place in the account's order, shown and unfolded.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SectionLayout {
    /// The keys in the order chosen; empty: the account's order.
    pub order: Vec<String>,
    pub hidden: BTreeSet<String>,
    pub folded: BTreeSet<String>,
}

/// A titled part of a list, folded or not.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ListSection {
    /// The section's [`Section::key`], a Rooms tab group's
    /// [`RoomLists::OUTSIDE_GROUP`] or space ID.
    pub key: String,
    pub title: String,
    pub rooms: Vec<OwnedRoomId>,
    /// Only its heading shows, with its unread count.
    pub folded: bool,
    /// Unread messages in its rooms.
    pub unread: u64,
    /// Any of them mentions us.
    pub mentions: bool,
    /// The space whose rooms it holds (a Rooms tab group's), which its
    /// heading opens.
    pub space: Option<OwnedRoomId>,
}

/// A section to show or hide, in the device's order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SectionChoice {
    pub key: String,
    pub title: String,
    pub shown: bool,
    /// The number of its rooms.
    pub rooms: u64,
}

/// The room lists, from one room list and what arranges it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RoomLists {
    /// Joined and invited rooms, most recently active first; tags, topics
    /// and pinned messages set here shown at once.
    pub rooms: Vec<RoomSummary>,
    /// See [`RoomListState::loaded`].
    pub loaded: bool,
    pub categories: RoomCategories,
    /// The Notes to self, while kept at the top.
    pub notes: Option<OwnedRoomId>,
    /// The direct chats, invites to them included; not the Notes to self
    /// at the top.
    pub chats: Vec<OwnedRoomId>,
    /// The phones' spaces page: the top-level spaces, then
    /// [`SpaceKey::Outside`] while it lists any.
    pub spaces: Vec<SpaceEntry>,
    /// The rooms with a draft on this device.
    pub drafts: BTreeSet<OwnedRoomId>,
    /// Rooms and spaces joined or made here that `rooms` does not list as
    /// joined yet: coming, not gone.
    pub joining: BTreeSet<OwnedRoomId>,
    pub badges: Badges,
    /// The joined members of the top-level spaces loaded so far.
    members: HashMap<OwnedRoomId, HashSet<OwnedUserId>>,
}

impl RoomLists {
    /// The key of the Rooms tab's group of [`SpaceKey::Outside`].
    pub const OUTSIDE_GROUP: &str = "home";

    /// A space whose `members` are not known lists no direct chats.
    pub fn new(
        state: RoomListState,
        categories: RoomCategories,
        members: HashMap<OwnedRoomId, HashSet<OwnedUserId>>,
        own_user: &UserId,
        drafts: BTreeSet<OwnedRoomId>,
    ) -> Self {
        let RoomListState { rooms, loaded } = state;
        let notes = (rooms.iter())
            .find(|r| r.is_notes(own_user))
            .filter(|_| categories.notes_at_top())
            .map(|r| r.id.clone());
        let chats = (rooms.iter())
            .filter(|r| r.kind == RoomKind::Direct && Some(&r.id) != notes.as_ref())
            .map(|r| r.id.clone())
            .collect();
        let mut lists = Self {
            rooms,
            loaded,
            categories,
            notes,
            chats,
            spaces: Vec::new(),
            drafts,
            joining: BTreeSet::new(),
            badges: Badges::default(),
            members,
        };
        lists.spaces = lists.space_entries();
        lists.badges = lists.count_badges();
        lists
    }

    fn space_entries(&self) -> Vec<SpaceEntry> {
        let grouping = Grouping::default();
        let mut entries: Vec<SpaceEntry> = top_level_spaces(&self.rooms)
            .into_iter()
            .map(|space| {
                let key = SpaceKey::Space(space.id.clone());
                SpaceEntry {
                    rooms: ids(self.listed(&key, &grouping)),
                    unread: self.unread(&key, &grouping),
                    key,
                }
            })
            .collect();
        let in_spaces: HashSet<&OwnedRoomId> = entries.iter().flat_map(|e| &e.rooms).collect();
        let outside: Vec<OwnedRoomId> = (self.rooms.iter())
            .filter(|r| listed(r) && r.kind != RoomKind::Direct)
            .filter(|r| !in_spaces.contains(&r.id))
            .map(|r| r.id.clone())
            .collect();
        if !outside.is_empty() {
            let unread = outside.iter().any(|id| self.room(id).is_some_and(unread));
            entries.push(SpaceEntry {
                key: SpaceKey::Outside,
                rooms: outside,
                unread,
            });
        }
        entries
    }

    fn count_badges(&self) -> Badges {
        let in_spaces: HashSet<&OwnedRoomId> = (self.spaces.iter())
            .filter(|e| matches!(e.key, SpaceKey::Space(_)))
            .flat_map(|e| &e.rooms)
            .collect();
        let sum = |keep: &dyn Fn(&RoomSummary) -> bool| {
            (self.rooms.iter())
                .filter(|r| keep(r))
                .map(|r| r.unread_messages)
                .sum()
        };
        Badges {
            chats: sum(&|r| r.kind == RoomKind::Direct),
            rooms: sum(&joined_room),
            spaces: sum(&|r| joined_room(r) && in_spaces.contains(&r.id)),
        }
    }

    pub fn room(&self, id: &RoomId) -> Option<&RoomSummary> {
        self.rooms.iter().find(|r| r.id == id)
    }

    fn rooms_of<'a>(&'a self, ids: &'a [OwnedRoomId]) -> impl Iterator<Item = &'a RoomSummary> {
        ids.iter().filter_map(|id| self.room(id))
    }

    /// Whether the Notes to self stay at the top.
    pub fn notes_at_top(&self) -> bool {
        self.categories.notes_at_top()
    }

    /// The phones' spaces page's entry of `key`, while listed.
    pub fn space(&self, key: &SpaceKey) -> Option<&SpaceEntry> {
        self.spaces.iter().find(|e| e.key == *key)
    }

    /// The top-level spaces, in order: where someone can be invited.
    pub fn top_level_spaces(&self) -> Vec<&RoomSummary> {
        (self.spaces.iter())
            .filter_map(|e| match &e.key {
                SpaceKey::Space(id) => self.room(id),
                _ => None,
            })
            .collect()
    }

    /// What `key` lists, in the room list's order, the Notes to self too.
    /// The desktop's Home by `grouping` (favourites of spaces only with the
    /// Favourites section).
    pub fn listed(&self, key: &SpaceKey, grouping: &Grouping) -> Vec<&RoomSummary> {
        match key {
            SpaceKey::Home => home_rooms(&self.rooms, &self.categories, grouping),
            SpaceKey::Outside => match self.space(key) {
                Some(entry) => self.rooms_of(&entry.rooms).collect(),
                None => Vec::new(),
            },
            SpaceKey::Space(space) => space_rooms(
                &self.rooms,
                space,
                self.members.get(space),
                self.categories.layout(Some(space)),
            ),
        }
    }

    /// The layout of `key`'s sections in the account; none for the phones'
    /// Home.
    fn layout(&self, key: &SpaceKey) -> Option<&Layout> {
        match key {
            SpaceKey::Space(space) => Some(self.categories.layout(Some(space))),
            SpaceKey::Home => Some(&self.categories.home),
            SpaceKey::Outside => None,
        }
    }

    /// `key`'s sections and their rooms, in the account's order, empty
    /// ones too; not the Notes to self at the top. The phones' Home has
    /// Invites, Direct messages and Rooms.
    pub fn sections(
        &self,
        key: &SpaceKey,
        grouping: &Grouping,
    ) -> Vec<(Section, Vec<&RoomSummary>)> {
        let listed =
            (self.listed(key, grouping).into_iter()).filter(|r| Some(&r.id) != self.notes.as_ref());
        match self.layout(key) {
            Some(layout) => layout.group(listed, grouping),
            None => {
                let (invites, joined): (Vec<_>, Vec<_>) = listed.partition(|r| r.invited);
                let (direct, rooms) = joined.into_iter().partition(|r| r.kind == RoomKind::Direct);
                vec![
                    (Section::Invites, invites),
                    (Section::People, direct),
                    (Section::Rooms, rooms),
                ]
            }
        }
    }

    /// What `section` of `key`'s list is called: a category its name; the
    /// desktop's Home's other rooms those with no space, unless it lists
    /// all.
    pub fn section_title(&self, key: &SpaceKey, section: &Section) -> String {
        let title = match section {
            Section::Invites => "Invites",
            Section::Favourites => "Favourites",
            Section::Category(id) => {
                (self.layout(key).and_then(|l| l.category(id))).map_or("", |c| c.name.as_str())
            }
            Section::Rooms if *key == SpaceKey::Home && !self.categories.at_home.all_rooms => {
                "Rooms with no space"
            }
            Section::Rooms => "Rooms",
            Section::People => "Direct messages",
            Section::LowPriority => "Low priority",
        };
        title.to_owned()
    }

    /// Whether anything `key` lists has unread messages: its dot. A space's
    /// direct chats do not count: it lists the chats with its members, and
    /// one message would light every space shared with its sender (#45).
    pub fn unread(&self, key: &SpaceKey, grouping: &Grouping) -> bool {
        (self.listed(key, grouping).into_iter())
            .filter(|r| !matches!(key, SpaceKey::Space(_)) || r.kind != RoomKind::Direct)
            .any(unread)
    }

    /// A list as a device shows it: `key`'s sections in `layout`'s order,
    /// those shown and with rooms, folded as `layout` says, each's rooms in
    /// `sort`'s order. While searching for `filter`, the rooms found, in
    /// the search's order, none folded.
    pub fn space_list(
        &self,
        key: &SpaceKey,
        layout: &SectionLayout,
        sort: RoomSort,
        grouping: &Grouping,
        filter: &str,
    ) -> Vec<ListSection> {
        let searching = !filter.trim().is_empty();
        let mut sections: HashMap<String, (Section, Vec<&RoomSummary>)> =
            (self.sections(key, grouping).into_iter())
                .map(|(section, rooms)| (section.key(), (section, rooms)))
                .collect();
        let keys = self.section_keys(key, grouping);
        (layout.arranged(&keys).into_iter())
            .filter(|k| !layout.hidden.contains(k))
            .filter_map(|k| {
                let (section, rooms) = sections.remove(&k)?;
                let shown = if searching {
                    found(rooms, filter)
                } else {
                    sort.apply(rooms)
                };
                (!shown.is_empty()).then(|| ListSection {
                    title: self.section_title(key, &section),
                    folded: !searching && layout.folded.contains(&k),
                    key: k,
                    ..ListSection::of(&shown)
                })
            })
            .collect()
    }

    fn section_keys(&self, key: &SpaceKey, grouping: &Grouping) -> Vec<String> {
        (self.sections(key, grouping).iter())
            .map(|(section, _)| section.key())
            .collect()
    }

    /// `key`'s sections in `layout`'s order, empty ones too, shown or not.
    pub fn section_choices(
        &self,
        key: &SpaceKey,
        layout: &SectionLayout,
        grouping: &Grouping,
    ) -> Vec<SectionChoice> {
        let mut sections: HashMap<String, (Section, usize)> =
            (self.sections(key, grouping).into_iter())
                .map(|(section, rooms)| (section.key(), (section, rooms.len())))
                .collect();
        let keys = self.section_keys(key, grouping);
        (layout.arranged(&keys).into_iter())
            .filter_map(|k| {
                let (section, rooms) = sections.remove(&k)?;
                Some(SectionChoice {
                    title: self.section_title(key, &section),
                    shown: !layout.hidden.contains(&k),
                    rooms: rooms as u64,
                    key: k,
                })
            })
            .collect()
    }

    /// The Rooms tab: the phones' Home, then each top-level space's rooms,
    /// in the spaces page's order, a room in several spaces in each; in
    /// each, the rooms by their latest message, newest first. Rooms and
    /// invites to them, no direct chats or spaces; only those `among`, if
    /// given (those shared with someone). Groups without rooms are left
    /// out; those whose key is in `folded` are folded.
    pub fn room_groups(
        &self,
        among: Option<&BTreeSet<OwnedRoomId>>,
        folded: &BTreeSet<String>,
    ) -> Vec<ListSection> {
        let group = |entry: &SpaceEntry| {
            let rooms: Vec<&RoomSummary> = (self.rooms_of(&entry.rooms))
                .filter(|r| r.kind == RoomKind::Room)
                .filter(|r| among.is_none_or(|among| among.contains(&r.id)))
                .collect();
            let rooms = RoomSort::MostRecent.apply(rooms);
            let (key, title, space) = match &entry.key {
                SpaceKey::Space(id) => {
                    let name = self.room(id).map_or("", |r| r.name.as_str());
                    (id.to_string(), name.to_owned(), Some(id.clone()))
                }
                _ => (Self::OUTSIDE_GROUP.to_owned(), HOME.to_owned(), None),
            };
            ListSection {
                folded: folded.contains(&key),
                key,
                title,
                space,
                ..ListSection::of(&rooms)
            }
        };
        let outside = self.space(&SpaceKey::Outside).map(group);
        let spaces = (self.spaces.iter())
            .filter(|e| matches!(e.key, SpaceKey::Space(_)))
            .map(group);
        (outside.into_iter().chain(spaces))
            .filter(|g| !g.rooms.is_empty())
            .collect()
    }

    /// What `key` lists (the direct chats and the Notes to self for `None`)
    /// whose name contains `filter`, those starting with it first, then by
    /// name; all of it, in order, without a filter.
    pub fn search(
        &self,
        key: Option<&SpaceKey>,
        grouping: &Grouping,
        filter: &str,
    ) -> Vec<&RoomSummary> {
        let rooms = match key {
            Some(key) => self.listed(key, grouping),
            None => (self.rooms.iter())
                .filter(|r| r.kind == RoomKind::Direct)
                .collect(),
        };
        found(rooms, filter)
    }

    /// The phones' spaces page's entries whose name contains `filter`, as
    /// [`Self::search`].
    pub fn search_spaces(&self, filter: &str) -> Vec<&SpaceEntry> {
        search_by(self.spaces.iter().collect(), filter, |e| self.entry_name(e))
    }

    /// The name of an entry of the spaces page.
    pub fn entry_name(&self, entry: &SpaceEntry) -> &str {
        match &entry.key {
            SpaceKey::Space(id) => self.room(id).map_or("", |r| r.name.as_str()),
            _ => HOME,
        }
    }

    /// The number on a room's or chat's back button: unread messages in the
    /// other joined rooms and the direct chats.
    pub fn unread_besides(&self, room: &RoomId) -> u64 {
        (self.rooms.iter())
            .filter(|r| r.id != room && (joined_room(r) || r.kind == RoomKind::Direct))
            .map(|r| r.unread_messages)
            .sum()
    }

    /// The first top-level space, in the spaces page's order, that lists
    /// `room` (through nested spaces too); none for a room in no space, and
    /// for direct chats and spaces.
    pub fn first_space_of(&self, room: &RoomId) -> Option<&RoomSummary> {
        self.room(room).filter(|r| r.kind == RoomKind::Room)?;
        self.top_level_spaces()
            .into_iter()
            .find(|space| self.in_space(&space.id, room))
    }

    /// The names of the top-level spaces that list `chat` (the direct
    /// chats with their members), in the spaces page's order.
    pub fn chat_spaces(&self, chat: &RoomId) -> Vec<String> {
        (self.top_level_spaces().into_iter())
            .filter(|space| self.in_space(&space.id, chat))
            .map(|space| space.name.clone())
            .collect()
    }

    fn in_space(&self, space: &OwnedRoomId, room: &RoomId) -> bool {
        (self.space(&SpaceKey::Space(space.clone())))
            .is_some_and(|e| e.rooms.iter().any(|r| r == room))
    }
}

/// The phones' Home's name.
const HOME: &str = "Home";

impl ListSection {
    /// A section of `rooms`, with their unread messages.
    fn of(rooms: &[&RoomSummary]) -> Self {
        Self {
            key: String::new(),
            title: String::new(),
            rooms: ids(rooms.iter().copied()),
            folded: false,
            unread: rooms.iter().map(|r| r.unread_messages).sum(),
            mentions: rooms.iter().any(|r| r.unread_mentions > 0),
            space: None,
        }
    }
}

impl Section {
    /// Stable across updates and devices: "invites", "favourites",
    /// "direct_messages", "rooms", "low_priority", or "category:<id>"; for
    /// remembering which sections a device shows and folds.
    pub fn key(&self) -> String {
        match self {
            Section::Invites => "invites".into(),
            Section::Favourites => "favourites".into(),
            Section::People => "direct_messages".into(),
            Section::Rooms => "rooms".into(),
            Section::LowPriority => "low_priority".into(),
            Section::Category(id) => format!("category:{id}"),
        }
    }
}

impl RoomSort {
    /// `rooms` in this order; ties keep their order.
    pub fn apply<'a>(&self, mut rooms: Vec<&'a RoomSummary>) -> Vec<&'a RoomSummary> {
        let by_name =
            |a: &&RoomSummary, b: &&RoomSummary| a.name.to_lowercase().cmp(&b.name.to_lowercase());
        match self {
            RoomSort::ListOrder => {}
            RoomSort::AToZ => rooms.sort_by(by_name),
            RoomSort::ZToA => rooms.sort_by(|a, b| by_name(b, a)),
            RoomSort::MostRecent => {
                rooms.sort_by(|a, b| match (a.last_activity, b.last_activity) {
                    (Some(a), Some(b)) => b.cmp(&a),
                    (Some(_), None) => Ordering::Less,
                    (None, Some(_)) => Ordering::Greater,
                    (None, None) => Ordering::Equal,
                })
            }
        }
        rooms
    }
}

impl SectionLayout {
    /// The account's section keys (`account`, in its order) in this
    /// layout's order: those reordered here as they were put, each new one
    /// after the key before it in the account's order (first if none is).
    /// Keys gone from the account are left out.
    pub fn arranged(&self, account: &[String]) -> Vec<String> {
        let mut arranged: Vec<String> = Vec::new();
        for key in &self.order {
            if account.contains(key) && !arranged.contains(key) {
                arranged.push(key.clone());
            }
        }
        for (at, key) in account.iter().enumerate() {
            if arranged.contains(key) {
                continue;
            }
            let before = account[..at].iter().rev().find(|k| arranged.contains(k));
            let place = before.and_then(|b| arranged.iter().position(|k| k == b));
            arranged.insert(place.map_or(0, |p| p + 1), key.clone());
        }
        arranged
    }
}

/// Whether a room list may show `room`: rooms and direct chats, and spaces
/// only when invited to, to be answered.
fn listed(room: &RoomSummary) -> bool {
    room.kind != RoomKind::Space || room.invited
}

fn joined_room(room: &RoomSummary) -> bool {
    room.kind == RoomKind::Room && !room.invited
}

fn unread(room: &RoomSummary) -> bool {
    room.unread_messages > 0
}

fn ids<'a>(rooms: impl IntoIterator<Item = &'a RoomSummary>) -> Vec<OwnedRoomId> {
    rooms.into_iter().map(|r| r.id.clone()).collect()
}

fn found<'a>(rooms: Vec<&'a RoomSummary>, filter: &str) -> Vec<&'a RoomSummary> {
    search_by(rooms, filter, |r| r.name.as_str())
}

/// `items` whose name contains `filter`, ignoring case, those starting with
/// it first, then by name; all of them, in order, without a filter.
fn search_by<T>(items: Vec<T>, filter: &str, name: impl Fn(&T) -> &str) -> Vec<T> {
    let filter = filter.trim().to_lowercase();
    if filter.is_empty() {
        return items;
    }
    let mut found: Vec<_> = (items.into_iter())
        .filter_map(|item| {
            let name = name(&item).to_lowercase();
            let at = name.find(&filter)?;
            Some((at > 0, name, item))
        })
        .collect();
    found.sort_by(|(a_later, a, _), (b_later, b, _)| (a_later, a).cmp(&(b_later, b)));
    found.into_iter().map(|(_, _, item)| item).collect()
}

impl Session {
    /// The room lists from `list`, kept up to date while held: as the list,
    /// the categories, the drafts and the changes made here (tags, topics,
    /// pinned messages, rooms joined) change. Spaces list their direct
    /// chats once their members are loaded, in a later update.
    pub fn room_lists(&self, list: &Live<RoomListState>) -> Live<Arc<RoomLists>> {
        let categories = self.room_categories_updates();
        let drafts = self.rooms_with_drafts_updates();
        let mut follow = Follow {
            client: self.client.clone(),
            own_user: self.user_id(),
            states: Box::pin(list.updates()),
            categories: Box::pin(categories.updates()),
            drafts: Box::pin(drafts.updates()),
            echo_updates: Box::pin(WatchStream::new(self.echoes.subscribe()).map(|_| ())),
            echoes: self.echoes.clone(),
            state: list.get(),
            current: (categories.get(), drafts.get()),
            members: SpaceMembers::default(),
            loads: JoinSet::new(),
        };
        let (first, timeout) = follow.lists();
        Live::spawn(Arc::new(first), |lists| follow.run(timeout, lists))
    }
}

type Updates<T> = std::pin::Pin<Box<dyn Stream<Item = T> + Send>>;

/// What [`Session::room_lists`] follows.
struct Follow {
    client: Client,
    own_user: OwnedUserId,
    states: Updates<RoomListState>,
    categories: Updates<RoomCategories>,
    drafts: Updates<BTreeSet<OwnedRoomId>>,
    echo_updates: Updates<()>,
    echoes: Arc<crate::echo::Echoes>,
    state: RoomListState,
    current: (RoomCategories, BTreeSet<OwnedRoomId>),
    members: SpaceMembers,
    loads: JoinSet<(OwnedRoomId, u64, Result<HashSet<OwnedUserId>>)>,
}

impl Follow {
    /// Send the lists whenever they change, until the room list ends; the
    /// changes made here that they show time out at `timeout`.
    async fn run(mut self, mut timeout: Option<Instant>, lists: watch::Sender<Arc<RoomLists>>) {
        loop {
            let timed_out = async move {
                match timeout {
                    Some(at) => tokio::time::sleep_until(at.into()).await,
                    None => std::future::pending().await,
                }
            };
            tokio::select! {
                next = self.states.next() => match next {
                    Some(state) => self.state = state,
                    None => return,
                },
                Some(next) = self.categories.next() => self.current.0 = next,
                Some(next) = self.drafts.next() => self.current.1 = next,
                Some(()) = self.echo_updates.next() => {}
                Some(Ok((space, count, loaded))) = self.loads.join_next() => match loaded {
                    Ok(ids) => self.members.loaded(space, count, ids),
                    Err(e) => warn!(%space, "could not load the space's members: {e}"),
                },
                () = timed_out => {}
            }
            let (now, next) = self.lists();
            timeout = next;
            lists.send_if_modified(|sent| {
                let changed = **sent != now;
                if changed {
                    *sent = Arc::new(now);
                }
                changed
            });
        }
    }

    /// The lists from what is known now, and when the changes made here
    /// that they show time out. Starts loading the members of spaces that
    /// are due.
    fn lists(&mut self) -> (RoomLists, Option<Instant>) {
        for (space, count) in self.members.due(&self.state.rooms) {
            let client = self.client.clone();
            self.loads.spawn(async move {
                let loaded = joined_member_ids(&client, &space).await;
                (space, count, loaded)
            });
        }
        let mut state = self.state.clone();
        let timeout = self.echoes.lay_over(&mut state.rooms, Instant::now());
        let (categories, drafts) = self.current.clone();
        let members = self.members.known().clone();
        let mut lists = RoomLists::new(state, categories, members, &self.own_user, drafts);
        lists.joining = self.echoes.joining();
        (lists, timeout)
    }
}

/// The joined members of `room`.
pub(crate) async fn joined_member_ids(
    client: &Client,
    room: &RoomId,
) -> Result<HashSet<OwnedUserId>> {
    let room =
        (client.get_room(room)).ok_or_else(|| crate::Error::RoomNotFound(room.to_string()))?;
    Ok(room
        .members(RoomMemberships::JOIN)
        .await?
        .iter()
        .map(|member| member.user_id().to_owned())
        .collect())
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, UNIX_EPOCH};

    use matrix_sdk::ruma::user_id;

    use super::*;
    use crate::{
        RoomTag,
        spaces::tests::{direct, list, room},
    };

    fn id(id: &str) -> OwnedRoomId {
        id.try_into().unwrap()
    }

    fn strs(ids: &[OwnedRoomId]) -> Vec<&str> {
        ids.iter().map(|r| r.as_str()).collect()
    }

    fn found_ids(rooms: Vec<&RoomSummary>) -> Vec<&str> {
        rooms.iter().map(|r| r.id.as_str()).collect()
    }

    fn lists(rooms: Vec<RoomSummary>, members: &[(&str, &[&str])]) -> RoomLists {
        lists_with(rooms, members, RoomCategories::default())
    }

    fn lists_with(
        rooms: Vec<RoomSummary>,
        members: &[(&str, &[&str])],
        categories: RoomCategories,
    ) -> RoomLists {
        let members = (members.iter())
            .map(|(space, users)| {
                let users = users.iter().map(|u| (*u).try_into().unwrap()).collect();
                (id(space), users)
            })
            .collect();
        let state = RoomListState {
            rooms,
            loaded: true,
        };
        RoomLists::new(
            state,
            categories,
            members,
            user_id!("@me:x"),
            BTreeSet::new(),
        )
    }

    fn named(name: &str, summary: RoomSummary) -> RoomSummary {
        RoomSummary {
            name: name.into(),
            ..summary
        }
    }

    fn invited(summary: RoomSummary) -> RoomSummary {
        RoomSummary {
            invited: true,
            ..summary
        }
    }

    fn unread(count: u64, summary: RoomSummary) -> RoomSummary {
        RoomSummary {
            unread_messages: count,
            ..summary
        }
    }

    /// The phones' rooms, most recently active first: spaces Team and
    /// Other; Lab in Team, Deep in Team's nested space Sub; Loose in no
    /// space; a direct chat with Bob, a member of Team (unread); invites to
    /// Offer, in Team, and from Carol, in no space, to a direct chat and
    /// Party. Loose is unread.
    fn phone() -> RoomLists {
        lists(
            vec![
                invited(named("Carol", direct("!carol", "@carol:x"))),
                invited(named("Party", room("!party", RoomKind::Room, &[]))),
                invited(named("Offer", room("!offer", RoomKind::Room, &["!team"]))),
                unread(2, named("Bob", direct("!bob", "@bob:x"))),
                unread(1, named("Loose", room("!loose", RoomKind::Room, &[]))),
                named("Deep", room("!deep", RoomKind::Room, &["!sub"])),
                named("Sub", room("!sub", RoomKind::Space, &["!team"])),
                named("Lab", room("!lab", RoomKind::Room, &["!team"])),
                named("Team", room("!team", RoomKind::Space, &[])),
                named("Other", room("!other", RoomKind::Space, &[])),
            ],
            &[("!team", &["@bob:x", "@me:x"])],
        )
    }

    #[test]
    fn spaces_list_their_rooms_in_list_order() {
        let lists = lists(list(), &[("!team:x", &["@bob:x", "@me:x"])]);
        let keys: Vec<_> = lists.spaces.iter().map(|e| e.key.clone()).collect();
        assert_eq!(
            keys,
            [
                SpaceKey::Space(id("!other:x")),
                SpaceKey::Space(id("!team:x")),
                SpaceKey::Outside
            ]
        );
        // Through the nested space (not itself), invites below it, direct
        // chats with its members.
        assert_eq!(
            strs(&lists.spaces[1].rooms),
            [
                "!bob:x", "!lab:x", "!offer:x", "!deep:x", "!club:x", "!both:x"
            ]
        );
        assert_eq!(strs(&lists.spaces[0].rooms), ["!both:x"]);
        // No direct chats, not even those no space lists.
        assert_eq!(strs(&lists.spaces[2].rooms), ["!loose:x", "!guild:x"]);

        // Without its members, a space lists no direct chats.
        let lists = super::tests::lists(list(), &[]);
        assert!(!lists.spaces[1].rooms.contains(&id("!bob:x")));
        assert!(!lists.spaces[2].rooms.contains(&id("!bob:x")));
    }

    #[test]
    fn rooms_put_into_a_category_of_a_space_are_listed_there() {
        let mut categories = RoomCategories::default();
        let layout = categories.layout_mut(Some(&id("!other:x")));
        let mine = layout.add_category("Mine");
        layout.place(&id("!loose:x"), Some(&mine));
        layout.place(&id("!carol:x"), Some(&mine));
        let lists = lists_with(list(), &[], categories);
        assert_eq!(
            strs(&lists.spaces[0].rooms),
            ["!loose:x", "!carol:x", "!both:x"]
        );
        assert_eq!(strs(&lists.spaces[2].rooms), ["!guild:x"]);
    }

    #[test]
    fn the_phones_home_comes_last_while_it_lists_any() {
        let lists = phone();
        assert_eq!(lists.spaces.len(), 3);
        assert_eq!(strs(&lists.spaces[2].rooms), ["!party", "!loose"]);
        let names: Vec<_> = lists.spaces.iter().map(|e| lists.entry_name(e)).collect();
        assert_eq!(names, ["Team", "Other", "Home"]);
        let top: Vec<_> = (lists.top_level_spaces().iter())
            .map(|r| r.id.as_str())
            .collect();
        assert_eq!(top, ["!team", "!other"]);

        let only_spaces = super::tests::lists(
            vec![
                room("!lab", RoomKind::Room, &["!team"]),
                room("!team", RoomKind::Space, &[]),
            ],
            &[],
        );
        assert_eq!(only_spaces.spaces.len(), 1);
        assert_eq!(only_spaces.space(&SpaceKey::Outside), None);
    }

    /// A space's direct chats don't light its dot; the phones' Home has
    /// none. The desktop's Home counts them.
    #[test]
    fn unread_dots_leave_out_a_spaces_direct_chats() {
        let mut lists = phone();
        let dots: Vec<_> = lists.spaces.iter().map(|e| e.unread).collect();
        assert_eq!(dots, [false, false, true]);
        let grouping = Grouping::default();
        assert!(lists.unread(&SpaceKey::Home, &grouping));

        lists.rooms[3].unread_messages = 0;
        lists.rooms[5].unread_messages = 1;
        let lists = RoomLists::new(
            RoomListState {
                rooms: lists.rooms,
                loaded: true,
            },
            lists.categories,
            lists.members,
            user_id!("@me:x"),
            BTreeSet::new(),
        );
        let dots: Vec<_> = lists.spaces.iter().map(|e| e.unread).collect();
        assert_eq!(dots, [true, false, true]);
        // Deep, through the nested space; Loose still lights Home.
        assert!(lists.unread(&SpaceKey::Space(id("!sub")), &grouping));
        assert!(lists.unread(&SpaceKey::Home, &grouping));
    }

    #[test]
    fn an_unread_room_in_a_category_of_the_space_lights_it() {
        let space = || room("!space:x", RoomKind::Space, &[]);
        let rooms = vec![space(), unread(1, room("!loose:x", RoomKind::Room, &[]))];
        let key = SpaceKey::Space(id("!space:x"));
        let grouping = Grouping::default();
        assert!(!lists(rooms.clone(), &[]).unread(&key, &grouping));
        let mut categories = RoomCategories::default();
        let layout = categories.layout_mut(Some(&id("!space:x")));
        let mine = layout.add_category("Mine");
        layout.place(&id("!loose:x"), Some(&mine));
        assert!(lists_with(rooms, &[], categories).unread(&key, &grouping));
    }

    #[test]
    fn chats_badges_and_back_counts() {
        let lists = phone();
        assert_eq!(strs(&lists.chats), ["!carol", "!bob"]);
        // Unread messages: Bob 2, Loose 1, and an unread invite.
        let mut rooms = lists.rooms.clone();
        rooms[1].unread_messages = 4;
        let lists = super::tests::lists(rooms, &[("!team", &["@bob:x"])]);
        assert_eq!(
            lists.badges,
            Badges {
                chats: 2,
                rooms: 1,
                spaces: 0
            }
        );
        // Not the invite.
        assert_eq!(lists.unread_besides(&id("!loose")), 2);
        assert_eq!(lists.unread_besides(&id("!bob")), 1);

        let mut rooms = lists.rooms.clone();
        rooms[7].unread_messages = 5;
        let lists = super::tests::lists(rooms, &[]);
        assert_eq!(lists.badges.spaces, 5);
        assert_eq!(lists.badges.rooms, 6);
    }

    #[test]
    fn the_notes_at_the_top_leave_the_chats_and_sections() {
        let rooms = vec![
            named("Notes", direct("!notes", "@me:x")),
            named("Ann", direct("!ann", "@ann:x")),
            named("Lab", room("!lab", RoomKind::Room, &[])),
        ];
        let lists = super::tests::lists(rooms.clone(), &[]);
        assert_eq!(lists.notes, Some(id("!notes")));
        assert_eq!(strs(&lists.chats), ["!ann"]);
        let grouping = Grouping::default();
        let home = lists.sections(&SpaceKey::Home, &grouping);
        assert_eq!(found_ids(home[1].1.clone()), ["!ann"]);
        // Found by the chats' search.
        assert_eq!(found_ids(lists.search(None, &grouping, "no")), ["!notes"]);

        let categories = RoomCategories {
            notes_at_top: Some(false),
            ..Default::default()
        };
        let off = lists_with(rooms, &[], categories);
        assert_eq!(off.notes, None);
        assert_eq!(strs(&off.chats), ["!notes", "!ann"]);
        let home = off.sections(&SpaceKey::Home, &grouping);
        assert_eq!(found_ids(home[1].1.clone()), ["!notes", "!ann"]);
    }

    /// The phones' Home first, then Team; Other has no rooms. No direct
    /// chats, nested spaces or invites to spaces; invites to rooms are
    /// among them.
    #[test]
    fn the_rooms_tabs_groups() {
        let lists = phone();
        let none = BTreeSet::new();
        let groups = lists.room_groups(None, &none);
        let keys: Vec<_> = groups.iter().map(|g| g.key.as_str()).collect();
        assert_eq!(keys, [RoomLists::OUTSIDE_GROUP, "!team"]);
        let titles: Vec<_> = groups.iter().map(|g| g.title.as_str()).collect();
        assert_eq!(titles, ["Home", "Team"]);
        assert_eq!(groups[0].space, None);
        assert_eq!(groups[1].space, Some(id("!team")));
        assert_eq!(strs(&groups[0].rooms), ["!party", "!loose"]);
        assert_eq!(strs(&groups[1].rooms), ["!offer", "!deep", "!lab"]);
        assert_eq!((groups[0].unread, groups[0].mentions), (1, false));

        // Among those shared with someone.
        let among = BTreeSet::from([id("!lab"), id("!bob"), id("!team"), id("!loose")]);
        let groups = lists.room_groups(Some(&among), &none);
        let rooms: Vec<_> = groups.iter().map(|g| strs(&g.rooms)).collect();
        assert_eq!(rooms, [vec!["!loose"], vec!["!lab"]]);
        let only_bob = BTreeSet::from([id("!bob")]);
        assert!(lists.room_groups(Some(&only_bob), &none).is_empty());

        let folded = BTreeSet::from(["!team".to_owned()]);
        let groups = lists.room_groups(None, &folded);
        assert_eq!((groups[0].folded, groups[1].folded), (false, true));
    }

    /// Newest first, those without activity last; a room in two spaces in
    /// both.
    #[test]
    fn the_rooms_tabs_groups_by_latest_message() {
        let at = |secs: u64| Some(UNIX_EPOCH + Duration::from_secs(secs));
        let active = |secs, summary| RoomSummary {
            last_activity: at(secs),
            ..summary
        };
        let lists = lists(
            vec![
                active(60, room("!a", RoomKind::Room, &["!one", "!two"])),
                room("!b", RoomKind::Room, &["!one", "!two"]),
                active(120, room("!c", RoomKind::Room, &["!one"])),
                room("!one", RoomKind::Space, &[]),
                room("!two", RoomKind::Space, &[]),
            ],
            &[],
        );
        let groups = lists.room_groups(None, &BTreeSet::new());
        let rooms: Vec<_> = groups.iter().map(|g| strs(&g.rooms)).collect();
        assert_eq!(rooms, [vec!["!c", "!a", "!b"], vec!["!a", "!b"]]);
    }

    /// Through the nested space, in the spaces page's order; none for a
    /// room in no space or a direct chat.
    #[test]
    fn a_rooms_first_space_and_a_chats_spaces() {
        let lists = phone();
        let first = |room| lists.first_space_of(&id(room)).map(|s| s.id.as_str());
        assert_eq!(first("!deep"), Some("!team"));
        assert_eq!(first("!loose"), None);
        assert_eq!(first("!bob"), None);
        assert_eq!(lists.chat_spaces(&id("!bob")), ["Team"]);
        assert!(lists.chat_spaces(&id("!carol")).is_empty());

        let both = super::tests::lists(
            vec![
                direct("!bob", "@bob:x"),
                room("!lab", RoomKind::Room, &["!one", "!two"]),
                named("One", room("!one", RoomKind::Space, &[])),
                named("Two", room("!two", RoomKind::Space, &[])),
            ],
            &[("!one", &["@bob:x"]), ("!two", &["@bob:x"])],
        );
        assert_eq!(both.chat_spaces(&id("!bob")), ["One", "Two"]);
        let first = both.first_space_of(&id("!lab")).map(|s| s.id.as_str());
        assert_eq!(first, Some("!one"));
    }

    /// A space's list, with Lab and Bob never active.
    fn sorted(sort: RoomSort, filter: &str) -> Vec<(String, Vec<String>)> {
        let at = |secs: u64| Some(UNIX_EPOCH + Duration::from_secs(1_000_000 - secs));
        let active = |secs, summary| RoomSummary {
            last_activity: at(secs),
            ..summary
        };
        let plain = |name: &str| named(name, room(&format!("!{name}"), RoomKind::Room, &["!s"]));
        let chat = |name: &str| named(name, direct(&format!("!{name}"), &format!("@{name}:x")));
        let lists = lists(
            vec![
                plain("Lab"),
                active(60, chat("anna")),
                active(3600, plain("Field work")),
                chat("Bob"),
                active(0, plain("general")),
                active(7200, chat("Cleo")),
                room("!s", RoomKind::Space, &[]),
            ],
            &[("!s", &["@anna:x", "@Bob:x", "@Cleo:x"])],
        );
        let sections = lists.space_list(
            &SpaceKey::Space(id("!s")),
            &SectionLayout::default(),
            sort,
            &Grouping::default(),
            filter,
        );
        (sections.into_iter())
            .map(|s| {
                let names = (s.rooms.iter())
                    .map(|r| lists.room(r).unwrap().name.clone())
                    .collect();
                (s.title, names)
            })
            .collect()
    }

    fn section(title: &str, names: &[&str]) -> (String, Vec<String>) {
        (title.into(), names.iter().map(|n| n.to_string()).collect())
    }

    #[test]
    fn a_list_in_each_sort() {
        assert_eq!(
            sorted(RoomSort::ListOrder, ""),
            [
                section("Direct messages", &["anna", "Bob", "Cleo"]),
                section("Rooms", &["Lab", "Field work", "general"]),
            ]
        );
        assert_eq!(
            sorted(RoomSort::AToZ, "")[1],
            section("Rooms", &["Field work", "general", "Lab"])
        );
        assert_eq!(
            sorted(RoomSort::ZToA, "")[0],
            section("Direct messages", &["Cleo", "Bob", "anna"])
        );
        assert_eq!(
            sorted(RoomSort::MostRecent, ""),
            [
                section("Direct messages", &["anna", "Cleo", "Bob"]),
                section("Rooms", &["general", "Field work", "Lab"]),
            ]
        );
        // Searching: names starting with the text first, whatever the sort.
        assert_eq!(
            sorted(RoomSort::ZToA, "l"),
            [
                section("Direct messages", &["Cleo"]),
                section("Rooms", &["Lab", "Field work", "general"]),
            ]
        );
    }

    #[test]
    fn equal_activity_keeps_the_lists_order() {
        let at = Some(UNIX_EPOCH);
        let rooms = [
            RoomSummary {
                last_activity: at,
                ..room("!b", RoomKind::Room, &[])
            },
            RoomSummary {
                last_activity: at,
                ..room("!a", RoomKind::Room, &[])
            },
            room("!c", RoomKind::Room, &[]),
            room("!d", RoomKind::Room, &[]),
        ];
        let sorted = RoomSort::MostRecent.apply(rooms.iter().collect());
        assert_eq!(found_ids(sorted), ["!b", "!a", "!c", "!d"]);
    }

    /// Team's sections: Ann's chat, Paper in Papers, Lab in Rooms, Old low
    /// priority; the Notes to self at the top.
    fn team() -> RoomLists {
        let mut categories = RoomCategories::default();
        let layout = categories.layout_mut(Some(&id("!team")));
        let papers = layout.add_category("Papers");
        layout.place(&id("!paper"), Some(&papers));
        lists_with(
            vec![
                named("Notes", direct("!notes", "@me:x")),
                unread(2, named("Ann", direct("!ann", "@ann:x"))),
                unread(3, named("Lab", room("!lab", RoomKind::Room, &["!team"]))),
                RoomSummary {
                    unread_mentions: 1,
                    ..unread(
                        1,
                        named("Paper", room("!paper", RoomKind::Room, &["!team"])),
                    )
                },
                RoomSummary {
                    tag: RoomTag::LowPriority,
                    ..named("Old", room("!old", RoomKind::Room, &["!team"]))
                },
                named("Team", room("!team", RoomKind::Space, &[])),
            ],
            &[("!team", &["@ann:x", "@me:x"])],
            categories,
        )
    }

    fn titles(sections: &[ListSection]) -> Vec<&str> {
        sections.iter().map(|s| s.title.as_str()).collect()
    }

    #[test]
    fn a_spaces_sections_in_order_without_empty_ones() {
        let lists = team();
        let key = SpaceKey::Space(id("!team"));
        let grouping = Grouping::default();
        let list = |layout: &SectionLayout, filter| {
            lists.space_list(&key, layout, RoomSort::ListOrder, &grouping, filter)
        };
        let sections = list(&SectionLayout::default(), "");
        assert_eq!(
            titles(&sections),
            ["Direct messages", "Papers", "Rooms", "Low priority"]
        );
        assert_eq!(strs(&sections[0].rooms), ["!ann"]);
        assert_eq!((sections[1].unread, sections[1].mentions), (1, true));

        // Folded, hidden and reordered here.
        let choices = lists.section_choices(&key, &SectionLayout::default(), &grouping);
        let keys: Vec<String> = choices.iter().map(|c| c.key.clone()).collect();
        let papers = keys[3].clone();
        assert_eq!(
            keys,
            [
                "invites",
                "direct_messages",
                "favourites",
                &papers,
                "rooms",
                "low_priority"
            ]
        );
        let layout = SectionLayout {
            order: vec!["rooms".into(), "invites".into()],
            hidden: BTreeSet::from(["low_priority".into()]),
            folded: BTreeSet::from(["rooms".into()]),
        };
        let sections = list(&layout, "");
        assert_eq!(titles(&sections), ["Rooms", "Direct messages", "Papers"]);
        assert!(sections[0].folded);
        assert_eq!(sections[0].unread, 3);
        // Searching unfolds them all.
        let found = list(&layout, "la");
        assert_eq!(titles(&found), ["Rooms"]);
        assert!(!found[0].folded);

        // The choices: all, in the order here, with their rooms.
        let choices = lists.section_choices(&key, &layout, &grouping);
        let shown: Vec<_> = (choices.iter())
            .map(|c| (c.title.as_str(), c.shown, c.rooms))
            .collect();
        assert_eq!(
            shown,
            [
                ("Rooms", true, 1),
                // New here: after the one before it in the account's order.
                ("Low priority", false, 1),
                ("Invites", true, 0),
                ("Direct messages", true, 1),
                ("Favourites", true, 0),
                ("Papers", true, 1),
            ]
        );
    }

    #[test]
    fn the_phones_home_has_invites_direct_messages_and_rooms() {
        let lists = phone();
        let choices = lists.section_choices(
            &SpaceKey::Outside,
            &SectionLayout::default(),
            &Grouping::default(),
        );
        let titles: Vec<_> = choices.iter().map(|c| c.title.as_str()).collect();
        assert_eq!(titles, ["Invites", "Direct messages", "Rooms"]);
        let sections = lists.space_list(
            &SpaceKey::Outside,
            &SectionLayout::default(),
            RoomSort::ListOrder,
            &Grouping::default(),
            "",
        );
        assert_eq!(super::tests::titles(&sections), ["Invites", "Rooms"]);
    }

    #[test]
    fn a_layout_keeps_new_sections_in_the_accounts_order() {
        let account: Vec<String> = [
            "invites",
            "direct_messages",
            "favourites",
            "category:a",
            "rooms",
            "low_priority",
        ]
        .map(String::from)
        .into();
        assert_eq!(SectionLayout::default().arranged(&account), account);
        let layout = SectionLayout {
            order: [
                "rooms",
                "invites",
                "direct_messages",
                "favourites",
                "category:a",
                "low_priority",
            ]
            .map(String::from)
            .into(),
            ..SectionLayout::default()
        };
        let mut with_b = account.clone();
        with_b.insert(4, "category:b".into());
        assert_eq!(
            layout.arranged(&with_b),
            [
                "rooms",
                "invites",
                "direct_messages",
                "favourites",
                "category:a",
                "category:b",
                "low_priority"
            ]
        );
        // A section first in the account's order, new: first.
        let mut with_new = vec!["new".to_owned()];
        with_new.extend(account.clone());
        assert_eq!(layout.arranged(&with_new)[0], "new");
        // Gone sections leave.
        let left = layout.arranged(&["rooms".into(), "invites".into()]);
        assert_eq!(left, ["rooms", "invites"]);
    }

    #[test]
    fn the_search_puts_names_starting_with_the_filter_first() {
        let lists = lists(
            vec![
                named("Sea ice", room("!1", RoomKind::Room, &[])),
                named("Ice sheets", room("!2", RoomKind::Room, &[])),
                named("Glaciers", room("!3", RoomKind::Room, &[])),
            ],
            &[],
        );
        let grouping = Grouping::default();
        let search = |filter| found_ids(lists.search(Some(&SpaceKey::Home), &grouping, filter));
        assert_eq!(search(" ICE"), ["!2", "!1"]);
        assert_eq!(search(""), ["!1", "!2", "!3"]);

        let phone = phone();
        let spaces: Vec<_> = (phone.search_spaces("h").into_iter())
            .map(|e| e.key.clone())
            .collect();
        assert_eq!(spaces, [SpaceKey::Outside, SpaceKey::Space(id("!other"))]);
    }

    /// The desktop: what Home or a space lists, nested spaces too; invites
    /// to spaces found.
    #[test]
    fn the_search_finds_what_the_space_or_home_lists() {
        let mut categories = RoomCategories::default();
        let lists = |categories: &RoomCategories| {
            lists_with(list(), &[("!team:x", &["@bob:x"])], categories.clone())
        };
        let grouping = Grouping::default();
        let found = |lists: &RoomLists, key: &SpaceKey, filter| {
            let rooms = lists.search(Some(key), &grouping, filter);
            rooms.iter().map(|r| r.id.to_string()).collect::<Vec<_>>()
        };
        let home = lists(&categories);
        assert_eq!(
            found(&home, &SpaceKey::Space(id("!sub:x")), ""),
            ["!deep:x"]
        );
        assert_eq!(
            found(&home, &SpaceKey::Space(id("!team:x")), " !B"),
            ["!bob:x", "!both:x"]
        );
        assert_eq!(found(&home, &SpaceKey::Home, "LOO"), ["!loose:x"]);
        assert_eq!(found(&home, &SpaceKey::Home, "gu"), ["!guild:x"]);
        assert!(found(&home, &SpaceKey::Home, "lab").is_empty());
        // Unless it lists all rooms.
        categories.at_home.all_rooms = true;
        let all = lists(&categories);
        assert_eq!(found(&all, &SpaceKey::Home, "lab"), ["!lab:x"]);
    }

    /// The desktop's Home: invites, direct chats, rooms in no space; the
    /// favourites of spaces or all rooms as set.
    #[test]
    fn the_desktops_home_sections() {
        let mut rooms = vec![
            room("!space:x", RoomKind::Space, &[]),
            unread(1, room("!inside:x", RoomKind::Room, &["!space:x"])),
            direct("!bob:x", "@bob:x"),
            room("!loose:x", RoomKind::Room, &[]),
        ];
        rooms.push(RoomSummary {
            tag: RoomTag::Favourite,
            ..room("!star:x", RoomKind::Room, &["!space:x"])
        });
        rooms.push(invited(room("!offer:x", RoomKind::Room, &["!space:x"])));
        let grouping = Grouping::default();
        let home = |categories: &RoomCategories| {
            let lists = lists_with(rooms.clone(), &[], categories.clone());
            let sections: Vec<_> = (lists.sections(&SpaceKey::Home, &grouping).into_iter())
                .filter(|(_, rooms)| !rooms.is_empty())
                .map(|(section, rooms)| {
                    let title = lists.section_title(&SpaceKey::Home, &section);
                    (title, rooms.iter().map(|r| r.id.to_string()).collect())
                })
                .collect();
            (sections, lists.unread(&SpaceKey::Home, &grouping))
        };
        let section = |name: &str, ids: &[&str]| -> (String, Vec<String>) {
            (name.into(), ids.iter().map(|id| id.to_string()).collect())
        };
        let mut categories = RoomCategories::default();
        assert_eq!(
            home(&categories),
            (
                vec![
                    section("Invites", &["!offer:x"]),
                    section("Direct messages", &["!bob:x"]),
                    section("Rooms with no space", &["!loose:x"]),
                ],
                false
            )
        );
        categories.at_home.favourites = true;
        assert_eq!(home(&categories).0[2], section("Favourites", &["!star:x"]));
        categories.at_home.all_rooms = true;
        assert_eq!(
            home(&categories),
            (
                vec![
                    section("Invites", &["!offer:x"]),
                    section("Direct messages", &["!bob:x"]),
                    section("Favourites", &["!star:x"]),
                    section("Rooms", &["!inside:x", "!loose:x"]),
                ],
                true
            )
        );
        // A space's rooms are always just "Rooms".
        let lists = lists_with(rooms.clone(), &[], categories);
        let key = SpaceKey::Space(id("!space:x"));
        assert_eq!(lists.section_title(&key, &Section::Rooms), "Rooms");
    }
}
