//! Pinned messages: the room's `m.room.pinned_events` state, a list of
//! event IDs in the order they were pinned. Pinning needs leave to send
//! that state event, as changing the topic does.

use std::time::Instant;

use matrix_sdk::{
    Room,
    ruma::{
        EventId, OwnedEventId, RoomId,
        events::{AnySyncTimelineEvent, StateEventType},
        serde::Raw,
    },
};
use matrix_sdk_ui::timeline::{RoomExt, TimelineFocus};
use serde::Deserialize;

use crate::{Result, RoomHandle, RoomTimeline, Session, timeline::timeline_error};

/// How a change of the pinned messages shows in the timeline: how many
/// were pinned and how many unpinned.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PinsChange {
    pub pinned: usize,
    pub unpinned: usize,
}

impl PinsChange {
    /// What the change did, after who did it: "pinned a message",
    /// "unpinned 2 messages", "pinned a message and unpinned a message".
    pub fn describe(&self) -> String {
        let messages = |n: usize| match n {
            1 => "a message".to_owned(),
            n => format!("{n} messages"),
        };
        match (self.pinned, self.unpinned) {
            (0, 0) => "reordered the pinned messages".to_owned(),
            (pinned, 0) => format!("pinned {}", messages(pinned)),
            (0, unpinned) => format!("unpinned {}", messages(unpinned)),
            (pinned, unpinned) => format!(
                "pinned {} and unpinned {}",
                messages(pinned),
                messages(unpinned)
            ),
        }
    }
}

impl Session {
    /// Pin the message `event_id` in the room, or unpin it. The room lists
    /// and the timelines' rows show it at once.
    pub async fn set_pinned(&self, room_id: &RoomId, event_id: &EventId, pin: bool) -> Result<()> {
        let room = self.sdk_room(room_id)?;
        let mut pinned = (self.echoes.pending_pinned(room_id))
            .or_else(|| room.pinned_event_ids())
            .unwrap_or_default();
        if !pin {
            pinned.retain(|id| id != event_id);
        } else if !pinned.iter().any(|id| id == event_id) {
            pinned.push(event_id.to_owned());
        }
        self.echoes.set_pinned(room_id, pinned.clone());
        let result = if pin {
            room.pin_event(event_id).await
        } else {
            room.unpin_event(event_id).await
        };
        (self.echoes).pinned_answered(room_id, &pinned, result.is_ok(), Instant::now());
        result?;
        Ok(())
    }

    /// Whether we may pin and unpin messages in the room.
    pub async fn can_pin(&self, room_id: &RoomId) -> Result<bool> {
        can_pin(&self.sdk_room(room_id)?).await
    }

    /// The room's pinned messages, as a timeline of their own: loaded as
    /// needed, also from long ago, and kept up to date as pins change.
    pub async fn pinned_timeline(&self, room_id: &RoomId) -> Result<RoomTimeline> {
        let timeline = self
            .sdk_room(room_id)?
            .timeline_builder()
            .with_focus(TimelineFocus::PinnedEvents)
            .build()
            .await
            .map_err(timeline_error)?;
        Ok(self.room_timeline(room_id, timeline).await)
    }
}

impl RoomHandle {
    /// Pin the message `event_id`, or unpin it. The room lists and the
    /// timelines' rows show it at once.
    pub async fn set_pinned(&self, event_id: &EventId, pin: bool) -> Result<()> {
        let (echoes, room) = (&self.session.echoes, &self.room);
        let mut pinned = (echoes.pending_pinned(self.id()))
            .or_else(|| room.pinned_event_ids())
            .unwrap_or_default();
        if !pin {
            pinned.retain(|id| id != event_id);
        } else if !pinned.iter().any(|id| id == event_id) {
            pinned.push(event_id.to_owned());
        }
        echoes.set_pinned(self.id(), pinned.clone());
        let result = if pin {
            room.pin_event(event_id).await
        } else {
            room.unpin_event(event_id).await
        };
        echoes.pinned_answered(self.id(), &pinned, result.is_ok(), Instant::now());
        result?;
        Ok(())
    }

    /// Whether we may pin and unpin messages in the room.
    pub async fn can_pin(&self) -> Result<bool> {
        can_pin(&self.room).await
    }
}

/// Whether we may pin and unpin messages in `room`.
pub(crate) async fn can_pin(room: &Room) -> Result<bool> {
    let levels = room.power_levels().await.map_err(matrix_sdk::Error::from)?;
    Ok(levels.user_can_send_state(room.own_user_id(), StateEventType::RoomPinnedEvents))
}

/// What an `m.room.pinned_events` event in the timeline changed, from its
/// content and the one before (`unsigned.prev_content`).
pub(crate) fn pins_change(
    event_type: &StateEventType,
    raw: Option<&Raw<AnySyncTimelineEvent>>,
) -> Option<PinsChange> {
    #[derive(Deserialize)]
    struct Content {
        #[serde(default)]
        pinned: Vec<OwnedEventId>,
    }
    #[derive(Deserialize)]
    struct Unsigned {
        prev_content: Option<Content>,
    }
    if *event_type != StateEventType::RoomPinnedEvents {
        return None;
    }
    let raw = raw?;
    let new = raw.get_field::<Content>("content").ok()??.pinned;
    let old = (raw.get_field::<Unsigned>("unsigned").ok().flatten())
        .and_then(|u| u.prev_content)
        .map(|c| c.pinned)
        .unwrap_or_default();
    Some(PinsChange {
        pinned: new.iter().filter(|id| !old.contains(id)).count(),
        unpinned: old.iter().filter(|id| !new.contains(id)).count(),
    })
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn counts_pins_and_unpins() {
        let change = |content, prev| {
            let raw = Raw::new(&json!({
                "type": "m.room.pinned_events",
                "state_key": "",
                "content": content,
                "unsigned": { "prev_content": prev },
            }))
            .unwrap()
            .cast_unchecked();
            pins_change(&StateEventType::RoomPinnedEvents, Some(&raw))
        };
        assert_eq!(
            change(
                json!({ "pinned": ["$a", "$b"] }),
                json!({ "pinned": ["$a"] })
            ),
            Some(PinsChange {
                pinned: 1,
                unpinned: 0
            })
        );
        assert_eq!(
            change(
                json!({ "pinned": ["$c"] }),
                json!({ "pinned": ["$a", "$b"] })
            ),
            Some(PinsChange {
                pinned: 1,
                unpinned: 2
            })
        );
        assert_eq!(
            change(json!({ "pinned": ["$a"] }), serde_json::Value::Null),
            Some(PinsChange {
                pinned: 1,
                unpinned: 0
            })
        );
        assert_eq!(pins_change(&StateEventType::RoomTopic, None), None);
    }

    #[test]
    fn describes_pins_and_unpins() {
        let describe = |pinned, unpinned| PinsChange { pinned, unpinned }.describe();
        assert_eq!(describe(1, 0), "pinned a message");
        assert_eq!(describe(0, 2), "unpinned 2 messages");
        assert_eq!(describe(1, 1), "pinned a message and unpinned a message");
        assert_eq!(describe(0, 0), "reordered the pinned messages");
    }
}
