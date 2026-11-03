//! A room's live timeline, and sending.

use std::sync::Arc;

use mtrx_core::{self as core, OwnedEventId, RoomTimeline};

use crate::{
    MtrxError, TaskHandle, TimelineRow,
    runtime::{self, InRuntime, follow, run},
};

/// Receives a timeline's rows, oldest first: all of them, now and after
/// every change (also of the pinned messages, whether we may pin, and
/// whether message keys are arriving).
#[uniffi::export(with_foreign)]
pub trait TimelineRowsListener: Send + Sync {
    fn on_update(&self, rows: Vec<TimelineRow>);
}

/// Receives the names of the others typing in a room, after every change.
#[uniffi::export(with_foreign)]
pub trait TypingListener: Send + Sync {
    fn on_update(&self, names: Vec<String>);
}

/// A room's timeline, kept up to date while the object lives and sync runs.
///
/// Actions on a message take its row's `TimelineRow.id`, which also
/// identifies a message not sent yet (it has no event ID until then).
#[derive(uniffi::Object)]
pub struct Timeline {
    inner: InRuntime<Arc<RoomTimeline>>,
}

impl Timeline {
    pub(crate) fn new(timeline: RoomTimeline) -> Arc<Self> {
        Arc::new(Self {
            inner: InRuntime::new(Arc::new(timeline)),
        })
    }

    fn core(&self) -> Arc<RoomTimeline> {
        Arc::clone(&self.inner)
    }

    /// The event of row `item_id` (`TimelineRow.id`), now in the timeline.
    fn event(&self, item_id: &str) -> Result<core::Event, MtrxError> {
        self.inner
            .current()
            .into_iter()
            .find_map(|entry| match entry {
                core::TimelineEntry::Event(event) if event.id == item_id => Some(event),
                _ => None,
            })
            .ok_or_else(|| MtrxError::invalid(format!("no message `{item_id}` in the timeline")))
    }
}

/// The rows as the apps get them.
fn timeline_rows(rows: Arc<Vec<core::Row>>) -> Vec<TimelineRow> {
    rows.iter().map(TimelineRow::from).collect()
}

#[uniffi::export]
impl Timeline {
    pub fn room_id(&self) -> String {
        self.inner.room_id().to_string()
    }

    /// Call `listener` with the rows, now and on every change. Ends when the
    /// timeline is dropped.
    pub fn subscribe_rows(&self, listener: Arc<dyn TimelineRowsListener>) -> Arc<TaskHandle> {
        follow(self.inner.rows(), timeline_rows, move |rows| {
            listener.on_update(rows)
        })
    }

    /// Load up to `events` older events (when scrolled to the top). Returns
    /// `true` once the room's start is reached.
    pub async fn paginate_back(&self, events: u16) -> Result<bool, MtrxError> {
        let timeline = self.core();
        run(async move { Ok(timeline.paginate_back(events).await?) }).await
    }

    /// Load older events until the message `event_id` is in the timeline
    /// (e.g. the original of a reply), the room's start, or 20 pages of 50
    /// events; returns its row's `TimelineRow.id`, to scroll to once the
    /// rows have it. [`MtrxError::EventNotFound`] if it is not there.
    /// Cancelling the call stops looking.
    pub async fn seek(&self, event_id: String) -> Result<String, MtrxError> {
        let event = OwnedEventId::try_from(event_id).map_err(MtrxError::invalid)?;
        let timeline = self.core();
        run(async move { Ok(timeline.seek(&event).await?) }).await
    }

    /// Wait, without loading anything, until the message `event_id` is in
    /// the timeline (e.g. the one a restored draft answers); returns its
    /// row's `TimelineRow.id`, to find once the rows have it. Cancelling
    /// the call stops waiting.
    pub async fn item_of(&self, event_id: String) -> Result<String, MtrxError> {
        let event = OwnedEventId::try_from(event_id).map_err(MtrxError::invalid)?;
        let timeline = self.core();
        run(async move { Ok(timeline.item_of(&event).await) }).await
    }

    /// Send a message written in Markdown. It shows in the timeline at once,
    /// as [`crate::SendState::Sending`], and is sent in the background, also
    /// later if the server is unreachable now.
    pub async fn send_text(&self, text: String) -> Result<(), MtrxError> {
        let timeline = self.core();
        run(async move {
            timeline.send_markdown(&text, None).await?;
            Ok(())
        })
        .await
    }

    /// Send a message written in Markdown as a reply to row `item_id`'s
    /// message, which must be sent (have an event ID). Shown and sent as
    /// [`Timeline::send_text`].
    pub async fn reply(&self, item_id: String, text: String) -> Result<(), MtrxError> {
        let event_id = self
            .event(&item_id)?
            .event_id
            .ok_or_else(|| MtrxError::invalid("the message is not sent yet"))?;
        let timeline = self.core();
        run(async move {
            timeline.send_markdown(&text, Some(&event_id)).await?;
            Ok(())
        })
        .await
    }

    /// Replace the text of row `item_id`'s message (one of ours, see
    /// `TimelineEvent.editable`) with `text`, written in Markdown. Shows
    /// at once, marked `Message.edited`.
    pub async fn edit(&self, item_id: String, text: String) -> Result<(), MtrxError> {
        let (timeline, event) = (self.core(), self.event(&item_id)?);
        run(async move { Ok(timeline.edit_markdown(&event.key, &text).await?) }).await
    }

    /// Add our reaction `key` (an emoji) to row `item_id`'s message, or
    /// remove it if we reacted with it already.
    pub async fn react(&self, item_id: String, key: String) -> Result<(), MtrxError> {
        let (timeline, event) = (self.core(), self.event(&item_id)?);
        run(async move { Ok(timeline.toggle_reaction(&event.key, &key).await?) }).await
    }

    /// Delete row `item_id`'s message for everyone (redact it): it shows as
    /// [`crate::EventContent::Redacted`].
    pub async fn delete(&self, item_id: String) -> Result<(), MtrxError> {
        let (timeline, event) = (self.core(), self.event(&item_id)?);
        run(async move { Ok(timeline.redact(&event.key).await?) }).await
    }

    /// Send again row `item_id`'s message, whose sending failed
    /// ([`crate::SendState::Failed`]). Fails if it is not such a message.
    pub async fn retry(&self, item_id: String) -> Result<(), MtrxError> {
        let (timeline, event) = (self.core(), self.event(&item_id)?);
        run(async move { Ok(timeline.retry(&event.key).await?) }).await
    }

    /// Drop row `item_id`'s message, not sent (yet), e.g. after sending
    /// failed; it disappears, and the messages queued after it are sent.
    /// Fails if it was sent already.
    pub async fn discard(&self, item_id: String) -> Result<(), MtrxError> {
        let (timeline, event) = (self.core(), self.event(&item_id)?);
        run(async move { Ok(timeline.discard(&event.key).await?) }).await
    }

    /// Tell the others we are typing (while the composer has text), or
    /// stopped (it was emptied or sent). Call it as the text changes: the
    /// library sends at most one notice every few seconds.
    pub async fn set_typing(&self, typing: bool) -> Result<(), MtrxError> {
        let timeline = self.core();
        run(async move { Ok(timeline.typing(typing).await?) }).await
    }

    /// Call `listener` with the names of the others typing (display names,
    /// else the user ID's localpart): nobody now, then on every change.
    /// Runs until cancelled.
    pub fn subscribe_typing(&self, listener: Arc<dyn TypingListener>) -> Arc<TaskHandle> {
        let typing = {
            let _runtime = runtime::enter();
            self.inner.typing_updates()
        };
        follow(typing, std::convert::identity, move |names| {
            listener.on_update(names)
        })
    }

    /// Mark everything up to the latest event as read.
    pub async fn mark_read(&self) -> Result<(), MtrxError> {
        let timeline = self.core();
        run(async move { Ok(timeline.mark_read().await?) }).await
    }
}
