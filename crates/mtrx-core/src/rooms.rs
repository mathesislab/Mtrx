//! Live room list (rooms, spaces, DMs) and room management.

use std::{
    collections::HashMap,
    sync::Arc,
    time::{Instant, SystemTime},
};

use eyeball_im::Vector;
use futures_util::{StreamExt, pin_mut};
use matrix_sdk::{
    Client, Room, RoomState,
    deserialized_responses::SyncOrStrippedState,
    ruma::{
        OwnedEventId, OwnedMxcUri, OwnedRoomId, OwnedServerName, OwnedUserId, RoomId,
        RoomOrAliasId, RoomVersionId, UserId,
        api::client::{
            discovery::get_capabilities::v3::RoomVersionStability,
            room::create_room::v3::{CreationContent, Request as CreateRoomRequest, RoomPreset},
        },
        events::{
            InitialStateEvent, SyncStateEvent,
            room::encryption::RoomEncryptionEventContent,
            space::{child::SpaceChildEventContent, parent::SpaceParentEventContent},
        },
        room::RoomType,
        serde::Raw,
    },
};
use matrix_sdk_ui::room_list_service::{
    RoomListItem, RoomListLoadingState, RoomListService, State as ServiceState,
    filters::new_filter_non_left,
};
use tokio::sync::watch;
use tracing::warn;

use crate::{
    Error, Live, Result, RoomTag, Session,
    permissions::{Permissions, open_power_levels_raw},
    synced, system_time,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RoomKind {
    Room,
    Space,
    Direct,
}

/// What a frontend needs to show one entry of the room list.
#[derive(Clone, Debug, PartialEq)]
pub struct RoomSummary {
    pub id: OwnedRoomId,
    pub name: String,
    /// The room's picture; for a DM without one, the other user's.
    pub avatar: Option<OwnedMxcUri>,
    pub topic: Option<String>,
    pub kind: RoomKind,
    /// We are invited but have not joined yet.
    pub invited: bool,
    pub encrypted: bool,
    /// Anyone may join (else only invited people, or members of a space).
    pub public: bool,
    pub unread_messages: u64,
    pub unread_mentions: u64,
    pub unread_notifications: u64,
    pub last_activity: Option<SystemTime>,
    /// Joined spaces that list this room as a child.
    pub parents: Vec<OwnedRoomId>,
    pub tag: RoomTag,
    /// For a direct chat, the other user.
    pub direct_user: Option<OwnedUserId>,
    /// The room's address, e.g. `#ice-sheets:example.org`.
    pub alias: Option<String>,
    pub joined_members: u64,
    /// The pinned messages, in the order they were pinned.
    pub pinned: Vec<OwnedEventId>,
}

impl RoomSummary {
    /// Our direct chat with ourselves: the Notes to self.
    pub fn is_notes(&self, own_user: &UserId) -> bool {
        self.direct_user.as_deref() == Some(own_user)
    }
}

/// The room list now, see [`Session::room_list`].
#[derive(Clone, Debug, Default)]
pub struct RoomListState {
    /// Joined and invited rooms, most recently active first.
    pub rooms: Vec<RoomSummary>,
    /// Synced with the server since the list was created, and every room the
    /// server reported is in `rooms`. Before that, `rooms` may be a stale cache.
    pub loaded: bool,
}

async fn run_room_list(
    client: Client,
    service: Arc<RoomListService>,
    tx: watch::Sender<RoomListState>,
) -> Result<()> {
    let list = service
        .all_rooms()
        .await
        .map_err(|e| Error::RoomList(e.to_string()))?;
    // One page that holds every room; the frontends do their own scrolling.
    let (entries, controller) = list.entries_with_dynamic_adapters(u32::MAX as usize);
    controller.set_filter(Box::new(new_filter_non_left()));
    // `loading` turns Loaded straight from the on-disk cache; `service_state`
    // only reaches Running after server responses in this process.
    let loading = list.loading_state();
    let mut loading_stream = loading.clone();
    let service_state = service.state();
    let mut service_state_stream = service_state.clone();
    // Space membership changes do not show up as room list diffs.
    let mut room_updates = client.subscribe_to_all_room_updates();
    pin_mut!(entries);

    let mut items: Vector<RoomListItem> = Vector::new();
    loop {
        tokio::select! {
            diffs = entries.next() => match diffs {
                Some(diffs) => diffs.into_iter().for_each(|d| d.apply(&mut items)),
                None => return Ok(()),
            },
            update = room_updates.recv() => {
                if let Err(tokio::sync::broadcast::error::RecvError::Closed) = update {
                    return Ok(());
                }
            }
            Some(_) = loading_stream.next() => {}
            Some(_) = service_state_stream.next() => {}
        }

        let synced = matches!(service_state.get(), ServiceState::Running);
        let loaded = synced
            && match loading.get() {
                RoomListLoadingState::NotLoaded => false,
                RoomListLoadingState::Loaded {
                    maximum_number_of_rooms,
                } => items.len() >= maximum_number_of_rooms.unwrap_or(0) as usize,
            };
        let rooms = summarize(&items).await;
        tx.send_replace(RoomListState { rooms, loaded });
    }
}

async fn summarize(items: &Vector<RoomListItem>) -> Vec<RoomSummary> {
    let mut parents: HashMap<OwnedRoomId, Vec<OwnedRoomId>> = HashMap::new();
    for space in items
        .iter()
        .filter(|r| r.is_space() && r.state() == RoomState::Joined)
    {
        for child in space_children(space).await {
            parents
                .entry(child)
                .or_default()
                .push(space.room_id().to_owned());
        }
    }

    let mut rooms = Vec::with_capacity(items.len());
    for room in items.iter() {
        let kind = if room.is_space() {
            RoomKind::Space
        } else if room.is_direct().await.unwrap_or(false) {
            RoomKind::Direct
        } else {
            RoomKind::Room
        };
        let tag = if room.is_favourite() {
            RoomTag::Favourite
        } else if room.is_low_priority() {
            RoomTag::LowPriority
        } else {
            RoomTag::None
        };
        let direct_user = match kind {
            RoomKind::Direct => room
                .direct_targets()
                .into_iter()
                .find_map(|t| t.into_user_id()),
            _ => None,
        };
        let avatar = match (room.avatar_url(), kind) {
            (None, RoomKind::Direct) => direct_avatar(room).await,
            (avatar, _) => avatar,
        };
        rooms.push(RoomSummary {
            id: room.room_id().to_owned(),
            name: room
                .cached_display_name()
                .map(|n| n.to_string())
                .unwrap_or_else(|| room.room_id().to_string()),
            avatar,
            topic: room.topic().filter(|t| !t.trim().is_empty()),
            kind,
            invited: room.state() == RoomState::Invited,
            encrypted: room.encryption_state().is_encrypted(),
            public: matches!(
                room.join_rule(),
                Some(matrix_sdk::ruma::events::room::join_rules::JoinRule::Public)
            ),
            unread_messages: room.num_unread_messages(),
            unread_mentions: room.num_unread_mentions(),
            unread_notifications: room.num_unread_notifications(),
            last_activity: room.latest_event_timestamp().map(system_time),
            parents: parents.remove(room.room_id()).unwrap_or_default(),
            tag,
            direct_user,
            alias: room.canonical_alias().map(|a| a.to_string()),
            joined_members: room.joined_members_count(),
            pinned: room.pinned_event_ids().unwrap_or_default(),
        });
    }
    rooms
}

/// The other user's avatar in a DM: from the sync's room heroes, else from
/// their member event in the store.
async fn direct_avatar(room: &Room) -> Option<OwnedMxcUri> {
    let target = room
        .direct_targets()
        .into_iter()
        .find_map(|t| t.into_user_id())?;
    if let Some(hero) = room
        .heroes()
        .await
        .into_iter()
        .find(|h| h.user_id == target)
    {
        return hero.avatar_url;
    }
    match room.get_member_no_sync(&target).await {
        Ok(member) => member?.avatar_url().map(ToOwned::to_owned),
        Err(e) => {
            warn!(room = %room.room_id(), "could not read DM member: {e}");
            None
        }
    }
}

/// Children listed by `m.space.child` events. Events without `via` mean the
/// child was removed.
pub(crate) async fn space_children(space: &Room) -> Vec<OwnedRoomId> {
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
                Some(e.state_key)
            }
            Ok(SyncOrStrippedState::Stripped(e))
                if e.content.via.as_ref().is_some_and(|v| !v.is_empty()) =>
            {
                Some(e.state_key)
            }
            _ => None,
        })
        .collect()
}

/// Options for [`Session::create_room`].
#[derive(Clone, Debug, Default)]
pub struct NewRoom {
    pub name: Option<String>,
    pub topic: Option<String>,
    pub space: bool,
    pub encrypted: bool,
    pub invite: Vec<OwnedUserId>,
    pub direct: bool,
    pub permissions: Permissions,
    /// Space to create the room in.
    pub parent: Option<OwnedRoomId>,
    /// Owners like its creator, invited too; only where the server offers
    /// owners ([`Session::offers_owners`]).
    pub co_owners: Vec<OwnedUserId>,
}

/// A user found by [`Session::search_users`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UserSummary {
    pub id: OwnedUserId,
    pub name: Option<String>,
    pub avatar: Option<OwnedMxcUri>,
}

impl Session {
    /// The joined and invited rooms, kept up to date while held. Sync must
    /// be running for it to fill.
    pub async fn room_list(&self) -> Result<Live<RoomListState>> {
        let (client, service) = (self.client.clone(), self.sync.room_list_service());
        Ok(Live::spawn(RoomListState::default(), |tx| async move {
            if let Err(e) = run_room_list(client, service, tx).await {
                warn!("room list stopped: {e}");
            }
        }))
    }

    /// Whether new rooms can have owners besides their creator: on room
    /// version 12, where the server offers it as stable.
    pub async fn offers_owners(&self) -> bool {
        self.client
            .homeserver_capabilities()
            .room_versions()
            .await
            .is_ok_and(|versions| {
                versions.available.get(&RoomVersionId::V12) == Some(&RoomVersionStability::Stable)
            })
    }

    /// Make a room or space; the room lists show it as coming
    /// ([`crate::RoomLists::joining`]) until they list it.
    pub async fn create_room(&self, new: NewRoom) -> Result<OwnedRoomId> {
        let mut request = CreateRoomRequest::new();
        request.name = new.name;
        request.topic = new.topic;
        request.invite = new.invite;
        for owner in &new.co_owners {
            if !request.invite.contains(owner) {
                request.invite.push(owner.clone());
            }
        }
        request.is_direct = new.direct;
        request.preset = Some(if new.direct {
            RoomPreset::TrustedPrivateChat
        } else {
            RoomPreset::PrivateChat
        });
        // Direct chats make both people admins anyway.
        if new.permissions == Permissions::Everyone && !new.direct {
            request.power_level_content_override = Some(open_power_levels_raw()?);
        }
        // The newest version where the server has it: its creators are
        // owners, above every admin.
        let owners = self.offers_owners().await;
        if owners {
            request.room_version = Some(RoomVersionId::V12);
        } else if !new.co_owners.is_empty() {
            return Err(Error::Permissions(
                "this server can't give rooms co-owners".into(),
            ));
        }
        if new.space || !new.co_owners.is_empty() {
            let mut creation = CreationContent::new();
            if new.space {
                creation.room_type = Some(RoomType::Space);
            }
            creation.additional_creators = new.co_owners;
            request.creation_content = Some(Raw::new(&creation)?);
        }
        if new.encrypted {
            let event = InitialStateEvent::with_empty_state_key(
                RoomEncryptionEventContent::with_recommended_defaults(),
            );
            request.initial_state.push(Raw::new(&event)?.cast());
        }
        if let Some(parent) = &new.parent {
            let mut content = SpaceParentEventContent::new(self.via());
            content.canonical = true;
            let event = InitialStateEvent::new(parent.clone(), content);
            request.initial_state.push(Raw::new(&event)?.cast());
        }
        let room = self.client.create_room(request).await?.room_id().to_owned();
        self.echoes.set_joining(&room);
        if let Some(parent) = &new.parent {
            self.add_to_space(parent, &room).await?;
        }
        Ok(room)
    }

    /// The Notes chat (notes to self), if there is one.
    pub fn notes_room(&self) -> Option<OwnedRoomId> {
        self.client
            .get_dm_room(&self.user_id())
            .map(|room| room.room_id().to_owned())
    }

    /// The direct chat with `user`, created if there is none (coming in
    /// the room lists until they list it). With ourselves it is a "Notes"
    /// room only we are in.
    pub async fn direct_chat(&self, user: &UserId) -> Result<OwnedRoomId> {
        if let Some(room) = self.client.get_dm_room(user) {
            return Ok(room.room_id().to_owned());
        }
        if user != self.user_id() {
            let room = self.client.create_dm(user).await?.room_id().to_owned();
            self.echoes.set_joining(&room);
            return Ok(room);
        }
        let notes = self
            .create_room(NewRoom {
                name: Some("Notes".into()),
                topic: Some("Notes to self".into()),
                encrypted: true,
                direct: true,
                ..Default::default()
            })
            .await?;
        self.client
            .account()
            .mark_as_dm(&notes, &[user.to_owned()])
            .await?;
        Ok(notes)
    }

    pub async fn set_room_name(&self, room: &RoomId, name: &str) -> Result<()> {
        self.room(room)?.set_name(name.to_owned()).await?;
        Ok(())
    }

    /// Set the room's topic; a blank one removes it. The room lists show it
    /// at once ([`Session::room_lists`]).
    pub async fn set_room_topic(&self, room_id: &RoomId, topic: &str) -> Result<()> {
        let room = self.room(room_id)?;
        // As the room list has it.
        let shown = Some(topic.to_owned()).filter(|t| !t.trim().is_empty());
        self.echoes.set_topic(room_id, shown.clone());
        let result = room.set_room_topic(topic).await;
        (self.echoes).topic_answered(room_id, &shown, result.is_ok(), Instant::now());
        result?;
        Ok(())
    }

    /// Users matching `term` by name or ID in the server's user directory
    /// (people sharing a room with us, or everyone if the server allows).
    pub async fn search_users(&self, term: &str, limit: u64) -> Result<Vec<UserSummary>> {
        let response = self.client.search_users(term, limit).await?;
        Ok(response
            .results
            .into_iter()
            .map(|user| UserSummary {
                id: user.user_id,
                name: user.display_name,
                avatar: user.avatar_url,
            })
            .collect())
    }

    /// Servers to name in space links: ours.
    fn via(&self) -> Vec<OwnedServerName> {
        vec![self.user_id().server_name().to_owned()]
    }

    /// Join by room ID or alias, or accept an invite; through `via`, the
    /// servers a space, directory or link names for it. The room lists show
    /// the room as coming ([`crate::RoomLists::joining`]) until they list it
    /// as joined: by ID from the start, by alias once joined.
    pub async fn join(&self, room: &RoomOrAliasId, via: &[OwnedServerName]) -> Result<OwnedRoomId> {
        let id = OwnedRoomId::try_from(room.as_str()).ok();
        if let Some(id) = &id {
            self.echoes.set_joining(id);
        }
        match self.client.join_room_by_id_or_alias(room, via).await {
            Ok(joined) => {
                let joined = joined.room_id().to_owned();
                self.echoes.set_joining(&joined);
                Ok(joined)
            }
            Err(e) => {
                if let Some(id) = &id {
                    self.echoes.drop_joining(id);
                }
                Err(e.into())
            }
        }
    }

    /// Leave the room, or decline its invite; it is not coming any more.
    pub async fn leave(&self, room: &OwnedRoomId) -> Result<()> {
        self.room(room)?.leave().await?;
        self.echoes.drop_joining(room);
        Ok(())
    }

    pub async fn invite(&self, room: &OwnedRoomId, user: &OwnedUserId) -> Result<()> {
        Ok(self.room(room)?.invite_user_by_id(user).await?)
    }

    /// Add `child` to `space` (sends `m.space.child` in the space).
    pub async fn add_to_space(&self, space: &OwnedRoomId, child: &OwnedRoomId) -> Result<()> {
        let content = SpaceChildEventContent::new(self.via());
        synced::send_state(&self.room(space)?, child, content).await
    }

    /// Find a known room by ID, alias or exact display name.
    ///
    /// Names are matched against rooms in the local store, so a fresh
    /// profile needs to have synced first.
    pub async fn find_room(&self, query: &str) -> Result<OwnedRoomId> {
        if let Ok(id) = <&RoomOrAliasId>::try_from(query) {
            match OwnedRoomId::try_from(id.as_str()) {
                Ok(room_id) => return Ok(room_id),
                Err(_) => {
                    let alias = id
                        .as_str()
                        .try_into()
                        .expect("alias is valid if not a room ID");
                    return Ok(self.client.resolve_room_alias(alias).await?.room_id);
                }
            }
        }
        let matches: Vec<_> = self
            .client
            .rooms()
            .into_iter()
            .filter(|r| r.state() != RoomState::Left)
            .filter(|r| {
                r.cached_display_name()
                    .is_some_and(|n| n.to_string() == query)
            })
            .map(|r| r.room_id().to_owned())
            .collect();
        match matches.as_slice() {
            [id] => Ok(id.clone()),
            [] => Err(Error::RoomNotFound(query.to_owned())),
            _ => Err(Error::AmbiguousRoom(query.to_owned(), matches)),
        }
    }

    pub(crate) fn room(&self, id: &RoomId) -> Result<Room> {
        self.client
            .get_room(id)
            .ok_or_else(|| Error::RoomNotFound(id.to_string()))
    }
}
