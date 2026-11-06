//! A room's settings as the user sees them: what they may change, and when
//! it notifies them. Kept up to date while shown; a change made here shows
//! at once, and its setter returns once the server took it.

use std::{collections::HashMap, path::Path, sync::Arc, time::Instant};

use futures_util::{
    StreamExt,
    stream::{self, BoxStream},
};
use matrix_sdk::{
    Room, RoomMemberships,
    notification_settings::{IsEncrypted, IsOneToOne, RoomNotificationMode},
    ruma::{
        OwnedRoomAliasId, OwnedRoomId, RoomAliasId, RoomId, UserId,
        api::client::{
            directory::{get_room_visibility, set_room_visibility},
            room::Visibility,
        },
        events::{
            EmptyStateKey, StateEventContent,
            room::{
                canonical_alias::RoomCanonicalAliasEventContent,
                history_visibility::{HistoryVisibility, RoomHistoryVisibilityEventContent},
                join_rules::{AllowRule, JoinRule as SdkJoinRule, RoomJoinRulesEventContent},
            },
            space::child::SpaceChildEventContent,
        },
        push::{Action, PushCondition, RuleKind, Ruleset},
    },
};
use tokio::sync::{OwnedMutexGuard, broadcast};
use tokio_stream::wrappers::BroadcastStream;
use tracing::warn;

use crate::{
    Error, JoinRule, Live, Permissions, Result, Rights, RoomHandle, Session, User,
    echo::{ECHO_TIMEOUT, Setting, SettingKey},
    media::guess_mimetype,
    permissions::{levels, rights},
};

/// When a room notifies (push rules, so on all the user's devices).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NotifyMode {
    All,
    /// Only for mentions and keywords.
    Mentions,
    Mute,
}

impl NotifyMode {
    /// In the menu's order.
    pub const ALL: [Self; 3] = [Self::All, Self::Mentions, Self::Mute];

    /// As the menu names it: "All messages", "Mentions", "Mute".
    pub fn title(self) -> &'static str {
        match self {
            Self::All => "All messages",
            Self::Mentions => "Mentions",
            Self::Mute => "Mute",
        }
    }

    /// What it does, in a sentence: "Every message notifies you."…
    pub fn explanation(self) -> &'static str {
        match self {
            Self::All => "Every message notifies you.",
            Self::Mentions => "Only mentions of you and your keywords notify you.",
            Self::Mute => "Nothing here notifies you.",
        }
    }
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

impl History {
    /// In the menu's order.
    pub const ALL: [Self; 4] = [Self::Joined, Self::Invited, Self::Members, Self::Anyone];

    /// As the menu names it: "Since joining", "Since invited", "Members",
    /// "Anyone".
    pub fn title(self) -> &'static str {
        match self {
            Self::Joined => "Since joining",
            Self::Invited => "Since invited",
            Self::Members => "Members",
            Self::Anyone => "Anyone",
        }
    }

    /// Who may read it, in a sentence: "Members, from when they joined."…
    pub fn explanation(self) -> &'static str {
        match self {
            Self::Joined => "Members, from when they joined.",
            Self::Invited => "Members, from when they were invited.",
            Self::Members => "Members, all of it, also from before they joined.",
            Self::Anyone => "Anyone, without joining.",
        }
    }
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
    /// What we may change; the same as the room list's.
    pub rights: Rights,
    pub notify: NotifyMode,
    pub join_rule: JoinRule,
    /// The rules to offer, in order: Space members only for a room in a
    /// joined space, or already so.
    pub join_rule_choices: Vec<JoinRule>,
    /// The address people join by.
    pub address: Option<OwnedRoomAliasId>,
    /// Listed in our server's room directory; `None` if it wouldn't say.
    pub listed: Option<bool>,
    pub history: History,
    pub permissions: Permissions,
    pub encrypted: bool,
    /// The room version, as "12".
    pub version: Option<String>,
    /// The joined spaces listing the room.
    pub spaces: Vec<SpaceLink>,
    /// Joined spaces not listing it whose rooms we may change.
    pub can_add_to: Vec<OwnedRoomId>,
    pub banned: Vec<Banned>,
}

/// Changes to rooms' notification modes, one after another in each room:
/// each starts once the one before is confirmed, so from the server's
/// rules.
#[derive(Default)]
pub(crate) struct NotifyTurns(std::sync::Mutex<HashMap<OwnedRoomId, Arc<tokio::sync::Mutex<()>>>>);

impl NotifyTurns {
    /// `room`'s turn, once the change before it is done.
    async fn take(&self, room: &RoomId) -> OwnedMutexGuard<()> {
        let turn = (self.0.lock().unwrap().entry(room.to_owned()))
            .or_default()
            .clone();
        turn.lock_owned().await
    }
}

impl Session {
    /// Show `room`'s `setting` at once while `request` runs; it goes if the
    /// request fails.
    pub(crate) async fn echoed<T>(
        &self,
        room: &RoomId,
        setting: Setting,
        request: impl Future<Output = Result<T>>,
    ) -> Result<T> {
        self.echoes.set_setting(room, setting.clone());
        let result = request.await;
        (self.echoes).setting_answered(room, &setting, result.is_ok(), Instant::now());
        result
    }

    /// The joined spaces listing the room, and those we may add it to.
    async fn space_links(&self, room_id: &RoomId) -> (Vec<SpaceLink>, Vec<OwnedRoomId>) {
        let graph = self.space_graph();
        let (mut links, mut others) = (Vec::new(), Vec::new());
        for space in self.client.joined_rooms() {
            if !space.is_space() || space.room_id() == room_id {
                continue;
            }
            let can_change = rights(&space).await.add_rooms;
            match graph.child(space.room_id(), room_id) {
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
}

/// Each message on `updates`, as a change; ends when it closes.
fn changes<T: Clone + Send + 'static>(updates: broadcast::Receiver<T>) -> BoxStream<'static, ()> {
    // A message missed while lagging still left its change.
    BroadcastStream::new(updates).map(|_| ()).boxed()
}

impl RoomHandle {
    /// The room's settings, once read, kept up to date while held: as its
    /// state, who is banned, the notification rules and the joined spaces
    /// change. The changes made here show at once, until the settings
    /// show them or [`ECHO_TIMEOUT`] after the server took them. Whether
    /// the room is listed is not room state: it is read when this starts,
    /// and again once the server took a change to it. A reading that fails
    /// (the power levels not known yet) is tried again at the next change.
    pub fn settings(&self) -> impl Future<Output = Live<RoomSettings>> + Send + use<> {
        let handle = self.clone();
        let session = &self.session;
        // Followed before the first reading, so no change is missed.
        let mut changes = stream::select_all([
            changes(session.client.subscribe_to_room_updates(self.id())),
            changes(session.notification_settings.subscribe_to_changes()),
            session.spaces.updates().map(|_| ()).boxed(),
        ]);
        let mut echoes = session.echoes.subscribe();
        async move {
            let mut listed = handle.listed().await;
            let mut synced = loop {
                if let Some(settings) = handle.read_settings(listed).await {
                    break settings;
                }
                if changes.next().await.is_none() {
                    // Nothing to read it again on.
                    std::future::pending::<()>().await;
                }
            };
            let (first, mut timeout) = handle.shown(&synced);
            Live::spawn(first, move |tx| async move {
                // When the server took the last change to whether it is
                // listed.
                let mut accepted = None;
                loop {
                    let timed_out = async move {
                        match timeout {
                            Some(at) => tokio::time::sleep_until(at.into()).await,
                            None => std::future::pending::<()>().await,
                        }
                    };
                    tokio::select! {
                        change = changes.next() => {
                            if change.is_none() {
                                return;
                            }
                            if let Some(settings) = handle.read_settings(listed).await {
                                synced = settings;
                            }
                        }
                        changed = echoes.changed() => {
                            if changed.is_err() {
                                return;
                            }
                            let echoes = &handle.session.echoes;
                            let taken = echoes.accepted(handle.id(), SettingKey::Listed);
                            if taken.is_some() && taken != accepted {
                                accepted = taken;
                                listed = handle.listed().await;
                                synced.listed = listed;
                            }
                        }
                        () = timed_out => {}
                    }
                    let (shown, next) = handle.shown(&synced);
                    timeout = next;
                    tx.send_if_modified(|current| {
                        let changed = *current != shown;
                        if changed {
                            *current = shown;
                        }
                        changed
                    });
                }
            })
        }
    }

    /// The settings as synced, with `listed`; none while the power levels
    /// are not known yet, or if those banned can't be read.
    async fn read_settings(&self, listed: Option<bool>) -> Option<RoomSettings> {
        let (session, room) = (&self.session, &self.room);
        let levels = levels(room).await?;
        let banned = match room.members_no_sync(RoomMemberships::BAN).await {
            Ok(banned) => banned,
            Err(e) => {
                warn!("could not read who is banned from {}: {e}", self.id());
                return None;
            }
        };
        let banned = banned
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
        let join_rule = JoinRule::from(room.join_rule().as_ref());
        let in_space = !spaces.is_empty();
        Some(RoomSettings {
            rights: Rights::of(&levels, &session.user_id()),
            join_rule,
            join_rule_choices: JoinRule::choices(join_rule, in_space),
            notify: session.notify_mode(room).await.into(),
            address: room.canonical_alias(),
            listed,
            history: room.history_visibility_or_default().into(),
            permissions: Permissions::of(&levels),
            encrypted: room.encryption_state().is_encrypted(),
            version: room.version().map(|v| v.to_string()),
            spaces,
            can_add_to,
            banned,
        })
    }

    /// `synced` with the changes made here laid over, and when the next of
    /// those times out.
    fn shown(&self, synced: &RoomSettings) -> (RoomSettings, Option<Instant>) {
        let mut shown = synced.clone();
        let timeout = (self.session.echoes).lay_settings(self.id(), &mut shown, Instant::now());
        (shown, timeout)
    }

    /// Whether our server's directory lists the room; none if it would not
    /// say.
    async fn listed(&self) -> Option<bool> {
        let request = get_room_visibility::v3::Request::new(self.id().to_owned());
        let response = self.session.client.send(request).await.ok()?;
        Some(response.visibility == Visibility::Public)
    }

    /// Send `content` as the room's state.
    async fn send_state(
        &self,
        content: impl StateEventContent<StateKey = EmptyStateKey>,
    ) -> Result<()> {
        self.room.send_state_event(content).await?;
        Ok(())
    }

    /// Change when the room notifies, shown at once; returns once the
    /// server took it.
    ///
    /// Changes to a room's mode go one after another: each waits until the
    /// one before is confirmed by the push rules sync brings back (or
    /// [`ECHO_TIMEOUT`]), so it starts from the server's rules, not from
    /// older ones still on their way.
    pub async fn set_notify(&self, mode: NotifyMode) -> Result<()> {
        let request = self.send_notify(mode.into());
        self.session
            .echoed(self.id(), Setting::Notify(mode), request)
            .await
    }

    /// In the room's turn, send `mode`; it is confirmed before the next
    /// change.
    ///
    /// `notification_settings` applies a change to its rules as soon as the
    /// server took it, before sync brings it back; a sync from midway
    /// through the change (a rule added, the old one not yet removed) may
    /// still replace them after that. So the change is confirmed by the
    /// rules as last synced (the store's, kept before event handlers run),
    /// once `notification_settings` applied that sync too.
    async fn send_notify(&self, mode: RoomNotificationMode) -> Result<()> {
        let turn = self.session.notify_turns.take(self.id()).await;
        if self.has_notify(mode).await {
            // Nothing to change, so nothing comes back.
            return Ok(());
        }
        let settings = &self.session.notification_settings;
        let mut synced = settings.subscribe_to_changes();
        settings
            .set_room_notification_mode(self.id(), mode)
            .await
            .map_err(|e| Error::Notifications(e.to_string()))?;
        let handle = self.clone();
        tokio::spawn(async move {
            let _turn = turn;
            let confirmed = async {
                loop {
                    match synced.recv().await {
                        Err(broadcast::error::RecvError::Closed) => return,
                        // A change missed while lagging still left its rules.
                        _ if handle.synced_notify().await == Some(mode)
                            && handle.has_notify(mode).await =>
                        {
                            return;
                        }
                        _ => {}
                    }
                }
            };
            // The server took the change either way.
            let _ = tokio::time::timeout(ECHO_TIMEOUT, confirmed).await;
        });
        Ok(())
    }

    /// Whether `notification_settings` gives the room `mode` of its own.
    async fn has_notify(&self, mode: RoomNotificationMode) -> bool {
        let settings = &self.session.notification_settings;
        settings
            .get_user_defined_room_notification_mode(self.id())
            .await
            == Some(mode)
    }

    /// The room's own mode, as the push rules last synced give it.
    async fn synced_notify(&self) -> Option<RoomNotificationMode> {
        let rules = self.session.client.account().push_rules().await.ok()?;
        user_defined_mode(&rules, self.id())
    }

    /// Change who may join; [`JoinRule::SpaceMembers`] lets in members of
    /// the joined spaces the room is in.
    pub async fn set_join_rule(&self, rule: JoinRule) -> Result<()> {
        let content = match rule {
            JoinRule::Invite => SdkJoinRule::Invite,
            JoinRule::Public => SdkJoinRule::Public,
            JoinRule::Knock => SdkJoinRule::Knock,
            JoinRule::SpaceMembers => {
                let spaces = self.session.space_graph().parents(self.id()).to_vec();
                if spaces.is_empty() {
                    return Err(Error::Permissions("the room is in no space".into()));
                }
                RoomJoinRulesEventContent::restricted(
                    spaces.into_iter().map(AllowRule::room_membership).collect(),
                )
                .join_rule
            }
        };
        let request = self.send_state(RoomJoinRulesEventContent::new(content));
        (self.session)
            .echoed(self.id(), Setting::JoinRule(rule), request)
            .await
    }

    /// Give the room the address `#name:our.server`, or none.
    pub async fn set_address(&self, name: Option<&str>) -> Result<()> {
        let alias = match name {
            Some(name) => {
                let own = self.session.user_id();
                let alias = RoomAliasId::parse(format!("#{name}:{}", own.server_name()))
                    .map_err(|_| Error::Permissions(format!("“{name}” can't be an address")))?;
                Some(alias)
            }
            None => None,
        };
        let request = async {
            let (client, room_id) = (&self.session.client, self.id());
            if let Some(alias) = &alias {
                // Ours already if it was before; taken, the server says so.
                let ours = client
                    .resolve_room_alias(alias)
                    .await
                    .is_ok_and(|r| r.room_id == room_id);
                if !ours {
                    client.create_room_alias(alias, room_id).await?;
                }
            }
            let mut content = RoomCanonicalAliasEventContent::new();
            content.alias = alias.clone();
            content.alt_aliases = self.room.alt_aliases();
            self.send_state(content).await?;
            Ok::<_, Error>(())
        };
        let setting = Setting::Address(alias.clone());
        self.session.echoed(self.id(), setting, request).await
    }

    /// List the room in our server's directory, or not.
    pub async fn set_listed(&self, listed: bool) -> Result<()> {
        let visibility = if listed {
            Visibility::Public
        } else {
            Visibility::Private
        };
        let request = async {
            let request = set_room_visibility::v3::Request::new(self.id().to_owned(), visibility);
            self.session.client.send(request).await?;
            Ok::<_, Error>(())
        };
        (self.session)
            .echoed(self.id(), Setting::Listed(listed), request)
            .await
    }

    pub async fn set_history(&self, history: History) -> Result<()> {
        let request = self.send_state(RoomHistoryVisibilityEventContent::new(history.into()));
        (self.session)
            .echoed(self.id(), Setting::History(history), request)
            .await
    }

    /// Encrypt the room's messages from now on; not to be undone.
    pub async fn encrypt(&self) -> Result<()> {
        let request = async {
            self.room.enable_encryption().await?;
            Ok::<_, Error>(())
        };
        (self.session)
            .echoed(self.id(), Setting::Encrypted(true), request)
            .await
    }

    pub async fn unban(&self, user: &UserId) -> Result<()> {
        let request = async {
            self.room.unban_user(user, None).await?;
            Ok::<_, Error>(())
        };
        let setting = Setting::Banned(user.to_owned(), false);
        self.session.echoed(self.id(), setting, request).await
    }

    /// Suggest `room` to this space's members, or not.
    pub async fn set_suggested(&self, room: &RoomId, suggested: bool) -> Result<()> {
        let graph = self.session.space_graph();
        let child =
            (graph.child(self.id(), room)).ok_or_else(|| Error::RoomNotFound(room.to_string()))?;
        let mut content = SpaceChildEventContent::new(child.via.clone());
        content.order = child.order.clone();
        content.suggested = suggested;
        self.session.set_child(&self.room, room, content).await
    }

    /// Take `room` out of this space: it no longer lists it.
    pub async fn remove_child(&self, room: &RoomId) -> Result<()> {
        let removed = SpaceChildEventContent::new(Vec::new());
        self.session.set_child(&self.room, room, removed).await
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

    #[tokio::test]
    async fn changes_to_a_rooms_mode_take_turns() {
        let turns = NotifyTurns::default();
        let (a, b) = (owned_room_id!("!a:x"), owned_room_id!("!b:x"));
        let first = turns.take(&a).await;
        // Another room's change does not wait.
        drop(turns.take(&b).await);
        let next = turns.take(&a);
        let mut next = std::pin::pin!(next);
        assert!(futures_util::poll!(&mut next).is_pending());
        drop(first);
        next.await;
    }
}
