//! Rooms and spaces to join: those a space lists (its hierarchy), a
//! server's public directory, and the one an address or link names.
//!
//! Servers list only rooms the user may see: public ones, ones members of a
//! space may join, ones that take requests to join, and those the user is
//! in or invited to. Invite-only rooms of others are never listed.

use std::{
    collections::HashMap,
    sync::Mutex,
    time::{Duration, Instant},
};

use matrix_sdk::{
    Room, RoomState,
    ruma::{
        MatrixToUri, MatrixUri, OwnedMxcUri, OwnedRoomAliasId, OwnedRoomId, OwnedRoomOrAliasId,
        OwnedServerName, RoomId, ServerName, UInt,
        api::client::{
            directory::get_public_rooms_filtered::v3::Request as PublicRoomsRequest,
            space::get_hierarchy::v1::Request as HierarchyRequest,
        },
        directory::{Filter, PublicRoomsChunk, RoomTypeFilter},
        events::{macros::EventContent, space::child::HierarchySpaceChildEvent},
        matrix_uri::MatrixId,
        room::RoomType,
        uint,
    },
};
use serde::{Deserialize, Serialize};

use crate::{
    Child, JoinAction, JoinRule, Live, Membership, Result, Role, Session, SpaceGraph, Topic,
    account_data::AccountData,
};

/// A room or space to join, as a space or a directory lists it.
#[derive(Clone, Debug, PartialEq)]
pub struct DirectoryRoom {
    pub id: OwnedRoomId,
    pub alias: Option<OwnedRoomAliasId>,
    /// The room's name, else its address, else its ID.
    pub name: String,
    pub topic: Topic,
    pub avatar: Option<OwnedMxcUri>,
    pub members: u64,
    pub is_space: bool,
    /// For a space, the rooms and spaces it lists.
    pub children: u64,
    pub join_rule: JoinRule,
    /// As the server or the store had it when listed; what to show is
    /// [`crate::RoomLists::membership`].
    pub membership: Option<Membership>,
    /// Ours, where we are a member.
    pub role: Option<Role>,
    /// Servers to join it through.
    pub via: Vec<OwnedServerName>,
    /// The space's admins suggest it.
    pub suggested: bool,
}

/// Directory results to show: rooms, spaces or both.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DirectoryKind {
    #[default]
    All,
    Spaces,
    Rooms,
}

/// One page of a server's public directory.
#[derive(Clone, Debug, Default)]
pub struct DirectoryPage {
    pub rooms: Vec<DirectoryRoom>,
    /// For [`Session::public_rooms`]'s `page`, if there are more.
    pub next: Option<String>,
}

/// Directory servers the user browsed, most recent first. Personal, so
/// stored as account data ([`crate::account_data`]) and synced.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DirectoryServers {
    #[serde(default)]
    pub servers: Vec<String>,
}

/// How many browsed servers are remembered.
const REMEMBERED: usize = 8;

/// How long a space's rooms as its server listed them are used again,
/// rather than asking it.
const LISTED_FOR: Duration = Duration::from_secs(60);

/// Each space's rooms as its server last listed them
/// ([`Session::space_children`]), kept for the session.
#[derive(Default)]
pub(crate) struct Listings(Mutex<HashMap<OwnedRoomId, Listing>>);

struct Listing {
    /// As the server listed them, and when.
    listed: Vec<DirectoryRoom>,
    at: Instant,
    /// As last given: with the joined rooms the server left out.
    shown: Vec<DirectoryRoom>,
}

impl Listings {
    /// What `space`'s server listed lately, and when.
    fn fresh(&self, space: &RoomId) -> Option<(Vec<DirectoryRoom>, Instant)> {
        let listings = self.0.lock().unwrap_or_else(|e| e.into_inner());
        let listing = listings
            .get(space)
            .filter(|l| l.at.elapsed() < LISTED_FOR)?;
        Some((listing.listed.clone(), listing.at))
    }

    fn keep(&self, space: &RoomId, listing: Listing) {
        let mut listings = self.0.lock().unwrap_or_else(|e| e.into_inner());
        listings.insert(space.to_owned(), listing);
    }

    fn shown(&self, space: &RoomId) -> Option<Vec<DirectoryRoom>> {
        let listings = self.0.lock().unwrap_or_else(|e| e.into_inner());
        Some(listings.get(space)?.shown.clone())
    }

    /// `room`'s name as a space's listing gave it, for rooms we never were
    /// in.
    pub(crate) fn name(&self, room: &RoomId) -> Option<String> {
        let listings = self.0.lock().unwrap_or_else(|e| e.into_inner());
        (listings.values())
            .flat_map(|listing| &listing.shown)
            .find(|listed| listed.id == room)
            .map(|listed| listed.name.clone())
    }
}

impl DirectoryServers {
    /// Put `server` first.
    pub fn remember(&mut self, server: &str) {
        self.servers.retain(|s| s != server);
        self.servers.insert(0, server.to_owned());
        self.servers.truncate(REMEMBERED);
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, EventContent)]
#[ruma_event(type = "org.mtrx.directory_servers", kind = GlobalAccountData)]
pub(crate) struct DirectoryServersEventContent {
    #[serde(flatten)]
    servers: DirectoryServers,
}

impl AccountData for DirectoryServers {
    type Content = DirectoryServersEventContent;

    fn from_content(content: Self::Content) -> Self {
        content.servers
    }

    fn into_content(self) -> Self::Content {
        DirectoryServersEventContent { servers: self }
    }
}

/// A room address, ID or link (matrix.to or `matrix:`), with the servers it
/// names to join through. `None` if `text` is none of these.
pub fn parse_address(text: &str) -> Option<(OwnedRoomOrAliasId, Vec<OwnedServerName>)> {
    let text = text.trim();
    if let Ok(id) = OwnedRoomOrAliasId::try_from(text) {
        return Some((id, Vec::new()));
    }
    let (id, via) = match MatrixToUri::parse(text) {
        Ok(uri) => (uri.id().clone(), uri.via().to_vec()),
        Err(_) => {
            let uri = MatrixUri::parse(text).ok()?;
            (uri.id().clone(), uri.via().to_vec())
        }
    };
    let id = match id {
        MatrixId::Room(id) => id.into(),
        MatrixId::RoomAlias(alias) => alias.into(),
        // A message's link names its room.
        MatrixId::Event(room, _) => room,
        _ => return None,
    };
    Some((id, via))
}

impl DirectoryRoom {
    /// What its button does, with our `membership` of it now
    /// ([`crate::RoomLists::membership`]).
    pub fn action(&self, membership: Option<Membership>) -> JoinAction {
        JoinAction::new(self.join_rule, membership)
    }
}

/// A room a space lists that we joined or are invited to, as we know it,
/// without our membership or role.
fn known_child(room: &Room, child: &Child, graph: &SpaceGraph) -> DirectoryRoom {
    let alias = room.canonical_alias();
    let name = room.cached_display_name().map(|n| n.to_string());
    DirectoryRoom {
        id: child.room.clone(),
        name: display_name(name.as_deref(), alias.as_ref(), &child.room),
        alias,
        topic: Topic::parse(&room.topic().unwrap_or_default()),
        avatar: room.avatar_url(),
        members: room.joined_members_count(),
        is_space: room.is_space(),
        children: graph.children(&child.room).len() as u64,
        join_rule: room.join_rule().as_ref().into(),
        membership: None,
        role: None,
        via: child.via.clone(),
        suggested: child.suggested,
    }
}

fn display_name(name: Option<&str>, alias: Option<&OwnedRoomAliasId>, id: &RoomId) -> String {
    name.filter(|n| !n.trim().is_empty())
        .map(str::to_owned)
        .or_else(|| alias.map(|a| a.to_string()))
        .unwrap_or_else(|| id.to_string())
}

impl Session {
    /// Our role in a joined room, from its power levels.
    async fn own_role(&self, room: &RoomId) -> Option<Role> {
        let levels = crate::permissions::levels(&self.client.get_room(room)?).await?;
        Some(crate::members::role(levels.for_user(&self.user_id())))
    }

    fn membership(&self, room: &RoomId) -> Option<Membership> {
        Membership::of(self.client.get_room(room)?.state())
    }

    /// The rooms and spaces `space` lists directly (subspaces' own rooms
    /// are not included), suggested ones first, then by name: as its
    /// server listed them in the last minute (else it is asked again), with
    /// the joined rooms it leaves out.
    pub async fn space_children(&self, space: &RoomId) -> Result<Vec<DirectoryRoom>> {
        let (listed, at) = match self.listings.fresh(space) {
            Some(fresh) => fresh,
            None => (self.list_space_children(space).await?, Instant::now()),
        };
        let shown = self.with_known(space, listed.clone()).await;
        let listing = Listing {
            listed,
            at,
            shown: shown.clone(),
        };
        self.listings.keep(space, listing);
        Ok(shown)
    }

    /// `space`'s rooms as [`Self::space_children`] last gave them, however
    /// long ago; none before.
    pub fn listed_space_children(&self, space: &RoomId) -> Option<Vec<DirectoryRoom>> {
        self.listings.shown(space)
    }

    /// `rooms`, of `space`, with the rooms it lists that we joined or are
    /// invited to that they leave out, and our membership and role in each
    /// now; suggested first, then by name.
    async fn with_known(
        &self,
        space: &RoomId,
        mut rooms: Vec<DirectoryRoom>,
    ) -> Vec<DirectoryRoom> {
        let graph = self.space_graph();
        for child in graph.children(space) {
            if rooms.iter().any(|r| r.id == child.room) {
                continue;
            }
            let known = self.client.get_room(&child.room);
            if let Some(room) =
                known.filter(|r| matches!(r.state(), RoomState::Joined | RoomState::Invited))
            {
                rooms.push(known_child(&room, child, &graph));
            }
        }
        for room in &mut rooms {
            room.membership = self.membership(&room.id);
            room.role = self.own_role(&room.id).await;
        }
        rooms.sort_by_cached_key(|r| (!r.suggested, r.name.to_lowercase()));
        rooms
    }

    /// What `space`'s server lists, without our membership or role.
    async fn list_space_children(&self, space: &RoomId) -> Result<Vec<DirectoryRoom>> {
        let mut rooms = Vec::new();
        let mut from = None;
        loop {
            let mut request = HierarchyRequest::new(space.to_owned());
            request.max_depth = Some(uint!(1));
            request.from = from.take();
            // Busy servers drop these at times: once more before giving up.
            let response = match self.client.send(request.clone()).await {
                Ok(response) => response,
                Err(_) => self.client.send(request).await?,
            };
            // The space's own entry carries its children's `via` and
            // `suggested`.
            let children: Vec<HierarchySpaceChildEvent> = response
                .rooms
                .iter()
                .filter(|chunk| chunk.summary.room_id == space)
                .flat_map(|chunk| &chunk.children_state)
                .filter_map(|raw| raw.deserialize().ok())
                .collect();
            for chunk in response.rooms {
                let summary = chunk.summary;
                if summary.room_id == space {
                    continue;
                }
                let child = children.iter().find(|c| c.state_key == summary.room_id);
                rooms.push(DirectoryRoom {
                    role: None,
                    name: display_name(
                        summary.name.as_deref(),
                        summary.canonical_alias.as_ref(),
                        &summary.room_id,
                    ),
                    membership: None,
                    alias: summary.canonical_alias,
                    topic: Topic::parse(summary.topic.as_deref().unwrap_or_default()),
                    avatar: summary.avatar_url,
                    members: summary.num_joined_members.into(),
                    is_space: summary.room_type == Some(RoomType::Space),
                    children: chunk.children_state.len() as u64,
                    join_rule: (&summary.join_rule).into(),
                    via: child.map(|c| c.content.via.clone()).unwrap_or_default(),
                    suggested: child.is_some_and(|c| c.content.suggested),
                    id: summary.room_id,
                });
            }
            match response.next_batch {
                Some(next) => from = Some(next),
                None => break,
            }
        }
        Ok(rooms)
    }

    /// One page of `server`'s public directory (ours if `None`), matching
    /// `query`. Servers may refuse to share their directory with others.
    pub async fn public_rooms(
        &self,
        server: Option<&ServerName>,
        query: &str,
        kind: DirectoryKind,
        page: Option<String>,
    ) -> Result<DirectoryPage> {
        const PAGE: u32 = 30;
        let mut filter = Filter::new();
        let query = query.trim();
        filter.generic_search_term = (!query.is_empty()).then(|| query.to_owned());
        filter.room_types = match kind {
            DirectoryKind::All => Vec::new(),
            DirectoryKind::Spaces => vec![RoomTypeFilter::Space],
            DirectoryKind::Rooms => vec![RoomTypeFilter::Default],
        };
        let mut request = PublicRoomsRequest::new();
        request.server = server.map(ToOwned::to_owned);
        request.limit = Some(UInt::from(PAGE));
        request.since = page;
        request.filter = filter;
        let response = self.client.public_rooms_filtered(request).await?;
        let via: Vec<OwnedServerName> = server.map(ToOwned::to_owned).into_iter().collect();
        let mut rooms: Vec<DirectoryRoom> = response
            .chunk
            .into_iter()
            .map(|chunk| self.public_room(chunk, via.clone()))
            .collect();
        for room in &mut rooms {
            room.role = self.own_role(&room.id).await;
        }
        Ok(DirectoryPage {
            rooms,
            next: response.next_batch,
        })
    }

    fn public_room(&self, chunk: PublicRoomsChunk, via: Vec<OwnedServerName>) -> DirectoryRoom {
        DirectoryRoom {
            role: None,
            name: display_name(
                chunk.name.as_deref(),
                chunk.canonical_alias.as_ref(),
                &chunk.room_id,
            ),
            membership: self.membership(&chunk.room_id),
            alias: chunk.canonical_alias,
            topic: Topic::parse(chunk.topic.as_deref().unwrap_or_default()),
            avatar: chunk.avatar_url,
            members: chunk.num_joined_members.into(),
            is_space: chunk.room_type == Some(RoomType::Space),
            children: 0,
            join_rule: (&chunk.join_rule).into(),
            via,
            suggested: false,
            id: chunk.room_id,
        }
    }

    /// The room or space an address or link names, as far as its server
    /// shows it to people outside.
    pub async fn preview(
        &self,
        address: &OwnedRoomOrAliasId,
        via: Vec<OwnedServerName>,
    ) -> Result<DirectoryRoom> {
        let preview = self.client.get_room_preview(address, via.clone()).await?;
        Ok(DirectoryRoom {
            role: self.own_role(&preview.room_id).await,
            name: display_name(
                preview.name.as_deref(),
                preview.canonical_alias.as_ref(),
                &preview.room_id,
            ),
            membership: self.membership(&preview.room_id),
            alias: preview.canonical_alias,
            topic: Topic::parse(preview.topic.as_deref().unwrap_or_default()),
            avatar: preview.avatar_url,
            members: preview.num_joined_members,
            is_space: preview.room_type == Some(RoomType::Space),
            children: 0,
            join_rule: preview
                .join_rule
                .as_ref()
                .map_or(JoinRule::Invite, JoinRule::from),
            via,
            suggested: false,
            id: preview.room_id,
        })
    }

    /// Ask to join a room that takes requests. The room lists show it as
    /// asked ([`crate::RoomLists::membership`]) until they list it: by ID
    /// from the start, by alias once asked.
    pub async fn knock(
        &self,
        room: &OwnedRoomOrAliasId,
        via: Vec<OwnedServerName>,
    ) -> Result<OwnedRoomId> {
        let id = OwnedRoomId::try_from(room.as_str()).ok();
        if let Some(id) = &id {
            self.echoes.set_knocking(id);
        }
        match self.client.knock(room.clone(), None, via).await {
            Ok(knocked) => {
                let knocked = knocked.room_id().to_owned();
                self.echoes.set_knocking(&knocked);
                Ok(knocked)
            }
            Err(e) => {
                if let Some(id) = &id {
                    self.echoes.drop_knocking(id);
                }
                Err(e.into())
            }
        }
    }

    pub fn directory_servers(&self) -> DirectoryServers {
        self.directory_servers.get()
    }

    /// The current servers, then every change, from this device or others.
    pub fn directory_servers_updates(&self) -> Live<DirectoryServers> {
        self.directory_servers.updates()
    }

    /// Put `server` first among the browsed servers.
    pub async fn remember_directory_server(&self, server: &ServerName) -> Result<()> {
        if self.user_id().server_name() == server {
            return Ok(());
        }
        self.directory_servers
            .change(|s| s.remember(server.as_str()))
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn addresses_and_links() {
        let (id, via) = parse_address(" #ice:example.org ").unwrap();
        assert_eq!(id.as_str(), "#ice:example.org");
        assert!(via.is_empty());
        let (id, via) =
            parse_address("https://matrix.to/#/!abc:example.org?via=other.org&via=x.org").unwrap();
        assert_eq!(id.as_str(), "!abc:example.org");
        assert_eq!(via.len(), 2);
        let (id, _) = parse_address("matrix:r/ice:example.org").unwrap();
        assert_eq!(id.as_str(), "#ice:example.org");
        assert!(parse_address("https://matrix.to/#/@alice:example.org").is_none());
        assert!(parse_address("ice sheets").is_none());
    }

    #[test]
    fn remembered_servers_most_recent_first() {
        let mut servers = DirectoryServers::default();
        for s in ["a.org", "b.org", "a.org"] {
            servers.remember(s);
        }
        assert_eq!(servers.servers, ["a.org", "b.org"]);
        for i in 0..20 {
            servers.remember(&format!("s{i}.org"));
        }
        assert_eq!(servers.servers.len(), REMEMBERED);
    }
}
