//! The room list by space: what Home and each space list.

use std::collections::{HashMap, HashSet};

use matrix_sdk::ruma::{OwnedRoomId, OwnedUserId, RoomId};

use crate::{
    Grouping, Layout, Membership, RoomCategories, RoomKind, RoomSummary, RoomTag, SpaceGraph,
};

/// Whether a room list may show `room`: rooms and direct chats, and spaces
/// only when invited to, to be answered.
fn listed(room: &RoomSummary) -> bool {
    room.kind != RoomKind::Space || room.membership == Membership::Invited
}

/// What Home lists, in the order of `rooms`: direct chats, invites (to
/// spaces too), rooms in no space and rooms the user put into Home's
/// categories; and, as [`RoomCategories::at_home`] says, every room or the
/// favourites in spaces (those only when the room list has its Favourites
/// section, by `grouping`). A room is in a space when a joined space lists it
/// as a child ([`SpaceGraph::parents`]).
pub(crate) fn home_rooms<'a>(
    rooms: &'a [RoomSummary],
    graph: &SpaceGraph,
    categories: &RoomCategories,
    grouping: &Grouping,
) -> Vec<&'a RoomSummary> {
    let at_home = categories.at_home;
    rooms
        .iter()
        .filter(|r| listed(r))
        .filter(|r| {
            at_home.all_rooms
                || r.membership == Membership::Invited
                || r.kind == RoomKind::Direct
                || graph.parents(&r.id).is_empty()
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
    graph: &SpaceGraph,
    space: &RoomId,
    members: Option<&HashSet<OwnedUserId>>,
    layout: &Layout,
) -> Vec<&'a RoomSummary> {
    let inside = graph.descendants(space);
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
    pub(crate) fn due(
        &mut self,
        rooms: &[RoomSummary],
        graph: &SpaceGraph,
    ) -> Vec<(OwnedRoomId, u64)> {
        let top = graph.top_level(rooms);
        let spaces: Vec<&RoomSummary> = rooms.iter().filter(|r| top.contains(&r.id)).collect();
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
    use std::sync::Arc;

    use matrix_sdk::ruma::{owned_room_id, owned_user_id};

    use super::*;
    use crate::{RoomListState, RoomTag, space_graph::tests::graph};

    pub(crate) fn room(id: &str, kind: RoomKind) -> RoomSummary {
        RoomSummary {
            id: OwnedRoomId::try_from(id).unwrap(),
            name: id.into(),
            avatar: None,
            topic: None,
            kind,
            membership: Membership::Joined,
            encrypted: false,
            join_rule: crate::JoinRule::Invite,
            unread_messages: 0,
            unread_mentions: 0,
            unread_notifications: 0,
            last_activity: None,
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
            ..room(id, RoomKind::Direct)
        }
    }

    pub(crate) fn invited(summary: RoomSummary) -> RoomSummary {
        RoomSummary {
            membership: Membership::Invited,
            ..summary
        }
    }

    /// `rooms`, synced, with the joined spaces listing what `spaces` says.
    pub(crate) fn state(rooms: Vec<RoomSummary>, spaces: &[(&str, &[&str])]) -> RoomListState {
        RoomListState {
            rooms,
            loaded: true,
            graph: Arc::new(graph(spaces)),
        }
    }

    /// What the spaces of [`list`] list: Lab, Offer, Sub, Club and Both in
    /// Team, Deep in Sub, and Both in Other too.
    pub(crate) const LINKS: &[(&str, &[&str])] = &[
        (
            "!team:x",
            &["!lab:x", "!offer:x", "!sub:x", "!club:x", "!both:x"],
        ),
        ("!sub:x", &["!deep:x"]),
        ("!other:x", &["!both:x"]),
    ];

    /// In list order: two top-level spaces, one nested, rooms in and out of
    /// them, direct chats, invites ([`LINKS`]).
    pub(crate) fn list() -> RoomListState {
        let rooms = vec![
            direct("!bob:x", "@bob:x"),
            room("!lab:x", RoomKind::Room),
            room("!other:x", RoomKind::Space),
            invited(room("!offer:x", RoomKind::Room)),
            room("!loose:x", RoomKind::Room),
            room("!deep:x", RoomKind::Room),
            room("!sub:x", RoomKind::Space),
            direct("!carol:x", "@carol:x"),
            invited(room("!guild:x", RoomKind::Space)),
            invited(room("!club:x", RoomKind::Space)),
            room("!team:x", RoomKind::Space),
            room("!both:x", RoomKind::Room),
        ];
        state(rooms, LINKS)
    }

    #[test]
    fn home_lists_chats_invites_and_rooms_in_no_space() {
        let mut rooms = list().rooms;
        rooms.push(RoomSummary {
            tag: RoomTag::Favourite,
            ..room("!star:x", RoomKind::Room)
        });
        let mut links = LINKS.to_vec();
        let other: &[&str] = &["!both:x", "!star:x"];
        links[2] = ("!other:x", other);
        let graph = graph(&links);
        let mut categories = RoomCategories::default();
        let home = |categories: &RoomCategories, grouping: &Grouping| -> Vec<String> {
            home_rooms(&rooms, &graph, categories, grouping)
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
            ..room("!a:x", RoomKind::Space)
        };
        let bob = HashSet::from([owned_user_id!("@bob:x")]);
        let graph = SpaceGraph::default();

        let rooms = [space(2)];
        assert_eq!(members.due(&rooms, &graph), [(owned_room_id!("!a:x"), 2)]);
        assert!(members.due(&rooms, &graph).is_empty(), "asked once");

        // A load for an older number is stale.
        let rooms = [space(3)];
        assert_eq!(members.due(&rooms, &graph), [(owned_room_id!("!a:x"), 3)]);
        members.loaded(owned_room_id!("!a:x"), 2, bob.clone());
        assert!(members.known().get(&owned_room_id!("!a:x")).is_none());
        members.loaded(owned_room_id!("!a:x"), 3, bob.clone());
        assert_eq!(members.known().get(&owned_room_id!("!a:x")), Some(&bob));

        // Gone spaces are forgotten.
        assert!(members.due(&[], &graph).is_empty());
        assert!(members.known.is_empty() && members.asked.is_empty());
    }
}
