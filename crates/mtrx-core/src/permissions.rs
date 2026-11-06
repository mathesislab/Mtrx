//! Who may do what in a room. Mtrx trusts members by default: anyone in a
//! room may invite, rename it, change its topic and picture, pin messages,
//! set the call link and add rooms to a space. Removing and banning people,
//! deleting others' messages, and changing permissions or who can join stay
//! with moderators and admins.
//!
//! Permissions are room state (`m.room.power_levels`) enforced by the
//! server, so they hold for members using other clients too.

use matrix_sdk::{
    Room, RoomState,
    deserialized_responses::{RawSyncOrStrippedState, SyncOrStrippedState},
    ruma::{
        Int, OwnedEventId, OwnedRoomId, UserId,
        events::{
            EmptyStateKey, MessageLikeEventType, StateEventType, SyncStateEvent, TimelineEventType,
            room::power_levels::{RoomPowerLevels, RoomPowerLevelsEventContent, UserPowerLevel},
        },
        int,
        serde::Raw,
    },
};
use serde_json::json;

use crate::{Error, Result, RoomHandle, synced};

/// What we may do in a room, from its power levels. All false while they
/// are not known yet, and in a room we have not joined.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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
    /// Change who may do what (the power levels themselves).
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

/// Not known yet: nothing allowed, at a plain member's level.
impl Default for Rights {
    fn default() -> Self {
        Self {
            send: false,
            redact_others: false,
            pin: false,
            mention_room: false,
            rename: false,
            change_topic: false,
            change_picture: false,
            invite: false,
            kick: false,
            ban: false,
            change_join_rule: false,
            change_history: false,
            change_address: false,
            change_levels: false,
            encrypt: false,
            add_rooms: false,
            set_call_service: false,
            level: Some(0),
            default_level: 0,
        }
    }
}

impl Rights {
    /// What `own` may do in a room with these `levels`.
    pub fn of(levels: &RoomPowerLevels, own: &UserId) -> Self {
        let state = |event_type: StateEventType| levels.user_can_send_state(own, event_type);
        Self {
            send: levels.user_can_send_message(own, MessageLikeEventType::RoomMessage),
            redact_others: levels.user_can_redact_event_of_other(own),
            pin: state(StateEventType::RoomPinnedEvents),
            mention_room: levels.user_can_trigger_room_notification(own),
            rename: state(StateEventType::RoomName),
            change_topic: state(StateEventType::RoomTopic),
            change_picture: state(StateEventType::RoomAvatar),
            invite: levels.user_can_invite(own),
            kick: levels.user_can_kick(own),
            ban: levels.user_can_ban(own),
            change_join_rule: state(StateEventType::RoomJoinRules),
            change_history: state(StateEventType::RoomHistoryVisibility),
            change_address: state(StateEventType::RoomCanonicalAlias),
            change_levels: state(StateEventType::RoomPowerLevels),
            encrypt: state(StateEventType::RoomEncryption),
            add_rooms: state(StateEventType::SpaceChild),
            set_call_service: state(StateEventType::from(CALL_SERVICE)),
            level: match levels.for_user(own) {
                UserPowerLevel::Int(level) => Some(level.into()),
                _ => None,
            },
            default_level: levels.users_default.into(),
        }
    }
}

/// `room`'s power levels: the one place they are read. `None` while they
/// are not known yet (synced after the room itself, e.g. just after
/// creating it), and for a room we have not joined.
pub(crate) async fn levels(room: &Room) -> Option<RoomPowerLevels> {
    if room.state() != RoomState::Joined {
        return None;
    }
    room.power_levels().await.ok()
}

/// What we may do in `room`.
pub(crate) async fn rights(room: &Room) -> Rights {
    match levels(room).await {
        Some(levels) => Rights::of(&levels, room.own_user_id()),
        None => Rights::default(),
    }
}

/// The ID of `room`'s power levels event, which changes with them; `None`
/// as for [`levels`].
pub(crate) async fn levels_event(room: &Room) -> Option<OwnedEventId> {
    if room.state() != RoomState::Joined {
        return None;
    }
    let raw = room
        .get_state_event_static::<RoomPowerLevelsEventContent>()
        .await
        .ok()??;
    match raw {
        RawSyncOrStrippedState::Sync(raw) => raw.get_field("event_id").ok()?,
        RawSyncOrStrippedState::Stripped(_) => None,
    }
}

/// `room`'s power levels, or that they are not known yet.
pub(crate) async fn known_levels(room: &Room) -> Result<RoomPowerLevels> {
    levels(room)
        .await
        .ok_or_else(|| Error::Permissions("the room's permissions are not known yet".into()))
}

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
    CALL_SERVICE,
];

/// A space's call service, as `calls.rs` sends it.
const CALL_SERVICE: &str = "org.mtrx.call_service";

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
fn everyone_can_edit(levels: &RoomPowerLevels) -> bool {
    let member = levels.users_default;
    levels.invite <= member
        && MEMBER_STATE
            .iter()
            .all(|t| levels.for_state(StateEventType::from(*t)) <= member)
}

impl Permissions {
    /// The preset `levels` match.
    pub(crate) fn of(levels: &RoomPowerLevels) -> Self {
        if everyone_can_edit(levels) {
            Self::Everyone
        } else {
            Self::Moderators
        }
    }
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
        Ok(Permissions::of(&known_levels(&self.room).await?))
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
            let levels = known_levels(&room.room).await?;
            if Permissions::of(&levels) == Permissions::Everyone {
                update.unchanged.push(id);
            } else if Rights::of(&levels, &self.session.user_id()).change_levels {
                room.set_permissions(Permissions::Everyone).await?;
                update.changed.push(id);
            } else {
                update.not_allowed.push(id);
            }
        }
        Ok(update)
    }
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
    fn a_member_of_an_open_room_edits_it_but_not_its_people() {
        let content: RoomPowerLevelsEventContent =
            serde_json::from_value(open_power_levels()).unwrap();
        let rights = Rights::of(&levels(content), user_id!("@member:example.org"));
        let Rights {
            send,
            pin,
            rename,
            change_topic,
            change_picture,
            invite,
            add_rooms,
            set_call_service,
            ..
        } = rights;
        assert!(send && pin && rename && change_topic && change_picture);
        assert!(invite && add_rooms && set_call_service);
        let Rights {
            redact_others,
            mention_room,
            kick,
            ban,
            change_join_rule,
            change_history,
            change_address,
            change_levels,
            encrypt,
            ..
        } = rights;
        assert!(!(redact_others || mention_room || kick || ban));
        assert!(!(change_join_rule || change_history || change_address));
        assert!(!(change_levels || encrypt));
        assert_eq!((rights.level, rights.default_level), (Some(0), 0));
    }

    #[test]
    fn a_moderator_manages_people_but_not_permissions() {
        let mut content = RoomPowerLevelsEventContent::new(&AuthorizationRules::V1);
        let moderator = user_id!("@moderator:example.org");
        content.users.insert(moderator.to_owned(), int!(50));
        for event_type in [
            "m.room.power_levels",
            "m.room.join_rules",
            "m.room.encryption",
        ] {
            content.events.insert(event_type.into(), int!(100));
        }
        let levels = levels(content);
        let rights = Rights::of(&levels, moderator);
        assert!(rights.kick && rights.ban && rights.redact_others && rights.mention_room);
        assert!(rights.rename && rights.change_address && rights.pin && rights.add_rooms);
        assert!(!rights.change_levels && !rights.change_join_rule && !rights.encrypt);
        assert_eq!(rights.level, Some(50));

        // A plain member of a room as Matrix makes it only sends and invites.
        let member = Rights::of(&levels, user_id!("@member:example.org"));
        assert!(member.send && member.invite);
        assert!(!member.rename && !member.pin && !member.kick && !member.add_rooms);
    }

    #[test]
    fn a_creator_may_do_everything() {
        let mut content = RoomPowerLevelsEventContent::new(&AuthorizationRules::V12);
        content.state_default = int!(100);
        let levels = RoomPowerLevels::new(
            RoomPowerLevelsSource::Original(content),
            &AuthorizationRules::V12,
            [user_id!("@creator:example.org").to_owned()],
        );
        let rights = Rights::of(&levels, user_id!("@creator:example.org"));
        assert!(rights.change_levels && rights.encrypt && rights.ban && rights.set_call_service);
        assert_eq!(rights.level, None);
    }

    #[test]
    fn nothing_is_allowed_while_not_known() {
        let rights = Rights::default();
        assert!(!rights.send && !rights.invite && !rights.pin && !rights.change_levels);
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
