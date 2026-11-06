//! A room or space we are in or invited to: its settings and people, and
//! the changes to them. Its timeline, pinned messages and draft are
//! `Session.timeline`, `Session.pinnedTimeline` and `Session.draftKeeper`.

use std::sync::Arc;

use mtrx_core::{self as core, OwnedEventId};

use crate::{
    History, JoinRule, Member, MtrxError, NotifyMode, Permissions, RoomSettings, Session,
    runtime::{InRuntime, run},
};

/// A room or space we know (`Session.room`).
#[derive(uniffi::Object)]
pub struct Room {
    inner: InRuntime<core::RoomHandle>,
}

impl Room {
    fn core(&self) -> core::RoomHandle {
        (*self.inner).clone()
    }
}

#[uniffi::export]
impl Session {
    /// A room or space we are in or invited to (`RoomSummary.id`).
    pub fn room(&self, room_id: String) -> Result<Arc<Room>, MtrxError> {
        let room = self.core().room(&crate::error::room_id(&room_id)?)?;
        Ok(Arc::new(Room {
            inner: InRuntime::new(room),
        }))
    }
}

#[uniffi::export]
impl Room {
    /// The room's settings, and which we may change.
    pub async fn settings(&self) -> Result<RoomSettings, MtrxError> {
        let room = self.core();
        run(async move { Ok(room.settings().await?.into()) }).await
    }

    /// Joined and invited members, admins first, then by name.
    pub async fn members(&self) -> Result<Vec<Member>, MtrxError> {
        let room = self.core();
        run(async move {
            let members = room.members().await?;
            Ok(members.into_iter().map(Member::from).collect())
        })
        .await
    }

    pub async fn set_name(&self, name: String) -> Result<(), MtrxError> {
        let room = self.core();
        run(async move { Ok(room.set_name(&name).await?) }).await
    }

    /// The topic, as `composeTopic` makes it from text and links.
    pub async fn set_topic(&self, topic: String) -> Result<(), MtrxError> {
        let room = self.core();
        run(async move { Ok(room.set_topic(&topic).await?) }).await
    }

    /// The image file at `path` as the room's picture.
    pub async fn set_picture(&self, path: String) -> Result<(), MtrxError> {
        let room = self.core();
        run(async move { Ok(room.set_picture(path.as_ref()).await?) }).await
    }

    pub async fn remove_picture(&self) -> Result<(), MtrxError> {
        let room = self.core();
        run(async move { Ok(room.remove_picture().await?) }).await
    }

    /// Who may join.
    pub async fn set_join_rule(&self, rule: JoinRule) -> Result<(), MtrxError> {
        let rule = core::JoinRule::from(rule);
        let room = self.core();
        run(async move { Ok(room.set_join_rule(rule).await?) }).await
    }

    pub async fn set_history(&self, history: History) -> Result<(), MtrxError> {
        let history = match history {
            History::Joined => core::History::Joined,
            History::Invited => core::History::Invited,
            History::Members => core::History::Members,
            History::Anyone => core::History::Anyone,
        };
        let room = self.core();
        run(async move { Ok(room.set_history(history).await?) }).await
    }

    /// The address `#name:ourserver`, or none.
    pub async fn set_address(&self, name: Option<String>) -> Result<(), MtrxError> {
        let room = self.core();
        run(async move { Ok(room.set_address(name.as_deref()).await?) }).await
    }

    /// Listed in our server's directory, or not.
    pub async fn set_listed(&self, listed: bool) -> Result<(), MtrxError> {
        let room = self.core();
        run(async move { Ok(room.set_listed(listed).await?) }).await
    }

    pub async fn set_permissions(&self, permissions: Permissions) -> Result<(), MtrxError> {
        let permissions = match permissions {
            Permissions::Everyone => core::Permissions::Everyone,
            Permissions::Moderators => core::Permissions::Moderators,
        };
        let room = self.core();
        run(async move { Ok(room.set_permissions(permissions).await?) }).await
    }

    /// Encrypt the room's messages from now on; not to be undone.
    pub async fn encrypt(&self) -> Result<(), MtrxError> {
        let room = self.core();
        run(async move { Ok(room.encrypt().await?) }).await
    }

    pub async fn set_notify(&self, mode: NotifyMode) -> Result<(), MtrxError> {
        let mode = match mode {
            NotifyMode::All => core::NotifyMode::All,
            NotifyMode::Mentions => core::NotifyMode::Mentions,
            NotifyMode::Mute => core::NotifyMode::Mute,
        };
        let room = self.core();
        run(async move { Ok(room.set_notify(mode).await?) }).await
    }

    /// Pin a message (its `TimelineEvent.eventId`), or unpin it.
    pub async fn set_pinned(&self, event_id: String, pin: bool) -> Result<(), MtrxError> {
        let event = OwnedEventId::try_from(event_id.as_str()).map_err(MtrxError::invalid)?;
        let room = self.core();
        run(async move { Ok(room.set_pinned(&event, pin).await?) }).await
    }

    /// Invite `user_id`.
    pub async fn invite(&self, user_id: String) -> Result<(), MtrxError> {
        let user = crate::error::user_id(&user_id)?;
        let room = self.core();
        run(async move { Ok(room.invite(&user).await?) }).await
    }

    /// Remove someone, or also keep them out (`ban`); they see `reason`.
    pub async fn remove(
        &self,
        user_id: String,
        ban: bool,
        reason: Option<String>,
    ) -> Result<(), MtrxError> {
        let user = crate::error::user_id(&user_id)?;
        let room = self.core();
        run(async move { Ok(room.remove(&user, ban, reason.as_deref()).await?) }).await
    }

    pub async fn unban(&self, user_id: String) -> Result<(), MtrxError> {
        let user = crate::error::user_id(&user_id)?;
        let room = self.core();
        run(async move { Ok(room.unban(&user).await?) }).await
    }

    /// Give someone, or ourselves, a power level (`roleLevel` for a role).
    pub async fn set_level(&self, user_id: String, level: i64) -> Result<(), MtrxError> {
        let user = crate::error::user_id(&user_id)?;
        let room = self.core();
        run(async move { Ok(room.set_level(&user, level).await?) }).await
    }

    /// Leave the room or space, or decline an invite to it.
    pub async fn leave(&self) -> Result<(), MtrxError> {
        let room = self.core();
        run(async move { Ok(room.leave().await?) }).await
    }

    /// List `room_id` in this space.
    pub async fn add_child(&self, room_id: String) -> Result<(), MtrxError> {
        let child = crate::error::room_id(&room_id)?;
        let room = self.core();
        run(async move { Ok(room.add_child(&child).await?) }).await
    }

    /// Take `room_id` out of this space.
    pub async fn remove_child(&self, room_id: String) -> Result<(), MtrxError> {
        let child = crate::error::room_id(&room_id)?;
        let room = self.core();
        run(async move { Ok(room.remove_child(&child).await?) }).await
    }

    /// Suggest `room_id` to this space's members, or not.
    pub async fn set_suggested(&self, room_id: String, suggested: bool) -> Result<(), MtrxError> {
        let child = crate::error::room_id(&room_id)?;
        let room = self.core();
        run(async move { Ok(room.set_suggested(&child, suggested).await?) }).await
    }
}
