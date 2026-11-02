//! Timeline rendering.

use std::{fmt::Write, time::SystemTime};

use chrono::{DateTime, Local};
use mtrx_core::{Event, EventContent, MembershipChange, MessageKind, SendState, TimelineEntry};

pub fn entry_line(entry: &TimelineEntry) -> String {
    match entry {
        TimelineEntry::Event(event) => event_line(event),
        TimelineEntry::DayDivider(day) => format!("--- {} ---", local(*day).format("%A %e %B %Y")),
        TimelineEntry::ReadMarker => "--- unread ---".to_owned(),
        TimelineEntry::Start => "--- start of the room ---".to_owned(),
    }
}

pub fn event_line(event: &Event) -> String {
    let time = local(event.timestamp).format("%H:%M");
    let who = event
        .sender_name
        .clone()
        .unwrap_or_else(|| event.sender.to_string());
    let text = match &event.content {
        EventContent::Message(m) => {
            let mut text = match m.kind {
                MessageKind::Emote => format!("* {who} {}", m.body),
                MessageKind::Text | MessageKind::Notice => match &m.in_reply_to {
                    Some(reply) => format!(
                        "{who} (replying to {}): {}",
                        reply.sender_name.as_deref().unwrap_or("a message"),
                        m.body
                    ),
                    None => format!("{who}: {}", m.body),
                },
                kind => format!("{who}: [{kind:?}] {}", m.body),
            };
            if m.edited {
                text += " (edited)";
            }
            for reaction in &m.reactions {
                let _ = write!(text, " [{} {}]", reaction.key, reaction.count);
            }
            text
        }
        EventContent::Redacted => format!("{who}: (deleted)"),
        EventContent::UnableToDecrypt => format!("{who}: (unable to decrypt)"),
        EventContent::Membership { user, name, change } => {
            let name = name.clone().unwrap_or_else(|| user.to_string());
            match change {
                MembershipChange::Joined | MembershipChange::InvitationAccepted => {
                    format!("{name} joined")
                }
                MembershipChange::Left => format!("{name} left"),
                MembershipChange::Invited => format!("{who} invited {name}"),
                MembershipChange::InvitationRejected => format!("{name} rejected the invite"),
                MembershipChange::InvitationRevoked => {
                    format!("{who} revoked the invite for {name}")
                }
                MembershipChange::Kicked => format!("{who} removed {name}"),
                MembershipChange::Banned => format!("{who} banned {name}"),
                MembershipChange::Unbanned => format!("{who} unbanned {name}"),
                MembershipChange::Knocked => format!("{name} asked to join"),
                MembershipChange::Other => format!("membership change for {name}"),
            }
        }
        EventContent::DisplayNameChange { user, new_name } => match new_name {
            Some(new) => format!("{user} is now {new}"),
            None => format!("{user} removed their display name"),
        },
        EventContent::Topic { old, new } => {
            match mtrx_core::describe_change(old.as_deref().unwrap_or_default(), new) {
                Some(change) => format!("{who} {change}"),
                None => format!("{who} changed the topic"),
            }
        }
        EventContent::Pins(change) => format!("{who} {}", change.describe()),
        EventContent::OtherState { event_type } => format!("{who} changed {event_type}"),
        EventContent::Other { description } => format!("{who}: [{description}]"),
    };
    let state = match &event.send_state {
        SendState::Sent => String::new(),
        SendState::Sending => " (sending)".to_owned(),
        SendState::Waiting => " (waiting for the server)".to_owned(),
        SendState::Failed(e) => format!(" (failed: {e})"),
    };
    format!("[{time}] {text}{state}")
}

fn local(time: SystemTime) -> DateTime<Local> {
    time.into()
}
