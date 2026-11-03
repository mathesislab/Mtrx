//! The room lists as the apps show them, computed by the core: the chats,
//! the spaces, a space's sections, the Rooms tab's groups, the search and
//! the unread counts.

use std::{
    collections::{BTreeSet, HashMap},
    sync::Arc,
};

use mtrx_core::{self as core, OwnedRoomId};

use crate::{
    MtrxError, RoomKind, RoomSummary, Session, TaskHandle,
    runtime::{follow, run},
};

/// What a room list lists.
#[derive(Clone, Debug, PartialEq, Eq, Hash, uniffi::Enum)]
pub enum SpaceKey {
    /// A space (its `RoomSummary.id`).
    Space { id: String },
    /// The desktop's Home, with the direct chats; the phones show
    /// `Outside` instead.
    Home,
    /// The phones' Home: rooms and invites no top-level space lists.
    Outside,
}

/// An entry of the spaces page: a top-level space, or `Outside`.
#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct SpaceEntry {
    pub key: SpaceKey,
    /// The space's summary, for its name and picture; none for `Outside`
    /// ("Home").
    pub space: Option<RoomSummary>,
    /// Any of its rooms has unread messages, not counting direct chats.
    pub unread: bool,
}

/// Unread messages for the tabs' badges.
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Record)]
pub struct Badges {
    /// In the direct chats, the Notes to self and invites too.
    pub chats: u64,
    /// In the joined rooms.
    pub rooms: u64,
    /// In the joined rooms the spaces list.
    pub spaces: u64,
}

/// The order of the rooms in a list's sections.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, uniffi::Enum)]
pub enum RoomSort {
    /// The room list's: most recently active first.
    ListOrder,
    AToZ,
    ZToA,
    /// By the last activity, newest first; rooms without one last.
    MostRecent,
}

/// How the phone shows a list's sections, by `ListSection.key`: their
/// order once reordered here (empty: the account's), those hidden and
/// those folded. Kept by the app.
#[derive(Clone, Debug, Default, PartialEq, Eq, uniffi::Record)]
pub struct SectionLayout {
    pub order: Vec<String>,
    pub hidden: Vec<String>,
    pub folded: Vec<String>,
}

/// A titled part of a list, folded or not.
#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct ListSection {
    /// Stable across updates and devices: a section's "invites",
    /// "favourites", "direct_messages", "rooms", "low_priority" or
    /// "category:<id>"; a Rooms tab group's space ID, or "home".
    pub key: String,
    pub title: String,
    pub rooms: Vec<RoomSummary>,
    /// Only its heading shows, with its unread count.
    pub folded: bool,
    /// Unread messages in its rooms.
    pub unread: u64,
    /// Any of them mentions us.
    pub mentions: bool,
    /// The space whose rooms it holds (a Rooms tab group's), which its
    /// heading opens.
    pub space: Option<String>,
}

/// A section to show or hide, in the phone's order.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct SectionChoice {
    pub key: String,
    pub title: String,
    pub shown: bool,
    /// The number of its rooms.
    pub rooms: u64,
}

/// A room or space below a space, with how deep (0 for what the space
/// lists itself).
#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct TreeEntry {
    pub depth: u32,
    pub room: RoomSummary,
}

/// A category of a space's list (Home's for no `space`), for the lists
/// made up in tests and previews.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct TestCategory {
    pub space: Option<String>,
    /// Its section's key is "category:<id>".
    pub id: String,
    pub name: String,
    pub rooms: Vec<String>,
}

/// Receives the room lists, now and after every change.
#[uniffi::export(with_foreign)]
pub trait RoomListsListener: Send + Sync {
    fn on_update(&self, lists: Arc<RoomLists>);
}

/// The room lists, from one room list and what arranges it.
#[derive(uniffi::Object)]
pub struct RoomLists {
    inner: Arc<core::RoomLists>,
}

impl RoomLists {
    fn summaries<'a>(&self, ids: impl IntoIterator<Item = &'a OwnedRoomId>) -> Vec<RoomSummary> {
        (ids.into_iter())
            .filter_map(|id| self.inner.room(id))
            .map(RoomSummary::from)
            .collect()
    }

    fn entry(&self, entry: &core::SpaceEntry) -> SpaceEntry {
        let space = match &entry.key {
            core::SpaceKey::Space(id) => self.inner.room(id).map(RoomSummary::from),
            _ => None,
        };
        SpaceEntry {
            key: (&entry.key).into(),
            space,
            unread: entry.unread,
        }
    }

    fn section(&self, section: core::ListSection) -> ListSection {
        ListSection {
            rooms: self.summaries(&section.rooms),
            key: section.key,
            title: section.title,
            folded: section.folded,
            unread: section.unread,
            mentions: section.mentions,
            space: section.space.map(|s| s.to_string()),
        }
    }
}

#[uniffi::export]
impl RoomLists {
    /// The lists of `rooms`, for tests and previews: as the library makes
    /// them, with what each joined space lists (`space_children`, room IDs
    /// by space ID, in order) and the members of the spaces by space ID,
    /// for their direct chats; the rooms tagged `favourites` and
    /// `low_priority` by ID, the `categories`, and the rooms `joining`.
    #[uniffi::constructor]
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        rooms: Vec<RoomSummary>,
        loaded: bool,
        own_user_id: String,
        notes_at_top: bool,
        space_children: HashMap<String, Vec<String>>,
        space_members: HashMap<String, Vec<String>>,
        drafts: Vec<String>,
        favourites: Vec<String>,
        low_priority: Vec<String>,
        categories: Vec<TestCategory>,
        joining: Vec<String>,
    ) -> Result<Arc<Self>, MtrxError> {
        let rooms = (rooms.iter())
            .map(|room| {
                let mut summary = core::RoomSummary::try_from(room)?;
                summary.tag = if favourites.contains(&room.id) {
                    core::RoomTag::Favourite
                } else if low_priority.contains(&room.id) {
                    core::RoomTag::LowPriority
                } else {
                    core::RoomTag::None
                };
                Ok(summary)
            })
            .collect::<Result<_, MtrxError>>()?;
        let graph = space_children
            .iter()
            .map(|(space, children)| {
                let children = (children.iter())
                    .map(|room| {
                        Ok(core::Child {
                            room: crate::error::room_id(room)?,
                            suggested: false,
                            via: Vec::new(),
                            order: None,
                        })
                    })
                    .collect::<Result<Vec<_>, MtrxError>>()?;
                Ok((crate::error::room_id(space)?, children))
            })
            .collect::<Result<core::SpaceGraph, MtrxError>>()?;
        let members = space_members
            .iter()
            .map(|(space, users)| {
                let users = users.iter().map(|u| crate::error::user_id(u));
                Ok((
                    crate::error::room_id(space)?,
                    users.collect::<Result<_, _>>()?,
                ))
            })
            .collect::<Result<_, MtrxError>>()?;
        let drafts = drafts
            .iter()
            .map(|id| crate::error::room_id(id))
            .collect::<Result<_, _>>()?;
        let mut room_categories = core::RoomCategories {
            notes_at_top: Some(notes_at_top),
            ..Default::default()
        };
        for category in categories {
            let space = (category.space.as_deref())
                .map(crate::error::room_id)
                .transpose()?;
            let rooms = (category.rooms.iter())
                .map(|id| crate::error::room_id(id))
                .collect::<Result<_, _>>()?;
            let layout = room_categories.layout_mut(space.as_deref());
            layout.categories.push(core::Category {
                id: category.id,
                name: category.name,
                rooms,
                sort: core::Sort::default(),
            });
        }
        let mut lists = core::RoomLists::new(
            core::RoomListState {
                rooms,
                loaded,
                graph: Arc::new(graph),
            },
            room_categories,
            members,
            &crate::error::user_id(&own_user_id)?,
            drafts,
        );
        lists.joining = joining
            .iter()
            .map(|id| crate::error::room_id(id))
            .collect::<Result<_, _>>()?;
        Ok(Arc::new(Self::from(Arc::new(lists))))
    }

    /// No rooms, as before the first update.
    #[uniffi::constructor]
    pub fn empty() -> Arc<Self> {
        Arc::new(Self::from(Arc::default()))
    }

    /// Every joined and invited room (spaces too), most recently active
    /// first.
    pub fn rooms(&self) -> Vec<RoomSummary> {
        self.inner.rooms.iter().map(RoomSummary::from).collect()
    }

    pub fn room(&self, room_id: String) -> Option<RoomSummary> {
        let id = OwnedRoomId::try_from(room_id).ok()?;
        self.inner.room(&id).map(RoomSummary::from)
    }

    /// Synced with the server since the list was created. Before that,
    /// the rooms may be a stale cache, or none on the first sign-in.
    pub fn loaded(&self) -> bool {
        self.inner.loaded
    }

    /// The Notes to self, while kept at the top of the Chats tab and of
    /// each space's list.
    pub fn notes(&self) -> Option<RoomSummary> {
        self.summaries(&self.inner.notes).pop()
    }

    /// Whether the Notes to self stay at the top (one choice for all the
    /// user's devices; set with `Session.setNotesAtTop`).
    pub fn notes_at_top(&self) -> bool {
        self.inner.notes_at_top()
    }

    /// The Chats tab: the direct chats, invites to them too; not the Notes
    /// to self at the top.
    pub fn chats(&self) -> Vec<RoomSummary> {
        self.summaries(&self.inner.chats)
    }

    /// The spaces page: the top-level spaces, then `Outside` while it
    /// lists any.
    pub fn spaces(&self) -> Vec<SpaceEntry> {
        self.inner.spaces.iter().map(|e| self.entry(e)).collect()
    }

    /// The spaces page's entry of `key`, while listed.
    pub fn space(&self, key: SpaceKey) -> Option<SpaceEntry> {
        Some(self.entry(self.inner.space(&key.core()?)?))
    }

    /// The spaces page's entries whose name contains `filter`, those
    /// starting with it first, then by name.
    pub fn search_spaces(&self, filter: String) -> Vec<SpaceEntry> {
        (self.inner.search_spaces(&filter).into_iter())
            .map(|e| self.entry(e))
            .collect()
    }

    /// The top-level spaces, in order: where someone can be invited.
    pub fn top_level_spaces(&self) -> Vec<RoomSummary> {
        (self.inner.top_level_spaces().into_iter())
            .map(RoomSummary::from)
            .collect()
    }

    /// IDs of the rooms with a draft on this device, marked with a pencil.
    pub fn drafts(&self) -> Vec<String> {
        self.inner.drafts.iter().map(ToString::to_string).collect()
    }

    /// IDs of the rooms and spaces joined or made here that are not listed
    /// as joined yet: coming, not gone.
    pub fn joining(&self) -> Vec<String> {
        self.inner.joining.iter().map(ToString::to_string).collect()
    }

    pub fn badges(&self) -> Badges {
        let core::Badges {
            chats,
            rooms,
            spaces,
        } = self.inner.badges;
        Badges {
            chats,
            rooms,
            spaces,
        }
    }

    /// `key`'s list: its sections in `layout`'s order, those shown and with
    /// rooms, folded as `layout` says, each's rooms in `sort`'s order; not
    /// the Notes to self at the top. While searching for `filter`, the
    /// rooms found, in the search's order, none folded.
    pub fn space_list(
        &self,
        key: SpaceKey,
        layout: SectionLayout,
        sort: RoomSort,
        filter: String,
    ) -> Vec<ListSection> {
        let Some(key) = key.core() else {
            return Vec::new();
        };
        let grouping = core::Grouping::default();
        (self
            .inner
            .space_list(&key, &layout.into(), sort.into(), &grouping, &filter))
        .into_iter()
        .map(|s| self.section(s))
        .collect()
    }

    /// `key`'s sections in `layout`'s order, empty ones too, shown or not:
    /// for choosing which show.
    pub fn section_choices(&self, key: SpaceKey, layout: SectionLayout) -> Vec<SectionChoice> {
        let Some(key) = key.core() else {
            return Vec::new();
        };
        let grouping = core::Grouping::default();
        (self.inner.section_choices(&key, &layout.into(), &grouping))
            .into_iter()
            .map(|c| SectionChoice {
                key: c.key,
                title: c.title,
                shown: c.shown,
                rooms: c.rooms,
            })
            .collect()
    }

    /// The Rooms tab: Home, then each top-level space's rooms, each newest
    /// first; no direct chats or spaces. Only those `among`, if given (the
    /// rooms shared with someone). Groups whose key is in `folded` are
    /// folded.
    pub fn room_groups(&self, among: Option<Vec<String>>, folded: Vec<String>) -> Vec<ListSection> {
        let among: Option<BTreeSet<OwnedRoomId>> = among.map(|ids| {
            ids.into_iter()
                .filter_map(|id| id.try_into().ok())
                .collect()
        });
        let folded = folded.into_iter().collect();
        (self.inner.room_groups(among.as_ref(), &folded))
            .into_iter()
            .map(|s| self.section(s))
            .collect()
    }

    /// What `key` lists (the direct chats and the Notes to self for none)
    /// whose name contains `filter`, those starting with it first, then by
    /// name.
    pub fn search(&self, key: Option<SpaceKey>, filter: String) -> Vec<RoomSummary> {
        let key = match key.map(|k| k.core()) {
            Some(None) => return Vec::new(),
            Some(key) => key,
            None => None,
        };
        let grouping = core::Grouping::default();
        (self.inner.search(key.as_ref(), &grouping, &filter))
            .into_iter()
            .map(RoomSummary::from)
            .collect()
    }

    /// The number on a room's or chat's back button: unread messages in
    /// the other joined rooms and the direct chats.
    pub fn unread_besides(&self, room_id: String) -> u64 {
        OwnedRoomId::try_from(room_id).map_or(0, |id| self.inner.unread_besides(&id))
    }

    /// The first top-level space, in the spaces page's order, that lists
    /// the room (through nested spaces too); none for a room in no space,
    /// and for direct chats and spaces.
    pub fn first_space_of(&self, room_id: String) -> Option<RoomSummary> {
        let id = OwnedRoomId::try_from(room_id).ok()?;
        self.inner.first_space_of(&id).map(RoomSummary::from)
    }

    /// The names of the top-level spaces that list a direct chat (those of
    /// its person), in the spaces page's order.
    pub fn chat_spaces(&self, chat_id: String) -> Vec<String> {
        match OwnedRoomId::try_from(chat_id) {
            Ok(id) => self.inner.chat_spaces(&id),
            Err(_) => Vec::new(),
        }
    }

    /// The joined spaces that list the room directly.
    pub fn parents(&self, room_id: String) -> Vec<RoomSummary> {
        let Ok(id) = OwnedRoomId::try_from(room_id) else {
            return Vec::new();
        };
        (self.inner.parents(&id).into_iter())
            .map(RoomSummary::from)
            .collect()
    }

    /// The joined spaces the room is in, through nested spaces too,
    /// nearest first.
    pub fn ancestors(&self, room_id: String) -> Vec<RoomSummary> {
        let Ok(id) = OwnedRoomId::try_from(room_id) else {
            return Vec::new();
        };
        (self.inner.ancestors(&id).into_iter())
            .map(RoomSummary::from)
            .collect()
    }

    /// What a space lists, through nested spaces too, depth first; rooms
    /// we are not in or invited to are left out.
    pub fn tree(&self, space_id: String) -> Vec<TreeEntry> {
        let Ok(id) = OwnedRoomId::try_from(space_id) else {
            return Vec::new();
        };
        (self.inner.tree(&id).into_iter())
            .map(|(depth, room)| TreeEntry {
                depth: u32::try_from(depth).unwrap_or(u32::MAX),
                room: room.into(),
            })
            .collect()
    }
}

impl From<Arc<core::RoomLists>> for RoomLists {
    fn from(inner: Arc<core::RoomLists>) -> Self {
        Self { inner }
    }
}

impl SpaceKey {
    /// None for a space ID that is not one.
    fn core(&self) -> Option<core::SpaceKey> {
        Some(match self {
            SpaceKey::Space { id } => core::SpaceKey::Space(id.as_str().try_into().ok()?),
            SpaceKey::Home => core::SpaceKey::Home,
            SpaceKey::Outside => core::SpaceKey::Outside,
        })
    }
}

impl From<&core::SpaceKey> for SpaceKey {
    fn from(key: &core::SpaceKey) -> Self {
        match key {
            core::SpaceKey::Space(id) => SpaceKey::Space { id: id.to_string() },
            core::SpaceKey::Home => SpaceKey::Home,
            core::SpaceKey::Outside => SpaceKey::Outside,
        }
    }
}

impl From<RoomSort> for core::RoomSort {
    fn from(sort: RoomSort) -> Self {
        match sort {
            RoomSort::ListOrder => core::RoomSort::ListOrder,
            RoomSort::AToZ => core::RoomSort::AToZ,
            RoomSort::ZToA => core::RoomSort::ZToA,
            RoomSort::MostRecent => core::RoomSort::MostRecent,
        }
    }
}

impl From<SectionLayout> for core::SectionLayout {
    fn from(layout: SectionLayout) -> Self {
        Self {
            order: layout.order,
            hidden: layout.hidden.into_iter().collect(),
            folded: layout.folded.into_iter().collect(),
        }
    }
}

impl TryFrom<&RoomSummary> for core::RoomSummary {
    type Error = MtrxError;

    /// Without its tag, which the apps don't get.
    fn try_from(room: &RoomSummary) -> Result<Self, MtrxError> {
        let direct_user = (room.direct_user.as_ref())
            .map(core::User::try_from)
            .transpose()?;
        let pinned = (room.pinned.iter())
            .map(|id| core::OwnedEventId::try_from(id.as_str()).map_err(MtrxError::invalid))
            .collect::<Result<_, _>>()?;
        Ok(Self {
            id: crate::error::room_id(&room.id)?,
            name: room.name.clone(),
            avatar: room.avatar_url.clone().map(Into::into),
            topic: room.topic.clone(),
            kind: match room.kind {
                RoomKind::Room => core::RoomKind::Room,
                RoomKind::Space => core::RoomKind::Space,
                RoomKind::Direct => core::RoomKind::Direct,
            },
            invited: room.invited,
            encrypted: room.encrypted,
            public: room.public,
            unread_messages: room.unread_messages,
            unread_mentions: room.unread_mentions,
            unread_notifications: room.unread_notifications,
            last_activity: room.last_activity,
            tag: core::RoomTag::None,
            direct_user,
            alias: room.alias.clone(),
            joined_members: room.joined_members,
            pinned,
        })
    }
}

#[uniffi::export]
impl Session {
    /// Call `listener` with the room lists, now and on every change. They
    /// fill while sync runs; spaces list their direct chats once their
    /// members are loaded, in a later update.
    pub async fn subscribe_room_lists(
        &self,
        listener: Arc<dyn RoomListsListener>,
    ) -> Result<Arc<TaskHandle>, MtrxError> {
        let session = self.core();
        run(async move {
            let list = session.room_list().await?;
            let lists = session.room_lists(&list);
            Ok(follow(lists, RoomLists::from, move |lists| {
                listener.on_update(Arc::new(lists))
            }))
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn summary(id: &str, kind: RoomKind, name: &str) -> RoomSummary {
        RoomSummary {
            id: id.into(),
            name: name.into(),
            avatar_url: None,
            topic: None,
            kind,
            invited: false,
            encrypted: false,
            unread_messages: 0,
            unread_mentions: 0,
            unread_notifications: 0,
            last_activity: None,
            direct_user: None,
            alias: None,
            joined_members: 0,
            public: false,
            pinned: Vec::new(),
        }
    }

    /// Team lists Sub, which lists Deep.
    #[test]
    fn spaces_from_what_they_list() {
        let lists = RoomLists::new(
            vec![
                summary("!deep:x", RoomKind::Room, "Deep"),
                summary("!sub:x", RoomKind::Space, "Sub"),
                summary("!team:x", RoomKind::Space, "Team"),
            ],
            true,
            "@me:x".into(),
            true,
            HashMap::from([
                ("!team:x".into(), vec!["!sub:x".into()]),
                ("!sub:x".into(), vec!["!deep:x".into()]),
            ]),
            HashMap::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
        )
        .unwrap();
        let names = |rooms: Vec<RoomSummary>| -> Vec<String> {
            rooms.into_iter().map(|r| r.name).collect()
        };
        assert_eq!(names(lists.parents("!deep:x".into())), ["Sub"]);
        assert_eq!(names(lists.ancestors("!deep:x".into())), ["Sub", "Team"]);
        assert_eq!(names(lists.top_level_spaces()), ["Team"]);
        let tree: Vec<_> = (lists.tree("!team:x".into()).into_iter())
            .map(|entry| (entry.depth, entry.room.name))
            .collect();
        assert_eq!(tree, [(0, "Sub".to_owned()), (1, "Deep".to_owned())]);
    }
}
