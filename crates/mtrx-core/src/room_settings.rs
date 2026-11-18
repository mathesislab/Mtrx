//! A room's settings as the user sees them: what they may change, and when
//! it notifies them. Kept up to date while shown; a change made here shows
//! at once, and its setter returns once the server took it.

use std::{
    collections::HashMap,
    path::Path,
    sync::Arc,
    time::{Instant, SystemTime},
};

use futures_util::{
    StreamExt,
    stream::{self, BoxStream},
};
use matrix_sdk::{
    Client, Room, RoomMemberships,
    notification_settings::{IsEncrypted, IsOneToOne, NotificationSettings, RoomNotificationMode},
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
use tokio_stream::wrappers::{BroadcastStream, WatchStream};
use tracing::warn;

use crate::{
    Error, JoinRule, Live, MuteFor, Permissions, Result, Rights, RoomHandle, Session, TimedMute,
    User,
    echo::{ECHO_TIMEOUT, Setting, SettingKey},
    media::guess_mimetype,
    permissions::{levels, rights},
    send_time::describe_until,
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

/// An entry of a room's notifications menu: a mode, or a mute.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NotifyAction {
    Mode(NotifyMode),
    Mute(MuteFor),
}

/// An entry of a room's notifications menu, with its words.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NotifyChoice {
    pub action: NotifyAction,
    /// "All messages", "Mentions", "Mute for 1 hour", "Mute for 8 hours",
    /// "Mute until tomorrow, 08:00", "Mute until I unmute"; "Mute" for a
    /// room muted without an end.
    pub title: String,
    /// The room's mode now.
    pub current: bool,
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
    /// The end of a timed mute.
    pub muted_until: Option<SystemTime>,
    /// "All messages", "Mentions", "Muted", "Muted until 14:00".
    pub notify_title: String,
    /// "Every message notifies you.", "Muted until 14:00, then all
    /// messages again."
    pub notify_explanation: String,
    /// The notifications menu, in order.
    pub notify_choices: Vec<NotifyChoice>,
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

/// What a room's settings say of its notifications.
#[derive(Debug, PartialEq, Eq)]
struct NotifyWords {
    muted_until: Option<SystemTime>,
    title: String,
    explanation: String,
    choices: Vec<NotifyChoice>,
}

impl NotifyWords {
    /// For a room whose mode is `mode`, and `mute` if muted for a time; it
    /// follows `default` without a mode of its own.
    fn new(
        mode: NotifyMode,
        mute: Option<&TimedMute>,
        default: NotifyMode,
        now: SystemTime,
    ) -> Self {
        let timed = mute.filter(|_| mode == NotifyMode::Mute);
        let (title, explanation) = match (timed, mode) {
            (Some(mute), _) => {
                let until = describe_until(mute.until, now);
                let then = match mute.before.unwrap_or(default) {
                    NotifyMode::All => "all messages again",
                    NotifyMode::Mentions => "mentions only",
                    NotifyMode::Mute => "still nothing",
                };
                (
                    format!("Muted until {until}"),
                    format!("Muted until {until}, then {then}."),
                )
            }
            (None, NotifyMode::Mute) => ("Muted".to_owned(), mode.explanation().to_owned()),
            (None, mode) => (mode.title().to_owned(), mode.explanation().to_owned()),
        };
        let choice = |choice: NotifyMode| NotifyChoice {
            action: NotifyAction::Mode(choice),
            title: choice.title().to_owned(),
            current: choice == mode,
        };
        let mut choices = vec![choice(NotifyMode::All), choice(NotifyMode::Mentions)];
        if mode == NotifyMode::Mute && timed.is_none() {
            // Muted without an end: no mode to go back to after a time.
            choices.push(choice(NotifyMode::Mute));
        } else {
            choices.extend(MuteFor::ALL.map(|duration| NotifyChoice {
                action: NotifyAction::Mute(duration),
                title: duration.title(now),
                current: false,
            }));
        }
        Self {
            muted_until: timed.map(|mute| mute.until),
            title,
            explanation,
            choices,
        }
    }

    fn lay(self, settings: &mut RoomSettings) {
        settings.muted_until = self.muted_until;
        settings.notify_title = self.title;
        settings.notify_explanation = self.explanation;
        settings.notify_choices = self.choices;
    }
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

/// A room's turn to change its mode, see [`RoomModes`].
pub(crate) type Turn = OwnedMutexGuard<()>;

/// Rooms' notification modes (push rules, so on all the user's devices),
/// read and changed in each room's turn ([`NotifyTurns`]).
#[derive(Clone)]
pub(crate) struct RoomModes {
    client: Client,
    /// Follows push rule changes, so it knows a room's mode right after
    /// setting it (sync echoes it later).
    settings: NotificationSettings,
    turns: Arc<NotifyTurns>,
}

impl RoomModes {
    pub(crate) fn new(client: Client, settings: NotificationSettings) -> Self {
        Self {
            client,
            settings,
            turns: Arc::default(),
        }
    }

    /// `room`'s turn, once the change before it is confirmed.
    pub(crate) async fn turn(&self, room: &RoomId) -> Turn {
        self.turns.take(room).await
    }

    /// The room's own mode; none if it follows the default for rooms like
    /// it.
    pub(crate) async fn own(&self, room: &RoomId) -> Option<NotifyMode> {
        let settings = &self.settings;
        let mode = settings.get_user_defined_room_notification_mode(room).await;
        mode.map(NotifyMode::from)
    }

    /// In its `turn`, give `room` `mode` of its own, or none (it follows
    /// the default then); it is confirmed before the next change.
    ///
    /// `notification_settings` applies a change to its rules as soon as the
    /// server took it, before sync brings it back; a sync from midway
    /// through the change (a rule added, the old one not yet removed) may
    /// still replace them after that. So the change is confirmed by the
    /// rules as last synced (the store's, kept before event handlers run),
    /// once `notification_settings` applied that sync too.
    pub(crate) async fn send(
        &self,
        turn: Turn,
        room: &RoomId,
        mode: Option<NotifyMode>,
    ) -> Result<()> {
        if self.own(room).await == mode {
            // Nothing to change, so nothing comes back.
            return Ok(());
        }
        let settings = &self.settings;
        let mut synced = settings.subscribe_to_changes();
        let changed = match mode {
            Some(mode) => settings.set_room_notification_mode(room, mode.into()).await,
            None => settings.delete_user_defined_room_rules(room).await,
        };
        changed.map_err(|e| Error::Notifications(e.to_string()))?;
        let (modes, room) = (self.clone(), room.to_owned());
        tokio::spawn(async move {
            let _turn = turn;
            let confirmed = async {
                loop {
                    match synced.recv().await {
                        Err(broadcast::error::RecvError::Closed) => return,
                        // A change missed while lagging still left its rules.
                        _ if modes.synced(&room).await == Some(mode)
                            && modes.own(&room).await == mode =>
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

    /// The room's own mode, as the push rules last synced give it; none if
    /// they can't be read.
    async fn synced(&self, room: &RoomId) -> Option<Option<NotifyMode>> {
        let rules = self.client.account().push_rules().await.ok()?;
        Some(user_defined_mode(&rules, room).map(NotifyMode::from))
    }
}

/// A room's settings as synced, and the mode it follows without one of
/// its own.
struct Read {
    settings: RoomSettings,
    default: NotifyMode,
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

    /// The account's mode for rooms like `room`, which it follows without
    /// one of its own.
    async fn default_notify(&self, room: &Room) -> NotifyMode {
        let encrypted = room.encryption_state().is_encrypted();
        // Two people, as push rules count them.
        let one_to_one = room.active_members_count() == 2;
        (self.notification_settings)
            .get_default_room_notification_mode(
                IsEncrypted::from(encrypted),
                IsOneToOne::from(one_to_one),
            )
            .await
            .into()
    }
}

/// Each message on `updates`, as a change; ends when it closes.
fn changes<T: Clone + Send + 'static>(updates: broadcast::Receiver<T>) -> BoxStream<'static, ()> {
    // A message missed while lagging still left its change.
    BroadcastStream::new(updates).map(|_| ()).boxed()
}

impl RoomHandle {
    /// The room's settings, once read, kept up to date while held: as its
    /// state, who is banned, the notification rules, its timed mute and
    /// the joined spaces change. The changes made here show at once, until
    /// the settings show them or [`ECHO_TIMEOUT`] after the server took them. Whether
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
            WatchStream::from_changes(session.mutes.subscribe())
                .map(|_| ())
                .boxed(),
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
                                synced.settings.listed = listed;
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
    async fn read_settings(&self, listed: Option<bool>) -> Option<Read> {
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
        let default = session.default_notify(room).await;
        let notify = session.room_modes.own(self.id()).await.unwrap_or(default);
        let mute = session.mutes.get(self.id());
        let words = NotifyWords::new(notify, mute.as_ref(), default, SystemTime::now());
        let settings = RoomSettings {
            rights: Rights::of(&levels, &session.user_id()),
            join_rule,
            join_rule_choices: JoinRule::choices(join_rule, in_space),
            notify,
            muted_until: words.muted_until,
            notify_title: words.title,
            notify_explanation: words.explanation,
            notify_choices: words.choices,
            address: room.canonical_alias(),
            listed,
            history: room.history_visibility_or_default().into(),
            permissions: Permissions::of(&levels),
            encrypted: room.encryption_state().is_encrypted(),
            version: room.version().map(|v| v.to_string()),
            spaces,
            can_add_to,
            banned,
        };
        Some(Read { settings, default })
    }

    /// The settings as synced with the changes made here laid over, and
    /// when the next of those times out.
    fn shown(&self, synced: &Read) -> (RoomSettings, Option<Instant>) {
        let mut shown = synced.settings.clone();
        let timeout = (self.session.echoes).lay_settings(self.id(), &mut shown, Instant::now());
        let mute = self.session.mutes.get(self.id());
        let now = SystemTime::now();
        NotifyWords::new(shown.notify, mute.as_ref(), synced.default, now).lay(&mut shown);
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
    /// server took it. Ends a timed mute.
    ///
    /// Changes to a room's mode go one after another: each waits until the
    /// one before is confirmed by the push rules sync brings back (or
    /// [`ECHO_TIMEOUT`]), so it starts from the server's rules, not from
    /// older ones still on their way.
    pub async fn set_notify(&self, mode: NotifyMode) -> Result<()> {
        let (session, id) = (&self.session, self.id());
        let request = async {
            let turn = session.room_modes.turn(id).await;
            session.room_modes.send(turn, id, Some(mode)).await?;
            session.mutes.set(&self.room, None).await
        };
        session.echoed(id, Setting::Notify(mode), request).await
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
        let session = &self.session;
        let graph = session.space_graph();
        let link = SettingKey::Link(self.id().to_owned());
        let mut content = match graph.child(self.id(), room) {
            Some(child) => {
                let mut content = SpaceChildEventContent::new(child.via.clone());
                content.order = child.order.clone();
                content
            }
            // Listed here, not synced yet.
            None if matches!(
                session.echoes.setting(room, link),
                Some(Setting::Link(_, Some(_)))
            ) =>
            {
                SpaceChildEventContent::new(session.via())
            }
            None => return Err(Error::RoomNotFound(room.to_string())),
        };
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

    fn at(s: &str) -> SystemTime {
        use chrono::TimeZone;
        let naive = chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M").unwrap();
        chrono::Local.from_local_datetime(&naive).unwrap().into()
    }

    fn titles(words: &NotifyWords) -> Vec<(&str, bool)> {
        (words.choices.iter())
            .map(|c| (c.title.as_str(), c.current))
            .collect()
    }

    #[test]
    fn a_rooms_notifications_in_words() {
        // Wednesday afternoon.
        let now = at("2026-10-21 14:00");
        let all = NotifyWords::new(NotifyMode::All, None, NotifyMode::All, now);
        assert_eq!(all.title, "All messages");
        assert_eq!(all.explanation, "Every message notifies you.");
        assert_eq!(all.muted_until, None);
        assert_eq!(
            titles(&all),
            [
                ("All messages", true),
                ("Mentions", false),
                ("Mute for 1 hour", false),
                ("Mute for 8 hours", false),
                ("Mute until tomorrow, 08:00", false),
                ("Mute until I unmute", false),
            ]
        );
        assert_eq!(all.choices[2].action, NotifyAction::Mute(MuteFor::Hour),);

        // Muted without an end: nothing to go back to after a time.
        let muted = NotifyWords::new(NotifyMode::Mute, None, NotifyMode::All, now);
        assert_eq!(muted.title, "Muted");
        assert_eq!(muted.explanation, "Nothing here notifies you.");
        assert_eq!(
            titles(&muted),
            [("All messages", false), ("Mentions", false), ("Mute", true)]
        );

        // Muted for a time: ended early, or a new end.
        let mute = TimedMute {
            until: at("2026-10-21 16:00"),
            before: Some(NotifyMode::Mentions),
        };
        let timed = NotifyWords::new(NotifyMode::Mute, Some(&mute), NotifyMode::All, now);
        assert_eq!(timed.title, "Muted until 16:00");
        assert_eq!(timed.explanation, "Muted until 16:00, then mentions only.");
        assert_eq!(timed.muted_until, Some(mute.until));
        assert_eq!(timed.choices.len(), 6);
        assert!(timed.choices.iter().all(|c| !c.current));

        // It followed the default: the default is named.
        let default = TimedMute {
            until: at("2026-10-22 08:00"),
            before: None,
        };
        let words = NotifyWords::new(NotifyMode::Mute, Some(&default), NotifyMode::All, now);
        assert_eq!(
            words.explanation,
            "Muted until tomorrow 08:00, then all messages again."
        );
        let words = NotifyWords::new(NotifyMode::Mute, Some(&default), NotifyMode::Mentions, now);
        assert_eq!(
            words.explanation,
            "Muted until tomorrow 08:00, then mentions only."
        );

        // A record left while the room isn't muted says nothing.
        let mentions = NotifyWords::new(NotifyMode::Mentions, Some(&mute), NotifyMode::All, now);
        assert_eq!(mentions.title, "Mentions");
        assert_eq!(mentions.muted_until, None);
        assert_eq!(
            mentions.choices[1],
            NotifyChoice {
                action: NotifyAction::Mode(NotifyMode::Mentions),
                title: "Mentions".to_owned(),
                current: true,
            }
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
