//! Plain data handed to the apps, converted from mtrx-core's types. IDs are
//! strings; times are `Date` (Swift) and `Instant` (Kotlin).

use std::time::{SystemTime, UNIX_EPOCH};

use mtrx_core as core;

/// Whether the server is reachable and still accepts the session.
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum ConnectionState {
    Online,
    /// The server is unreachable; sync retries until it answers. Messages
    /// wait in the send queue meanwhile.
    Offline,
    /// The server no longer accepts the session (it expired or was signed
    /// out elsewhere): call `Session.forget()` and sign in again.
    SignedOut,
}

impl From<core::Connection> for ConnectionState {
    fn from(connection: core::Connection) -> Self {
        match connection {
            core::Connection::Online => Self::Online,
            core::Connection::Offline => Self::Offline,
            core::Connection::SignedOut => Self::SignedOut,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum RoomKind {
    Room,
    Space,
    /// A direct chat with one person.
    Direct,
}

/// One entry of the room list.
#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct RoomSummary {
    /// `!…:server`; pass it to `Session.timeline`.
    pub id: String,
    pub name: String,
    /// `mxc://` URI, for `Session.avatar`. For a direct chat without its own
    /// picture, the other person's.
    pub avatar_url: Option<String>,
    pub topic: Option<String>,
    pub kind: RoomKind,
    /// We are invited but have not joined yet.
    pub invited: bool,
    pub encrypted: bool,
    pub unread_messages: u64,
    pub unread_mentions: u64,
    pub unread_notifications: u64,
    pub last_activity: Option<SystemTime>,
    /// For a direct chat, the other person's user ID.
    pub direct_user_id: Option<String>,
    /// The room's address, e.g. `#ice-sheets:example.org`.
    pub alias: Option<String>,
    pub joined_members: u64,
    /// The joined spaces that list it.
    #[uniffi(default)]
    pub parents: Vec<String>,
    /// Anyone may join.
    #[uniffi(default)]
    pub public: bool,
    /// Event IDs of its pinned messages, first pinned first.
    #[uniffi(default)]
    pub pinned: Vec<String>,
}

impl From<&core::RoomSummary> for RoomSummary {
    fn from(room: &core::RoomSummary) -> Self {
        Self {
            id: room.id.to_string(),
            name: room.name.clone(),
            avatar_url: room.avatar.as_ref().map(ToString::to_string),
            topic: room.topic.clone(),
            kind: match room.kind {
                core::RoomKind::Room => RoomKind::Room,
                core::RoomKind::Space => RoomKind::Space,
                core::RoomKind::Direct => RoomKind::Direct,
            },
            invited: room.invited,
            encrypted: room.encrypted,
            unread_messages: room.unread_messages,
            unread_mentions: room.unread_mentions,
            unread_notifications: room.unread_notifications,
            last_activity: room.last_activity,
            direct_user_id: room.direct_user.as_ref().map(ToString::to_string),
            alias: room.alias.clone(),
            joined_members: room.joined_members,
            parents: room.parents.iter().map(ToString::to_string).collect(),
            public: room.public,
            pinned: room.pinned.iter().map(ToString::to_string).collect(),
        }
    }
}

/// Someone's public profile.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct UserProfile {
    /// `@user:server`.
    pub user_id: String,
    /// Display name, if they set one.
    pub name: Option<String>,
    /// `mxc://` URI, for `Session.avatar`.
    pub avatar_url: Option<String>,
}

impl From<core::UserSummary> for UserProfile {
    fn from(user: core::UserSummary) -> Self {
        Self {
            user_id: user.id.to_string(),
            name: user.name,
            avatar_url: user.avatar.map(|uri| uri.to_string()),
        }
    }
}

/// The room list: every joined and invited room (spaces too), most recently
/// active first, and the rooms by space as the desktop shows them.
#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct RoomListState {
    pub rooms: Vec<RoomSummary>,
    /// Synced with the server since the list was created. Before that,
    /// `rooms` may be a stale cache, or empty on the first sign-in.
    pub loaded: bool,
    /// The desktop's space column: joined spaces in no joined space, in the
    /// order of `rooms`, each with the rooms it lists.
    #[uniffi(default)]
    pub spaces: Vec<SpaceRooms>,
    /// IDs of the rooms no space lists, in the order of `rooms`: rooms and
    /// invites to spaces; no direct chats, no joined spaces.
    #[uniffi(default)]
    pub rooms_outside_spaces: Vec<String>,
    /// The Notes to self stay at the top of the Chats tab and of each
    /// space's list (one choice for all the user's devices; set with
    /// `Session.setNotesAtTop`).
    #[uniffi(default = true)]
    pub notes_at_top: bool,
    /// The Notes to self's room, if there is one.
    #[uniffi(default)]
    pub notes_room_id: Option<String>,
    /// IDs of the rooms with a draft on this device (`Session.saveDraft`),
    /// marked with a pencil.
    #[uniffi(default)]
    pub drafts: Vec<String>,
}

/// What a section of a space's list holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum SectionKind {
    Invites,
    Favourites,
    /// One of the user's categories (made on the desktop).
    Category,
    /// Direct chats not in a category: "Direct messages".
    DirectMessages,
    /// Rooms not in a category.
    Rooms,
    LowPriority,
}

/// A section of a space's list, as the desktop arranges it.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct RoomSection {
    /// Stable across updates and devices: "invites", "favourites",
    /// "direct_messages", "rooms", "low_priority", or "category:<id>"; for
    /// remembering which sections are shown and folded.
    pub key: String,
    pub kind: SectionKind,
    /// A category's name; empty for the others, which the app names.
    pub name: String,
    /// In the order to show them.
    pub room_ids: Vec<String>,
}

/// A top-level space and what it lists.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct SpaceRooms {
    /// Its `RoomSummary.id`.
    pub space_id: String,
    /// IDs of the rooms it lists, in the order of `RoomListState.rooms`:
    /// rooms below it (through nested spaces, which are not listed
    /// themselves), invites to rooms and spaces below it, direct chats with
    /// its joined members, and rooms put into its categories on the desktop.
    pub room_ids: Vec<String>,
    /// The same rooms in the desktop's sections, in its order: every
    /// built-in section (even empty) and the space's categories.
    #[uniffi(default)]
    pub sections: Vec<RoomSection>,
}

impl RoomListState {
    /// `notes`: the Notes to self's room, if any.
    pub(crate) fn new(lists: &core::RoomLists, notes: Option<&core::OwnedRoomId>) -> Self {
        let ids = |ids: &[core::OwnedRoomId]| ids.iter().map(ToString::to_string).collect();
        let grouping = core::Grouping::default();
        let mut spaces = Vec::new();
        let mut outside = Vec::new();
        for entry in &lists.spaces {
            match &entry.key {
                core::SpaceKey::Space(space) => {
                    let key = core::SpaceKey::Space(space.clone());
                    let layout = lists.categories.layout(Some(space));
                    spaces.push(SpaceRooms {
                        space_id: space.to_string(),
                        room_ids: ids(&entry.rooms),
                        sections: (lists.sections(&key, &grouping).iter())
                            .map(|(section, rooms)| {
                                let rooms = rooms.iter().map(|r| r.id.to_string()).collect();
                                section_of(section, layout, rooms)
                            })
                            .collect(),
                    });
                }
                _ => outside = ids(&entry.rooms),
            }
        }
        Self {
            rooms: lists.rooms.iter().map(RoomSummary::from).collect(),
            loaded: lists.loaded,
            spaces,
            rooms_outside_spaces: outside,
            notes_at_top: lists.notes_at_top(),
            notes_room_id: notes.map(ToString::to_string),
            drafts: lists.drafts.iter().map(ToString::to_string).collect(),
        }
    }
}

fn section_of(
    section: &core::Section,
    layout: &core::Layout,
    room_ids: Vec<String>,
) -> RoomSection {
    let (kind, name) = match section {
        core::Section::Invites => (SectionKind::Invites, String::new()),
        core::Section::Favourites => (SectionKind::Favourites, String::new()),
        core::Section::People => (SectionKind::DirectMessages, String::new()),
        core::Section::Rooms => (SectionKind::Rooms, String::new()),
        core::Section::LowPriority => (SectionKind::LowPriority, String::new()),
        core::Section::Category(id) => (
            SectionKind::Category,
            layout
                .category(id)
                .map(|c| c.name.clone())
                .unwrap_or_default(),
        ),
    };
    RoomSection {
        key: section.key(),
        kind,
        name,
        room_ids,
    }
}

/// One row of a timeline.
#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct TimelineItem {
    /// Unique in the timeline and stable while the item stays (also when a
    /// sent message's local echo becomes the server's event): use it as the
    /// row's identity in lists.
    pub id: String,
    pub content: TimelineItemContent,
}

// Nearly all items are events, so boxing would only add indirection.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug, PartialEq, uniffi::Enum)]
pub enum TimelineItemContent {
    /// The room's beginning; back-pagination reached it.
    Start,
    Event {
        event: TimelineEvent,
    },
    /// The first event of a new day follows.
    DayDivider {
        date: SystemTime,
    },
    /// Everything below was not read yet.
    ReadMarker,
}

#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct TimelineEvent {
    /// The server's ID; `None` while a message we sent is not sent yet.
    pub event_id: Option<String>,
    pub sender_id: String,
    pub sender_name: Option<String>,
    /// `mxc://` URI, for `Session.avatar`.
    pub sender_avatar_url: Option<String>,
    pub timestamp: SystemTime,
    /// We sent it.
    pub own: bool,
    pub content: EventContent,
    pub send_state: SendState,
    /// We may edit it (our own text message): `Timeline.edit`.
    #[uniffi(default)]
    pub editable: bool,
    /// Others whose read receipt is on this event: they read up to here.
    /// Moved off state events to the message before, as the desktop shows
    /// them. Names and avatars arrive in a later update when not yet known.
    #[uniffi(default)]
    pub read_by: Vec<Reader>,
    /// For `EventContent.unableToDecrypt`, why, as far as known.
    #[uniffi(default)]
    pub utd_cause: Option<UtdCause>,
}

/// Why a message could not be decrypted (the SDK's best guess).
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum UtdCause {
    /// The key is missing, without a known reason: it may still arrive
    /// (from the key backup, or the sender's device).
    Unknown,
    /// Sent before we joined the room; its key was not shared with us.
    SentBeforeWeJoined,
    /// The sender's identity was verified before but has changed since.
    VerificationViolation,
    /// The sender's device is not signed by its owner.
    UnsignedDevice,
    /// The sender's device is unknown (deleted, say).
    UnknownDevice,
    /// Sent before this device existed, and the account has no key backup.
    HistoricalMessageAndBackupIsDisabled,
    /// Sent before this device existed; the key backup can be used once this
    /// device is verified.
    HistoricalMessageAndDeviceIsUnverified,
    /// The sender withheld the key as this device is not verified or
    /// insecure.
    WithheldForUnverifiedOrInsecureDevice,
    /// The sender did not, or could not, share the key with this device.
    WithheldBySender,
}

impl From<core::UtdCause> for UtdCause {
    fn from(cause: core::UtdCause) -> Self {
        match cause {
            core::UtdCause::Unknown => Self::Unknown,
            core::UtdCause::SentBeforeWeJoined => Self::SentBeforeWeJoined,
            core::UtdCause::VerificationViolation => Self::VerificationViolation,
            core::UtdCause::UnsignedDevice => Self::UnsignedDevice,
            core::UtdCause::UnknownDevice => Self::UnknownDevice,
            core::UtdCause::HistoricalMessageAndBackupIsDisabled => {
                Self::HistoricalMessageAndBackupIsDisabled
            }
            core::UtdCause::HistoricalMessageAndDeviceIsUnverified => {
                Self::HistoricalMessageAndDeviceIsUnverified
            }
            core::UtdCause::WithheldForUnverifiedOrInsecureDevice => {
                Self::WithheldForUnverifiedOrInsecureDevice
            }
            core::UtdCause::WithheldBySender => Self::WithheldBySender,
        }
    }
}

#[derive(Clone, Debug, PartialEq, uniffi::Enum)]
pub enum SendState {
    Sent,
    Sending,
    /// Not sent because the server is unreachable; sent once it is back.
    Waiting,
    /// The server refused it: `Timeline.retry` or `Timeline.discard` it.
    /// Later messages wait until then.
    Failed {
        error: String,
    },
}

/// Someone who read up to an event, or reacted to it.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct Reader {
    pub user_id: String,
    /// Display name, once known.
    pub name: Option<String>,
    /// `mxc://` URI, for `Session.avatar`.
    pub avatar_url: Option<String>,
}

/// Everyone who reacted to a message with one key.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct Reaction {
    /// Usually an emoji.
    pub key: String,
    pub count: u64,
    /// We reacted with it: `Timeline.react` with it removes ours.
    pub by_us: bool,
    /// Who reacted, us included, in the order they did.
    pub senders: Vec<Reader>,
}

/// The message a reply answers, as quoted above the reply.
#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct ReplyQuote {
    /// The original's event ID: `TimelineEvent.eventId` of its row once
    /// loaded in the timeline.
    pub event_id: String,
    pub state: QuoteState,
    /// The original's sender, once loaded.
    pub sender_id: Option<String>,
    pub sender_name: Option<String>,
    /// The original's kind once loaded, if it is a message (`None` if it was
    /// deleted, or is a poll, say).
    pub kind: Option<MessageKind>,
    /// Its text on one line, shortened, once loaded: for a file its name
    /// (or caption); "Message deleted" or "…" if it is no message.
    pub body: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum QuoteState {
    /// Being fetched from the server; the quote updates by itself.
    Loading,
    Loaded,
    /// Not available to us, e.g. sent before we joined, or no such event.
    Failed,
}

// Most events are messages, so boxing would only add indirection.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug, PartialEq, uniffi::Enum)]
pub enum EventContent {
    Message {
        message: Message,
    },
    /// Deleted.
    Redacted,
    /// Encrypted, and no key (yet). Keys may still arrive.
    UnableToDecrypt,
    Membership {
        user_id: String,
        name: Option<String>,
        change: MembershipChange,
    },
    DisplayNameChange {
        user_id: String,
        new_name: Option<String>,
    },
    /// A state event shown only by its type, e.g. `m.room.topic`.
    OtherState {
        event_type: String,
    },
    /// An event shown only by a description, e.g. `poll`.
    Other {
        description: String,
    },
}

#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct Message {
    pub kind: MessageKind,
    /// The text, without formatting. Math keeps its LaTeX source.
    pub body: String,
    pub edited: bool,
    /// The file of an image, video, audio or file message.
    pub media: Option<Media>,
    /// Reactions, in the order they were first given.
    #[uniffi(default)]
    pub reactions: Vec<Reaction>,
    /// The message this one answers.
    #[uniffi(default)]
    pub reply_to: Option<ReplyQuote>,
    /// The text as sent, unformatted: for our messages, the Markdown typed.
    /// An edit starts from it.
    #[uniffi(default)]
    pub raw_body: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum MessageKind {
    Text,
    Notice,
    Emote,
    Image,
    Video,
    Audio,
    File,
    Location,
    Other,
}

/// A file sent in a message. Pass it to `Session.media_preview` or
/// `Session.media_file`.
#[derive(Clone, Debug, PartialEq, Eq, Hash, uniffi::Record)]
pub struct Media {
    pub kind: MediaKind,
    pub filename: String,
    /// In bytes.
    pub size: Option<u64>,
    pub mimetype: Option<String>,
    /// In pixels, for images and videos: lay out before the image loads.
    pub width: Option<u32>,
    pub height: Option<u32>,
    /// Where the file is (and its decryption key): opaque, for the library.
    pub source: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, uniffi::Enum)]
pub enum MediaKind {
    Image,
    Video,
    Audio,
    File,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum MembershipChange {
    Joined,
    Left,
    Invited,
    InvitationAccepted,
    InvitationRejected,
    InvitationRevoked,
    Kicked,
    Banned,
    Unbanned,
    Knocked,
    Other,
}

/// The timeline as the apps see it, oldest first.
pub(crate) fn timeline_items(entries: &[core::TimelineEntry]) -> Vec<TimelineItem> {
    entries.iter().map(timeline_item).collect()
}

fn timeline_item(entry: &core::TimelineEntry) -> TimelineItem {
    match entry {
        core::TimelineEntry::Start => TimelineItem {
            id: "start".into(),
            content: TimelineItemContent::Start,
        },
        core::TimelineEntry::Event(event) => TimelineItem {
            id: event.id.clone(),
            content: TimelineItemContent::Event {
                event: TimelineEvent::from(event),
            },
        },
        core::TimelineEntry::DayDivider(date) => TimelineItem {
            // One per day.
            id: format!(
                "day-{}",
                date.duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis()
            ),
            content: TimelineItemContent::DayDivider { date: *date },
        },
        core::TimelineEntry::ReadMarker => TimelineItem {
            id: "read-marker".into(),
            content: TimelineItemContent::ReadMarker,
        },
    }
}

impl From<&core::Event> for TimelineEvent {
    fn from(event: &core::Event) -> Self {
        Self {
            event_id: event.event_id.as_ref().map(ToString::to_string),
            sender_id: event.sender.to_string(),
            sender_name: event.sender_name.clone(),
            sender_avatar_url: event.sender_avatar.as_ref().map(ToString::to_string),
            timestamp: event.timestamp,
            own: event.own,
            content: EventContent::from(&event.content),
            send_state: match &event.send_state {
                core::SendState::Sent => SendState::Sent,
                core::SendState::Sending => SendState::Sending,
                core::SendState::Waiting => SendState::Waiting,
                core::SendState::Failed(error) => SendState::Failed {
                    error: error.clone(),
                },
            },
            editable: event.editable,
            read_by: event.read_by.iter().map(Reader::from).collect(),
            utd_cause: event.utd_cause.map(UtdCause::from),
        }
    }
}

impl From<&core::Reader> for Reader {
    fn from(reader: &core::Reader) -> Self {
        Self {
            user_id: reader.id.to_string(),
            name: reader.name.clone(),
            avatar_url: reader.avatar.as_ref().map(ToString::to_string),
        }
    }
}

impl From<&core::Reaction> for Reaction {
    fn from(reaction: &core::Reaction) -> Self {
        Self {
            key: reaction.key.clone(),
            count: reaction.count as u64,
            by_us: reaction.own,
            senders: reaction.by.iter().map(Reader::from).collect(),
        }
    }
}

impl From<&core::Reply> for ReplyQuote {
    fn from(reply: &core::Reply) -> Self {
        Self {
            event_id: reply.event_id.to_string(),
            state: if reply.failed {
                QuoteState::Failed
            } else if reply.preview.is_some() {
                QuoteState::Loaded
            } else {
                QuoteState::Loading
            },
            sender_id: reply.sender.as_ref().map(ToString::to_string),
            sender_name: reply.sender_name.clone(),
            kind: reply.kind.map(message_kind),
            body: reply.preview.clone(),
        }
    }
}

impl From<&core::EventContent> for EventContent {
    fn from(content: &core::EventContent) -> Self {
        match content {
            core::EventContent::Message(message) => Self::Message {
                message: Message::from(message),
            },
            core::EventContent::Redacted => Self::Redacted,
            core::EventContent::UnableToDecrypt => Self::UnableToDecrypt,
            core::EventContent::Membership { user, name, change } => Self::Membership {
                user_id: user.to_string(),
                name: name.clone(),
                change: membership_change(*change),
            },
            core::EventContent::DisplayNameChange { user, new_name } => Self::DisplayNameChange {
                user_id: user.to_string(),
                new_name: new_name.clone(),
            },
            core::EventContent::CallLink { .. } => Self::OtherState {
                event_type: "org.mtrx.call_link".into(),
            },
            core::EventContent::Topic { .. } => Self::OtherState {
                event_type: "m.room.topic".into(),
            },
            core::EventContent::Pins(_) => Self::OtherState {
                event_type: "m.room.pinned_events".into(),
            },
            core::EventContent::OtherState { event_type } => Self::OtherState {
                event_type: event_type.clone(),
            },
            core::EventContent::Other { description } => Self::Other {
                description: description.clone(),
            },
        }
    }
}

impl From<&core::Message> for Message {
    fn from(message: &core::Message) -> Self {
        Self {
            kind: message_kind(message.kind),
            body: message.document.to_plain_text(),
            edited: message.edited,
            media: message.media.as_ref().map(Media::from),
            reactions: message.reactions.iter().map(Reaction::from).collect(),
            reply_to: message.in_reply_to.as_ref().map(ReplyQuote::from),
            raw_body: message.body.clone(),
        }
    }
}

fn message_kind(kind: core::MessageKind) -> MessageKind {
    match kind {
        core::MessageKind::Text => MessageKind::Text,
        core::MessageKind::Notice => MessageKind::Notice,
        core::MessageKind::Emote => MessageKind::Emote,
        core::MessageKind::Image => MessageKind::Image,
        core::MessageKind::Video => MessageKind::Video,
        core::MessageKind::Audio => MessageKind::Audio,
        core::MessageKind::File => MessageKind::File,
        core::MessageKind::Location => MessageKind::Location,
        core::MessageKind::Other => MessageKind::Other,
    }
}

impl From<&core::Media> for Media {
    fn from(media: &core::Media) -> Self {
        Self {
            kind: match media.kind {
                core::MediaKind::Image => MediaKind::Image,
                core::MediaKind::Video => MediaKind::Video,
                core::MediaKind::Audio => MediaKind::Audio,
                core::MediaKind::File => MediaKind::File,
            },
            filename: media.filename.clone(),
            size: media.size,
            mimetype: media.mimetype.clone(),
            width: media.width,
            height: media.height,
            source: serde_json::to_string(&media.source).expect("media sources serialize"),
        }
    }
}

impl Media {
    /// The core's reference to the file, from [`Media::source`].
    pub(crate) fn media_ref(&self) -> Result<core::MediaRef, crate::MtrxError> {
        serde_json::from_str(&self.source).map_err(crate::MtrxError::invalid)
    }
}

fn membership_change(change: core::MembershipChange) -> MembershipChange {
    match change {
        core::MembershipChange::Joined => MembershipChange::Joined,
        core::MembershipChange::Left => MembershipChange::Left,
        core::MembershipChange::Invited => MembershipChange::Invited,
        core::MembershipChange::InvitationAccepted => MembershipChange::InvitationAccepted,
        core::MembershipChange::InvitationRejected => MembershipChange::InvitationRejected,
        core::MembershipChange::InvitationRevoked => MembershipChange::InvitationRevoked,
        core::MembershipChange::Kicked => MembershipChange::Kicked,
        core::MembershipChange::Banned => MembershipChange::Banned,
        core::MembershipChange::Unbanned => MembershipChange::Unbanned,
        core::MembershipChange::Knocked => MembershipChange::Knocked,
        core::MembershipChange::Other => MembershipChange::Other,
    }
}

#[cfg(test)]
mod tests {
    use std::{
        collections::{HashMap, HashSet},
        time::Duration,
    };

    use mtrx_format::Document;

    use super::*;

    fn at(seconds: u64) -> SystemTime {
        UNIX_EPOCH + Duration::from_secs(seconds)
    }

    fn event(id: &str, content: core::EventContent) -> core::Event {
        core::Event {
            id: id.into(),
            key: core::EventKey::local(id),
            event_id: None,
            sender: "@alice:example.org".try_into().unwrap(),
            sender_name: Some("Alice".into()),
            sender_avatar: Some("mxc://example.org/alice".into()),
            timestamp: at(1_700_000_000),
            own: true,
            editable: true,
            content,
            send_state: core::SendState::Sending,
            read_by: Vec::new(),
            utd_cause: None,
        }
    }

    #[test]
    fn undecryptable_events_say_why() {
        let mut undecryptable = event("a", core::EventContent::UnableToDecrypt);
        undecryptable.utd_cause = Some(core::UtdCause::SentBeforeWeJoined);
        let entries = [
            core::TimelineEntry::Event(undecryptable),
            core::TimelineEntry::Event(event("b", core::EventContent::Redacted)),
        ];
        let causes: Vec<_> = timeline_items(&entries)
            .into_iter()
            .map(|item| match item.content {
                TimelineItemContent::Event { event } => (event.content, event.utd_cause),
                content => panic!("not an event: {content:?}"),
            })
            .collect();
        assert_eq!(
            causes,
            [
                (
                    EventContent::UnableToDecrypt,
                    Some(UtdCause::SentBeforeWeJoined)
                ),
                (EventContent::Redacted, None),
            ]
        );
    }

    #[test]
    fn profiles_keep_what_is_set() {
        let profile = UserProfile::from(core::UserSummary {
            id: "@bob:example.org".try_into().unwrap(),
            name: Some("Bob".into()),
            avatar: None,
        });
        assert_eq!(
            profile,
            UserProfile {
                user_id: "@bob:example.org".into(),
                name: Some("Bob".into()),
                avatar_url: None,
            }
        );
    }

    fn message(document: Document, media: Option<core::Media>) -> core::EventContent {
        core::EventContent::Message(core::Message {
            kind: if media.is_some() {
                core::MessageKind::Image
            } else {
                core::MessageKind::Text
            },
            body: "**raw** body".into(),
            document,
            edited: false,
            in_reply_to: None,
            reactions: Vec::new(),
            media,
        })
    }

    #[test]
    fn timeline_items_have_unique_ids() {
        let entries = vec![
            core::TimelineEntry::Start,
            core::TimelineEntry::DayDivider(at(86_400)),
            core::TimelineEntry::Event(event("a", core::EventContent::Redacted)),
            core::TimelineEntry::ReadMarker,
            core::TimelineEntry::DayDivider(at(2 * 86_400)),
            core::TimelineEntry::Event(event("b", core::EventContent::UnableToDecrypt)),
        ];
        let ids: Vec<_> = timeline_items(&entries).into_iter().map(|i| i.id).collect();
        assert_eq!(
            ids,
            [
                "start",
                "day-86400000",
                "a",
                "read-marker",
                "day-172800000",
                "b"
            ]
        );
    }

    #[test]
    fn events_carry_plain_ids_and_text() {
        let entries = [core::TimelineEntry::Event(event(
            "a",
            message(Document::from_html("<b>bold</b> text"), None),
        ))];
        let [item] = timeline_items(&entries).try_into().unwrap();
        let TimelineItemContent::Event { event } = item.content else {
            panic!("not an event: {:?}", item.content);
        };
        assert_eq!(event.sender_id, "@alice:example.org");
        assert_eq!(
            event.sender_avatar_url.as_deref(),
            Some("mxc://example.org/alice")
        );
        assert_eq!(event.timestamp, at(1_700_000_000));
        assert_eq!(event.send_state, SendState::Sending);
        assert_eq!(
            event.content,
            EventContent::Message {
                message: Message {
                    kind: MessageKind::Text,
                    body: "bold text".into(),
                    edited: false,
                    media: None,
                    reactions: Vec::new(),
                    reply_to: None,
                    raw_body: "**raw** body".into(),
                }
            }
        );
    }

    #[test]
    fn messages_carry_reactions_quotes_and_readers() {
        let bob = |name: Option<&str>| core::Reader {
            id: "@bob:example.org".try_into().unwrap(),
            name: name.map(Into::into),
            avatar: Some("mxc://example.org/bob".into()),
        };
        let reply = |preview: Option<&str>, failed| core::Reply {
            event_id: "$original:example.org".try_into().unwrap(),
            sender: preview.map(|_| "@bob:example.org".try_into().unwrap()),
            sender_name: preview.map(|_| "Bob".into()),
            preview: preview.map(Into::into),
            kind: preview.map(|_| core::MessageKind::Image),
            failed,
        };
        let mut event = event("a", message(Document::from_plain("hi"), None));
        event.editable = false;
        event.read_by = vec![bob(None)];
        let core::EventContent::Message(m) = &mut event.content else {
            unreachable!()
        };
        m.reactions = vec![core::Reaction {
            key: "👍".into(),
            count: 1,
            own: false,
            by: vec![bob(Some("Bob"))],
        }];
        m.in_reply_to = Some(reply(Some("ice.png"), false));

        let converted = TimelineEvent::from(&event);
        assert!(!converted.editable);
        let bob_reader = |name: Option<&str>| Reader {
            user_id: "@bob:example.org".into(),
            name: name.map(Into::into),
            avatar_url: Some("mxc://example.org/bob".into()),
        };
        assert_eq!(converted.read_by, [bob_reader(None)]);
        let EventContent::Message { message } = converted.content else {
            panic!("not a message");
        };
        assert_eq!(
            message.reactions,
            [Reaction {
                key: "👍".into(),
                count: 1,
                by_us: false,
                senders: vec![bob_reader(Some("Bob"))],
            }]
        );
        assert_eq!(
            message.reply_to,
            Some(ReplyQuote {
                event_id: "$original:example.org".into(),
                state: QuoteState::Loaded,
                sender_id: Some("@bob:example.org".into()),
                sender_name: Some("Bob".into()),
                kind: Some(MessageKind::Image),
                body: Some("ice.png".into()),
            })
        );

        let state = |reply| ReplyQuote::from(&reply).state;
        assert_eq!(state(reply(None, false)), QuoteState::Loading);
        assert_eq!(state(reply(None, true)), QuoteState::Failed);
    }

    #[test]
    fn media_sources_round_trip() {
        let core_media = core::Media {
            kind: core::MediaKind::Image,
            filename: "ice.png".into(),
            size: Some(1234),
            mimetype: Some("image/png".into()),
            width: Some(800),
            height: Some(600),
            source: core::MediaRef::plain("mxc://example.org/ice"),
        };
        let media = Media::from(&core_media);
        assert_eq!((media.width, media.height), (Some(800), Some(600)));
        assert_eq!(media.media_ref().unwrap(), core_media.source);
        assert_eq!(
            media.media_ref().unwrap().uri().as_str(),
            "mxc://example.org/ice"
        );

        let tampered = Media {
            source: "{}".into(),
            ..media
        };
        assert!(matches!(
            tampered.media_ref(),
            Err(crate::MtrxError::InvalidInput { .. })
        ));
    }

    #[test]
    fn room_summaries_use_plain_ids() {
        let room = core::RoomSummary {
            id: "!room:example.org".try_into().unwrap(),
            name: "Ice sheets".into(),
            avatar: None,
            topic: Some("Greenland".into()),
            kind: core::RoomKind::Direct,
            invited: false,
            encrypted: true,
            public: false,
            unread_messages: 3,
            unread_mentions: 1,
            unread_notifications: 2,
            last_activity: Some(at(5)),
            parents: Vec::new(),
            tag: core::RoomTag::None,
            direct_user: Some("@bob:example.org".try_into().unwrap()),
            alias: None,
            joined_members: 2,
            pinned: Vec::new(),
        };
        let space = core::RoomSummary {
            id: "!space:example.org".try_into().unwrap(),
            kind: core::RoomKind::Space,
            direct_user: None,
            ..room.clone()
        };
        let members = HashMap::from([(
            space.id.clone(),
            HashSet::from(["@bob:example.org".try_into().unwrap()]),
        )]);
        let lists = core::RoomLists::new(
            core::RoomListState {
                rooms: vec![room, space],
                loaded: true,
            },
            Default::default(),
            members,
            &core::OwnedUserId::try_from("@me:example.org").unwrap(),
            Default::default(),
        );
        let state = RoomListState::new(&lists, None);
        assert!(state.notes_at_top && state.notes_room_id.is_none());
        assert!(state.loaded);
        let summary = &state.rooms[0];
        assert_eq!(summary.id, "!room:example.org");
        assert_eq!(summary.kind, RoomKind::Direct);
        assert_eq!(summary.direct_user_id.as_deref(), Some("@bob:example.org"));
        assert_eq!(summary.last_activity, Some(at(5)));
        assert_eq!(summary.unread_mentions, 1);
        let space = &state.spaces[0];
        assert_eq!(space.space_id, "!space:example.org");
        assert_eq!(space.room_ids, ["!room:example.org"]);
        let direct = space
            .sections
            .iter()
            .find(|s| s.kind == SectionKind::DirectMessages);
        assert_eq!(
            direct,
            Some(&RoomSection {
                key: "direct_messages".into(),
                kind: SectionKind::DirectMessages,
                name: String::new(),
                room_ids: vec!["!room:example.org".into()],
            })
        );
    }

    #[test]
    fn connection_states_map_one_to_one() {
        assert_eq!(
            ConnectionState::from(core::Connection::SignedOut),
            ConnectionState::SignedOut
        );
        assert_eq!(
            ConnectionState::from(core::Connection::Offline),
            ConnectionState::Offline
        );
    }
}
