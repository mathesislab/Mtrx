//! The timeline as the apps show it, row by row: messages grouped by
//! sender, runs of state events in one line, days, the read marker, and
//! what each message offers.

use std::time::{Duration, SystemTime};

use matrix_sdk::ruma::OwnedEventId;

use crate::{Event, EventContent, MembershipChange, SendState, TimelineEntry, describe_change};

/// A message this soon after the one before, by the same sender, continues
/// its group.
pub const GROUP_GAP: Duration = Duration::from_secs(5 * 60);

/// One row of a timeline.
// Nearly all rows are messages, so boxing would only add indirection.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug, PartialEq)]
pub enum Row {
    /// The room's beginning; back-pagination reached it.
    Start,
    /// The first event of a new day follows.
    Day(SystemTime),
    /// Everything below was not read yet.
    ReadMarker,
    /// A run of state events (joins, renames, ...) in one line, e.g. "alice
    /// created and configured the room · bob joined". `id`: its first
    /// event's [`Event::id`].
    Summary { id: String, text: String },
    /// A message, or a deleted or undecryptable one.
    Message(MessageRow),
}

impl Row {
    /// The event of a message's row.
    pub fn event(&self) -> Option<&Event> {
        match self {
            Row::Message(message) => Some(&message.event),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct MessageRow {
    pub event: Event,
    /// Starts a group of messages by one sender.
    pub first_in_group: bool,
    /// Show the sender's name above it and their avatar beside it: the
    /// first of a group by someone else, outside direct chats.
    pub show_sender: bool,
    /// Someone else's message outside a direct chat: the avatars' column is
    /// on its left.
    pub avatar_column: bool,
    /// Undecryptable while message keys arrive: it may decrypt meanwhile.
    pub decrypting: bool,
    /// What holding it offers, in the menu's order.
    pub actions: Vec<MessageAction>,
}

/// What can be done with a message, in the menu's order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MessageAction {
    /// Add (or remove) a reaction.
    React,
    /// Answer it, quoting it.
    Reply,
    /// Copy its text.
    Copy,
    /// Show its text to select from.
    SelectText,
    /// Change its text (ours).
    Edit,
    /// Pin it to the room's topic panel, or take it off (where we may).
    Pin,
    Unpin,
    /// Delete it for everyone (ours).
    Delete,
    /// Send it again after the server refused it.
    Retry,
    /// Drop it after the server refused it.
    Discard,
}

/// What the rows depend on besides the timeline.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RowContext {
    /// A direct chat, where who wrote what is clear: no names or avatars.
    pub direct: bool,
    /// Message keys are arriving (see [`crate::EncryptionState`]).
    pub keys_arriving: bool,
    /// We may pin and unpin messages.
    pub can_pin: bool,
    /// The room's pinned messages.
    pub pinned: Vec<OwnedEventId>,
}

/// The rows showing `entries` (oldest first). A message continues the
/// group of the one before by the same sender less than [`GROUP_GAP`]
/// earlier; a day, the read marker, a summary or the start ends a group.
pub fn rows(entries: &[TimelineEntry], context: &RowContext) -> Vec<Row> {
    let mut rows = Vec::with_capacity(entries.len());
    let mut previous: Option<&Event> = None;
    let mut state: Vec<&Event> = Vec::new();
    for entry in entries {
        if let TimelineEntry::Event(event) = entry
            && event.is_state()
        {
            state.push(event);
            continue;
        }
        if let Some(summary) = summary_row(&std::mem::take(&mut state)) {
            rows.push(summary);
            previous = None;
        }
        match entry {
            TimelineEntry::Event(event) => {
                let first = previous.is_none_or(|p| !continues(p, event));
                rows.push(Row::Message(message_row(event, first, context)));
                previous = Some(event);
            }
            TimelineEntry::Start => {
                rows.push(Row::Start);
                previous = None;
            }
            TimelineEntry::DayDivider(day) => {
                rows.push(Row::Day(*day));
                previous = None;
            }
            TimelineEntry::ReadMarker => {
                rows.push(Row::ReadMarker);
                previous = None;
            }
        }
    }
    rows.extend(summary_row(&state));
    rows
}

/// `event` continues the group of `previous`.
fn continues(previous: &Event, event: &Event) -> bool {
    previous.sender == event.sender
        && (event.timestamp)
            .duration_since(previous.timestamp)
            .unwrap_or_default()
            < GROUP_GAP
}

fn message_row(event: &Event, first_in_group: bool, context: &RowContext) -> MessageRow {
    let others = !event.own && !context.direct;
    MessageRow {
        event: event.clone(),
        first_in_group,
        show_sender: first_in_group && others,
        avatar_column: others,
        decrypting: context.keys_arriving && event.content == EventContent::UnableToDecrypt,
        actions: actions(event, context),
    }
}

/// What holding `event` offers: React and Reply once it is sent, Copy and
/// Select text with a text (a file's caption, not its name), Edit where
/// allowed, Pin or Unpin where we may,
/// Delete for our own sent ones, Retry and Discard after a refused send.
fn actions(event: &Event, context: &RowContext) -> Vec<MessageAction> {
    let mut actions = Vec::new();
    let sent = event.event_id.as_ref();
    if let EventContent::Message(message) = &event.content {
        if sent.is_some() {
            actions.extend([MessageAction::React, MessageAction::Reply]);
        }
        // A file's body is its name unless captioned.
        let text = match &message.media {
            Some(media) => message.body != media.filename,
            None => !message.document.to_plain_text().is_empty(),
        };
        if text {
            actions.extend([MessageAction::Copy, MessageAction::SelectText]);
        }
    }
    if event.editable {
        actions.push(MessageAction::Edit);
    }
    if let Some(id) = sent
        && context.can_pin
        && matches!(event.content, EventContent::Message(_))
    {
        actions.push(match context.pinned.contains(id) {
            true => MessageAction::Unpin,
            false => MessageAction::Pin,
        });
    }
    if event.own && sent.is_some() && event.content != EventContent::Redacted {
        actions.push(MessageAction::Delete);
    }
    if let SendState::Failed(_) = event.send_state {
        actions.extend([MessageAction::Retry, MessageAction::Discard]);
    }
    actions
}

/// The row of a run of state events, unless it says nothing (e.g. a topic
/// set again as it was).
fn summary_row(events: &[&Event]) -> Option<Row> {
    let first = events.first()?;
    let text = summary(events);
    (!text.is_empty()).then(|| Row::Summary {
        id: first.id.clone(),
        text,
    })
}

/// One line for a run of state events, e.g. "alice created and configured
/// the room · alice invited bob · bob joined".
fn summary(events: &[&Event]) -> String {
    let creators: Vec<_> = (events.iter())
        .filter(|e| matches!(&e.content, EventContent::OtherState { event_type } if event_type == "m.room.create"))
        .map(|e| &e.sender)
        .collect();
    let mut parts: Vec<String> = Vec::new();
    let mut joined: Vec<String> = Vec::new();
    let mut left: Vec<String> = Vec::new();
    for event in events {
        let name = display_name(event);
        let part = match &event.content {
            EventContent::OtherState { event_type } if event_type == "m.room.create" => {
                format!("{name} created and configured the room")
            }
            // Initial settings belong to "created and configured".
            EventContent::OtherState { .. }
            | EventContent::Topic { .. }
            | EventContent::Pins(_)
            | EventContent::CallLink { .. }
                if creators.contains(&&event.sender) =>
            {
                continue;
            }
            EventContent::Topic { old, new } => {
                match describe_change(old.as_deref().unwrap_or_default(), new) {
                    Some(change) => format!("{name} {change}"),
                    // Set again as it was.
                    None if old.is_some() => continue,
                    None => format!("{name} changed the topic"),
                }
            }
            EventContent::Pins(change) => format!("{name} {}", change.describe()),
            EventContent::CallLink { url: Some(url) } => {
                format!("{name} changed the call link to {url}")
            }
            EventContent::CallLink { url: None } => format!("{name} removed the call link"),
            EventContent::OtherState { event_type } => match event_type.as_str() {
                "m.room.name" => format!("{name} renamed the room"),
                "m.room.topic" => format!("{name} changed the topic"),
                "m.room.avatar" => format!("{name} changed the room picture"),
                "m.room.pinned_events" => format!("{name} changed the pinned messages"),
                _ => format!("{name} changed the room settings"),
            },
            EventContent::Membership {
                user,
                name: target,
                change,
            } => {
                let target = (target.clone()).unwrap_or_else(|| user.localpart().to_owned());
                match change {
                    MembershipChange::Joined | MembershipChange::InvitationAccepted => {
                        if !creators.contains(&user) && !joined.contains(&target) {
                            joined.push(target);
                        }
                        continue;
                    }
                    MembershipChange::Left => {
                        left.push(target);
                        continue;
                    }
                    MembershipChange::Invited => format!("{name} invited {target}"),
                    MembershipChange::InvitationRejected => format!("{target} declined the invite"),
                    MembershipChange::InvitationRevoked => {
                        format!("{name} withdrew the invite for {target}")
                    }
                    MembershipChange::Kicked => format!("{name} removed {target}"),
                    MembershipChange::Banned => format!("{name} banned {target}"),
                    MembershipChange::Unbanned => format!("{name} unbanned {target}"),
                    MembershipChange::Knocked => format!("{target} asked to join"),
                    MembershipChange::Other => continue,
                }
            }
            EventContent::DisplayNameChange { user, new_name } => match new_name {
                Some(new) => format!("{} is now {new}", user.localpart()),
                None => format!("{} removed their display name", user.localpart()),
            },
            EventContent::Message(_)
            | EventContent::Redacted
            | EventContent::UnableToDecrypt
            | EventContent::Other { .. } => continue,
        };
        if parts.last() != Some(&part) {
            parts.push(part);
        }
    }
    if !joined.is_empty() {
        parts.push(format!("{} joined", joined.join(", ")));
    }
    if !left.is_empty() {
        parts.push(format!("{} left", left.join(", ")));
    }
    parts.join(" · ")
}

/// The sender's display name, else their user ID's localpart.
fn display_name(event: &Event) -> String {
    (event.sender_name.clone()).unwrap_or_else(|| event.sender.localpart().to_owned())
}

#[cfg(test)]
mod tests {
    use matrix_sdk::ruma::OwnedUserId;
    use mtrx_format::Document;

    use super::*;
    use crate::{EventKey, Media, MediaKind, MediaRef, Message, MessageKind, PinsChange};

    fn user(name: &str) -> OwnedUserId {
        format!("@{name}:example.org").try_into().unwrap()
    }

    /// A sent message `id` from `sender` ("me" is us), `minutes` after the
    /// epoch.
    fn message(id: &str, sender: &str, minutes: u64) -> Event {
        Event {
            event_id: Some(format!("${id}").try_into().unwrap()),
            timestamp: SystemTime::UNIX_EPOCH + Duration::from_secs(minutes * 60),
            own: sender == "me",
            ..Event::test_message(id, user(sender), "Hi")
        }
    }

    fn state(id: &str, sender: &str, content: EventContent) -> Event {
        Event {
            content,
            ..message(id, sender, 0)
        }
    }

    /// `target`'s membership changed, named as `target` capitalised.
    fn membership(id: &str, sender: &str, target: &str, change: MembershipChange) -> Event {
        let name = Some(format!("{}{}", target[..1].to_uppercase(), &target[1..]));
        let user = user(target);
        state(id, sender, EventContent::Membership { user, name, change })
    }

    fn other_state(id: &str, sender: &str, event_type: &str) -> Event {
        let event_type = event_type.to_owned();
        state(id, sender, EventContent::OtherState { event_type })
    }

    fn entries(events: impl IntoIterator<Item = Event>) -> Vec<TimelineEntry> {
        events.into_iter().map(TimelineEntry::Event).collect()
    }

    fn in_room() -> RowContext {
        RowContext::default()
    }

    /// The message rows among `rows`.
    fn messages(rows: &[Row]) -> Vec<&MessageRow> {
        (rows.iter())
            .filter_map(|row| match row {
                Row::Message(message) => Some(message),
                _ => None,
            })
            .collect()
    }

    /// The row of `event` alone.
    fn row(event: Event, context: &RowContext) -> MessageRow {
        match rows(&entries([event]), context).remove(0) {
            Row::Message(message) => message,
            row => panic!("not a message: {row:?}"),
        }
    }

    fn summary_of(events: &[Event]) -> String {
        summary(&events.iter().collect::<Vec<_>>())
    }

    #[test]
    fn groups_messages_by_sender_and_time() {
        let rows = rows(
            &entries([
                message("1", "alice", 0),
                message("2", "alice", 1),
                message("3", "alice", 10),
                message("4", "me", 11),
                message("5", "bob", 12),
            ]),
            &in_room(),
        );
        let messages = messages(&rows);
        let first: Vec<_> = messages.iter().map(|m| m.first_in_group).collect();
        assert_eq!(first, [true, false, true, true, true]);
        // Our own messages carry no name, nor an avatars' column.
        let senders: Vec<_> = messages.iter().map(|m| m.show_sender).collect();
        assert_eq!(senders, [true, false, true, false, true]);
        let column: Vec<_> = messages.iter().map(|m| m.avatar_column).collect();
        assert_eq!(column, [true, true, true, false, true]);
    }

    #[test]
    fn direct_chats_show_no_names() {
        let direct = RowContext {
            direct: true,
            ..in_room()
        };
        let row = row(message("1", "alice", 0), &direct);
        assert!(row.first_in_group);
        assert!(!row.show_sender && !row.avatar_column);
    }

    #[test]
    fn days_the_read_marker_summaries_and_the_start_end_a_group() {
        let day = SystemTime::UNIX_EPOCH;
        let topic_again = EventContent::Topic {
            old: Some("Ice".into()),
            new: "Ice".into(),
        };
        let rows = rows(
            &[
                TimelineEntry::Start,
                TimelineEntry::Event(message("1", "alice", 0)),
                TimelineEntry::DayDivider(day),
                TimelineEntry::Event(message("2", "alice", 1)),
                TimelineEntry::ReadMarker,
                TimelineEntry::Event(message("3", "alice", 2)),
                TimelineEntry::Event(membership("j", "bob", "bob", MembershipChange::Joined)),
                TimelineEntry::Event(message("4", "alice", 3)),
                // A topic set again as it was says nothing: no line, no end.
                TimelineEntry::Event(state("t", "bob", topic_again)),
                TimelineEntry::Event(message("5", "alice", 4)),
                // Deleted and undecryptable messages are grouped as others.
                TimelineEntry::Event(Event {
                    content: EventContent::Redacted,
                    ..message("6", "alice", 5)
                }),
                TimelineEntry::Event(Event {
                    content: EventContent::UnableToDecrypt,
                    ..message("7", "alice", 6)
                }),
            ],
            &in_room(),
        );
        assert_eq!(rows[0], Row::Start);
        assert_eq!(rows[2], Row::Day(day));
        assert_eq!(rows[4], Row::ReadMarker);
        assert_eq!(
            rows[6],
            Row::Summary {
                id: "j".into(),
                text: "Bob joined".into()
            }
        );
        let first: Vec<_> = (messages(&rows).iter())
            .map(|m| (m.event.id.as_str(), m.first_in_group))
            .collect();
        assert_eq!(
            first,
            [
                ("1", true),
                ("2", true),
                ("3", true),
                ("4", true),
                ("5", false),
                ("6", false),
                ("7", false)
            ]
        );
    }

    #[test]
    fn folds_state_events_into_one_line() {
        let rows = rows(
            &entries([
                other_state("c", "alice", "m.room.create"),
                membership("j1", "alice", "alice", MembershipChange::Joined),
                other_state("n", "alice", "m.room.name"),
                membership("i", "alice", "bob", MembershipChange::Invited),
                membership("j2", "bob", "bob", MembershipChange::Joined),
                message("m", "bob", 1),
                other_state("t", "bob", "m.room.topic"),
            ]),
            &in_room(),
        );
        assert_eq!(rows.len(), 3);
        assert_eq!(
            rows[0],
            Row::Summary {
                id: "c".into(),
                text: "alice created and configured the room · alice invited Bob · Bob joined"
                    .into()
            }
        );
        assert!(matches!(&rows[1], Row::Message(m) if m.show_sender));
        assert_eq!(
            rows[2],
            Row::Summary {
                id: "t".into(),
                text: "bob changed the topic".into()
            }
        );
    }

    #[test]
    fn summarises_memberships_and_names() {
        let renamed = |id, new_name: Option<&str>| {
            let (user, new_name) = (user("dave"), new_name.map(Into::into));
            state(
                id,
                "dave",
                EventContent::DisplayNameChange { user, new_name },
            )
        };
        let events = [
            membership("a", "carol", "carol", MembershipChange::Joined),
            membership("b", "carol", "carol", MembershipChange::Joined),
            membership("c", "dave", "dave", MembershipChange::Left),
            membership("d", "alice", "eve", MembershipChange::Kicked),
            membership("e", "alice", "eve", MembershipChange::Banned),
            membership("f", "frank", "frank", MembershipChange::Knocked),
            renamed("g", Some("Dave")),
            renamed("h", None),
        ];
        assert_eq!(
            summary_of(&events),
            "alice removed Eve · alice banned Eve · Frank asked to join · dave is now Dave · \
             dave removed their display name · Carol joined · Dave left"
        );
    }

    #[test]
    fn summarises_call_link_pin_and_topic_changes() {
        let link = |id, url: Option<&str>| {
            let url = url.map(Into::into);
            state(id, "alice", EventContent::CallLink { url })
        };
        assert_eq!(
            summary_of(&[link("a", Some("https://meet.jit.si/ice")), link("b", None)]),
            "alice changed the call link to https://meet.jit.si/ice · alice removed the call link"
        );

        let topic = |id, old: Option<&str>, new: &str| {
            let (old, new) = (old.map(Into::into), new.into());
            state(id, "alice", EventContent::Topic { old, new })
        };
        let call = "[call] https://zoom.us/j/1";
        assert_eq!(
            summary_of(&[
                topic("a", None, "Weekly"),
                topic("b", Some("Weekly"), &format!("Weekly\n{call}")),
                topic("c", Some(call), call),
            ]),
            "alice changed the topic · alice added the call Zoom"
        );

        let pins = PinsChange {
            pinned: 1,
            unpinned: 0,
        };
        assert_eq!(
            summary_of(&[state("a", "alice", EventContent::Pins(pins))]),
            "alice pinned a message"
        );
        // A creator's first settings are part of making the room.
        assert_eq!(
            summary_of(&[
                other_state("c", "alice", "m.room.create"),
                topic("t", None, "Weekly"),
                state("p", "alice", EventContent::Pins(pins)),
                other_state("h", "alice", "m.room.history_visibility"),
            ]),
            "alice created and configured the room"
        );
    }

    #[test]
    fn undecryptable_messages_say_decrypting_while_keys_arrive() {
        let undecryptable = Event {
            content: EventContent::UnableToDecrypt,
            ..message("1", "alice", 0)
        };
        let arriving = RowContext {
            keys_arriving: true,
            ..in_room()
        };
        assert!(!row(undecryptable.clone(), &in_room()).decrypting);
        assert!(row(undecryptable, &arriving).decrypting);
        // Other rows are the same either way.
        let text = message("2", "alice", 0);
        assert_eq!(row(text.clone(), &arriving), row(text, &in_room()));
    }

    #[test]
    fn the_menu_offers_what_applies() {
        use MessageAction::*;
        let actions = |event: Event| row(event, &in_room()).actions;
        let ours = |id| Event {
            editable: true,
            ..message(id, "me", 0)
        };
        let unsent = |id, send_state| Event {
            event_id: None,
            key: EventKey::local(id),
            send_state,
            ..ours(id)
        };

        assert_eq!(
            actions(message("1", "bob", 0)),
            [React, Reply, Copy, SelectText]
        );
        assert_eq!(
            actions(ours("2")),
            [React, Reply, Copy, SelectText, Edit, Delete]
        );
        // Not sent yet: no event ID, so no reaction, reply or deletion.
        assert_eq!(
            actions(unsent("3", SendState::Sending)),
            [Copy, SelectText, Edit]
        );
        let refused = Event {
            editable: false,
            ..unsent("4", SendState::Failed("Too large".into()))
        };
        assert_eq!(actions(refused), [Copy, SelectText, Retry, Discard]);
        let deleted = Event {
            content: EventContent::Redacted,
            ..message("5", "me", 0)
        };
        assert_eq!(actions(deleted), []);
        let undecryptable = Event {
            content: EventContent::UnableToDecrypt,
            ..message("6", "me", 0)
        };
        assert_eq!(actions(undecryptable), [Delete]);
        // An image: its name is no text to copy, its caption is.
        let image = |body: &str| Event {
            content: EventContent::Message(Message {
                kind: MessageKind::Image,
                body: body.into(),
                document: Document::from_plain(body),
                edited: false,
                in_reply_to: None,
                reactions: Vec::new(),
                media: Some(Media {
                    kind: MediaKind::Image,
                    filename: "ice.png".into(),
                    size: None,
                    mimetype: None,
                    width: None,
                    height: None,
                    source: MediaRef::plain("mxc://x/ice"),
                }),
            }),
            ..message("7", "bob", 0)
        };
        assert_eq!(actions(image("ice.png")), [React, Reply]);
        assert_eq!(
            actions(image("The ice edge")),
            [React, Reply, Copy, SelectText]
        );
    }

    #[test]
    fn pin_or_unpin_where_we_may() {
        use MessageAction::*;
        let context = RowContext {
            can_pin: true,
            pinned: vec!["$2".try_into().unwrap()],
            ..in_room()
        };
        let actions = |event: Event| row(event, &context).actions;
        assert_eq!(
            actions(message("1", "bob", 0)),
            [React, Reply, Copy, SelectText, Pin]
        );
        // Before Delete.
        let ours = Event {
            editable: true,
            ..message("2", "me", 0)
        };
        assert_eq!(
            actions(ours),
            [React, Reply, Copy, SelectText, Edit, Unpin, Delete]
        );
        // Only sent messages.
        let unsent = Event {
            event_id: None,
            ..message("3", "bob", 0)
        };
        assert_eq!(actions(unsent), [Copy, SelectText]);
        let deleted = Event {
            content: EventContent::Redacted,
            ..message("4", "bob", 0)
        };
        assert_eq!(actions(deleted), []);
        // Not where we may not.
        assert_eq!(
            row(message("5", "bob", 0), &in_room()).actions,
            [React, Reply, Copy, SelectText]
        );
    }
}
