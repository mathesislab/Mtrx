//! A room's pinned messages: Pin and Unpin in a message's menu, a count
//! beside the topic's caret, and their list in the topic panel, from a
//! timeline of their own (see [`mtrx_core::Session::pinned_timeline`]).

use std::sync::Arc;

use iced::{
    Alignment, Element, Length,
    widget::{button, column, row, text},
};
use mtrx_core::{Event, EventContent, OwnedEventId, OwnedRoomId, RoomTimeline, TimelineEntry};

use crate::{
    actions,
    app::{self, Shared},
    ellipsis::ellipsis,
    fonts,
    hint::{Position, hint},
    icons, room, style,
};

#[derive(Debug, Clone)]
pub enum Message {
    /// Whether we may pin; `None` if unknown.
    CanPin(Option<bool>),
    /// The pinned messages' timeline, made.
    Loaded(Result<Shared<RoomTimeline>, String>),
    Updated(Vec<TimelineEntry>),
    /// Show the pinned message in the room's timeline.
    GoTo(OwnedEventId),
}

/// The open room's pinned messages, as far as loaded.
#[derive(Default)]
pub struct Pins {
    pub can_pin: Option<bool>,
    /// Their own timeline, once made.
    pub timeline: Option<Arc<RoomTimeline>>,
    /// The timeline was asked for: once while the room is open.
    requested: bool,
    /// The pinned messages loaded, oldest first.
    events: Vec<Event>,
}

impl Pins {
    /// Whether to make the timeline now: there are pins, and it was not
    /// asked for yet.
    pub fn request(&mut self, pinned: &[OwnedEventId]) -> bool {
        let request = !pinned.is_empty() && !self.requested;
        self.requested |= request;
        request
    }

    pub fn update(&mut self, message: Message) {
        match message {
            Message::CanPin(can) => self.can_pin = can,
            Message::Loaded(Ok(Shared(timeline))) => {
                self.events = events(timeline.current());
                self.timeline = Some(timeline);
            }
            Message::Loaded(Err(e)) => tracing::warn!("could not load the pinned messages: {e}"),
            Message::Updated(entries) => self.events = events(entries),
            // The app goes there.
            Message::GoTo(_) => {}
        }
    }

    /// The pinned messages, most recently pinned first, from `pinned` (the
    /// room's list): the "Pinned" part of the topic panel, if any.
    pub fn view<'a>(
        &'a self,
        room: &OwnedRoomId,
        pinned: &[OwnedEventId],
    ) -> Option<Element<'a, app::Message>> {
        if pinned.is_empty() {
            return None;
        }
        let can_pin = self.can_pin == Some(true);
        let mut list = column![
            text("Pinned")
                .size(11)
                .font(fonts::SEMIBOLD)
                .style(text::secondary)
        ]
        .spacing(2);
        let shown: Vec<&Event> = (pinned.iter().rev())
            .filter_map(|id| self.events.iter().find(|e| e.event_id.as_ref() == Some(id)))
            .collect();
        if shown.is_empty() {
            list = list.push(
                text("Loading the pinned messages…")
                    .size(12)
                    .style(text::secondary),
            );
        }
        for event in shown {
            let Some(id) = event.event_id.clone() else {
                continue;
            };
            let sender = event
                .sender_name
                .clone()
                .unwrap_or_else(|| event.sender.to_string());
            let go = button(
                column![
                    text(sender).size(12).font(fonts::SEMIBOLD),
                    ellipsis(preview(event)).size(13),
                ]
                .spacing(1),
            )
            .width(Length::Fill)
            .padding([4, 8])
            .style(style::list_item(false))
            .on_press(room::Message::Pins(Message::GoTo(id.clone())).to(room));
            let mut line = row![go].spacing(4).align_y(Alignment::Center);
            if can_pin {
                line = line.push(hint(
                    button(icons::pin(13.0, |theme| {
                        theme
                            .extended_palette()
                            .background
                            .base
                            .text
                            .scale_alpha(0.55)
                    }))
                    .padding(6)
                    .style(style::text_button)
                    .on_press(actions::act(room, actions::Message::Pin(id, false))),
                    "Unpin",
                    Position::Top,
                ));
            }
            list = list.push(line);
        }
        Some(list.into())
    }
}

/// The messages among `entries`.
fn events(entries: Vec<TimelineEntry>) -> Vec<Event> {
    entries
        .into_iter()
        .filter_map(|entry| match entry {
            TimelineEntry::Event(event) => Some(event),
            _ => None,
        })
        .collect()
}

/// A pinned message in a line: its text's first line, or its file's name.
fn preview(event: &Event) -> String {
    match &event.content {
        EventContent::Message(m) => match &m.media {
            Some(media) => media.filename.clone(),
            // As shown, not its Markdown source.
            None => (m.document.to_plain_text().lines().next())
                .unwrap_or_default()
                .to_owned(),
        },
        EventContent::UnableToDecrypt => "Encrypted message".to_owned(),
        EventContent::Redacted => "Deleted message".to_owned(),
        _ => String::new(),
    }
}

/// Pin or Unpin, for the menu of a message in `room`.
pub fn menu_item(room: &OwnedRoomId, event: &OwnedEventId, pinned: bool) -> (String, app::Message) {
    let label = if pinned { "Unpin" } else { "Pin" };
    (
        label.to_owned(),
        actions::act(room, actions::Message::Pin(event.clone(), !pinned)),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn asks_for_the_timeline_once_there_are_pins() {
        let mut pins = Pins::default();
        assert!(!pins.request(&[]));
        let pinned: OwnedEventId = "$a:x".try_into().unwrap();
        assert!(pins.request(std::slice::from_ref(&pinned)));
        assert!(!pins.request(&[pinned]), "once");
    }

    #[test]
    fn pin_or_unpin() {
        let id: OwnedEventId = "$a:x".try_into().unwrap();
        let room: OwnedRoomId = "!r:x".try_into().unwrap();
        assert_eq!(menu_item(&room, &id, false).0, "Pin");
        assert!(matches!(
            menu_item(&room, &id, true).1,
            app::Message::Room(_, room::Message::Act(actions::Message::Pin(_, false)))
        ));
    }
}
