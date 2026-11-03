//! The joined spaces and what each lists (`m.space.child`), read once after
//! each sync for the room lists, the room settings and the calls.

use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

use matrix_sdk::{
    Client, Room,
    deserialized_responses::SyncOrStrippedState,
    ruma::{
        OwnedRoomId, OwnedServerName, OwnedSpaceChildOrder, RoomId,
        events::{SyncStateEvent, space::child::SpaceChildEventContent},
    },
};
use tokio::sync::broadcast::error::RecvError;
use tracing::warn;

use crate::{Live, Result, RoomKind, RoomSummary, Session, synced};

/// Joined spaces and what they list, with the servers to join through.
/// `m.space.parent` is not read.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SpaceGraph {
    /// Each joined space's children, in event order.
    children: HashMap<OwnedRoomId, Vec<Child>>,
    /// The joined spaces listing each room.
    parents: HashMap<OwnedRoomId, Vec<OwnedRoomId>>,
}

/// A room or space a space lists.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Child {
    pub room: OwnedRoomId,
    /// The space's admins suggest it to its members.
    pub suggested: bool,
    /// Servers to join it through.
    pub via: Vec<OwnedServerName>,
    /// Its place among the space's children, if set.
    pub order: Option<OwnedSpaceChildOrder>,
}

impl SpaceGraph {
    /// What `spaces` list.
    pub(crate) async fn read<'a>(spaces: impl IntoIterator<Item = &'a Room>) -> Self {
        let mut listed = Vec::new();
        for space in spaces {
            listed.push((space.room_id().to_owned(), children_of(space).await));
        }
        listed.into_iter().collect()
    }

    /// The joined spaces listing `room` directly.
    pub fn parents(&self, room: &RoomId) -> &[OwnedRoomId] {
        self.parents.get(room).map_or(&[], Vec::as_slice)
    }

    /// The joined spaces `room` is in, through nested spaces too, nearest
    /// first.
    pub fn ancestors(&self, room: &RoomId) -> Vec<OwnedRoomId> {
        let mut found: Vec<OwnedRoomId> = Vec::new();
        let mut next: Vec<&OwnedRoomId> = self.parents(room).iter().collect();
        while !next.is_empty() {
            let mut further = Vec::new();
            for space in next {
                // Space graphs may contain cycles.
                if space.as_str() != room.as_str() && !found.contains(space) {
                    further.extend(self.parents(space));
                    found.push(space.clone());
                }
            }
            next = further;
        }
        found
    }

    /// What `space` lists directly, if we joined it.
    pub fn children(&self, space: &RoomId) -> &[Child] {
        self.children.get(space).map_or(&[], Vec::as_slice)
    }

    /// `space`'s link to `room`, if it lists it.
    pub fn child(&self, space: &RoomId, room: &RoomId) -> Option<&Child> {
        self.children(space).iter().find(|c| c.room == room)
    }

    /// Rooms below `space`, through nested spaces too.
    pub fn descendants(&self, space: &RoomId) -> HashSet<OwnedRoomId> {
        let mut found = HashSet::new();
        let mut stack = vec![space];
        while let Some(id) = stack.pop() {
            for child in self.children(id) {
                // Space graphs may contain cycles.
                if found.insert(child.room.clone()) {
                    stack.push(&child.room);
                }
            }
        }
        found
    }

    /// Joined spaces of `rooms` in no joined space of `rooms`, in their
    /// order (the list's, most recently active first). Nested spaces show
    /// through these.
    pub fn top_level(&self, rooms: &[RoomSummary]) -> Vec<OwnedRoomId> {
        let ids: HashSet<&OwnedRoomId> = rooms.iter().map(|r| &r.id).collect();
        rooms
            .iter()
            .filter(|r| r.kind == RoomKind::Space && !r.invited)
            .filter(|r| !self.parents(&r.id).iter().any(|p| ids.contains(p)))
            .map(|r| r.id.clone())
            .collect()
    }

    /// Everything below `space`, depth first in each space's order, with
    /// its depth (0 for what `space` lists). A space already above itself
    /// is listed but not walked again.
    pub fn tree(&self, space: &RoomId) -> Vec<(usize, OwnedRoomId)> {
        let mut tree = Vec::new();
        self.walk(&mut vec![space.to_owned()], &mut tree);
        tree
    }

    /// The children of the last of `path`, and theirs.
    fn walk(&self, path: &mut Vec<OwnedRoomId>, tree: &mut Vec<(usize, OwnedRoomId)>) {
        let Some(space) = path.last().cloned() else {
            return;
        };
        for child in self.children(&space) {
            tree.push((path.len() - 1, child.room.clone()));
            if !path.contains(&child.room) {
                path.push(child.room.clone());
                self.walk(path, tree);
                path.pop();
            }
        }
    }
}

/// From each space and what it lists.
impl FromIterator<(OwnedRoomId, Vec<Child>)> for SpaceGraph {
    fn from_iter<I: IntoIterator<Item = (OwnedRoomId, Vec<Child>)>>(spaces: I) -> Self {
        let mut graph = Self::default();
        for (space, children) in spaces {
            for child in &children {
                (graph.parents.entry(child.room.clone()).or_default()).push(space.clone());
            }
            graph.children.insert(space, children);
        }
        for parents in graph.parents.values_mut() {
            parents.sort();
        }
        graph
    }
}

/// What `space` lists by its `m.space.child` events. Events without `via`
/// mean the child was removed.
async fn children_of(space: &Room) -> Vec<Child> {
    let events = match space
        .get_state_events_static::<SpaceChildEventContent>()
        .await
    {
        Ok(events) => events,
        Err(e) => {
            warn!(space = %space.room_id(), "could not read m.space.child events: {e}");
            return Vec::new();
        }
    };
    events
        .into_iter()
        .filter_map(|raw| match raw.deserialize() {
            Ok(SyncOrStrippedState::Sync(SyncStateEvent::Original(e)))
                if !e.content.via.is_empty() =>
            {
                Some(Child {
                    room: e.state_key,
                    suggested: e.content.suggested,
                    via: e.content.via,
                    order: e.content.order,
                })
            }
            Ok(SyncOrStrippedState::Stripped(e)) => Some(Child {
                via: e.content.via.filter(|v| !v.is_empty())?,
                suggested: e.content.suggested,
                order: e.content.order,
                room: e.state_key,
            }),
            _ => None,
        })
        .collect()
}

/// The space graph of the joined spaces in the store now.
async fn read_joined(client: &Client) -> SpaceGraph {
    let spaces: Vec<Room> = (client.joined_rooms().into_iter())
        .filter(|r| r.is_space())
        .collect();
    SpaceGraph::read(&spaces).await
}

/// The space graph from the store, then read again after each sync that
/// brought room updates.
pub(crate) async fn follow(client: Client) -> Live<Arc<SpaceGraph>> {
    let mut updates = client.subscribe_to_all_room_updates();
    let initial = read_joined(&client).await;
    Live::spawn(Arc::new(initial), |graphs| async move {
        loop {
            // An update missed while lagging still left its state.
            if let Err(RecvError::Closed) = updates.recv().await {
                return;
            }
            let graph = read_joined(&client).await;
            graphs.send_if_modified(move |sent| {
                let changed = **sent != graph;
                if changed {
                    *sent = Arc::new(graph);
                }
                changed
            });
        }
    })
}

impl Session {
    /// The joined spaces and what they list, as last synced.
    pub(crate) fn space_graph(&self) -> Arc<SpaceGraph> {
        self.spaces.get()
    }

    /// Send `space`'s link to `room` (none: `via` empty), returning once the
    /// space graph has it (or after [`synced::CONFIRM`]): what is read next
    /// shows it.
    pub(crate) async fn set_child(
        &self,
        space: &Room,
        room: &RoomId,
        content: SpaceChildEventContent,
    ) -> Result<()> {
        // As the graph will have it: listed, suggested or not; or not.
        let listed = (!content.via.is_empty()).then_some(content.suggested);
        space.send_state_event_for_key(room, content).await?;
        let space = space.room_id();
        let shown = self
            .spaces
            .wait_for(|graph| graph.child(space, room).map(|c| c.suggested) == listed);
        let _ = tokio::time::timeout(synced::CONFIRM, shown).await;
        Ok(())
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    fn id(id: &str) -> OwnedRoomId {
        id.try_into().unwrap()
    }

    fn ids(ids: &[OwnedRoomId]) -> Vec<&str> {
        ids.iter().map(|id| id.as_str()).collect()
    }

    /// Each space with the rooms it lists, in order.
    pub(crate) fn graph(spaces: &[(&str, &[&str])]) -> SpaceGraph {
        (spaces.iter())
            .map(|(space, children)| {
                let children = (children.iter())
                    .map(|room| Child {
                        room: id(room),
                        suggested: false,
                        via: Vec::new(),
                        order: None,
                    })
                    .collect();
                (id(space), children)
            })
            .collect()
    }

    /// Team lists Lab and Sub, which lists Deep; Other lists Lab too.
    fn team() -> SpaceGraph {
        graph(&[
            ("!team:x", &["!lab:x", "!sub:x"]),
            ("!sub:x", &["!deep:x"]),
            ("!other:x", &["!lab:x"]),
        ])
    }

    #[test]
    fn parents_children_and_links() {
        let graph = team();
        assert_eq!(ids(graph.parents(&id("!lab:x"))), ["!other:x", "!team:x"]);
        assert!(graph.parents(&id("!team:x")).is_empty());
        let children: Vec<_> = (graph.children(&id("!team:x")).iter())
            .map(|c| c.room.as_str())
            .collect();
        assert_eq!(children, ["!lab:x", "!sub:x"]);
        assert!(graph.child(&id("!sub:x"), &id("!deep:x")).is_some());
        assert!(graph.child(&id("!team:x"), &id("!deep:x")).is_none());
        assert!(graph.children(&id("!lab:x")).is_empty());
    }

    #[test]
    fn ancestors_nearest_first() {
        let graph = team();
        assert_eq!(ids(&graph.ancestors(&id("!deep:x"))), ["!sub:x", "!team:x"]);
        assert_eq!(
            ids(&graph.ancestors(&id("!lab:x"))),
            ["!other:x", "!team:x"]
        );
        assert!(graph.ancestors(&id("!team:x")).is_empty());
    }

    #[test]
    fn walks_below_a_space() {
        let graph = team();
        let below = graph.descendants(&id("!team:x"));
        let mut below: Vec<_> = below.iter().map(|r| r.as_str()).collect();
        below.sort();
        assert_eq!(below, ["!deep:x", "!lab:x", "!sub:x"]);
        let tree: Vec<_> = (graph.tree(&id("!team:x")).into_iter())
            .map(|(depth, room)| (depth, room.to_string()))
            .collect();
        assert_eq!(
            tree,
            [
                (0, "!lab:x".to_owned()),
                (0, "!sub:x".to_owned()),
                (1, "!deep:x".to_owned())
            ]
        );
    }

    #[test]
    fn cycles_end() {
        let graph = graph(&[("!a:x", &["!b:x", "!r:x"]), ("!b:x", &["!a:x"])]);
        assert_eq!(graph.descendants(&id("!a:x")).len(), 3);
        assert_eq!(ids(&graph.ancestors(&id("!a:x"))), ["!b:x"]);
        assert_eq!(ids(&graph.ancestors(&id("!r:x"))), ["!a:x", "!b:x"]);
        let tree: Vec<_> = (graph.tree(&id("!a:x")).into_iter())
            .map(|(depth, room)| (depth, room.to_string()))
            .collect();
        assert_eq!(
            tree,
            [
                (0, "!b:x".to_owned()),
                (1, "!a:x".to_owned()),
                (0, "!r:x".to_owned())
            ]
        );
    }

    #[test]
    fn top_level_spaces_are_in_no_joined_space() {
        let room = crate::spaces::tests::room;
        let invited = RoomSummary {
            invited: true,
            ..room("!guild:x", RoomKind::Space)
        };
        let rooms = [
            room("!sub:x", RoomKind::Space),
            room("!other:x", RoomKind::Space),
            invited,
            room("!lab:x", RoomKind::Room),
            room("!team:x", RoomKind::Space),
        ];
        assert_eq!(ids(&team().top_level(&rooms)), ["!other:x", "!team:x"]);
        // Unless that space isn't in the list.
        assert_eq!(ids(&team().top_level(&rooms[..4])), ["!sub:x", "!other:x"]);
        let cycle = graph(&[("!a:x", &["!b:x"]), ("!b:x", &["!a:x"])]);
        let spaces = [room("!a:x", RoomKind::Space), room("!b:x", RoomKind::Space)];
        assert!(cycle.top_level(&spaces).is_empty());
    }
}
