//! How the room list is arranged: the user's own categories per space (and
//! Home), plus built-in sections for favourites, direct chats and low
//! priority rooms, and which rooms Home lists.
//!
//! Categories are personal. They are stored as account data
//! ([`crate::account_data`]), so they sync to the user's devices but nobody
//! else sees them. Favourites and low priority are the standard Matrix room
//! tags, shared with other clients.

use std::{collections::BTreeMap, time::Instant};

use matrix_sdk::{
    Room,
    ruma::{OwnedRoomId, RoomId, events::macros::EventContent},
};
use serde::{Deserialize, Serialize};

use crate::{
    Live, Result, RoomHandle, RoomKind, RoomSummary, Session, account_data::AccountData,
    scheduled::random_id,
};

/// The user's arrangement of the room list, for Home and each space.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct RoomCategories {
    #[serde(default)]
    pub home: Layout,
    #[serde(default)]
    pub spaces: BTreeMap<OwnedRoomId, Layout>,
    #[serde(default)]
    pub at_home: AtHome,
    /// The Notes to self at the top of the room list, in every space and on
    /// every device; `None` until set (a desktop then saves its own
    /// earlier choice here).
    #[serde(default)]
    pub notes_at_top: Option<bool>,
}

/// Which rooms of spaces Home lists too. It always lists direct chats,
/// invites, rooms in no space and those put into its categories
/// ([`crate::SpaceKey::Home`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AtHome {
    /// Every room, as if there were no spaces.
    pub all_rooms: bool,
    /// Favourites, in the Favourites section (when the room list has it).
    pub favourites: bool,
}

/// The arrangement of one space's room list (or Home's).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Layout {
    /// In the order they take among the sections until the user reorders
    /// them.
    #[serde(default)]
    pub categories: Vec<Category>,
    /// The sections below Invites in display order, once the user reordered
    /// them; empty for the default order ([`Layout::order`]).
    #[serde(default)]
    pub order: Vec<Section>,
    /// Sections shown as their heading only.
    #[serde(default)]
    pub collapsed: Vec<Section>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Category {
    pub id: String,
    pub name: String,
    /// A room is in at most one category of a layout.
    #[serde(default)]
    pub rooms: Vec<OwnedRoomId>,
    #[serde(default)]
    pub sort: Sort,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Sort {
    /// Most recently active first.
    #[default]
    Activity,
    /// A–Z.
    Name,
}

/// A section of the room list, in display order.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Section {
    Invites,
    Favourites,
    /// One of the user's categories, by id.
    Category(String),
    /// Direct chats not in a category.
    People,
    /// Rooms not in a category.
    Rooms,
    LowPriority,
}

/// The standard room tags Mtrx shows as sections.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RoomTag {
    #[default]
    None,
    Favourite,
    LowPriority,
}

/// Which built-in sections the room list has.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Grouping {
    pub favourites: bool,
    /// Direct chats apart from rooms.
    pub people: bool,
    pub low_priority: bool,
    /// Rooms with unread messages first in each section.
    pub unread_first: bool,
}

impl Default for Grouping {
    fn default() -> Self {
        Self {
            favourites: true,
            people: true,
            low_priority: true,
            unread_first: false,
        }
    }
}

impl Grouping {
    /// Whether the room list has `section` (if it is a category: if the
    /// layout has it).
    pub fn has(&self, section: &Section) -> bool {
        match section {
            Section::Favourites => self.favourites,
            Section::People => self.people,
            Section::LowPriority => self.low_priority,
            _ => true,
        }
    }
}

impl RoomCategories {
    /// Whether the Notes to self stay at the top: on until turned off.
    pub fn notes_at_top(&self) -> bool {
        self.notes_at_top.unwrap_or(true)
    }

    /// The layout of `space`, or Home's for `None`.
    pub fn layout(&self, space: Option<&RoomId>) -> &Layout {
        static EMPTY: Layout = Layout {
            categories: Vec::new(),
            order: Vec::new(),
            collapsed: Vec::new(),
        };
        match space {
            Some(space) => self.spaces.get(space).unwrap_or(&EMPTY),
            None => &self.home,
        }
    }

    pub fn layout_mut(&mut self, space: Option<&RoomId>) -> &mut Layout {
        match space {
            Some(space) => self.spaces.entry(space.to_owned()).or_default(),
            None => &mut self.home,
        }
    }
}

impl Layout {
    /// Add an empty category at the end; returns its id.
    pub fn add_category(&mut self, name: &str) -> String {
        let id = random_id();
        self.categories.push(Category {
            id: id.clone(),
            name: name.trim().to_owned(),
            rooms: Vec::new(),
            sort: Sort::default(),
        });
        id
    }

    pub fn category(&self, id: &str) -> Option<&Category> {
        self.categories.iter().find(|c| c.id == id)
    }

    pub fn category_mut(&mut self, id: &str) -> Option<&mut Category> {
        self.categories.iter_mut().find(|c| c.id == id)
    }

    /// Delete a category; its rooms go back to their default section.
    pub fn remove_category(&mut self, id: &str) {
        let section = Section::Category(id.to_owned());
        self.categories.retain(|c| c.id != id);
        self.order.retain(|s| *s != section);
        self.collapsed.retain(|s| *s != section);
    }

    /// The sections below Invites, which always come first, in display
    /// order; the built-in ones whether the room list has them or not. By
    /// default direct chats, favourites, the categories, other rooms and
    /// low priority; sections missing from the user's order (categories
    /// added since) go last.
    pub fn order(&self) -> Vec<Section> {
        let mut order: Vec<Section> = Vec::new();
        for section in &self.order {
            let known = match section {
                Section::Invites => false,
                Section::Category(id) => self.category(id).is_some(),
                _ => true,
            };
            if known && !order.contains(section) {
                order.push(section.clone());
            }
        }
        let default = [Section::People, Section::Favourites]
            .into_iter()
            .chain((self.categories.iter()).map(|c| Section::Category(c.id.clone())))
            .chain([Section::Rooms, Section::LowPriority]);
        for section in default {
            if !order.contains(&section) {
                order.push(section);
            }
        }
        order
    }

    /// Move `section` to the place of `to`, which moves one place towards
    /// where `section` was, as do the sections between them.
    pub fn move_section(&mut self, section: &Section, to: &Section) {
        let mut order = self.order();
        let place = |s| order.iter().position(|o| o == s);
        if let (Some(from), Some(to)) = (place(section), place(to)) {
            let section = order.remove(from);
            order.insert(to, section);
            self.order = order;
        }
    }

    /// Put `room` into a category, or out of all of them for `None`.
    pub fn place(&mut self, room: &RoomId, category: Option<&str>) {
        for c in &mut self.categories {
            c.rooms.retain(|r| r != room);
        }
        if let Some(c) = category.and_then(|id| self.category_mut(id)) {
            c.rooms.push(room.to_owned());
        }
    }

    pub fn category_of(&self, room: &RoomId) -> Option<&Category> {
        self.categories
            .iter()
            .find(|c| c.rooms.iter().any(|r| r == room))
    }

    pub fn is_collapsed(&self, section: &Section) -> bool {
        self.collapsed.contains(section)
    }

    pub fn set_collapsed(&mut self, section: &Section, collapsed: bool) {
        self.collapsed.retain(|s| s != section);
        if collapsed {
            self.collapsed.push(section.clone());
        }
    }

    pub fn toggle_collapsed(&mut self, section: &Section) {
        self.set_collapsed(section, !self.is_collapsed(section));
    }

    /// The section `room` belongs in.
    pub fn section_of(&self, room: &RoomSummary, grouping: &Grouping) -> Section {
        if room.invited {
            Section::Invites
        } else if grouping.favourites && room.tag == RoomTag::Favourite {
            Section::Favourites
        } else if grouping.low_priority && room.tag == RoomTag::LowPriority {
            Section::LowPriority
        } else if let Some(category) = self.category_of(&room.id) {
            Section::Category(category.id.clone())
        } else if grouping.people && room.kind == RoomKind::Direct {
            Section::People
        } else {
            Section::Rooms
        }
    }

    /// `rooms` (most recently active first) sorted into sections, in display
    /// order. Every section the grouping has is listed, even when empty.
    pub fn group<'a>(
        &self,
        rooms: impl IntoIterator<Item = &'a RoomSummary>,
        grouping: &Grouping,
    ) -> Vec<(Section, Vec<&'a RoomSummary>)> {
        let mut sections = vec![(Section::Invites, Vec::new())];
        sections.extend(
            (self.order().into_iter())
                .filter(|s| grouping.has(s))
                .map(|s| (s, Vec::new())),
        );
        for room in rooms {
            let section = self.section_of(room, grouping);
            if let Some((_, list)) = sections.iter_mut().find(|(s, _)| *s == section) {
                list.push(room);
            }
        }
        for (section, list) in &mut sections {
            let by_name = matches!(section, Section::Category(id)
                if self.category(id).is_some_and(|c| c.sort == Sort::Name));
            if by_name {
                list.sort_by_cached_key(|r| r.name.to_lowercase());
            }
            if grouping.unread_first {
                list.sort_by_key(|r| r.unread_messages == 0);
            }
        }
        sections
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, EventContent)]
#[ruma_event(type = "org.mtrx.room_categories", kind = GlobalAccountData)]
pub(crate) struct RoomCategoriesEventContent {
    #[serde(flatten)]
    categories: RoomCategories,
}

impl AccountData for RoomCategories {
    type Content = RoomCategoriesEventContent;

    fn from_content(content: Self::Content) -> Self {
        content.categories
    }

    fn into_content(self) -> Self::Content {
        RoomCategoriesEventContent { categories: self }
    }
}

impl Session {
    pub fn room_categories(&self) -> RoomCategories {
        self.categories.get()
    }

    /// The current categories, then every change, from this device or others.
    pub fn room_categories_updates(&self) -> Live<RoomCategories> {
        self.categories.updates()
    }

    /// Change the categories. The change is published at once and then saved
    /// on the server; if saving fails, the server's version is restored.
    pub async fn change_room_categories(
        &self,
        change: impl FnOnce(&mut RoomCategories),
    ) -> Result<()> {
        self.categories.change(change).await
    }
}

impl RoomHandle {
    /// Set the room's favourite or low priority tag, or remove both. The
    /// room list shows it at once ([`Session::room_lists`]).
    pub async fn set_tag(&self, tag: RoomTag) -> Result<()> {
        let echoes = &self.session.echoes;
        echoes.set_tag(self.id(), tag);
        let result = save_tag(&self.room, tag).await;
        echoes.tag_answered(self.id(), tag, result.is_ok(), Instant::now());
        result
    }
}

async fn save_tag(room: &Room, tag: RoomTag) -> Result<()> {
    match tag {
        RoomTag::Favourite => room.set_is_favourite(true, None).await?,
        RoomTag::LowPriority => room.set_is_low_priority(true, None).await?,
        RoomTag::None => {
            if room.is_favourite() {
                room.set_is_favourite(false, None).await?;
            }
            if room.is_low_priority() {
                room.set_is_low_priority(false, None).await?;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use matrix_sdk::ruma::owned_room_id;

    use super::*;

    fn room(id: &str, name: &str, kind: RoomKind) -> RoomSummary {
        RoomSummary {
            id: OwnedRoomId::try_from(id).unwrap(),
            name: name.into(),
            avatar: None,
            topic: None,
            kind,
            invited: false,
            encrypted: false,
            public: false,
            unread_messages: 0,
            unread_mentions: 0,
            unread_notifications: 0,
            last_activity: None,
            parents: Vec::new(),
            tag: RoomTag::None,
            direct_user: None,
            alias: None,
            joined_members: 0,
            pinned: Vec::new(),
        }
    }

    fn names(sections: &[(Section, Vec<&RoomSummary>)], section: &Section) -> Vec<String> {
        sections
            .iter()
            .find(|(s, _)| s == section)
            .map(|(_, rooms)| rooms.iter().map(|r| r.name.clone()).collect())
            .unwrap_or_default()
    }

    #[test]
    fn rooms_go_to_tags_then_categories_then_their_kind() {
        let mut rooms = vec![
            room("!a:x", "zeta", RoomKind::Room),
            room("!b:x", "alpha", RoomKind::Room),
            room("!c:x", "bob", RoomKind::Direct),
            room("!d:x", "fav", RoomKind::Room),
            room("!e:x", "later", RoomKind::Room),
            room("!f:x", "plain", RoomKind::Room),
        ];
        rooms[3].tag = RoomTag::Favourite;
        rooms[4].tag = RoomTag::LowPriority;
        let mut layout = Layout::default();
        let work = layout.add_category("Work");
        for id in ["!a:x", "!b:x", "!c:x", "!d:x"] {
            layout.place(&OwnedRoomId::try_from(id).unwrap(), Some(&work));
        }
        let grouping = Grouping::default();
        let sections = layout.group(&rooms, &grouping);
        let order: Vec<_> = sections.iter().map(|(s, _)| s.clone()).collect();
        assert_eq!(
            order,
            [
                Section::Invites,
                Section::People,
                Section::Favourites,
                Section::Category(work.clone()),
                Section::Rooms,
                Section::LowPriority,
            ]
        );
        let category = Section::Category(work.clone());
        assert_eq!(names(&sections, &category), ["zeta", "alpha", "bob"]);
        assert_eq!(names(&sections, &Section::Favourites), ["fav"]);
        assert_eq!(names(&sections, &Section::LowPriority), ["later"]);
        assert_eq!(names(&sections, &Section::Rooms), ["plain"]);

        layout.category_mut(&work).unwrap().sort = Sort::Name;
        let sections = layout.group(&rooms, &grouping);
        assert_eq!(names(&sections, &category), ["alpha", "bob", "zeta"]);

        // Without the built-in sections, tags and kinds are ignored.
        let plain = Grouping {
            favourites: false,
            people: false,
            low_priority: false,
            unread_first: false,
        };
        let sections = layout.group(&rooms, &plain);
        assert_eq!(names(&sections, &category), ["alpha", "bob", "fav", "zeta"]);
        assert_eq!(names(&sections, &Section::Rooms), ["later", "plain"]);
    }

    #[test]
    fn unread_first_and_category_changes() {
        let mut rooms = vec![
            room("!a:x", "quiet", RoomKind::Room),
            room("!b:x", "busy", RoomKind::Room),
        ];
        rooms[1].unread_messages = 3;
        let grouping = Grouping {
            unread_first: true,
            ..Grouping::default()
        };
        let mut layout = Layout::default();
        assert_eq!(
            names(&layout.group(&rooms, &grouping), &Section::Rooms),
            ["busy", "quiet"]
        );

        let a = layout.add_category("A");
        let b = layout.add_category("B");
        let room = owned_room_id!("!a:x");
        layout.place(&room, Some(&a));
        layout.place(&room, Some(&b));
        assert_eq!(
            layout.category_of(&room).map(|c| c.id.clone()),
            Some(b.clone())
        );
        layout.toggle_collapsed(&Section::Category(b.clone()));
        layout.move_section(&Section::Category(b.clone()), &Section::People);
        layout.remove_category(&b);
        assert!(layout.category_of(&room).is_none());
        assert!(layout.collapsed.is_empty());
        assert!(!layout.order.contains(&Section::Category(b)));
    }

    #[test]
    fn sections_are_reordered() {
        let mut layout = Layout::default();
        let a = Section::Category(layout.add_category("A"));
        assert_eq!(
            layout.order(),
            [
                Section::People,
                Section::Favourites,
                a.clone(),
                Section::Rooms,
                Section::LowPriority,
            ]
        );

        // Rooms to A's place, with Favourites not shown: Favourites stays.
        layout.move_section(&Section::Rooms, &a);
        // New categories go last; Invites and unknown sections are ignored.
        let b = Section::Category(layout.add_category("B"));
        layout.move_section(&Section::Invites, &b);
        layout.order.push(Section::Category("gone".into()));
        let order = [
            Section::People,
            Section::Favourites,
            Section::Rooms,
            a.clone(),
            Section::LowPriority,
            b.clone(),
        ];
        assert_eq!(layout.order(), order);
        // Down past several: those between move up.
        layout.move_section(&Section::People, &a);
        assert_eq!(
            layout.order(),
            [&order[1..4], &order[..1], &order[4..]].concat()
        );
        layout.move_section(&Section::People, &Section::Favourites);
        assert_eq!(layout.order(), order);

        let no_favourites = Grouping {
            favourites: false,
            ..Grouping::default()
        };
        let sections: Vec<_> = (layout.group([], &no_favourites).into_iter())
            .map(|(s, _)| s)
            .collect();
        assert_eq!(sections[0], Section::Invites);
        assert_eq!(sections[1..], [&order[..1], &order[2..]].concat());

        layout.set_collapsed(&a, true);
        layout.set_collapsed(&a, true);
        assert_eq!(layout.collapsed, std::slice::from_ref(&a));
        layout.set_collapsed(&a, false);
        assert!(layout.collapsed.is_empty());
    }

    #[test]
    fn stored_format() {
        let mut categories = RoomCategories::default();
        let id = categories
            .layout_mut(Some(&owned_room_id!("!space:x")))
            .add_category("Projects");
        categories.at_home.favourites = true;
        categories.home.collapsed.push(Section::People);
        categories
            .home
            .collapsed
            .push(Section::Category(id.clone()));
        let json = serde_json::to_value(RoomCategoriesEventContent {
            categories: categories.clone(),
        })
        .unwrap();
        assert_eq!(
            json["home"]["collapsed"],
            serde_json::json!(["people", { "category": id }])
        );
        assert_eq!(
            json["at_home"],
            serde_json::json!({ "all_rooms": false, "favourites": true })
        );
        let back: RoomCategoriesEventContent = serde_json::from_value(json).unwrap();
        assert_eq!(back.categories, categories);
        // Older or partial content still reads.
        let empty: RoomCategoriesEventContent = serde_json::from_str("{}").unwrap();
        assert_eq!(empty.categories, RoomCategories::default());
    }
}
