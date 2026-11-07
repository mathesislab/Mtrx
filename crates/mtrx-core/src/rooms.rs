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
    ruma::{
        OwnedEventId, OwnedMxcUri, OwnedRoomId, OwnedServerName, OwnedUserId, RoomId,
        RoomOrAliasId, RoomVersionId, UserId,
        api::client::{
            discovery::get_capabilities::v3::RoomVersionStability,
            room::create_room::v3::{CreationContent, Request as CreateRoomRequest, RoomPreset},
        },
        events::{
            InitialStateEvent,
            room::{encryption::RoomEncryptionEventContent, join_rules::JoinRule as SdkJoinRule},
            space::{child::SpaceChildEventContent, parent::SpaceParentEventContent},
        },
        room::{JoinRuleKind, JoinRuleSummary, RoomType},
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
    Error, Live, Result, Rights, RoomHandle, RoomTag, Session, SpaceGraph, Topic, User,
    permissions::{self, Permissions, open_power_levels_raw},
    system_time,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RoomKind {
    Room,
    Space,
    Direct,
}

/// Who may join a room: one type for its settings, directory rows and the
/// room list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JoinRule {
    /// Only invited people.
    Invite,
    /// Anyone may ask to join; members let them in.
    Knock,
    /// Members of the spaces the room names (and invited people).
    SpaceMembers,
    /// Anyone.
    Public,
}

impl From<Option<&SdkJoinRule>> for JoinRule {
    fn from(rule: Option<&SdkJoinRule>) -> Self {
        match rule {
            Some(SdkJoinRule::Public) => Self::Public,
            Some(SdkJoinRule::Restricted(_) | SdkJoinRule::KnockRestricted(_)) => {
                Self::SpaceMembers
            }
            Some(SdkJoinRule::Knock) => Self::Knock,
            _ => Self::Invite,
        }
    }
}

impl From<&JoinRuleSummary> for JoinRule {
    fn from(rule: &JoinRuleSummary) -> Self {
        match rule {
            JoinRuleSummary::Public => Self::Public,
            JoinRuleSummary::Restricted(_) | JoinRuleSummary::KnockRestricted(_) => {
                Self::SpaceMembers
            }
            JoinRuleSummary::Knock => Self::Knock,
            _ => Self::Invite,
        }
    }
}

impl From<&JoinRuleKind> for JoinRule {
    fn from(rule: &JoinRuleKind) -> Self {
        match rule {
            JoinRuleKind::Public => Self::Public,
            JoinRuleKind::Restricted | JoinRuleKind::KnockRestricted => Self::SpaceMembers,
            JoinRuleKind::Knock => Self::Knock,
            _ => Self::Invite,
        }
    }
}

impl JoinRule {
    /// As the settings' menu names it: "Invited people", "Ask to join",
    /// "Space members", "Anyone".
    pub fn title(self) -> &'static str {
        match self {
            Self::Invite => "Invited people",
            Self::Knock => "Ask to join",
            Self::SpaceMembers => "Space members",
            Self::Public => "Anyone",
        }
    }

    /// As a room's facts say it: "Public: anyone can join", "Private: by
    /// invitation".
    pub fn fact(self) -> &'static str {
        match self {
            Self::Invite => "Private: by invitation",
            Self::Knock => "Private: people ask to join",
            Self::SpaceMembers => "Private: space members can join",
            Self::Public => "Public: anyone can join",
        }
    }

    /// The rules a room's settings offer, in the menu's order, its rule
    /// `current`: Space members only for a room in a space (`in_space`),
    /// or already so.
    pub(crate) fn choices(current: JoinRule, in_space: bool) -> Vec<JoinRule> {
        [Self::Invite, Self::Knock, Self::SpaceMembers, Self::Public]
            .into_iter()
            .filter(|&rule| rule != Self::SpaceMembers || in_space || current == rule)
            .collect()
    }
}

/// Our membership of a room.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Membership {
    Joined,
    Invited,
    /// We asked to join, and wait to be let in.
    Asked,
}

impl Membership {
    /// Ours as the store has it; none once left or banned.
    pub(crate) fn of(state: RoomState) -> Option<Self> {
        match state {
            RoomState::Joined => Some(Self::Joined),
            RoomState::Invited => Some(Self::Invited),
            RoomState::Knocked => Some(Self::Asked),
            _ => None,
        }
    }
}

/// What a listed room's button does, as the room lets us in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JoinAction {
    /// We are in: open it.
    Open,
    Join,
    AskToJoin,
    /// An invite, to answer.
    Invited,
    /// We asked, and wait: nothing to do.
    Asked,
    /// Only invited people may join: nothing to do.
    InviteOnly,
}

impl JoinAction {
    /// For a room `rule` lets in, with our `membership` of it.
    pub fn new(rule: JoinRule, membership: Option<Membership>) -> Self {
        match membership {
            Some(Membership::Joined) => Self::Open,
            Some(Membership::Invited) => Self::Invited,
            Some(Membership::Asked) => Self::Asked,
            None => match rule {
                JoinRule::Public | JoinRule::SpaceMembers => Self::Join,
                JoinRule::Knock => Self::AskToJoin,
                JoinRule::Invite => Self::InviteOnly,
            },
        }
    }

    /// Its button's or note's words: "Join", "Ask to join", "Open",
    /// "Invited", "Asked", "Invite only".
    pub fn title(self) -> &'static str {
        match self {
            Self::Open => "Open",
            Self::Join => "Join",
            Self::AskToJoin => "Ask to join",
            Self::Invited => "Invited",
            Self::Asked => "Asked",
            Self::InviteOnly => "Invite only",
        }
    }
}

/// What a frontend needs to show one entry of the room list.
#[derive(Clone, Debug, PartialEq)]
pub struct RoomSummary {
    pub id: OwnedRoomId,
    pub name: String,
    /// The room's picture; for a DM without one, the other user's.
    pub avatar: Option<OwnedMxcUri>,
    /// Parsed once here; empty without one.
    pub topic: Topic,
    pub kind: RoomKind,
    pub membership: Membership,
    pub encrypted: bool,
    pub join_rule: JoinRule,
    /// Unread since our read receipt; in the room lists, a room read here
    /// counts only what came since at once, before sync has the receipt.
    pub unread_messages: u64,
    pub unread_mentions: u64,
    pub unread_notifications: u64,
    pub last_activity: Option<SystemTime>,
    pub tag: RoomTag,
    /// For a direct chat, the other user.
    pub direct_user: Option<User>,
    /// The room's address, e.g. `#ice-sheets:example.org`.
    pub alias: Option<String>,
    pub joined_members: u64,
    /// The pinned messages, in the order they were pinned.
    pub pinned: Vec<OwnedEventId>,
    /// What we may do in it.
    pub rights: Rights,
}

impl RoomSummary {
    /// Our direct chat with ourselves: the Notes to self.
    pub fn is_notes(&self, own_user: &UserId) -> bool {
        self.direct_user
            .as_ref()
            .is_some_and(|u| &*u.id == own_user)
    }
}

/// The room list now, see [`Session::room_list`].
#[derive(Clone, Debug, Default)]
pub struct RoomListState {
    /// Joined and invited rooms, and those we asked to join, most recently
    /// active first.
    pub rooms: Vec<RoomSummary>,
    /// Synced with the server since the list was created, and every room the
    /// server reported is in `rooms`. Before that, `rooms` may be a stale cache.
    pub loaded: bool,
    /// The joined spaces and what they list.
    pub graph: Arc<SpaceGraph>,
}

async fn run_room_list(
    client: Client,
    service: Arc<RoomListService>,
    spaces: Live<Arc<SpaceGraph>>,
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
    let graphs = spaces.updates();
    pin_mut!(entries, graphs);

    let mut items: Vector<RoomListItem> = Vector::new();
    let mut rights = RightsCache::default();
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
            Some(_) = graphs.next() => {}
        }

        let synced = matches!(service_state.get(), ServiceState::Running);
        let loaded = synced
            && match loading.get() {
                RoomListLoadingState::NotLoaded => false,
                RoomListLoadingState::Loaded {
                    maximum_number_of_rooms,
                } => items.len() >= maximum_number_of_rooms.unwrap_or(0) as usize,
            };
        let rooms = summarize(&items, &mut rights).await;
        tx.send_replace(RoomListState {
            rooms,
            loaded,
            graph: spaces.get(),
        });
    }
}

/// Each listed room's rights, with the power levels event they were read
/// from: only rooms whose levels changed are read again.
#[derive(Default)]
struct RightsCache(HashMap<OwnedRoomId, (OwnedEventId, Rights)>);

impl RightsCache {
    /// `room`'s rights, read again only if its levels changed; kept in
    /// `next`.
    async fn rights(&self, room: &Room, next: &mut RightsCache) -> Rights {
        let Some(event) = permissions::levels_event(room).await else {
            return Rights::default();
        };
        let rights = match self.0.get(room.room_id()) {
            Some((seen, rights)) if *seen == event => *rights,
            _ => permissions::rights(room).await,
        };
        next.0.insert(room.room_id().to_owned(), (event, rights));
        rights
    }
}

async fn summarize(items: &Vector<RoomListItem>, cache: &mut RightsCache) -> Vec<RoomSummary> {
    // Rooms no longer listed are forgotten.
    let mut next = RightsCache::default();
    let mut rooms = Vec::with_capacity(items.len());
    for room in items.iter() {
        // Left since the list filtered it.
        let Some(membership) = Membership::of(room.state()) else {
            continue;
        };
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
            RoomKind::Direct => direct_user(room).await,
            _ => None,
        };
        let avatar =
            (room.avatar_url()).or_else(|| direct_user.as_ref().and_then(|u| u.avatar.clone()));
        rooms.push(RoomSummary {
            id: room.room_id().to_owned(),
            name: room
                .cached_display_name()
                .map(|n| n.to_string())
                .unwrap_or_else(|| room.room_id().to_string()),
            avatar,
            topic: Topic::parse(&room.topic().unwrap_or_default()),
            kind,
            membership,
            encrypted: room.encryption_state().is_encrypted(),
            join_rule: room.join_rule().as_ref().into(),
            unread_messages: room.num_unread_messages(),
            unread_mentions: room.num_unread_mentions(),
            unread_notifications: room.num_unread_notifications(),
            last_activity: room.latest_event_timestamp().map(system_time),
            tag,
            direct_user,
            alias: room.canonical_alias().map(|a| a.to_string()),
            joined_members: room.joined_members_count(),
            pinned: room.pinned_event_ids().unwrap_or_default(),
            rights: cache.rights(room, &mut next).await,
        });
    }
    *cache = next;
    rooms
}

/// The other user in a DM, with their name and avatar from the sync's room
/// heroes, else from their member event in the store.
async fn direct_user(room: &Room) -> Option<User> {
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
        return Some(User {
            id: target,
            name: hero.display_name,
            avatar: hero.avatar_url,
        });
    }
    match room.get_member_no_sync(&target).await {
        Ok(Some(member)) => Some(User {
            id: target,
            name: member.display_name().map(ToOwned::to_owned),
            avatar: member.avatar_url().map(ToOwned::to_owned),
        }),
        Ok(None) => Some(User::from(target)),
        Err(e) => {
            warn!(room = %room.room_id(), "could not read DM member: {e}");
            Some(User::from(target))
        }
    }
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

impl Session {
    /// The joined and invited rooms, and those we asked to join, kept up to
    /// date while held. Sync must be running for it to fill.
    pub async fn room_list(&self) -> Result<Live<RoomListState>> {
        let (client, service) = (self.client.clone(), self.sync.room_list_service());
        let spaces = self.spaces.clone();
        Ok(Live::spawn(RoomListState::default(), |tx| async move {
            if let Err(e) = run_room_list(client, service, spaces, tx).await {
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
            let content = SpaceChildEventContent::new(self.via());
            self.set_child(&self.sdk_room(parent)?, &room, content)
                .await?;
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

    /// Users matching `term` by name or ID in the server's user directory
    /// (people sharing a room with us, or everyone if the server allows).
    pub async fn search_users(&self, term: &str, limit: u64) -> Result<Vec<User>> {
        let response = self.client.search_users(term, limit).await?;
        Ok(response
            .results
            .into_iter()
            .map(|user| User {
                id: user.user_id,
                name: user.display_name,
                avatar: user.avatar_url,
            })
            .collect())
    }

    /// Servers to name in space links: ours.
    pub(crate) fn via(&self) -> Vec<OwnedServerName> {
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
}

impl RoomHandle {
    /// Rename the room. The room lists show a name at once; a blank one
    /// (the room is then named after its members) once synced.
    pub async fn set_name(&self, name: &str) -> Result<()> {
        let echoes = &self.session.echoes;
        let shown = !name.trim().is_empty();
        if shown {
            echoes.set_name(self.id(), name.to_owned());
        }
        let result = self.room.set_name(name.to_owned()).await;
        if shown {
            echoes.name_answered(self.id(), name, result.is_ok(), Instant::now());
        }
        result?;
        Ok(())
    }

    /// Set the topic; a blank one removes it. The room lists show it at
    /// once ([`Session::room_lists`]).
    pub async fn set_topic(&self, topic: &str) -> Result<()> {
        let echoes = &self.session.echoes;
        // As the room list has it.
        let shown = Topic::parse(topic);
        echoes.set_topic(self.id(), shown.clone());
        let result = self.room.set_room_topic(topic).await;
        echoes.topic_answered(self.id(), &shown, result.is_ok(), Instant::now());
        result?;
        Ok(())
    }

    /// Leave the room, or decline its invite; it is not coming any more.
    pub async fn leave(&self) -> Result<()> {
        self.room.leave().await?;
        self.session.echoes.drop_joining(self.id());
        Ok(())
    }

    pub async fn invite(&self, user: &UserId) -> Result<()> {
        Ok(self.room.invite_user_by_id(user).await?)
    }

    /// List `room` in this space (sends `m.space.child`).
    pub async fn add_child(&self, room: &RoomId) -> Result<()> {
        let content = SpaceChildEventContent::new(self.session.via());
        self.session.set_child(&self.room, room, content).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn space_members_are_offered_in_a_space_or_when_so() {
        use JoinRule::*;
        assert_eq!(JoinRule::choices(Invite, false), [Invite, Knock, Public]);
        let all = [Invite, Knock, SpaceMembers, Public];
        assert_eq!(JoinRule::choices(Invite, true), all);
        assert_eq!(JoinRule::choices(SpaceMembers, false), all);
    }

    /// Our membership first, then what the room lets us do.
    #[test]
    fn join_actions() {
        let action = JoinAction::new;
        assert_eq!(action(JoinRule::Public, None), JoinAction::Join);
        assert_eq!(action(JoinRule::SpaceMembers, None), JoinAction::Join);
        assert_eq!(action(JoinRule::Knock, None), JoinAction::AskToJoin);
        assert_eq!(action(JoinRule::Invite, None), JoinAction::InviteOnly);
        let joined = Some(Membership::Joined);
        assert_eq!(action(JoinRule::Knock, joined), JoinAction::Open);
        let invited = Some(Membership::Invited);
        assert_eq!(action(JoinRule::Invite, invited), JoinAction::Invited);
        let asked = Some(Membership::Asked);
        assert_eq!(action(JoinRule::Knock, asked), JoinAction::Asked);
        assert_eq!(JoinAction::AskToJoin.title(), "Ask to join");
        assert_eq!(JoinRule::Knock.fact(), "Private: people ask to join");
    }
}
