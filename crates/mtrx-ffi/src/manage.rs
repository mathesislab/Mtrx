//! A room's settings and people: what we may change, the changes, who is in
//! it and their roles, and its pinned messages.

use std::sync::Arc;

use mtrx_core as core;

use crate::{
    JoinRule, JoinRuleChoice, Membership, MtrxError, Session, Timeline, User, runtime::run,
};

/// When a room notifies (on all our devices).
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum NotifyMode {
    All,
    /// Only for mentions and keywords.
    Mentions,
    Mute,
}

/// Who may read a room's history.
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum History {
    /// Members, from when they joined.
    Joined,
    /// Members, from when they were invited.
    Invited,
    /// Members, all of it.
    Members,
    /// Anyone, without joining.
    Anyone,
}

/// Who may invite and edit a room (name, topic, picture, pins).
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum Permissions {
    Everyone,
    Moderators,
}

impl From<core::NotifyMode> for NotifyMode {
    fn from(mode: core::NotifyMode) -> Self {
        match mode {
            core::NotifyMode::All => Self::All,
            core::NotifyMode::Mentions => Self::Mentions,
            core::NotifyMode::Mute => Self::Mute,
        }
    }
}

impl From<NotifyMode> for core::NotifyMode {
    fn from(mode: NotifyMode) -> Self {
        match mode {
            NotifyMode::All => Self::All,
            NotifyMode::Mentions => Self::Mentions,
            NotifyMode::Mute => Self::Mute,
        }
    }
}

impl From<core::History> for History {
    fn from(history: core::History) -> Self {
        match history {
            core::History::Joined => Self::Joined,
            core::History::Invited => Self::Invited,
            core::History::Members => Self::Members,
            core::History::Anyone => Self::Anyone,
        }
    }
}

impl From<History> for core::History {
    fn from(history: History) -> Self {
        match history {
            History::Joined => Self::Joined,
            History::Invited => Self::Invited,
            History::Members => Self::Members,
            History::Anyone => Self::Anyone,
        }
    }
}

impl From<core::Permissions> for Permissions {
    fn from(permissions: core::Permissions) -> Self {
        match permissions {
            core::Permissions::Everyone => Self::Everyone,
            core::Permissions::Moderators => Self::Moderators,
        }
    }
}

impl From<Permissions> for core::Permissions {
    fn from(permissions: Permissions) -> Self {
        match permissions {
            Permissions::Everyone => Self::Everyone,
            Permissions::Moderators => Self::Moderators,
        }
    }
}

/// A notification mode a room's settings offer, with its words.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct NotifyChoice {
    pub mode: NotifyMode,
    /// As the menu names it: "All messages", "Mentions", "Mute".
    pub title: String,
    /// What it does: "Every message notifies you."…
    pub explanation: String,
}

/// Who may read the history, as a room's settings offer it, with its words.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct HistoryChoice {
    pub history: History,
    /// As the menu names it: "Since joining", "Since invited", "Members",
    /// "Anyone".
    pub title: String,
    /// Who may read it: "Members, from when they joined."…
    pub explanation: String,
}

/// Who may edit, as a room's settings offer it, with its words.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct PermissionsChoice {
    pub permissions: Permissions,
    /// As the menu names it: "Everyone", "Moderators".
    pub title: String,
    /// What it lets members do: "Everyone can invite, rename and change
    /// the topic."…
    pub explanation: String,
}

/// What we may do in a room, from its power levels. All false while they
/// are not known yet, and in a room we have not joined.
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Record)]
pub struct Rights {
    pub send: bool,
    /// Delete others' messages.
    pub redact_others: bool,
    pub pin: bool,
    /// Notify everyone with `@room`.
    pub mention_room: bool,
    pub rename: bool,
    pub change_topic: bool,
    pub change_picture: bool,
    pub invite: bool,
    pub kick: bool,
    pub ban: bool,
    pub change_join_rule: bool,
    pub change_history: bool,
    pub change_address: bool,
    /// Change who may do what (the permissions).
    pub change_levels: bool,
    pub encrypt: bool,
    /// For a space: list rooms in it.
    pub add_rooms: bool,
    /// For a space: set the call service of the rooms in it.
    pub set_call_service: bool,
    /// Our power level; `None` for a creator of the room (an owner, above
    /// every level).
    pub level: Option<i64>,
    /// A plain member's level.
    pub default_level: i64,
}

impl From<core::Rights> for Rights {
    fn from(r: core::Rights) -> Self {
        Self {
            send: r.send,
            redact_others: r.redact_others,
            pin: r.pin,
            mention_room: r.mention_room,
            rename: r.rename,
            change_topic: r.change_topic,
            change_picture: r.change_picture,
            invite: r.invite,
            kick: r.kick,
            ban: r.ban,
            change_join_rule: r.change_join_rule,
            change_history: r.change_history,
            change_address: r.change_address,
            change_levels: r.change_levels,
            encrypt: r.encrypt,
            add_rooms: r.add_rooms,
            set_call_service: r.set_call_service,
            level: r.level,
            default_level: r.default_level,
        }
    }
}

impl From<Rights> for core::Rights {
    fn from(r: Rights) -> Self {
        Self {
            send: r.send,
            redact_others: r.redact_others,
            pin: r.pin,
            mention_room: r.mention_room,
            rename: r.rename,
            change_topic: r.change_topic,
            change_picture: r.change_picture,
            invite: r.invite,
            kick: r.kick,
            ban: r.ban,
            change_join_rule: r.change_join_rule,
            change_history: r.change_history,
            change_address: r.change_address,
            change_levels: r.change_levels,
            encrypt: r.encrypt,
            add_rooms: r.add_rooms,
            set_call_service: r.set_call_service,
            level: r.level,
            default_level: r.default_level,
        }
    }
}

/// A space a room is in.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct SpaceLink {
    pub space_id: String,
    /// Suggested to the space's members.
    pub suggested: bool,
    /// We may change the space's rooms.
    pub can_change: bool,
}

/// Someone banned from a room.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct Banned {
    pub user: User,
    pub reason: Option<String>,
}

/// A room's settings, and which we may change; the changes made here show
/// at once.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct RoomSettings {
    /// What we may change; the same as `RoomSummary.rights`.
    pub rights: Rights,
    pub notify: NotifyMode,
    /// The modes to offer, in order, with their words.
    pub notify_choices: Vec<NotifyChoice>,
    pub join_rule: JoinRule,
    /// Who may join, in words: "Public: anyone can join"…
    pub join_rule_fact: String,
    /// The rules to offer, in order, with their words: Space members only
    /// for a room in a space, or already so.
    pub join_rule_choices: Vec<JoinRuleChoice>,
    /// `#name:server`, if it has one.
    pub address: Option<String>,
    /// Listed in our server's directory; `None` if it would not say.
    pub listed: Option<bool>,
    pub history: History,
    /// Who may read the history, in order, with their words.
    pub history_choices: Vec<HistoryChoice>,
    pub permissions: Permissions,
    /// Who may edit, in order, with their words.
    pub permissions_choices: Vec<PermissionsChoice>,
    pub encrypted: bool,
    /// The room version, as "12".
    pub version: Option<String>,
    /// The joined spaces it is in.
    pub spaces: Vec<SpaceLink>,
    /// Joined spaces it is not in whose rooms we may change.
    pub can_add_to: Vec<String>,
    pub banned: Vec<Banned>,
}

impl From<core::RoomSettings> for RoomSettings {
    fn from(s: core::RoomSettings) -> Self {
        Self {
            rights: s.rights.into(),
            notify: s.notify.into(),
            notify_choices: (core::NotifyMode::ALL.into_iter())
                .map(|mode| NotifyChoice {
                    mode: mode.into(),
                    title: mode.title().to_owned(),
                    explanation: mode.explanation().to_owned(),
                })
                .collect(),
            join_rule: s.join_rule.into(),
            join_rule_fact: s.join_rule.fact().to_owned(),
            join_rule_choices: s.join_rule_choices.into_iter().map(Into::into).collect(),
            address: s.address.map(|a| a.to_string()),
            listed: s.listed,
            history: s.history.into(),
            history_choices: (core::History::ALL.into_iter())
                .map(|history| HistoryChoice {
                    history: history.into(),
                    title: history.title().to_owned(),
                    explanation: history.explanation().to_owned(),
                })
                .collect(),
            permissions: s.permissions.into(),
            permissions_choices: (core::Permissions::ALL.into_iter())
                .map(|permissions| PermissionsChoice {
                    permissions: permissions.into(),
                    title: permissions.title().to_owned(),
                    explanation: permissions.explanation().to_owned(),
                })
                .collect(),
            encrypted: s.encrypted,
            version: s.version,
            spaces: s
                .spaces
                .into_iter()
                .map(|l| SpaceLink {
                    space_id: l.space.to_string(),
                    suggested: l.suggested,
                    can_change: l.can_change,
                })
                .collect(),
            can_add_to: s.can_add_to.iter().map(ToString::to_string).collect(),
            banned: s
                .banned
                .into_iter()
                .map(|b| Banned {
                    user: User::from(&b.user),
                    reason: b.reason,
                })
                .collect(),
        }
    }
}

/// A role, as the levels it stands for: admin 100, moderator 50, member
/// the room's default.
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum Role {
    Admin,
    Moderator,
    Member,
}

impl From<core::Role> for Role {
    fn from(role: core::Role) -> Self {
        match role {
            core::Role::Admin => Self::Admin,
            core::Role::Moderator => Self::Moderator,
            core::Role::Member => Self::Member,
        }
    }
}

/// What we may do to a member.
#[derive(Clone, Debug, Default, PartialEq, Eq, uniffi::Record)]
pub struct Manage {
    pub remove: bool,
    pub ban: bool,
    /// The highest level we may give them, if we may change theirs: ours,
    /// or below it for ourselves (stepping down).
    pub up_to: Option<i64>,
    /// A plain member's level in the room.
    pub default_level: i64,
    /// Ourselves, and nobody else joined has our level or more: stepping
    /// down leaves nobody able to do all we can.
    pub alone: bool,
}

impl From<Manage> for core::Manage {
    fn from(m: Manage) -> Self {
        Self {
            remove: m.remove,
            ban: m.ban,
            up_to: m.up_to,
            default_level: m.default_level,
            alone: m.alone,
        }
    }
}

/// A level in a member's menu.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct LevelChoice {
    pub title: String,
    pub level: i64,
    /// Theirs: ticked.
    pub current: bool,
    /// What to ask before giving it, if it can't be undone.
    pub confirmation: Option<LevelConfirmation>,
}

/// A question before a change of level that can't be undone.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct LevelConfirmation {
    pub title: String,
    pub message: String,
    pub button: String,
}

impl From<core::Confirmation> for LevelConfirmation {
    fn from(c: core::Confirmation) -> Self {
        Self {
            title: c.title,
            message: c.message,
            button: c.button,
        }
    }
}

/// Someone in a room.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct Member {
    pub user: User,
    pub role: Role,
    /// Their power level; `None` for a creator of the room (an owner, above
    /// every level).
    pub level: Option<i64>,
    /// Joined, or invited and not joined yet.
    pub membership: Membership,
    /// Ourselves.
    pub own: bool,
    pub manage: Manage,
    /// "Owner", "Admin", "Mod", "Level 20", or nothing for a plain member.
    pub label: Option<String>,
    /// The levels we may give them, theirs ticked; empty if we may not
    /// change theirs.
    pub choices: Vec<LevelChoice>,
    /// We may do something to them: change their level, remove or ban them.
    pub manageable: bool,
}

impl From<core::Member> for Member {
    fn from(m: core::Member) -> Self {
        Self {
            label: m.label(),
            choices: m
                .level_choices()
                .into_iter()
                .map(|c| LevelChoice {
                    title: c.title,
                    level: c.level,
                    current: c.current,
                    confirmation: c.confirmation.map(Into::into),
                })
                .collect(),
            manageable: m.manageable(),
            user: User::from(&m.user),
            role: m.role.into(),
            level: m.level,
            membership: m.membership.into(),
            own: m.own,
            manage: Manage {
                remove: m.manage.remove,
                ban: m.manage.ban,
                up_to: m.manage.up_to,
                default_level: m.manage.default_level,
                alone: m.manage.alone,
            },
        }
    }
}

/// What to ask before giving `member` a `level` typed in, or nothing to
/// just give it: any change to our own, or a level as high as ours.
#[uniffi::export]
pub fn level_confirmation(member: Member, level: i64) -> Option<LevelConfirmation> {
    let name = &member.user.display_name;
    core::level_confirmation(name, member.own, &member.manage.into(), level).map(Into::into)
}

/// A level typed for a custom level: a number, at most `up_to`.
#[uniffi::export]
pub fn typed_level(text: String, up_to: i64) -> Option<i64> {
    core::typed_level(&text, up_to)
}

/// The level `role` stands for, `default_level` for a member.
#[uniffi::export]
pub fn role_level(role: Role, default_level: i64) -> i64 {
    match role {
        Role::Admin => core::Role::Admin,
        Role::Moderator => core::Role::Moderator,
        Role::Member => core::Role::Member,
    }
    .level(default_level)
}

#[uniffi::export]
impl Session {
    /// Keep the Notes to self at the top, on all the user's devices.
    pub async fn set_notes_at_top(&self, on: bool) -> Result<(), MtrxError> {
        let session = self.core();
        run(async move {
            Ok(session
                .change_room_categories(|c| c.notes_at_top = Some(on))
                .await?)
        })
        .await
    }

    /// The room's pinned messages, as a timeline of their own.
    pub async fn pinned_timeline(&self, room_id: String) -> Result<Arc<Timeline>, MtrxError> {
        let room = crate::error::room_id(&room_id)?;
        let session = self.core();
        run(async move { Ok(Timeline::new(session.pinned_timeline(&room).await?)) }).await
    }
}
