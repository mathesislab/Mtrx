//! A room's settings and people: what we may change, the changes, who is in
//! it and their roles, and its pinned messages.

use std::sync::Arc;

use mtrx_core::{self as core, OwnedEventId};

use crate::{MtrxError, Session, Timeline, runtime::run};

/// When a room notifies (on all our devices).
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum NotifyMode {
    All,
    /// Only for mentions and keywords.
    Mentions,
    Mute,
}

/// Who may join a room.
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum Access {
    /// Only invited people.
    Invite,
    /// Members of the spaces it is in.
    SpaceMembers,
    /// Anyone may ask; members let them in.
    Knock,
    Public,
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
    pub user_id: String,
    pub name: Option<String>,
    pub reason: Option<String>,
}

/// A room's settings, and which we may change.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct RoomSettings {
    pub can_rename: bool,
    pub can_change_topic: bool,
    pub can_change_picture: bool,
    pub can_invite: bool,
    pub notify: NotifyMode,
    pub access: Access,
    pub can_change_access: bool,
    /// For a space: we may list rooms in it.
    pub can_add_rooms: bool,
    /// `#name:server`, if it has one.
    pub address: Option<String>,
    pub can_change_address: bool,
    /// Listed in our server's directory; `None` if it would not say.
    pub listed: Option<bool>,
    pub history: History,
    pub can_change_history: bool,
    pub permissions: Permissions,
    pub can_change_permissions: bool,
    pub encrypted: bool,
    pub can_encrypt: bool,
    /// The room version, as "12".
    pub version: Option<String>,
    /// The joined spaces it is in.
    pub spaces: Vec<SpaceLink>,
    /// Joined spaces it is not in whose rooms we may change.
    pub can_add_to: Vec<String>,
    pub banned: Vec<Banned>,
    pub can_unban: bool,
}

impl From<core::RoomSettings> for RoomSettings {
    fn from(s: core::RoomSettings) -> Self {
        Self {
            can_rename: s.can_rename,
            can_change_topic: s.can_change_topic,
            can_change_picture: s.can_change_picture,
            can_invite: s.can_invite,
            notify: match s.notify {
                core::NotifyMode::All => NotifyMode::All,
                core::NotifyMode::Mentions => NotifyMode::Mentions,
                core::NotifyMode::Mute => NotifyMode::Mute,
            },
            access: match s.access {
                core::Access::Invite => Access::Invite,
                core::Access::SpaceMembers => Access::SpaceMembers,
                core::Access::Knock => Access::Knock,
                core::Access::Public => Access::Public,
            },
            can_change_access: s.can_change_access,
            can_add_rooms: s.can_add_rooms,
            address: s.address.map(|a| a.to_string()),
            can_change_address: s.can_change_address,
            listed: s.listed,
            history: match s.history {
                core::History::Joined => History::Joined,
                core::History::Invited => History::Invited,
                core::History::Members => History::Members,
                core::History::Anyone => History::Anyone,
            },
            can_change_history: s.can_change_history,
            permissions: match s.permissions {
                core::Permissions::Everyone => Permissions::Everyone,
                core::Permissions::Moderators => Permissions::Moderators,
            },
            can_change_permissions: s.can_change_permissions,
            encrypted: s.encrypted,
            can_encrypt: s.can_encrypt,
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
                    user_id: b.id.to_string(),
                    name: b.name,
                    reason: b.reason,
                })
                .collect(),
            can_unban: s.can_unban,
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
    pub user_id: String,
    pub name: Option<String>,
    /// `mxc://` URI, for `Session.avatar`.
    pub avatar_url: Option<String>,
    pub role: Role,
    /// Their power level; `None` for a creator of the room (an owner, above
    /// every level).
    pub level: Option<i64>,
    /// Invited, not joined yet.
    pub invited: bool,
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
            user_id: m.id.to_string(),
            name: m.name,
            avatar_url: m.avatar.map(|a| a.to_string()),
            role: m.role.into(),
            level: m.level,
            invited: m.invited,
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
    let name = core::shown_name(member.name.as_deref(), &member.user_id);
    core::level_confirmation(&name, member.own, &member.manage.into(), level).map(Into::into)
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
    /// A room's settings, and which we may change.
    pub async fn room_settings(&self, room_id: String) -> Result<RoomSettings, MtrxError> {
        let room = crate::error::room_id(&room_id)?;
        let session = self.core();
        run(async move { Ok(session.room_settings(&room).await?.into()) }).await
    }

    pub async fn set_notify_mode(
        &self,
        room_id: String,
        mode: NotifyMode,
    ) -> Result<(), MtrxError> {
        let room = crate::error::room_id(&room_id)?;
        let mode = match mode {
            NotifyMode::All => core::NotifyMode::All,
            NotifyMode::Mentions => core::NotifyMode::Mentions,
            NotifyMode::Mute => core::NotifyMode::Mute,
        };
        let session = self.core();
        run(async move { Ok(session.set_notify_mode(&room, mode).await?) }).await
    }

    pub async fn set_room_access(&self, room_id: String, access: Access) -> Result<(), MtrxError> {
        let room = crate::error::room_id(&room_id)?;
        let access = match access {
            Access::Invite => core::Access::Invite,
            Access::SpaceMembers => core::Access::SpaceMembers,
            Access::Knock => core::Access::Knock,
            Access::Public => core::Access::Public,
        };
        let session = self.core();
        run(async move { Ok(session.set_room_access(&room, access).await?) }).await
    }

    pub async fn set_room_name(&self, room_id: String, name: String) -> Result<(), MtrxError> {
        let room = crate::error::room_id(&room_id)?;
        let session = self.core();
        run(async move { Ok(session.set_room_name(&room, &name).await?) }).await
    }

    /// The topic, as `composeTopic` makes it from text and links.
    pub async fn set_room_topic(&self, room_id: String, topic: String) -> Result<(), MtrxError> {
        let room = crate::error::room_id(&room_id)?;
        let session = self.core();
        run(async move { Ok(session.set_room_topic(&room, &topic).await?) }).await
    }

    /// The image file at `path` as the room's picture.
    pub async fn set_room_picture(&self, room_id: String, path: String) -> Result<(), MtrxError> {
        let room = crate::error::room_id(&room_id)?;
        let session = self.core();
        run(async move { Ok(session.set_room_picture(&room, path.as_ref()).await?) }).await
    }

    pub async fn remove_room_picture(&self, room_id: String) -> Result<(), MtrxError> {
        let room = crate::error::room_id(&room_id)?;
        let session = self.core();
        run(async move { Ok(session.remove_room_picture(&room).await?) }).await
    }

    /// The address `#name:ourserver`, or none.
    pub async fn set_address(
        &self,
        room_id: String,
        name: Option<String>,
    ) -> Result<(), MtrxError> {
        let room = crate::error::room_id(&room_id)?;
        let session = self.core();
        run(async move { Ok(session.set_address(&room, name.as_deref()).await?) }).await
    }

    /// Listed in our server's directory, or not.
    pub async fn set_listed(&self, room_id: String, listed: bool) -> Result<(), MtrxError> {
        let room = crate::error::room_id(&room_id)?;
        let session = self.core();
        run(async move { Ok(session.set_listed(&room, listed).await?) }).await
    }

    pub async fn set_history(&self, room_id: String, history: History) -> Result<(), MtrxError> {
        let room = crate::error::room_id(&room_id)?;
        let history = match history {
            History::Joined => core::History::Joined,
            History::Invited => core::History::Invited,
            History::Members => core::History::Members,
            History::Anyone => core::History::Anyone,
        };
        let session = self.core();
        run(async move { Ok(session.set_history(&room, history).await?) }).await
    }

    pub async fn set_permissions(
        &self,
        room_id: String,
        permissions: Permissions,
    ) -> Result<(), MtrxError> {
        let room = crate::error::room_id(&room_id)?;
        let permissions = match permissions {
            Permissions::Everyone => core::Permissions::Everyone,
            Permissions::Moderators => core::Permissions::Moderators,
        };
        let session = self.core();
        run(async move { Ok(session.set_permissions(&room, permissions).await?) }).await
    }

    /// Encrypt the room's messages from now on; not to be undone.
    pub async fn encrypt_room(&self, room_id: String) -> Result<(), MtrxError> {
        let room = crate::error::room_id(&room_id)?;
        let session = self.core();
        run(async move { Ok(session.encrypt_room(&room).await?) }).await
    }

    /// Suggest a room to a space's members, or not.
    pub async fn set_suggested(
        &self,
        space_id: String,
        room_id: String,
        suggested: bool,
    ) -> Result<(), MtrxError> {
        let (space, room) = (
            crate::error::room_id(&space_id)?,
            crate::error::room_id(&room_id)?,
        );
        let session = self.core();
        run(async move { Ok(session.set_suggested(&space, &room, suggested).await?) }).await
    }

    /// List a room in a space.
    pub async fn add_to_space(&self, space_id: String, room_id: String) -> Result<(), MtrxError> {
        let (space, room) = (
            crate::error::room_id(&space_id)?,
            crate::error::room_id(&room_id)?,
        );
        let session = self.core();
        run(async move { Ok(session.add_to_space(&space, &room).await?) }).await
    }

    /// Take a room out of a space.
    pub async fn remove_from_space(
        &self,
        space_id: String,
        room_id: String,
    ) -> Result<(), MtrxError> {
        let (space, room) = (
            crate::error::room_id(&space_id)?,
            crate::error::room_id(&room_id)?,
        );
        let session = self.core();
        run(async move { Ok(session.remove_from_space(&space, &room).await?) }).await
    }

    /// Joined and invited members, admins first, then by name.
    pub async fn members(&self, room_id: String) -> Result<Vec<Member>, MtrxError> {
        let room = crate::error::room_id(&room_id)?;
        let session = self.core();
        run(async move {
            Ok(session
                .members(&room)
                .await?
                .into_iter()
                .map(Member::from)
                .collect())
        })
        .await
    }

    /// Give someone, or ourselves, a power level (`roleLevel` for a role).
    pub async fn set_level(
        &self,
        room_id: String,
        user_id: String,
        level: i64,
    ) -> Result<(), MtrxError> {
        let (room, user) = (
            crate::error::room_id(&room_id)?,
            crate::error::user_id(&user_id)?,
        );
        let session = self.core();
        run(async move { Ok(session.set_level(&room, &user, level).await?) }).await
    }

    /// Remove someone from a room, or also keep them out (`ban`); they see
    /// `reason`.
    pub async fn remove_member(
        &self,
        room_id: String,
        user_id: String,
        reason: Option<String>,
        ban: bool,
    ) -> Result<(), MtrxError> {
        let (room, user) = (
            crate::error::room_id(&room_id)?,
            crate::error::user_id(&user_id)?,
        );
        let session = self.core();
        run(async move {
            Ok(session
                .remove_member(&room, &user, reason.as_deref(), ban)
                .await?)
        })
        .await
    }

    pub async fn unban(&self, room_id: String, user_id: String) -> Result<(), MtrxError> {
        let (room, user) = (
            crate::error::room_id(&room_id)?,
            crate::error::user_id(&user_id)?,
        );
        let session = self.core();
        run(async move { Ok(session.unban(&room, &user).await?) }).await
    }

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

    /// Pin a message (its `TimelineEvent.eventId`), or unpin it.
    pub async fn set_pinned(
        &self,
        room_id: String,
        event_id: String,
        pin: bool,
    ) -> Result<(), MtrxError> {
        let room = crate::error::room_id(&room_id)?;
        let event = OwnedEventId::try_from(event_id.as_str()).map_err(MtrxError::invalid)?;
        let session = self.core();
        run(async move { Ok(session.set_pinned(&room, &event, pin).await?) }).await
    }

    /// Whether we may pin and unpin messages in a room.
    pub async fn can_pin(&self, room_id: String) -> Result<bool, MtrxError> {
        let room = crate::error::room_id(&room_id)?;
        let session = self.core();
        run(async move { Ok(session.can_pin(&room).await?) }).await
    }

    /// The room's pinned messages, as a timeline of their own.
    pub async fn pinned_timeline(&self, room_id: String) -> Result<Arc<Timeline>, MtrxError> {
        let room = crate::error::room_id(&room_id)?;
        let session = self.core();
        run(async move { Ok(Timeline::new(session.pinned_timeline(&room).await?)) }).await
    }
}
