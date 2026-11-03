//! Who may do what in a room. Mtrx trusts members by default: anyone in a
//! room may invite, rename it, change its topic and picture, pin messages,
//! set the call link and add rooms to a space. Removing and banning people,
//! deleting others' messages, and changing permissions or who can join stay
//! with moderators and admins.
//!
//! Permissions are room state (`m.room.power_levels`) enforced by the
//! server, so they hold for members using other clients too.

use matrix_sdk::{
    Room,
    deserialized_responses::SyncOrStrippedState,
    ruma::{
        Int, OwnedRoomId,
        events::{
            EmptyStateKey, StateEventType, SyncStateEvent, TimelineEventType,
            room::power_levels::{RoomPowerLevels, RoomPowerLevelsEventContent},
        },
        int,
        serde::Raw,
    },
};
use serde_json::json;

use crate::{Error, Result, RoomHandle, synced};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Permissions {
    /// Every member may invite and edit the room (Mtrx's default).
    #[default]
    Everyone,
    /// Only moderators may (the usual Matrix default).
    Moderators,
}

/// State only admins change: permissions and who can get in.
const ADMIN_STATE: [&str; 7] = [
    "m.room.power_levels",
    "m.room.server_acl",
    "m.room.tombstone",
    "m.room.encryption",
    "m.room.join_rules",
    "m.room.history_visibility",
    "m.room.guest_access",
];

/// State every member changes in an open room.
const MEMBER_STATE: [&str; 7] = [
    "m.room.name",
    "m.room.topic",
    "m.room.avatar",
    "m.room.pinned_events",
    "m.space.child",
    "m.space.parent",
    "org.mtrx.call_service",
];

/// Power levels for a new room open to its members, for `createRoom`'s
/// `power_level_content_override`. The server merges it into its defaults,
/// replacing the `events` map as a whole.
pub(crate) fn open_power_levels() -> serde_json::Value {
    let mut events: serde_json::Map<_, _> = ADMIN_STATE
        .iter()
        .map(|t| (t.to_string(), json!(100)))
        .collect();
    events.insert("m.room.canonical_alias".into(), json!(50));
    json!({
        "invite": 0,
        "state_default": 0,
        "events_default": 0,
        "events": events,
    })
}

/// Open existing power levels to members, keeping admin-only state at its
/// level (at least moderator) and everything about people as it is.
fn open_up(content: &mut RoomPowerLevelsEventContent) {
    let moderator = int!(50);
    for event_type in ADMIN_STATE {
        let event_type = TimelineEventType::from(event_type);
        let level = content
            .events
            .get(&event_type)
            .copied()
            .unwrap_or(content.state_default)
            .max(moderator);
        content.events.insert(event_type, level);
    }
    for event_type in MEMBER_STATE {
        content.events.remove(&TimelineEventType::from(event_type));
    }
    content.state_default = Int::from(0);
    content.events_default = Int::from(0);
    content.invite = Int::from(0);
}

/// Leave inviting and editing to moderators, as Matrix does by default;
/// messages stay open to all.
fn close_down(content: &mut RoomPowerLevelsEventContent) {
    let moderator = int!(50);
    for event_type in MEMBER_STATE {
        content
            .events
            .insert(TimelineEventType::from(event_type), moderator);
    }
    content.state_default = moderator;
    content.invite = moderator;
}

/// Whether a member without special rights may invite and edit.
pub(crate) fn everyone_can_edit(levels: &RoomPowerLevels) -> bool {
    let member = levels.users_default;
    levels.invite <= member
        && MEMBER_STATE
            .iter()
            .all(|t| levels.for_state(StateEventType::from(*t)) <= member)
}

/// Result of [`RoomHandle::let_everyone_edit_all`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SpaceUpdate {
    /// The space and rooms now open to everyone.
    pub changed: Vec<OwnedRoomId>,
    /// Already open.
    pub unchanged: Vec<OwnedRoomId>,
    /// We may not change their permissions.
    pub not_allowed: Vec<OwnedRoomId>,
}

impl RoomHandle {
    pub async fn permissions(&self) -> Result<Permissions> {
        Ok(if everyone_can_edit(&power_levels(&self.room).await?) {
            Permissions::Everyone
        } else {
            Permissions::Moderators
        })
    }

    /// Whether we may change who may do what in the room.
    pub async fn can_change_permissions(&self) -> Result<bool> {
        let levels = power_levels(&self.room).await?;
        Ok(levels.user_can_send_state(&self.session.user_id(), StateEventType::RoomPowerLevels))
    }

    /// Whether we may notify everyone in the room with `@room`.
    pub async fn can_mention_room(&self) -> Result<bool> {
        let levels = power_levels(&self.room).await?;
        Ok(levels.user_can_trigger_room_notification(&self.session.user_id()))
    }

    /// Let every member invite and edit the room (see the module docs), or
    /// only moderators.
    pub async fn set_permissions(&self, permissions: Permissions) -> Result<()> {
        let mut content = power_levels_content(&self.room).await?;
        match permissions {
            Permissions::Everyone => open_up(&mut content),
            Permissions::Moderators => close_down(&mut content),
        }
        synced::send_state(&self.room, &EmptyStateKey, content).await
    }

    /// Let everyone edit this space and every room and space in it that we
    /// have joined.
    pub async fn let_everyone_edit_all(&self) -> Result<SpaceUpdate> {
        let mut update = SpaceUpdate::default();
        let below = self.session.space_graph().descendants(self.id());
        let mut rooms = vec![self.id().to_owned()];
        rooms.extend(
            below
                .into_iter()
                .filter(|id| id.as_str() != self.id().as_str()),
        );
        for id in rooms {
            let Ok(room) = self.session.room(&id) else {
                continue;
            };
            if room.permissions().await? == Permissions::Everyone {
                update.unchanged.push(id);
            } else if room.can_change_permissions().await? {
                room.set_permissions(Permissions::Everyone).await?;
                update.changed.push(id);
            } else {
                update.not_allowed.push(id);
            }
        }
        Ok(update)
    }
}

pub(crate) async fn power_levels(room: &Room) -> Result<RoomPowerLevels> {
    // Synced after the room itself, e.g. just after creating it.
    if room
        .get_state_event_static::<RoomPowerLevelsEventContent>()
        .await?
        .is_none()
    {
        return Err(Error::Permissions(
            "the room's permissions are not known yet".into(),
        ));
    }
    Ok(room.power_levels().await.map_err(matrix_sdk::Error::from)?)
}

async fn power_levels_content(room: &Room) -> Result<RoomPowerLevelsEventContent> {
    let raw = room
        .get_state_event_static::<RoomPowerLevelsEventContent>()
        .await?
        .ok_or_else(|| Error::Permissions("the room's permissions are not known yet".into()))?;
    match raw.deserialize()? {
        SyncOrStrippedState::Sync(SyncStateEvent::Original(event)) => Ok(event.content),
        _ => Err(Error::Permissions(
            "the room's permissions are not readable".into(),
        )),
    }
}

/// [`open_power_levels`] as `createRoom`'s override field.
pub(crate) fn open_power_levels_raw<T>() -> Result<Raw<T>> {
    Ok(Raw::from_json(serde_json::value::to_raw_value(
        &open_power_levels(),
    )?))
}

#[cfg(test)]
mod tests {
    use matrix_sdk::ruma::{
        events::room::power_levels::RoomPowerLevelsSource, room_version_rules::AuthorizationRules,
        user_id,
    };

    use super::*;

    fn levels(content: RoomPowerLevelsEventContent) -> RoomPowerLevels {
        RoomPowerLevels::new(
            RoomPowerLevelsSource::Original(content),
            &AuthorizationRules::V1,
            [user_id!("@creator:example.org").to_owned()],
        )
    }

    #[test]
    fn opening_up_keeps_people_and_permissions_with_moderators() {
        let mut content = RoomPowerLevelsEventContent::new(&AuthorizationRules::V1);
        content.invite = int!(50);
        content.events.insert("m.room.name".into(), int!(50));
        assert!(!everyone_can_edit(&levels(content.clone())));

        open_up(&mut content);
        let opened = levels(content.clone());
        let mut closed = content.clone();
        close_down(&mut closed);
        assert!(!everyone_can_edit(&levels(closed)));
        assert!(everyone_can_edit(&opened));
        assert_eq!(opened.invite, int!(0));
        assert_eq!(opened.kick, int!(50));
        assert_eq!(opened.ban, int!(50));
        assert_eq!(opened.redact, int!(50));
        for event_type in ADMIN_STATE {
            assert!(
                opened.for_state(StateEventType::from(event_type)) >= int!(50),
                "{event_type}"
            );
        }
    }

    #[test]
    fn new_room_levels_match_opening_up() {
        let content: RoomPowerLevelsEventContent =
            serde_json::from_value(open_power_levels()).unwrap();
        let levels = levels(content);
        assert!(everyone_can_edit(&levels));
        assert_eq!(levels.for_state(StateEventType::RoomPowerLevels), int!(100));
        assert_eq!(levels.for_state(StateEventType::RoomJoinRules), int!(100));
    }
}
