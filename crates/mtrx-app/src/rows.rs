//! The timeline as it is shown, row by row: messages under their sender's
//! name, runs of state events in one line, days between. Each row has parts
//! whose text can be selected, in reading order. The view and the selection
//! both go through here, so the text copied is the text shown. See
//! `docs/text-selection.md`.

use std::{collections::HashMap, time::SystemTime};

use chrono::{DateTime, Local};
use iced::advanced::widget::Id;
use mtrx_core::{Event, EventContent, MessageKind, Reply, Row, User};
use mtrx_format::{Block, Document, Inline};

use crate::{
    clipboard::Copied,
    message,
    selection::{Bounds, Pos, Shown, Unit, units_text},
};

/// A part of a row whose text can be selected.
pub struct Part<'a> {
    /// Its place in a selection ([`crate::selection::Point::event`]): a
    /// message's text has the message's event id.
    pub key: String,
    pub text: Text<'a>,
}

/// What a part shows.
pub enum Text<'a> {
    /// The sender's name and the time, above their messages.
    Sender(&'a Event),
    /// The message replied to: its sender's name and the start of it.
    Reply(&'a Reply),
    /// A message's text.
    Message(&'a Document),
    /// A line: in place of a message (deleted, say), for state events, a
    /// day, the start of the room.
    Line(String),
}

/// The parts of `row`, in reading order. A message's sender heads its
/// group.
pub fn row_parts(row: &Row) -> Vec<Part<'_>> {
    let line = |key: String, line: String| Part {
        key,
        text: Text::Line(line),
    };
    match row {
        Row::Message(message) => {
            let event = &message.event;
            let mut parts = Vec::new();
            if message.first_in_group {
                parts.push(Part {
                    key: format!("{}/sender", event.id),
                    text: Text::Sender(event),
                });
            }
            match &event.content {
                EventContent::Message(m) => {
                    if let Some(reply) = &m.in_reply_to {
                        parts.push(Part {
                            key: reply_key(&event.id),
                            text: Text::Reply(reply),
                        });
                    }
                    if matches!(
                        m.kind,
                        MessageKind::Text | MessageKind::Notice | MessageKind::Emote
                    ) {
                        parts.push(Part {
                            key: event.id.clone(),
                            text: Text::Message(&m.document),
                        });
                    }
                }
                content => parts.extend(
                    placeholder(content, message.decrypting).map(|l| line(event.id.clone(), l)),
                ),
            }
            parts
        }
        Row::Summary { id, text } => vec![line(format!("{id}/state"), text.clone())],
        Row::Day(day) => {
            let since = day
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap_or_default();
            let day: DateTime<Local> = (*day).into();
            vec![line(
                format!("day/{}", since.as_secs()),
                day.format("%A, %e %B %Y").to_string(),
            )]
        }
        Row::Start => vec![line(
            "start".into(),
            "This is the start of the room.".into(),
        )],
        // A divider, without text to select.
        Row::ReadMarker => Vec::new(),
    }
}

/// The line shown in place of a message that is not one, if it is shown;
/// an undecryptable one may still be `decrypting`.
fn placeholder(content: &EventContent, decrypting: bool) -> Option<String> {
    match content {
        EventContent::UnableToDecrypt if decrypting => Some("Decrypting…".into()),
        EventContent::UnableToDecrypt => Some("Could not decrypt this message.".into()),
        EventContent::Redacted => Some("Message deleted".into()),
        EventContent::Other { description } => Some(format!("sent a {description}")),
        _ => None,
    }
}

/// The part of the message `event` that is the message it replies to.
fn reply_key(event: &str) -> String {
    format!("{event}/reply")
}

/// The id under which the reply preview of part `key` reports the text it
/// shows, cut to fit (see [`crate::selectable::text::Selectable::id`]).
pub fn shown_id(key: &str) -> Id {
    Id::from(format!("shown:{key}"))
}

impl Part<'_> {
    /// Its text, in units; a reply's preview as `shown` says, if it does.
    pub fn units(&self, shown: &Shown) -> Vec<Unit> {
        let unit = |text: String, prefix: &str, gap| Unit {
            text,
            prefix: prefix.into(),
            gap,
        };
        match &self.text {
            // Copied as the name and time put before messages are.
            Text::Sender(event) => vec![
                unit(event.sender.display_name(), "", ""),
                unit(time(event), "", ", "),
            ],
            Text::Reply(reply) => {
                let (name, preview) = reply_text(reply);
                let preview = shown.get(&self.key).cloned().unwrap_or(preview);
                vec![unit(name, "> ", ""), unit(preview, "", ": ")]
            }
            Text::Message(document) => message::text_map(document).units,
            Text::Line(line) => vec![unit(line.clone(), "", "")],
        }
    }
}

/// The parts of each of `rows`.
pub fn parts(rows: &[Row]) -> Vec<Vec<Part<'_>>> {
    rows.iter().map(row_parts).collect()
}

/// The parts of all rows (see [`parts`]) in reading order, each with its
/// row.
pub fn in_order<'p, 'a>(parts: &'p [Vec<Part<'a>>]) -> Vec<(usize, &'p Part<'a>)> {
    (parts.iter().enumerate())
        .flat_map(|(row, parts)| parts.iter().map(move |part| (row, part)))
        .collect()
}

/// Each part's place in reading order, by key, for ordering a selection.
pub fn order<'p>(parts: &'p [Vec<Part<'_>>]) -> HashMap<&'p str, usize> {
    (in_order(parts).into_iter().enumerate())
        .map(|(i, (_, part))| (part.key.as_str(), i))
        .collect()
}

impl Part<'_> {
    /// What copying it from `from` to `to` gives: a message's text and its
    /// HTML, other text as a paragraph of it.
    fn copied(&self, from: Pos, to: Pos, shown: &Shown) -> Copied {
        if let Text::Message(document) = &self.text {
            return message::copied(document, from, to);
        }
        let text = units_text(&self.units(shown), from, to);
        let html = Document {
            blocks: vec![Block::Paragraph(vec![Inline::Text(text.clone())])],
        }
        .to_html();
        Copied { text, html }
    }
}

/// What copying the parts of `rows` (see [`parts`]) from the start of
/// `bounds` to its end gives, as shown (`shown` says what cut texts show).
/// Parts of a row are a line apart, rows a blank line. With several
/// messages, each sender's name and time go before their messages, unless
/// their name is in it already.
pub fn copied(
    rows: &[Row],
    parts: &[Vec<Part<'_>>],
    bounds: Bounds,
    shown: &Shown,
) -> Option<Copied> {
    let parts = in_order(parts);
    let copies: Vec<_> = (bounds.start.0..=bounds.end.0)
        .filter_map(|i| {
            let (row, part) = parts.get(i)?;
            let (from, to) = bounds.in_message(i)?;
            let copied = part.copied(from, to, shown);
            (!copied.text.is_empty()).then_some((*row, *part, copied))
        })
        .collect();
    let mut messages: Vec<usize> = (copies.iter())
        .filter(|(row, ..)| rows[*row].event().is_some())
        .map(|(row, ..)| *row)
        .collect();
    messages.dedup();
    let several = messages.len() > 1;
    let mut out = Copied::default();
    let mut sender = None;
    let mut last_row = None;
    for (row, part, mut copied) in copies {
        match (&part.text, rows[row].event()) {
            (Text::Sender(event), _) => sender = Some(&event.sender.id),
            (_, Some(event)) if several && sender != Some(&event.sender.id) => {
                copied = Copied::byline(&event.sender.display_name(), &time(event)).then(copied);
                sender = Some(&event.sender.id);
            }
            _ => {}
        }
        if last_row == Some(row) {
            out.push_line(copied);
        } else {
            out.push(copied);
        }
        last_row = Some(row);
    }
    (!out.text.is_empty()).then_some(out)
}

/// The name of the sender of a reply's message and the start of it, as
/// shown before it is cut to fit.
fn reply_text(reply: &Reply) -> (String, String) {
    let sender = (reply.sender.as_ref()).map_or_else(|| "…".to_owned(), User::display_name);
    let preview = reply.preview.as_deref().unwrap_or(if reply.failed {
        "Could not load the message"
    } else {
        "Loading…"
    });
    (sender, preview.to_owned())
}

pub fn time(event: &Event) -> String {
    DateTime::<Local>::from(event.timestamp)
        .format("%H:%M")
        .to_string()
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use mtrx_core::{MembershipChange, OwnedUserId, RowContext, TimelineEntry};

    use super::*;
    use crate::selection::{Point, Pos, Selection};

    fn user(name: &str) -> OwnedUserId {
        format!("@{name}:x").try_into().unwrap()
    }

    /// A message from `sender`, `minutes` after the epoch.
    fn message(id: &str, sender: &str, minutes: u64, body: &str) -> Event {
        Event {
            timestamp: SystemTime::UNIX_EPOCH + Duration::from_secs(minutes * 60),
            ..Event::test_message(id, user(sender), body)
        }
    }

    /// A day; alice twice, the second a reply; dave joins; bob, then his
    /// message deleted; alice again much later.
    fn entries() -> Vec<TimelineEntry> {
        let mut reply = message("$b", "alice", 1, "Second");
        if let EventContent::Message(m) = &mut reply.content {
            m.in_reply_to = Some(Reply {
                event_id: "$x:x".try_into().unwrap(),
                sender: Some(User::from(user("carol"))),
                preview: Some("Where is the data from?".into()),
                kind: None,
                failed: false,
            });
        }
        let joined = Event {
            content: EventContent::Membership {
                user: user("dave"),
                name: None,
                change: MembershipChange::Joined,
            },
            ..message("$s", "dave", 2, "")
        };
        let deleted = Event {
            content: EventContent::Redacted,
            ..message("$d", "bob", 4, "")
        };
        [
            TimelineEntry::DayDivider(SystemTime::UNIX_EPOCH),
            TimelineEntry::Event(message("$a", "alice", 0, "First")),
            TimelineEntry::Event(reply),
            TimelineEntry::Event(joined),
            TimelineEntry::Event(message("$c", "bob", 3, "Third")),
            TimelineEntry::Event(deleted),
            TimelineEntry::Event(message("$e", "alice", 20, "Fourth")),
        ]
        .into()
    }

    fn rows(entries: &[TimelineEntry]) -> Vec<Row> {
        mtrx_core::rows(entries, &RowContext::default())
    }

    fn pos(unit: usize, offset: usize) -> Pos {
        Pos { unit, offset }
    }

    /// The text copied from `anchor` to `focus` (parts and places).
    fn copy(
        entries: &[TimelineEntry],
        anchor: (&str, Pos),
        focus: (&str, Pos),
        shown: &Shown,
    ) -> String {
        let rows = rows(entries);
        let parts = parts(&rows);
        let point = |(event, pos): (&str, Pos)| Point {
            event: event.into(),
            pos,
        };
        let selection = Selection {
            anchor: point(anchor),
            focus: point(focus),
            dragging: false,
        };
        let bounds = selection.ordered(&order(&parts)).unwrap();
        (copied(&rows, &parts, bounds, shown)).map_or_else(String::new, |copied| copied.text)
    }

    #[test]
    fn rows_group_messages_under_their_sender() {
        let entries = entries();
        let rows = rows(&entries);
        let parts = parts(&rows);
        let in_order = in_order(&parts);
        let keys: Vec<&str> = in_order.iter().map(|(_, part)| part.key.as_str()).collect();
        assert_eq!(
            keys,
            [
                "day/0",
                "$a/sender",
                "$a",
                "$b/reply",
                "$b",
                "$s/state",
                "$c/sender",
                "$c",
                "$d",
                "$e/sender",
                "$e"
            ]
        );
        let line = |key: &str| {
            let (_, part) = in_order.iter().find(|(_, part)| part.key == key).unwrap();
            part.units(&Shown::new()).remove(0).text
        };
        assert_eq!(line("$s/state"), "dave joined");
        assert_eq!(line("$d"), "Message deleted");
    }

    #[test]
    fn the_read_marker_has_no_text_and_starts_a_group() {
        let entries = [
            TimelineEntry::Event(message("$a", "alice", 0, "First")),
            TimelineEntry::ReadMarker,
            TimelineEntry::Event(message("$b", "alice", 1, "Second")),
        ];
        let rows = rows(&entries);
        let parts = parts(&rows);
        assert!(parts[1].is_empty());
        let keys: Vec<_> = (in_order(&parts).iter())
            .map(|(_, part)| part.key.as_str())
            .collect();
        assert_eq!(keys, ["$a/sender", "$a", "$b/sender", "$b"]);
    }

    #[test]
    fn copies_names_once_and_quotes_as_shown() {
        let entries = entries();
        let at = |id: &str| {
            (entries.iter())
                .find_map(|entry| match entry {
                    TimelineEntry::Event(event) if event.id == id => Some(time(event)),
                    _ => None,
                })
                .unwrap()
        };
        let none = Shown::new();
        // From inside alice's first message: her name and time put first;
        // bob's are selected, so not put again.
        assert_eq!(
            copy(&entries, ("$a", pos(0, 2)), ("$c", pos(0, 3)), &none),
            format!(
                "alice, {}\nrst\n\n> carol: Where is the data from?\nSecond\n\n\
                 dave joined\n\nbob, {}\nThi",
                at("$a"),
                at("$c")
            )
        );
        // As HTML too: lines as paragraphs, the messages' parts in theirs.
        let rows = rows(&entries);
        let parts = parts(&rows);
        let all = Selection {
            anchor: Point {
                event: "day/0".into(),
                pos: Pos::default(),
            },
            focus: Point {
                event: "$e".into(),
                pos: Pos::END,
            },
            dragging: false,
        };
        let bounds = all.ordered(&order(&parts)).unwrap();
        let html = copied(&rows, &parts, bounds, &none).unwrap().html;
        for shown in ["dave joined", "Message deleted", "First", "Fourth"] {
            assert!(html.contains(shown), "{shown}: {html}");
        }
        // From her name: as selected.
        assert_eq!(
            copy(&entries, ("$a/sender", pos(0, 0)), ("$a", Pos::END), &none),
            format!("alice, {}\nFirst", at("$a"))
        );
        // Within one message, or a line: no name.
        assert_eq!(
            copy(&entries, ("$a", pos(0, 1)), ("$a", Pos::END), &none),
            "irst"
        );
        assert_eq!(
            copy(&entries, ("$d", pos(0, 8)), ("$d", Pos::END), &none),
            "deleted"
        );
        // The quote as cut to fit.
        let shown = Shown::from([("$b/reply".to_owned(), "Where is…".to_owned())]);
        assert_eq!(
            copy(&entries, ("$b/reply", pos(0, 0)), ("$b", Pos::END), &shown),
            "> carol: Where is…\nSecond"
        );
    }
}
