//! A room's settings as the user sees them: what they may change, and when
//! it notifies them.

use std::path::Path;

use matrix_sdk::{
    Room, RoomMemberships,
    deserialized_responses::SyncOrStrippedState,
    notification_settings::{IsEncrypted, IsOneToOne, RoomNotificationMode},
    ruma::{
        OwnedRoomAliasId, OwnedRoomId, RoomAliasId, RoomId, UserId,
        api::client::{
            directory::{get_room_visibility, set_room_visibility},
            room::Visibility,
        },
        events::{
            EmptyStateKey, StateEventType, SyncStateEvent,
            room::{
                canonical_alias::RoomCanonicalAliasEventContent,
                history_visibility::{HistoryVisibility, RoomHistoryVisibilityEventContent},
                join_rules::{AllowRule, JoinRule, RoomJoinRulesEventContent},
                member::MembershipState,
            },
            space::child::SpaceChildEventContent,
        },
        push::{Action, PushCondition, RuleKind, Ruleset},
    },
};
use tokio::sync::broadcast::error::RecvError;

use crate::{
    Error, Permissions, Result, RoomHandle, Session, User,
    media::guess_mimetype,
    permissions::{everyone_can_edit, power_levels},
    rooms::space_children,
    synced,
};

/// When a room notifies (push rules, so on all the user's devices).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NotifyMode {
    All,
    /// Only for mentions and keywords.
    Mentions,
    Mute,
}

impl From<RoomNotificationMode> for NotifyMode {
    fn from(mode: RoomNotificationMode) -> Self {
        match mode {
            RoomNotificationMode::AllMessages => Self::All,
            RoomNotificationMode::MentionsAndKeywordsOnly => Self::Mentions,
            RoomNotificationMode::Mute => Self::Mute,
        }
    }
}

impl From<NotifyMode> for RoomNotificationMode {
    fn from(mode: NotifyMode) -> Self {
        match mode {
            NotifyMode::All => Self::AllMessages,
            NotifyMode::Mentions => Self::MentionsAndKeywordsOnly,
            NotifyMode::Mute => Self::Mute,
        }
    }
}

/// Who may join a room.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Access {
    /// Only invited people.
    Invite,
    /// Members of the spaces the room is in (and invited people).
    SpaceMembers,
    /// Anyone may ask to join; members let them in.
    Knock,
    /// Anyone.
    Public,
}

impl From<Option<JoinRule>> for Access {
    fn from(rule: Option<JoinRule>) -> Self {
        match rule {
            Some(JoinRule::Public) => Self::Public,
            Some(JoinRule::Restricted(_) | JoinRule::KnockRestricted(_)) => Self::SpaceMembers,
            Some(JoinRule::Knock) => Self::Knock,
            _ => Self::Invite,
        }
    }
}

/// Who may read a room's history.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum History {
    /// Members, from when they joined.
    Joined,
    /// Members, from when they were invited.
    Invited,
    /// Members, all of it.
    Members,
    /// Anyone, members or not.
    Anyone,
}

impl From<HistoryVisibility> for History {
    fn from(visibility: HistoryVisibility) -> Self {
        match visibility {
            HistoryVisibility::Joined => Self::Joined,
            HistoryVisibility::Invited => Self::Invited,
            HistoryVisibility::WorldReadable => Self::Anyone,
            _ => Self::Members,
        }
    }
}

impl From<History> for HistoryVisibility {
    fn from(history: History) -> Self {
        match history {
            History::Joined => Self::Joined,
            History::Invited => Self::Invited,
            History::Members => Self::Shared,
            History::Anyone => Self::WorldReadable,
        }
    }
}

/// A joined space a room is in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SpaceLink {
    pub space: OwnedRoomId,
    /// Suggested to the space's members.
    pub suggested: bool,
    /// We may change the space's rooms.
    pub can_change: bool,
}

/// Someone banned from a room.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Banned {
    pub user: User,
    pub reason: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RoomSettings {
    pub can_rename: bool,
    pub can_change_topic: bool,
    pub can_change_picture: bool,
    pub can_invite: bool,
    pub notify: NotifyMode,
    pub access: Access,
    pub can_change_access: bool,
    /// For a space: may list rooms in it.
    pub can_add_rooms: bool,
    /// The address people join by.
    pub address: Option<OwnedRoomAliasId>,
    pub can_change_address: bool,
    /// Listed in our server's room directory; `None` if it wouldn't say.
    pub listed: Option<bool>,
    pub history: History,
    pub can_change_history: bool,
    pub permissions: Permissions,
    pub can_change_permissions: bool,
    pub encrypted: bool,
    pub can_encrypt: bool,
    /// The room version, as "12".
    pub version: Option<String>,
    /// The joined spaces listing the room.
    pub spaces: Vec<SpaceLink>,
    /// Joined spaces not listing it whose rooms we may change.
    pub can_add_to: Vec<OwnedRoomId>,
    pub banned: Vec<Banned>,
    pub can_unban: bool,
}

impl Session {
    /// The joined spaces listing the room, and those we may add it to.
    async fn space_links(&self, room_id: &RoomId) -> (Vec<SpaceLink>, Vec<OwnedRoomId>) {
        let own = self.user_id();
        let (mut links, mut others) = (Vec::new(), Vec::new());
        for space in self.client.joined_rooms() {
            if !space.is_space() || space.room_id() == room_id {
                continue;
            }
            let can_change = power_levels(&space)
                .await
                .is_ok_and(|levels| levels.user_can_send_state(&own, StateEventType::SpaceChild));
            match child_link(&space, room_id).await {
                Some(link) => links.push(SpaceLink {
                    space: space.room_id().to_owned(),
                    suggested: link.suggested,
                    can_change,
                }),
                None if can_change => others.push(space.room_id().to_owned()),
                None => {}
            }
        }
        (links, others)
    }

    /// The room's own mode, else the default for rooms like it.
    async fn notify_mode(&self, room: &Room) -> RoomNotificationMode {
        let settings = &self.notification_settings;
        if let Some(mode) = settings
            .get_user_defined_room_notification_mode(room.room_id())
            .await
        {
            return mode;
        }
        let encrypted = room.encryption_state().is_encrypted();
        // Two people, as push rules count them.
        let one_to_one = room.active_members_count() == 2;
        settings
            .get_default_room_notification_mode(
                IsEncrypted::from(encrypted),
                IsOneToOne::from(one_to_one),
            )
            .await
    }

    /// Joined spaces listing `room_id` as a child.
    async fn parent_spaces(&self, room_id: &RoomId) -> Vec<OwnedRoomId> {
        let mut parents = Vec::new();
        for space in self
            .client
            .joined_rooms()
            .into_iter()
            .filter(|r| r.is_space())
        {
            if space_children(&space)
                .await
                .iter()
                .any(|child| child == room_id)
            {
                parents.push(space.room_id().to_owned());
            }
        }
        parents
    }
}

impl RoomHandle {
    pub async fn settings(&self) -> Result<RoomSettings> {
        let (session, room) = (&self.session, &self.room);
        let levels = power_levels(room).await?;
        let own = session.user_id();
        let can = |state| levels.user_can_send_state(&own, state);
        let listed = session
            .client
            .send(get_room_visibility::v3::Request::new(self.id().to_owned()))
            .await
            .ok()
            .map(|response| response.visibility == Visibility::Public);
        let banned = room
            .members_no_sync(RoomMemberships::BAN)
            .await?
            .into_iter()
            .map(|member| Banned {
                user: User {
                    id: member.user_id().to_owned(),
                    name: member.display_name().map(ToOwned::to_owned),
                    avatar: member.avatar_url().map(ToOwned::to_owned),
                },
                reason: member.event().reason().map(ToOwned::to_owned),
            })
            .collect();
        let (spaces, can_add_to) = session.space_links(self.id()).await;
        Ok(RoomSettings {
            can_rename: can(StateEventType::RoomName),
            can_change_topic: can(StateEventType::RoomTopic),
            can_change_picture: can(StateEventType::RoomAvatar),
            can_invite: levels.user_can_invite(&own),
            access: room.join_rule().into(),
            can_change_access: can(StateEventType::RoomJoinRules),
            can_add_rooms: can(StateEventType::SpaceChild),
            notify: session.notify_mode(room).await.into(),
            address: room.canonical_alias(),
            can_change_address: can(StateEventType::RoomCanonicalAlias),
            listed,
            history: room.history_visibility_or_default().into(),
            can_change_history: can(StateEventType::RoomHistoryVisibility),
            permissions: if everyone_can_edit(&levels) {
                Permissions::Everyone
            } else {
                Permissions::Moderators
            },
            can_change_permissions: can(StateEventType::RoomPowerLevels),
            encrypted: room.encryption_state().is_encrypted(),
            can_encrypt: can(StateEventType::RoomEncryption),
            version: room.version().map(|v| v.to_string()),
            spaces,
            can_add_to,
            banned,
            can_unban: levels.user_can_ban(&own),
        })
    }

    /// Change when the room notifies, returning once the push rules that
    /// sync brings back give it that mode (or after a while): a further
    /// change then starts from the server's rules, not from older ones
    /// still on their way.
    ///
    /// `notification_settings` applies a change to its rules as soon as the
    /// server took it, before sync brings it back; a sync from midway
    /// through the change (a rule added, the old one not yet removed) may
    /// still replace them after that. So the change is confirmed by the
    /// rules as last synced (the store's, kept before event handlers run),
    /// once `notification_settings` applied that sync too.
    pub async fn set_notify(&self, mode: NotifyMode) -> Result<()> {
        let (session, room_id) = (&self.session, self.id());
        let settings = &session.notification_settings;
        let mode = RoomNotificationMode::from(mode);
        let has_mode = || async {
            settings
                .get_user_defined_room_notification_mode(room_id)
                .await
                == Some(mode)
        };
        let synced_mode = || async {
            let rules = session.client.account().push_rules().await.ok()?;
            user_defined_mode(&rules, room_id)
        };
        if has_mode().await {
            // Nothing to change, so nothing comes back.
            return Ok(());
        }
        let mut synced = settings.subscribe_to_changes();
        settings
            .set_room_notification_mode(room_id, mode)
            .await
            .map_err(|e| Error::Notifications(e.to_string()))?;
        let confirmed = async {
            loop {
                match synced.recv().await {
                    Err(RecvError::Closed) => return,
                    // A change missed while lagging still left its rules.
                    _ if synced_mode().await == Some(mode) && has_mode().await => return,
                    _ => {}
                }
            }
        };
        // The server took the change either way.
        let _ = tokio::time::timeout(synced::CONFIRM, confirmed).await;
        Ok(())
    }

    /// Change who may join; [`Access::SpaceMembers`] lets in members of the
    /// joined spaces the room is in.
    pub async fn set_join_rule(&self, access: Access) -> Result<()> {
        let rule = match access {
            Access::Invite => JoinRule::Invite,
            Access::Public => JoinRule::Public,
            Access::Knock => JoinRule::Knock,
            Access::SpaceMembers => {
                let spaces = self.session.parent_spaces(self.id()).await;
                if spaces.is_empty() {
                    return Err(Error::Permissions("the room is in no space".into()));
                }
                RoomJoinRulesEventContent::restricted(
                    spaces.into_iter().map(AllowRule::room_membership).collect(),
                )
                .join_rule
            }
        };
        let content = RoomJoinRulesEventContent::new(rule);
        synced::send_state(&self.room, &EmptyStateKey, content).await
    }

    /// Give the room the address `#name:our.server`, or none.
    pub async fn set_address(&self, name: Option<&str>) -> Result<()> {
        let (client, room_id) = (&self.session.client, self.id());
        let alias = match name {
            Some(name) => {
                let own = self.session.user_id();
                let alias = RoomAliasId::parse(format!("#{name}:{}", own.server_name()))
                    .map_err(|_| Error::Permissions(format!("“{name}” can't be an address")))?;
                // Ours already if it was before; taken, the server says so.
                let ours = client
                    .resolve_room_alias(&alias)
                    .await
                    .is_ok_and(|r| r.room_id == room_id);
                if !ours {
                    client.create_room_alias(&alias, room_id).await?;
                }
                Some(alias)
            }
            None => None,
        };
        let mut content = RoomCanonicalAliasEventContent::new();
        content.alias = alias;
        content.alt_aliases = self.room.alt_aliases();
        synced::send_state(&self.room, &EmptyStateKey, content).await
    }

    /// List the room in our server's directory, or not.
    pub async fn set_listed(&self, listed: bool) -> Result<()> {
        let visibility = if listed {
            Visibility::Public
        } else {
            Visibility::Private
        };
        let request = set_room_visibility::v3::Request::new(self.id().to_owned(), visibility);
        self.session.client.send(request).await?;
        Ok(())
    }

    pub async fn set_history(&self, history: History) -> Result<()> {
        let content = RoomHistoryVisibilityEventContent::new(history.into());
        synced::send_state(&self.room, &EmptyStateKey, content).await
    }

    /// Encrypt the room's messages from now on; not to be undone.
    pub async fn encrypt(&self) -> Result<()> {
        self.room.enable_encryption().await?;
        Ok(())
    }

    pub async fn unban(&self, user: &UserId) -> Result<()> {
        let room = &self.room;
        let updates = self.session.client.subscribe_to_room_updates(self.id());
        room.unban_user(user, None).await?;
        synced::until(updates, || async {
            !matches!(
                room.get_member_no_sync(user).await,
                Ok(Some(member)) if *member.membership() == MembershipState::Ban
            )
        })
        .await;
        Ok(())
    }

    /// Suggest `room` to this space's members, or not.
    pub async fn set_suggested(&self, room: &RoomId, suggested: bool) -> Result<()> {
        let mut content = child_link(&self.room, room)
            .await
            .ok_or_else(|| Error::RoomNotFound(room.to_string()))?;
        content.suggested = suggested;
        synced::send_state(&self.room, room, content).await
    }

    /// Take `room` out of this space: it no longer lists it.
    pub async fn remove_child(&self, room: &RoomId) -> Result<()> {
        let removed = SpaceChildEventContent::new(Vec::new());
        synced::send_state(&self.room, room, removed).await
    }

    /// Make the image at `path` the room's picture.
    pub async fn set_picture(&self, path: &Path) -> Result<()> {
        let mimetype = guess_mimetype(&path.to_string_lossy());
        if mimetype.type_() != mime_guess::mime::IMAGE {
            return Err(Error::InvalidMediaType(mimetype.to_string()));
        }
        let data = tokio::fs::read(path).await?;
        self.room.upload_avatar(&mimetype, data, None).await?;
        Ok(())
    }

    pub async fn remove_picture(&self) -> Result<()> {
        self.room.remove_avatar().await?;
        Ok(())
    }
}

/// The space's link to a room, if it lists it.
async fn child_link(space: &Room, room_id: &RoomId) -> Option<SpaceChildEventContent> {
    let raw = space
        .get_state_event_static_for_key::<SpaceChildEventContent, _>(room_id)
        .await
        .ok()??;
    match raw.deserialize().ok()? {
        SyncOrStrippedState::Sync(SyncStateEvent::Original(event))
            if !event.content.via.is_empty() =>
        {
            Some(event.content)
        }
        SyncOrStrippedState::Stripped(event) => {
            let mut content =
                SpaceChildEventContent::new(event.content.via.filter(|v| !v.is_empty())?);
            content.suggested = event.content.suggested;
            content.order = event.content.order;
            Some(content)
        }
        _ => None,
    }
}

/// The mode `rules` give the room, if the user chose one: as
/// `notification_settings` reads its own rules (which it does not offer
/// for others).
fn user_defined_mode(rules: &Ruleset, room_id: &RoomId) -> Option<RoomNotificationMode> {
    let muted = rules.override_.iter().any(|rule| {
        rule.enabled
            && rule.conditions.iter().any(|condition| {
                matches!(
                    condition,
                    PushCondition::EventMatch(data)
                        if data.key == "room_id" && data.pattern == *room_id
                )
            })
            && !rule.actions.iter().any(Action::should_notify)
    });
    if muted {
        return Some(RoomNotificationMode::Mute);
    }
    let rule = rules.get(RuleKind::Room, room_id)?;
    Some(if rule.triggers_notification() {
        RoomNotificationMode::AllMessages
    } else {
        RoomNotificationMode::MentionsAndKeywordsOnly
    })
}

#[cfg(test)]
mod tests {
    use matrix_sdk::ruma::{
        owned_room_id, owned_user_id,
        push::{NewConditionalPushRule, NewPushRule, NewSimplePushRule},
    };

    use super::*;

    #[test]
    fn modes_are_read_from_rules_as_the_sdk_does() {
        let room = owned_room_id!("!r:example.org");
        let mut rules = Ruleset::server_default(&owned_user_id!("@me:example.org"));
        assert_eq!(user_defined_mode(&rules, &room), None);
        // Mentions: a room rule that does not notify.
        rules
            .insert(
                NewPushRule::Room(NewSimplePushRule::new(room.clone(), vec![])),
                None,
                None,
            )
            .unwrap();
        assert_eq!(
            user_defined_mode(&rules, &room),
            Some(RoomNotificationMode::MentionsAndKeywordsOnly)
        );
        // Mute wins over it, as midway from Mute to Mentions.
        let condition =
            PushCondition::EventMatch(matrix_sdk::ruma::push::EventMatchConditionData::new(
                "room_id".into(),
                room.to_string(),
            ));
        rules
            .insert(
                NewPushRule::Override(NewConditionalPushRule::new(
                    room.to_string(),
                    vec![condition],
                    vec![],
                )),
                None,
                None,
            )
            .unwrap();
        assert_eq!(
            user_defined_mode(&rules, &room),
            Some(RoomNotificationMode::Mute)
        );
    }
}
