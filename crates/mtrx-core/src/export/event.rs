//! A room's events as an export keeps them in its spool: what each shows,
//! read from the event as the server sent it, decrypted.

use std::time::SystemTime;

use matrix_sdk::{
    deserialized_responses::{TimelineEvent, TimelineEventKind, UnableToDecryptReason},
    ruma::{
        OwnedEventId, OwnedUserId,
        events::{
            AnySyncMessageLikeEvent, AnySyncStateEvent, AnySyncTimelineEvent, StateEventType,
            SyncMessageLikeEvent, SyncStateEvent,
            room::{
                MediaSource,
                member::MembershipChange,
                message::{MessageType, Relation, sanitize::remove_plain_reply_fallback},
            },
        },
        serde::Raw,
    },
};
use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;

use crate::{MediaKind, pins, timeline};

/// An event as the spool keeps it.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct Item {
    pub id: Option<OwnedEventId>,
    pub sender: OwnedUserId,
    /// Milliseconds since the Unix epoch.
    pub ts: u64,
    pub content: Content,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reply_to: Option<OwnedEventId>,
    /// The thread's first message, for a message in a thread.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread: Option<OwnedEventId>,
    /// The event as sent, decrypted: for JSON only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw: Option<Box<RawValue>>,
}

/// What an event shows.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) enum Content {
    Message(Body),
    /// Why, if known: "the key was not shared with this device".
    Undecryptable(Option<String>),
    Deleted,
    /// A change to the room, after who made it: "joined", "renamed the room
    /// to “Lab”".
    Change(String),
    /// A message-like event not shown in detail, after who sent it: "sent a
    /// sticker".
    Other(String),
    /// Shown by itself only in JSON: edits, reactions, deletions, other
    /// state.
    Hidden,
}

/// A message's text and file.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct Body {
    pub style: Style,
    /// The plain text; for a file, its caption.
    pub text: String,
    /// The formatted text (Matrix HTML), if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub html: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<FileRef>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(super) enum Style {
    Text,
    Notice,
    Emote,
}

/// A message's file, as sent.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct FileRef {
    pub kind: MediaKind,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mimetype: Option<String>,
    pub source: MediaSource,
}

/// What an event changes about another, read in the fetch pass.
#[derive(Clone, Debug)]
pub(super) enum Related {
    /// The newest text of `target`, if `sender` sent it.
    Edit {
        target: OwnedEventId,
        sender: OwnedUserId,
        body: Body,
    },
    Reaction {
        target: OwnedEventId,
        sender: OwnedUserId,
        key: String,
    },
}

/// An event read, and what it changes about another.
pub(super) struct Converted {
    pub item: Item,
    pub related: Option<Related>,
}

impl Item {
    /// It counts as a message: one written for itself, not a change.
    pub fn is_message(&self) -> bool {
        matches!(
            self.content,
            Content::Message(_) | Content::Undecryptable(_) | Content::Deleted | Content::Other(_)
        )
    }

    pub fn time(&self) -> SystemTime {
        SystemTime::UNIX_EPOCH + std::time::Duration::from_millis(self.ts)
    }
}

/// Read `event`; with its JSON if `keep_raw`.
pub(super) fn convert(event: &TimelineEvent, keep_raw: bool) -> Converted {
    let raw = event.raw();
    let parsed = raw.deserialize().ok();
    let sender = (event.sender())
        .or_else(|| parsed.as_ref().map(|e| e.sender().to_owned()))
        .unwrap_or_else(|| "@unknown:invalid".try_into().expect("a user ID"));
    let ts = (event.timestamp())
        .or_else(|| parsed.as_ref().map(|e| e.origin_server_ts()))
        .map_or(0, |ts| ts.get().into());
    let mut item = Item {
        id: event.event_id().map(ToOwned::to_owned),
        sender,
        ts,
        content: Content::Hidden,
        reply_to: None,
        thread: None,
        raw: keep_raw.then(|| raw.json().to_owned()),
    };
    let mut related = None;
    if let TimelineEventKind::UnableToDecrypt { utd_info, .. } = &event.kind {
        item.content = Content::Undecryptable(utd_reason(&utd_info.reason));
        return Converted { item, related };
    }
    let Some(parsed) = parsed else {
        return Converted { item, related };
    };
    match &parsed {
        AnySyncTimelineEvent::MessageLike(message) => match message {
            AnySyncMessageLikeEvent::RoomMessage(SyncMessageLikeEvent::Original(message)) => {
                let content = &message.content;
                match &content.relates_to {
                    Some(Relation::Replacement(replacement)) => {
                        related = Some(Related::Edit {
                            target: replacement.event_id.clone(),
                            sender: message.sender.clone(),
                            body: body(&replacement.new_content.msgtype, false),
                        });
                        return Converted { item, related };
                    }
                    Some(Relation::Reply(reply)) => {
                        item.reply_to = Some(reply.in_reply_to.event_id.clone());
                    }
                    Some(Relation::Thread(thread)) => {
                        item.thread = Some(thread.event_id.clone());
                        item.reply_to = (thread.in_reply_to.as_ref())
                            .filter(|_| !thread.is_falling_back)
                            .map(|r| r.event_id.clone());
                    }
                    _ => {}
                }
                item.content = Content::Message(body(&content.msgtype, item.reply_to.is_some()));
            }
            AnySyncMessageLikeEvent::Reaction(SyncMessageLikeEvent::Original(reaction)) => {
                let annotation = &reaction.content.relates_to;
                related = Some(Related::Reaction {
                    target: annotation.event_id.clone(),
                    sender: reaction.sender.clone(),
                    key: annotation.key.clone(),
                });
            }
            AnySyncMessageLikeEvent::RoomRedaction(_) | AnySyncMessageLikeEvent::Reaction(_) => {}
            message if message.is_redacted() => item.content = Content::Deleted,
            AnySyncMessageLikeEvent::Sticker(_) => {
                item.content = Content::Other("sent a sticker".into())
            }
            AnySyncMessageLikeEvent::PollStart(_)
            | AnySyncMessageLikeEvent::UnstablePollStart(_) => {
                item.content = Content::Other("started a poll".into())
            }
            AnySyncMessageLikeEvent::RoomEncrypted(_) => {
                item.content = Content::Undecryptable(None)
            }
            _ => {}
        },
        AnySyncTimelineEvent::State(state) => {
            if let Some(change) = change(state, raw) {
                item.content = Content::Change(change);
            }
        }
    }
    Converted { item, related }
}

/// A message's text and file; a reply's quote of its original dropped from
/// a plain body.
fn body(msgtype: &MessageType, reply: bool) -> Body {
    let style = match msgtype {
        MessageType::Notice(_) => Style::Notice,
        MessageType::Emote(_) => Style::Emote,
        _ => Style::Text,
    };
    let html = timeline::html_body(msgtype).map(ToOwned::to_owned);
    let file = timeline::media(msgtype).map(|media| FileRef {
        kind: media.kind,
        name: media.filename,
        size: media.size,
        mimetype: media.mimetype,
        source: media.source.file,
    });
    let text = match msgtype {
        MessageType::Image(c) => c.caption().unwrap_or_default(),
        MessageType::Video(c) => c.caption().unwrap_or_default(),
        MessageType::Audio(c) => c.caption().unwrap_or_default(),
        MessageType::File(c) => c.caption().unwrap_or_default(),
        other if reply => remove_plain_reply_fallback(other.body()),
        other => other.body(),
    };
    Body {
        style,
        text: text.to_owned(),
        html,
        file,
    }
}

/// A change to the room, after who made it, as the timeline shows them;
/// `None` for changes not shown.
fn change(state: &AnySyncStateEvent, raw: &Raw<AnySyncTimelineEvent>) -> Option<String> {
    Some(match state {
        AnySyncStateEvent::RoomMember(SyncStateEvent::Original(member)) => {
            let target = (member.content.displayname.clone())
                .filter(|name| !name.trim().is_empty())
                .unwrap_or_else(|| member.state_key.localpart().to_owned());
            match member.membership_change() {
                MembershipChange::Joined
                | MembershipChange::InvitationAccepted
                | MembershipChange::KnockAccepted => "joined".into(),
                MembershipChange::Left => "left".into(),
                MembershipChange::Invited => format!("invited {target}"),
                MembershipChange::InvitationRejected => "declined the invite".into(),
                MembershipChange::InvitationRevoked => format!("withdrew the invite for {target}"),
                MembershipChange::Kicked => format!("removed {target}"),
                MembershipChange::Banned | MembershipChange::KickedAndBanned => {
                    format!("banned {target}")
                }
                MembershipChange::Unbanned => format!("unbanned {target}"),
                MembershipChange::Knocked => "asked to join".into(),
                MembershipChange::ProfileChanged {
                    displayname_change: Some(change),
                    ..
                } => match change.new {
                    Some(name) => format!("changed their name to {name}"),
                    None => "removed their display name".into(),
                },
                _ => return None,
            }
        }
        AnySyncStateEvent::RoomName(SyncStateEvent::Original(name)) => {
            match name.content.name.trim() {
                "" => "removed the room's name".into(),
                new => format!("renamed the room to “{new}”"),
            }
        }
        AnySyncStateEvent::RoomTopic(SyncStateEvent::Original(topic)) => {
            match topic.content.topic.trim() {
                "" => "removed the topic".into(),
                new => format!("changed the topic to “{new}”"),
            }
        }
        AnySyncStateEvent::RoomPinnedEvents(_) => {
            pins::pins_change(&StateEventType::RoomPinnedEvents, Some(raw))?.describe()
        }
        AnySyncStateEvent::RoomCreate(_) => "created the room".into(),
        _ => return None,
    })
}

/// Why an event could not be decrypted, when it says more than "the key is
/// missing".
fn utd_reason(reason: &UnableToDecryptReason) -> Option<String> {
    Some(
        match reason {
            UnableToDecryptReason::MissingMegolmSession {
                withheld_code: Some(_),
            } => "the sender didn't share the key with this device",
            UnableToDecryptReason::MissingMegolmSession {
                withheld_code: None,
            }
            | UnableToDecryptReason::UnknownMegolmMessageIndex => "the key is missing",
            UnableToDecryptReason::SenderIdentityNotTrusted(_) => "the sender isn't trusted",
            UnableToDecryptReason::MalformedEncryptedEvent
            | UnableToDecryptReason::PayloadDeserializationFailure => "it is damaged",
            _ => return None,
        }
        .into(),
    )
}
