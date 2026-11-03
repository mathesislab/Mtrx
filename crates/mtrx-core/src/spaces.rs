//! The room list by space: what Home and each space list.

use std::collections::{HashMap, HashSet};

use matrix_sdk::ruma::{OwnedRoomId, OwnedUserId, RoomId};

use crate::{Grouping, Layout, RoomCategories, RoomKind, RoomSummary, RoomTag};

/// Joined spaces in no joined space, in the order of `rooms` (the list's,
/// most recently active first). Nested spaces show through these.
pub fn top_level_spaces(rooms: &[RoomSummary]) -> Vec<&RoomSummary> {
    let ids: HashSet<&OwnedRoomId> = rooms.iter().map(|r| &r.id).collect();
    rooms
        .iter()
        .filter(|r| r.kind == RoomKind::Space && !r.invited)
        .filter(|r| !r.parents.iter().any(|p| ids.contains(p)))
        .collect()
}

/// Rooms below `space`, through nested spaces too.
pub fn descendants(rooms: &[RoomSummary], space: &RoomId) -> HashSet<OwnedRoomId> {
    let mut children: HashMap<&RoomId, Vec<&OwnedRoomId>> = HashMap::new();
    for room in rooms {
        for parent in &room.parents {
            children.entry(parent).or_default().push(&room.id);
        }
    }
    let mut found = HashSet::new();
    let mut stack = vec![space];
    while let Some(id) = stack.pop() {
        for child in children.get(id).into_iter().flatten() {
            // Space graphs may contain cycles.
            if found.insert((*child).clone()) {
                stack.push(child);
            }
        }
    }
    found
}

/// Whether a room list may show `room`: rooms and direct chats, and spaces
/// only when invited to, to be answered.
fn listed(room: &RoomSummary) -> bool {
    room.kind != RoomKind::Space || room.invited
}

/// What Home lists, in the order of `rooms`: direct chats, invites (to
/// spaces too), rooms in no space and rooms the user put into Home's
/// categories; and, as [`RoomCategories::at_home`] says, every room or the
/// favourites in spaces (those only when the room list has its Favourites
/// section, by `grouping`). A room is in a space when a joined space lists it
/// as a child ([`RoomSummary::parents`]).
pub(crate) fn home_rooms<'a>(
    rooms: &'a [RoomSummary],
    categories: &RoomCategories,
    grouping: &Grouping,
) -> Vec<&'a RoomSummary> {
    let at_home = categories.at_home;
    rooms
        .iter()
        .filter(|r| listed(r))
        .filter(|r| {
            at_home.all_rooms
                || r.invited
                || r.kind == RoomKind::Direct
                || r.parents.is_empty()
                || categories.home.category_of(&r.id).is_some()
                || (at_home.favourites && grouping.favourites && r.tag == RoomTag::Favourite)
        })
        .collect()
}

/// What a space lists, in the order of `rooms`: everything below it
/// (through nested spaces), direct chats with its joined `members`, and rooms
/// the user put into its categories (`layout`). Spaces are not listed, except
/// invites to them, to be answered.
pub(crate) fn space_rooms<'a>(
    rooms: &'a [RoomSummary],
    space: &RoomId,
    members: Option<&HashSet<OwnedUserId>>,
    layout: &Layout,
) -> Vec<&'a RoomSummary> {
    let inside = descendants(rooms, space);
    rooms
        .iter()
        .filter(|r| listed(r))
        .filter(|r| {
            inside.contains(&r.id)
                || r.direct_user
                    .as_ref()
                    .is_some_and(|u| members.is_some_and(|m| m.contains(&u.id)))
                || layout.category_of(&r.id).is_some()
        })
        .collect()
}

/// The joined members of the top-level spaces, for the direct chats they
/// list. Loaded when a space appears and again when its number of joined
/// members changes; a failed load is tried again then.
#[derive(Clone, Debug, Default)]
pub(crate) struct SpaceMembers {
    known: HashMap<OwnedRoomId, HashSet<OwnedUserId>>,
    /// The number of joined members each space had when its members were
    /// last asked for.
    asked: HashMap<OwnedRoomId, u64>,
}

impl SpaceMembers {
    /// Spaces whose members to load now ([`RoomHandle::joined_member_ids`]),
    /// with their number of members, to pass back to [`Self::loaded`].
    ///
    /// [`RoomHandle::joined_member_ids`]: crate::RoomHandle::joined_member_ids
    pub(crate) fn due(&mut self, rooms: &[RoomSummary]) -> Vec<(OwnedRoomId, u64)> {
        let spaces = top_level_spaces(rooms);
        self.asked
            .retain(|id, _| spaces.iter().any(|space| &space.id == id));
        self.known
            .retain(|id, _| spaces.iter().any(|space| &space.id == id));
        spaces
            .into_iter()
            .filter(|space| {
                let count = space.joined_members;
                self.asked.insert(space.id.clone(), count) != Some(count)
            })
            .map(|space| (space.id.clone(), space.joined_members))
            .collect()
    }

    /// Members loaded for a space that had `count` joined members; dropped
    /// if a newer load was started since.
    pub(crate) fn loaded(&mut self, space: OwnedRoomId, count: u64, ids: HashSet<OwnedUserId>) {
        if self.asked.get(&space) == Some(&count) {
            self.known.insert(space, ids);
        }
    }

    /// The members of each space loaded.
    pub(crate) fn known(&self) -> &HashMap<OwnedRoomId, HashSet<OwnedUserId>> {
        &self.known
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use matrix_sdk::ruma::{owned_room_id, owned_user_id};

    use super::*;
    use crate::RoomTag;

    pub(crate) fn room(id: &str, kind: RoomKind, parents: &[&str]) -> RoomSummary {
        RoomSummary {
            id: OwnedRoomId::try_from(id).unwrap(),
            name: id.into(),
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
            parents: parents
                .iter()
                .map(|p| OwnedRoomId::try_from(*p).unwrap())
                .collect(),
            tag: RoomTag::None,
            direct_user: None,
            alias: None,
            joined_members: 0,
            pinned: Vec::new(),
        }
    }

    pub(crate) fn direct(id: &str, user: &str) -> RoomSummary {
        RoomSummary {
            direct_user: Some(crate::User::from(OwnedUserId::try_from(user).unwrap())),
            ..room(id, RoomKind::Direct, &[])
        }
    }

    /// In list order: two top-level spaces, one nested, rooms in and out of
    /// them, direct chats, invites.
    pub(crate) fn list() -> Vec<RoomSummary> {
        let mut invited_room = room("!offer:x", RoomKind::Room, &["!team:x"]);
        invited_room.invited = true;
        let mut invited_space = room("!guild:x", RoomKind::Space, &[]);
        invited_space.invited = true;
        let mut invited_nested = room("!club:x", RoomKind::Space, &["!team:x"]);
        invited_nested.invited = true;
        vec![
            direct("!bob:x", "@bob:x"),
            room("!lab:x", RoomKind::Room, &["!team:x"]),
            room("!other:x", RoomKind::Space, &[]),
            invited_room,
            room("!loose:x", RoomKind::Room, &[]),
            room("!deep:x", RoomKind::Room, &["!sub:x"]),
            room("!sub:x", RoomKind::Space, &["!team:x"]),
            direct("!carol:x", "@carol:x"),
            invited_space,
            invited_nested,
            room("!team:x", RoomKind::Space, &[]),
            room("!both:x", RoomKind::Room, &["!team:x", "!other:x"]),
        ]
    }

    #[test]
    fn home_lists_chats_invites_and_rooms_in_no_space() {
        let mut rooms = list();
        rooms.push(RoomSummary {
            tag: RoomTag::Favourite,
            ..room("!star:x", RoomKind::Room, &["!other:x"])
        });
        let mut categories = RoomCategories::default();
        let home = |categories: &RoomCategories, grouping: &Grouping| -> Vec<String> {
            home_rooms(&rooms, categories, grouping)
                .iter()
                .map(|r| r.id.to_string())
                .collect()
        };
        let grouping = Grouping::default();
        // No joined spaces; the nested invite is an invite.
        assert_eq!(
            home(&categories, &grouping),
            [
                "!bob:x", "!offer:x", "!loose:x", "!carol:x", "!guild:x", "!club:x"
            ]
        );
        // Rooms of spaces put into Home's categories.
        let mine = categories.home.add_category("Mine");
        categories
            .home
            .place(&owned_room_id!("!lab:x"), Some(&mine));
        assert_eq!(home(&categories, &grouping)[1], "!lab:x");
        categories.home.remove_category(&mine);

        // Favourites of spaces, in the Favourites section.
        categories.at_home.favourites = true;
        assert_eq!(home(&categories, &grouping).last().unwrap(), "!star:x");
        let no_favourites = Grouping {
            favourites: false,
            ..grouping
        };
        assert!(!home(&categories, &no_favourites).contains(&"!star:x".to_owned()));

        // Everything.
        categories.at_home.all_rooms = true;
        assert_eq!(
            home(&categories, &grouping),
            [
                "!bob:x", "!lab:x", "!offer:x", "!loose:x", "!deep:x", "!carol:x", "!guild:x",
                "!club:x", "!both:x", "!star:x"
            ]
        );
    }

    #[test]
    fn members_load_again_when_their_number_changes() {
        let mut members = SpaceMembers::default();
        let space = |joined_members| RoomSummary {
            joined_members,
            ..room("!a:x", RoomKind::Space, &[])
        };
        let bob = HashSet::from([owned_user_id!("@bob:x")]);

        let rooms = [space(2)];
        assert_eq!(members.due(&rooms), [(owned_room_id!("!a:x"), 2)]);
        assert!(members.due(&rooms).is_empty(), "asked once");

        // A load for an older number is stale.
        let rooms = [space(3)];
        assert_eq!(members.due(&rooms), [(owned_room_id!("!a:x"), 3)]);
        members.loaded(owned_room_id!("!a:x"), 2, bob.clone());
        assert!(members.known().get(&owned_room_id!("!a:x")).is_none());
        members.loaded(owned_room_id!("!a:x"), 3, bob.clone());
        assert_eq!(members.known().get(&owned_room_id!("!a:x")), Some(&bob));

        // Gone spaces are forgotten.
        assert!(members.due(&[]).is_empty());
        assert!(members.known.is_empty() && members.asked.is_empty());
    }

    #[test]
    fn cycles_end() {
        let rooms = vec![
            room("!a:x", RoomKind::Space, &["!b:x"]),
            room("!b:x", RoomKind::Space, &["!a:x"]),
            room("!r:x", RoomKind::Room, &["!a:x"]),
        ];
        assert!(top_level_spaces(&rooms).is_empty());
        let below = descendants(&rooms, &owned_room_id!("!a:x"));
        assert_eq!(below.len(), 3);
    }
}
