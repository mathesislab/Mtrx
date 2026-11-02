//! Live room timeline: messages, state changes, sending.

use std::{
    collections::{HashMap, HashSet},
    path::Path,
    sync::Arc,
    time::SystemTime,
};

use eyeball_im::Vector;
use futures_util::{Stream, StreamExt, stream};
use matrix_sdk::ruma::{
    EventId, OwnedEventId, OwnedMxcUri, OwnedRoomId, OwnedTransactionId, OwnedUserId, RoomId, UInt,
    UserId,
    api::client::receipt::create_receipt::v3::ReceiptType,
    events::{
        Mentions,
        relation::Reply as ReplyRelation,
        room::{
            MediaSource,
            message::{AddMentions, MessageType, Relation, RoomMessageEventContent},
        },
    },
};
use matrix_sdk::{
    Room, RoomMemberships,
    room::{
        edit::EditedContent,
        reply::{EnforceThread, Reply as SdkReply},
    },
    send_queue::{RoomSendQueueUpdate, SendHandle},
};
use matrix_sdk_ui::timeline::{
    EncryptedMessage, EventSendState, EventTimelineItem, InReplyToDetails,
    MembershipChange as SdkMembershipChange, MsgLikeKind, RoomExt, Timeline, TimelineDetails,
    TimelineEventItemId, TimelineItem, TimelineItemContent, TimelineItemKind, VirtualTimelineItem,
};
use tokio::{
    sync::{Mutex, broadcast, mpsc, watch},
    task::JoinHandle,
};
use tokio_stream::wrappers::{BroadcastStream, WatchStream};
use tracing::warn;

use crate::{
    Error, PinsChange, Result, Session, calls,
    media::{self, MediaRef},
    pins, room_links,
    rows::{self, Row, RowContext},
    system_time,
};
use mtrx_format::{Document, Mention, MentionTarget, markdown_to_matrix};

/// One row of a timeline.
// Nearly all entries are events, so boxing would only add indirection.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug, PartialEq)]
pub enum TimelineEntry {
    /// The first entry once back-pagination reached the room's creation.
    /// Gone again if the timeline is reset (e.g. after a gap in sync).
    Start,
    Event(Event),
    DayDivider(SystemTime),
    /// Everything below was not read yet.
    ReadMarker,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Event {
    /// Stable across the local echo becoming the remote event.
    pub id: String,
    /// Identifies the event for actions (reply, edit, react, delete).
    pub key: EventKey,
    pub event_id: Option<OwnedEventId>,
    pub sender: OwnedUserId,
    pub sender_name: Option<String>,
    pub sender_avatar: Option<OwnedMxcUri>,
    pub timestamp: SystemTime,
    pub own: bool,
    /// We may edit it (our own text message).
    pub editable: bool,
    pub content: EventContent,
    pub send_state: SendState,
    /// Others whose read receipt is on this event: they read up to here.
    pub read_by: Vec<Reader>,
    /// Why an [`EventContent::UnableToDecrypt`] could not be decrypted, as
    /// far as known; `None` for other content.
    pub utd_cause: Option<UtdCause>,
}

/// The SDK's best guess why an event could not be decrypted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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
    /// The sender withheld the key from this device as it is not verified
    /// or insecure.
    WithheldForUnverifiedOrInsecureDevice,
    /// The sender did not, or could not, share the key with this device.
    WithheldBySender,
}

impl From<matrix_sdk_crypto::types::events::UtdCause> for UtdCause {
    fn from(cause: matrix_sdk_crypto::types::events::UtdCause) -> Self {
        use matrix_sdk_crypto::types::events::UtdCause as Sdk;
        match cause {
            Sdk::Unknown => Self::Unknown,
            Sdk::SentBeforeWeJoined => Self::SentBeforeWeJoined,
            Sdk::VerificationViolation => Self::VerificationViolation,
            Sdk::UnsignedDevice => Self::UnsignedDevice,
            Sdk::UnknownDevice => Self::UnknownDevice,
            Sdk::HistoricalMessageAndBackupIsDisabled => Self::HistoricalMessageAndBackupIsDisabled,
            Sdk::HistoricalMessageAndDeviceIsUnverified => {
                Self::HistoricalMessageAndDeviceIsUnverified
            }
            Sdk::WithheldForUnverifiedOrInsecureDevice => {
                Self::WithheldForUnverifiedOrInsecureDevice
            }
            Sdk::WithheldBySender => Self::WithheldBySender,
        }
    }
}

/// Someone who read up to an event, or reacted to it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reader {
    pub id: OwnedUserId,
    /// Display name and avatar, once known.
    pub name: Option<String>,
    pub avatar: Option<OwnedMxcUri>,
}

/// Identifies a timeline event for actions on it: its event ID once the
/// server accepted it, also while the local echo still shows; before that,
/// the local echo's transaction ID.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct EventKey(TimelineEventItemId);

impl EventKey {
    /// The event ID, which still finds the event once the server's copy
    /// replaced the local echo: the transaction ID does not, as the
    /// server's copy does not always carry it.
    fn of(event: &EventTimelineItem) -> Self {
        Self(match event.event_id() {
            Some(id) => TimelineEventItemId::EventId(id.to_owned()),
            None => event.identifier(),
        })
    }
}

impl EventKey {
    /// The key of a local echo, for tests of frontends.
    #[cfg(feature = "test-support")]
    pub fn local(transaction_id: &str) -> Self {
        Self(TimelineEventItemId::TransactionId(transaction_id.into()))
    }
}

impl Event {
    /// A state change (membership, name, room settings), shown folded into
    /// a summary line.
    pub fn is_state(&self) -> bool {
        matches!(
            self.content,
            EventContent::Membership { .. }
                | EventContent::DisplayNameChange { .. }
                | EventContent::CallLink { .. }
                | EventContent::Topic { .. }
                | EventContent::Pins(_)
                | EventContent::OtherState { .. }
        )
    }
}

#[cfg(feature = "test-support")]
impl Event {
    /// A text message `id` from `sender`, sent, for tests.
    pub fn test_message(id: &str, sender: OwnedUserId, body: &str) -> Self {
        Self {
            id: id.to_owned(),
            key: EventKey(TimelineEventItemId::TransactionId(id.into())),
            event_id: None,
            sender,
            sender_name: None,
            sender_avatar: None,
            timestamp: SystemTime::UNIX_EPOCH,
            own: false,
            editable: false,
            content: EventContent::Message(Message {
                kind: MessageKind::Text,
                body: body.to_owned(),
                document: Document::from_plain(body),
                edited: false,
                in_reply_to: None,
                reactions: Vec::new(),
                media: None,
            }),
            send_state: SendState::Sent,
            read_by: Vec::new(),
            utd_cause: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum SendState {
    Sent,
    Sending,
    /// Not sent because the server is unreachable; sent once it is back.
    Waiting,
    Failed(String),
}

// Most events are messages, so boxing would only add indirection.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug, PartialEq)]
pub enum EventContent {
    Message(Message),
    Redacted,
    /// Encrypted and no key (yet). Keys may still arrive from backup or other devices.
    UnableToDecrypt,
    Membership {
        user: OwnedUserId,
        name: Option<String>,
        change: MembershipChange,
    },
    DisplayNameChange {
        user: OwnedUserId,
        new_name: Option<String>,
    },
    /// The room's old standing call link was set, or removed (`None`).
    CallLink {
        url: Option<String>,
    },
    /// The room's topic changed, from `old` if known: its text, or its
    /// calls and links (see [`crate::describe_change`]).
    Topic {
        old: Option<String>,
        new: String,
    },
    /// Messages were pinned or unpinned.
    Pins(PinsChange),
    /// A state event we do not render in detail, e.g. `m.room.name`.
    OtherState {
        event_type: String,
    },
    /// A message-like event we do not render, e.g. a poll or sticker.
    Other {
        description: String,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct Message {
    pub kind: MessageKind,
    /// Plain-text body (fallback for every kind).
    pub body: String,
    /// The formatted body, or the plain body if there is none. Math keeps its
    /// LaTeX source.
    pub document: Document,
    pub edited: bool,
    pub in_reply_to: Option<Reply>,
    pub reactions: Vec<Reaction>,
    /// The file of an image, video, audio or file message.
    pub media: Option<Media>,
}

/// A file sent in a message.
#[derive(Clone, Debug, PartialEq)]
pub struct Media {
    pub kind: MediaKind,
    pub filename: String,
    /// In bytes.
    pub size: Option<u64>,
    pub mimetype: Option<String>,
    /// In pixels, for images and videos.
    pub width: Option<u32>,
    pub height: Option<u32>,
    /// For downloading it, see [`Session::media_content`].
    pub source: MediaRef,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MediaKind {
    Image,
    Video,
    Audio,
    File,
}

/// The message a reply answers.
#[derive(Clone, Debug, PartialEq)]
pub struct Reply {
    pub event_id: OwnedEventId,
    /// `None` until loaded.
    pub sender: Option<OwnedUserId>,
    pub sender_name: Option<String>,
    /// Its text, shortened; `None` until loaded.
    pub preview: Option<String>,
    /// Its kind once loaded, if it is a message (not deleted, say).
    pub kind: Option<MessageKind>,
    /// It could not be loaded, e.g. as it is not visible to us.
    pub failed: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reaction {
    /// Usually an emoji.
    pub key: String,
    pub count: usize,
    /// We reacted with it.
    pub own: bool,
    /// Everyone who reacted with it, us included, in the order they did.
    pub by: Vec<Reader>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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

/// A room's timeline, kept up to date while it is alive.
pub struct RoomTimeline {
    room_id: OwnedRoomId,
    timeline: Arc<Timeline>,
    send_lock: Arc<Mutex<()>>,
    entries: watch::Receiver<Vec<TimelineEntry>>,
    task: JoinHandle<()>,
    /// What the rows depend on besides the entries.
    context: watch::Receiver<RowContext>,
    context_task: JoinHandle<()>,
}

impl Drop for RoomTimeline {
    fn drop(&mut self) {
        self.task.abort();
        self.context_task.abort();
    }
}

impl RoomTimeline {
    pub fn room_id(&self) -> &OwnedRoomId {
        &self.room_id
    }

    /// Oldest first.
    pub fn current(&self) -> Vec<TimelineEntry> {
        self.entries.borrow().clone()
    }

    /// The current entries, then every change.
    pub fn updates(&self) -> impl Stream<Item = Vec<TimelineEntry>> + use<> {
        WatchStream::new(self.entries.clone())
    }

    /// The rows showing the entries now (see [`rows::rows`]), oldest first.
    pub fn current_rows(&self) -> Vec<Row> {
        rows::rows(&self.entries.borrow(), &self.context.borrow())
    }

    /// The current rows, then after every change: of the entries, the
    /// pinned messages, whether we may pin, and whether keys are arriving.
    pub fn rows(&self) -> impl Stream<Item = Arc<Vec<Row>>> + use<> {
        let (mut entries, context) = (self.entries.clone(), self.context.clone());
        entries.mark_changed();
        stream::unfold(
            (entries, context),
            |(mut entries, mut context)| async move {
                tokio::select! {
                    changed = entries.changed() => changed.ok()?,
                    // Once it ended, the context stays as it is.
                    Ok(()) = context.changed() => {}
                }
                let rows = rows::rows(&entries.borrow_and_update(), &context.borrow_and_update());
                Some((Arc::new(rows), (entries, context)))
            },
        )
    }

    /// The files being uploaded for messages sent here from now on, after
    /// each step of any; empty once all are sent, or failed.
    pub async fn uploads(&self) -> Result<impl Stream<Item = Vec<Upload>> + use<>> {
        let (_, updates) = (self.timeline.room().send_queue())
            .subscribe()
            .await
            .map_err(timeline_error)?;
        Ok(BroadcastStream::new(updates)
            .scan(Vec::<Upload>::new(), |uploads, update| {
                let changed = match update {
                    Ok(RoomSendQueueUpdate::MediaUpload {
                        related_to,
                        progress,
                        ..
                    }) => {
                        let message = EventKey(TimelineEventItemId::TransactionId(related_to));
                        uploads.retain(|u| u.message != message);
                        uploads.push(Upload {
                            message,
                            sent: progress.current,
                            total: progress.total,
                        });
                        true
                    }
                    Ok(
                        RoomSendQueueUpdate::SentEvent { transaction_id, .. }
                        | RoomSendQueueUpdate::SendError { transaction_id, .. }
                        | RoomSendQueueUpdate::CancelledLocalEvent { transaction_id },
                    ) => {
                        let message = EventKey(TimelineEventItemId::TransactionId(transaction_id));
                        let before = uploads.len();
                        uploads.retain(|u| u.message != message);
                        uploads.len() != before
                    }
                    _ => false,
                };
                std::future::ready(Some(changed.then(|| uploads.clone())))
            })
            .filter_map(std::future::ready))
    }

    /// Load up to `events` older events. Returns `true` once the start of the
    /// room is reached.
    pub async fn paginate_back(&self, events: u16) -> Result<bool> {
        self.timeline
            .paginate_backwards(events)
            .await
            .map_err(timeline_error)
    }

    /// Queue a message written in Markdown, with `$…$` / `$$…$$` math. Its local echo shows as
    /// [`SendState::Sending`] until the server confirms; await
    /// [`PendingSend::sent`] to wait for that.
    pub async fn send_markdown(
        &self,
        text: &str,
        reply_to: Option<&EventId>,
    ) -> Result<PendingSend> {
        self.send(text, Vec::new(), reply_to).await
    }

    /// Queue a message: its text (Markdown), then each file as a message of
    /// its own, in that order. Either may be empty, not both. The reply goes
    /// on the first. Returns the last, which is sent once all are.
    pub async fn send(
        &self,
        text: &str,
        files: Vec<Attachment>,
        reply_to: Option<&EventId>,
    ) -> Result<PendingSend> {
        send_message(self.timeline.room(), &self.send_lock, text, files, reply_to).await
    }

    /// Replace the text of one of our messages.
    pub async fn edit_markdown(&self, event: &EventKey, text: &str) -> Result<()> {
        let content = markdown_content(self.timeline.room(), text).await;
        self.timeline
            .edit(&event.0, EditedContent::RoomMessage(content.into()))
            .await
            .map_err(timeline_error)
    }

    /// Add our reaction `key` (an emoji) to the event, or remove it.
    pub async fn toggle_reaction(&self, event: &EventKey, key: &str) -> Result<()> {
        self.timeline
            .toggle_reaction(&event.0, key)
            .await
            .map(|_| ())
            .map_err(timeline_error)
    }

    /// Delete (redact) an event.
    pub async fn redact(&self, event: &EventKey) -> Result<()> {
        self.timeline
            .redact(&event.0, None)
            .await
            .map_err(timeline_error)
    }

    /// Send again one of our messages whose sending failed. A message sent
    /// meanwhile keeps its handle until the server's copy replaces it, and
    /// is not sent twice.
    pub async fn retry(&self, event: &EventKey) -> Result<()> {
        let item = self.local_echo(event).await?;
        if !matches!(
            item.send_state(),
            Some(EventSendState::SendingFailed { .. })
        ) {
            return Err(Error::Timeline(
                "only a message whose sending failed can be sent again".into(),
            ));
        }
        send_handle(&item)?
            .unwedge()
            .await
            .map_err(timeline_error)?;
        self.resume_sending();
        Ok(())
    }

    /// Drop one of our messages not sent (yet), e.g. after sending failed.
    /// The messages queued after it are then sent.
    pub async fn discard(&self, event: &EventKey) -> Result<()> {
        let dropped = send_handle(&self.local_echo(event).await?)?
            .abort()
            .await
            .map_err(timeline_error)?;
        if !dropped {
            return Err(Error::Timeline("the message was sent already".into()));
        }
        self.resume_sending();
        Ok(())
    }

    /// The room's send queue stops after any failed send, also one that
    /// cannot succeed; the connection watch resumes it only after network
    /// errors.
    fn resume_sending(&self) {
        self.timeline.room().send_queue().set_enabled(true);
    }

    /// Our message `event` while it is a local echo.
    async fn local_echo(&self, event: &EventKey) -> Result<EventTimelineItem> {
        self.timeline
            .items()
            .await
            .iter()
            .rev()
            .filter_map(|item| item.as_event())
            .find(|item| EventKey::of(item) == *event)
            .filter(|item| item.is_local_echo())
            .cloned()
            .ok_or_else(|| Error::Timeline("not a message being sent".into()))
    }

    /// Tell others we are typing, or stopped. The SDK sends at most one
    /// notice every few seconds.
    pub async fn typing(&self, typing: bool) -> Result<()> {
        Ok(self.timeline.room().typing_notice(typing).await?)
    }

    /// Names of the others typing, then every change.
    pub fn typing_updates(&self) -> impl Stream<Item = Vec<String>> + use<> {
        let room = self.timeline.room().clone();
        let own = room.own_user_id().to_owned();
        let (guard, receiver) = room.subscribe_to_typing_notifications();
        // The handler stays registered only while `guard` lives.
        stream::unfold(
            (guard, BroadcastStream::new(receiver), room),
            move |(guard, mut users, room)| {
                let own = own.clone();
                async move {
                    let ids = loop {
                        match users.next().await? {
                            Ok(ids) => break ids,
                            Err(_lagged) => continue,
                        }
                    };
                    let mut names = Vec::new();
                    for id in ids.into_iter().filter(|id| *id != own) {
                        let member = room.get_member_no_sync(&id).await.ok().flatten();
                        names.push(
                            member
                                .and_then(|m| m.display_name().map(ToOwned::to_owned))
                                .unwrap_or_else(|| id.localpart().to_owned()),
                        );
                    }
                    Some((names, (guard, users, room)))
                }
            },
        )
    }

    /// Mark everything up to the latest event as read.
    pub async fn mark_read(&self) -> Result<()> {
        self.timeline.mark_as_read(ReceiptType::Read).await?;
        Ok(())
    }
}

/// A file to send.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Attachment {
    pub filename: String,
    pub mimetype: String,
    pub bytes: Vec<u8>,
}

impl Attachment {
    /// The file at `path`, its MIME type guessed from its name.
    pub async fn read(path: &Path) -> Result<Self> {
        let filename = path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| Error::Timeline(format!("not a file: {}", path.display())))?;
        Ok(Self {
            filename: filename.to_owned(),
            mimetype: media::guess_mimetype(filename).to_string(),
            bytes: tokio::fs::read(path).await?,
        })
    }
}

/// Queue a message in `room`'s send queue, which persists it and sends it
/// in the background; timelines show it as a local echo. The text goes
/// first, then each file as a message of its own; see [`RoomTimeline::send`].
///
/// The SDK's send handle does not expose the transaction ID, so we take it
/// from the new local echoes after subscribing. `send_lock`, shared by all
/// sends of a session, makes sure no other send enqueues in between.
pub(crate) async fn send_message(
    room: &Room,
    send_lock: &Mutex<()>,
    text: &str,
    files: Vec<Attachment>,
    reply_to: Option<&EventId>,
) -> Result<PendingSend> {
    let text = (!text.trim().is_empty()).then_some(text);
    if text.is_none() && files.is_empty() {
        return Err(Error::Timeline("nothing to send".into()));
    }
    // Images are decoded for their size and thumbnail, before queueing.
    let mut prepared = Vec::with_capacity(files.len());
    for file in files {
        let mimetype: mime_guess::Mime = file
            .mimetype
            .parse()
            .map_err(|_| Error::InvalidMediaType(file.mimetype.clone()))?;
        prepared.push(
            tokio::task::spawn_blocking(move || {
                let config = media::attachment_config(&mimetype, &file.bytes);
                (file.filename, mimetype, file.bytes, config)
            })
            .await
            .map_err(timeline_error)?,
        );
    }
    let mut reply_to = reply_to.map(ToOwned::to_owned);
    let content = match text {
        Some(text) => Some(markdown_content(room, text).await),
        None => None,
    };

    let _guard = send_lock.lock().await;
    let queue = room.send_queue();
    let (_, mut updates) = queue.subscribe().await.map_err(timeline_error)?;
    let mut queued = 0;
    if let Some(mut content) = content {
        if let Some(event_id) = reply_to.take() {
            // No reply fallback in the body: Matrix 1.13 dropped it.
            content.relates_to = Some(Relation::Reply(ReplyRelation::with_event_id(event_id)));
        }
        queue.send(content.into()).await.map_err(timeline_error)?;
        queued += 1;
    }
    for (filename, mimetype, bytes, config) in prepared {
        let reply = reply_to.take().map(|event_id| SdkReply {
            event_id,
            enforce_thread: EnforceThread::Unthreaded,
            add_mentions: AddMentions::No,
        });
        queue
            .send_attachment(filename, mimetype, bytes, config.reply(reply))
            .await
            .map_err(timeline_error)?;
        queued += 1;
    }
    let mut last = None;
    while queued > 0 {
        match updates.try_recv() {
            Ok(RoomSendQueueUpdate::NewLocalEvent(echo)) => {
                last = Some(echo.transaction_id);
                queued -= 1;
            }
            Ok(_) => {}
            Err(e) => return Err(Error::Timeline(format!("no local echo after send: {e}"))),
        }
    }
    Ok(PendingSend {
        transaction_id: last.expect("something was queued"),
        updates,
    })
}

/// `text` as a message to `room`: Markdown, with `@` mentions of its
/// members and of the room in `m.mentions`.
///
/// Mentions resolve against the members in the store, so a message never
/// waits on the server before its local echo shows. Only text with an `@`
/// in a room whose member list was never loaded loads it first; when that
/// fails (e.g. offline), the store's members still resolve mentions.
async fn markdown_content(room: &Room, text: &str) -> RoomMessageEventContent {
    let memberships = RoomMemberships::JOIN | RoomMemberships::INVITE;
    let loaded = if text.contains('@') && !room.are_members_synced() {
        room.members(memberships)
            .await
            .inspect_err(|e| warn!("member list not loaded for mentions: {e}"))
            .ok()
    } else {
        None
    };
    let members = match loaded {
        Some(members) => members,
        None => room.members_no_sync(memberships).await.unwrap_or_default(),
    };
    let (targets, users): (Vec<_>, Vec<_>) =
        mention_targets(members.iter().map(|m| (m.user_id(), m.display_name())))
            .into_iter()
            .unzip();
    let formatted = markdown_to_matrix(text, &targets);
    let mut content = match formatted.html {
        Some(html) => RoomMessageEventContent::text_html(formatted.body, html),
        None => RoomMessageEventContent::text_plain(formatted.body),
    };
    if !formatted.mentions.is_empty() {
        let mut mentions = Mentions::new();
        for index in formatted.mentions {
            match &users[index] {
                Some(user) => {
                    mentions.user_ids.insert(user.clone());
                }
                None => mentions.room = true,
            }
        }
        content.mentions = Some(mentions);
    }
    content
}

/// What `@` can mention among a room's `members` (ID and display name
/// each): each member by ID, and by display name when no other member has
/// it; and the room. With the user each target mentions, `None` for the
/// room.
pub fn mention_targets<'a>(
    members: impl IntoIterator<Item = (&'a UserId, Option<&'a str>)>,
) -> Vec<(MentionTarget, Option<OwnedUserId>)> {
    let members: Vec<_> = members.into_iter().collect();
    let mut names: HashMap<&str, usize> = HashMap::new();
    for name in members.iter().filter_map(|(_, name)| *name) {
        *names.entry(name).or_default() += 1;
    }
    let mut targets = Vec::new();
    for (id, name) in members {
        let user = |text: &str| {
            let mention = Mention::User {
                href: id.matrix_to_uri().to_string(),
                name: name.map_or_else(|| id.to_string(), ToOwned::to_owned),
            };
            let text = text.to_owned();
            (MentionTarget { text, mention }, Some(id.to_owned()))
        };
        targets.push(user(&id.as_str()[1..]));
        if let Some(name) = name.filter(|n| names[n] == 1) {
            targets.push(user(name));
        }
    }
    targets.push((
        MentionTarget {
            text: "room".into(),
            mention: Mention::Room,
        },
        None,
    ));
    targets
}

/// A file being uploaded for a message being sent.
#[derive(Clone, Debug, PartialEq)]
pub struct Upload {
    /// The message, by its local echo's key.
    pub message: EventKey,
    /// Bytes sent so far, and in all.
    pub sent: usize,
    pub total: usize,
}

/// A message in the send queue.
pub struct PendingSend {
    transaction_id: OwnedTransactionId,
    updates: broadcast::Receiver<RoomSendQueueUpdate>,
}

impl PendingSend {
    /// Wait until the server accepted the message, also through times the
    /// server is unreachable.
    pub async fn sent(mut self) -> Result<OwnedEventId> {
        loop {
            match self.updates.recv().await {
                Ok(RoomSendQueueUpdate::SentEvent {
                    transaction_id,
                    event_id,
                }) if transaction_id == self.transaction_id => {
                    return Ok(event_id);
                }
                Ok(RoomSendQueueUpdate::SendError {
                    transaction_id,
                    error,
                    is_recoverable: false,
                }) if transaction_id == self.transaction_id => {
                    return Err(Error::Timeline(format!("sending failed: {error}")));
                }
                Ok(RoomSendQueueUpdate::CancelledLocalEvent { transaction_id })
                    if transaction_id == self.transaction_id =>
                {
                    return Err(Error::Timeline("sending was cancelled".into()));
                }
                Ok(_) | Err(broadcast::error::RecvError::Lagged(_)) => {}
                Err(broadcast::error::RecvError::Closed) => {
                    return Err(Error::Timeline("send queue closed".into()));
                }
            }
        }
    }
}

impl Session {
    pub async fn timeline(&self, room_id: &OwnedRoomId) -> Result<RoomTimeline> {
        if self.room(room_id)?.encryption_state().is_encrypted() {
            let (client, keys, room) =
                (self.client.clone(), self.room_keys.clone(), room_id.clone());
            tokio::spawn(async move { keys.download(&client, &room).await });
        }
        // Files sent report their upload's progress (see
        // [`RoomTimeline::uploads`]).
        self.client.send_queue().enable_upload_progress(true);
        // The open room is subscribed: sync then brings all of its state,
        // its pinned messages too, which the room list leaves out.
        (self.sync.room_list_service())
            .set_room_subscriptions(&[room_id])
            .await;
        let timeline = self
            .room(room_id)?
            .timeline()
            .await
            .map_err(timeline_error)?;
        Ok(self.room_timeline(room_id, timeline).await)
    }

    /// `timeline` of the room `room_id`, its items converted to entries as
    /// they come.
    pub(crate) async fn room_timeline(&self, room_id: &RoomId, timeline: Timeline) -> RoomTimeline {
        let timeline = Arc::new(timeline);
        let (items, stream) = timeline.subscribe().await;
        // Convert each item once, when it is added or changed, by mirroring
        // the SDK's diffs onto converted entries.
        let mut converter = Converter {
            own: self.user_id(),
            missing_replies: Vec::new(),
            profiles: HashMap::new(),
            missing_profiles: Vec::new(),
        };
        let mut items = items;
        let mut converted: Vector<Option<TimelineEntry>> =
            items.iter().map(|item| converter.convert(item)).collect();
        let (tx, entries) = watch::channel(flatten(&converted));
        let mut stream = Box::pin(stream);
        let (profile_tx, mut profile_rx) = mpsc::unbounded_channel();
        let task = tokio::spawn({
            let timeline = timeline.clone();
            async move {
                // Readers not seen as senders are fetched once. The original
                // of a reply is fetched whenever it shows as not loaded: the
                // SDK marks it pending meanwhile, and an item it makes anew
                // may lack it again.
                let mut requested_profiles = HashSet::new();
                loop {
                    converter.missing_replies.sort();
                    converter.missing_replies.dedup();
                    for event_id in converter.missing_replies.drain(..) {
                        let timeline = timeline.clone();
                        tokio::spawn(async move {
                            if let Err(e) = timeline.fetch_details_for_event(&event_id).await {
                                warn!("could not load the replied-to message: {e}");
                            }
                        });
                    }
                    for user in converter.missing_profiles.drain(..) {
                        if requested_profiles.insert(user.clone()) {
                            let (room, profile_tx) = (timeline.room().clone(), profile_tx.clone());
                            tokio::spawn(async move {
                                let profile = member_profile(&room, &user).await;
                                profile_tx.send((user, profile)).ok();
                            });
                        }
                    }
                    tokio::select! {
                        diffs = stream.next() => {
                            let Some(diffs) = diffs else {
                                return;
                            };
                            for diff in diffs {
                                diff.clone().apply(&mut items);
                                diff.map(|item| converter.convert(&item))
                                    .apply(&mut converted);
                            }
                        }
                        Some((user, profile)) = profile_rx.recv() => {
                            converter.profiles.insert(user.clone(), profile);
                            // Redraw the events this person read up to or
                            // reacted to.
                            for (index, item) in items.iter().enumerate() {
                                let shown = item.as_event().is_some_and(|e| {
                                    e.read_receipts().contains_key(&user)
                                        || e.content().reactions().is_some_and(|reactions| {
                                            reactions.values().any(|by| by.contains_key(&user))
                                        })
                                });
                                if shown {
                                    converted.set(index, converter.convert(item));
                                }
                            }
                        }
                    }
                    tx.send_replace(flatten(&converted));
                }
            }
        });
        let room = timeline.room().clone();
        let keys_arriving = (self.encryption_state().await).is_ok_and(|state| state.keys_arriving);
        let (context_tx, context) = watch::channel(row_context(&room, keys_arriving).await);
        let keys = self.encryption_states().map(|state| state.keys_arriving);
        let context_task = tokio::spawn(follow_row_context(room, keys, context_tx));
        RoomTimeline {
            room_id: room_id.to_owned(),
            timeline,
            send_lock: self.send_lock.clone(),
            entries,
            task,
            context,
            context_task,
        }
    }
}

/// What `room`'s rows depend on besides its entries, now.
async fn row_context(room: &Room, keys_arriving: bool) -> RowContext {
    RowContext {
        direct: room.is_direct().await.unwrap_or(false),
        keys_arriving,
        can_pin: pins::can_pin(room).await.unwrap_or(false),
        pinned: room.pinned_event_ids().unwrap_or_default(),
    }
}

/// Keep `context` up to date as `room` changes (its pinned messages, power
/// levels, being a direct chat) and as keys start or stop arriving.
async fn follow_row_context(
    room: Room,
    keys_arriving: impl Stream<Item = bool>,
    context: watch::Sender<RowContext>,
) {
    let mut info = room.subscribe_info();
    let mut keys_arriving = std::pin::pin!(keys_arriving);
    let mut arriving = context.borrow().keys_arriving;
    loop {
        tokio::select! {
            Some(_) = info.next() => {}
            Some(now) = keys_arriving.next() => arriving = now,
            else => return,
        }
        let now = row_context(&room, arriving).await;
        context.send_if_modified(|current| {
            let changed = *current != now;
            *current = now;
            changed
        });
    }
}

/// A member's name and avatar from the store, else from their profile.
async fn member_profile(room: &Room, user: &OwnedUserId) -> Profile {
    if let Ok(Some(member)) = room.get_member_no_sync(user).await {
        return (
            member.display_name().map(ToOwned::to_owned),
            member.avatar_url().map(ToOwned::to_owned),
        );
    }
    match room.client().account().fetch_user_profile_of(user).await {
        Ok(profile) => (
            profile
                .get_static::<matrix_sdk::ruma::api::client::profile::DisplayName>()
                .ok()
                .flatten(),
            profile
                .get_static::<matrix_sdk::ruma::api::client::profile::AvatarUrl>()
                .ok()
                .flatten(),
        ),
        Err(e) => {
            warn!("could not load the profile of {user}: {e}");
            (None, None)
        }
    }
}

/// The send queue's handle of a local echo.
fn send_handle(item: &EventTimelineItem) -> Result<SendHandle> {
    item.local_echo_send_handle()
        .ok_or_else(|| Error::Timeline("not a message being sent".into()))
}

pub(crate) fn timeline_error(e: impl std::fmt::Display) -> Error {
    Error::Timeline(e.to_string())
}

fn flatten(entries: &Vector<Option<TimelineEntry>>) -> Vec<TimelineEntry> {
    let mut entries: Vec<_> = entries.iter().flatten().cloned().collect();
    receipts_off_state(&mut entries);
    entries
}

/// Readers whose receipt is on a state event move to the nearest earlier
/// event that is not one: state events show as summary lines without
/// readers, and the readers did read that event. Without an earlier one
/// they stay.
fn receipts_off_state(entries: &mut [TimelineEntry]) {
    let mut target = None;
    for index in 0..entries.len() {
        let TimelineEntry::Event(event) = &mut entries[index] else {
            continue;
        };
        if !event.is_state() {
            target = Some(index);
            continue;
        }
        let Some(target) = target.filter(|_| !event.read_by.is_empty()) else {
            continue;
        };
        let readers = std::mem::take(&mut event.read_by);
        let TimelineEntry::Event(target) = &mut entries[target] else {
            unreachable!("the target is an event");
        };
        for reader in readers {
            if !target.read_by.iter().any(|r| r.id == reader.id) {
                target.read_by.push(reader);
            }
        }
    }
}

/// Converts SDK timeline items, noting replies whose original must be fetched.
struct Converter {
    own: OwnedUserId,
    /// Replies whose original is not loaded.
    missing_replies: Vec<OwnedEventId>,
    /// Names and avatars of senders, readers and those who reacted.
    profiles: HashMap<OwnedUserId, Profile>,
    /// Readers and those who reacted whose profile is not known.
    missing_profiles: Vec<OwnedUserId>,
}

/// Display name and avatar.
type Profile = (Option<String>, Option<OwnedMxcUri>);

impl Converter {
    fn convert(&mut self, item: &TimelineItem) -> Option<TimelineEntry> {
        match item.kind() {
            TimelineItemKind::Event(event) => Some(TimelineEntry::Event(
                self.convert_event(event, &item.unique_id().0)?,
            )),
            TimelineItemKind::Virtual(VirtualTimelineItem::DateDivider(ts)) => {
                Some(TimelineEntry::DayDivider(system_time(*ts)))
            }
            TimelineItemKind::Virtual(VirtualTimelineItem::ReadMarker) => {
                Some(TimelineEntry::ReadMarker)
            }
            TimelineItemKind::Virtual(VirtualTimelineItem::TimelineStart) => {
                Some(TimelineEntry::Start)
            }
        }
    }

    fn convert_event(&mut self, event: &EventTimelineItem, id: &str) -> Option<Event> {
        let mut utd_cause = None;
        let content = match event.content() {
            TimelineItemContent::MsgLike(msg_like) => match &msg_like.kind {
                MsgLikeKind::Message(message) => EventContent::Message(Message {
                    kind: message_kind(message.msgtype()),
                    body: message.body().to_owned(),
                    document: match html_body(message.msgtype()) {
                        Some(html) => Document::from_html(html),
                        None => Document::from_plain(message.body()),
                    },
                    edited: message.is_edited(),
                    in_reply_to: msg_like
                        .in_reply_to
                        .as_ref()
                        .map(|r| self.reply(r, event.event_id())),
                    reactions: msg_like
                        .reactions
                        .iter()
                        .map(|(key, senders)| Reaction {
                            key: key.clone(),
                            count: senders.len(),
                            own: senders.contains_key(&self.own),
                            by: senders.keys().map(|user| self.reader(user)).collect(),
                        })
                        .collect(),
                    media: media(message.msgtype()),
                }),
                MsgLikeKind::Redacted => EventContent::Redacted,
                MsgLikeKind::UnableToDecrypt(encrypted) => {
                    utd_cause = Some(match encrypted {
                        EncryptedMessage::MegolmV1AesSha2 { cause, .. } => (*cause).into(),
                        _ => UtdCause::Unknown,
                    });
                    EventContent::UnableToDecrypt
                }
                MsgLikeKind::Sticker(_) => EventContent::Other {
                    description: "sticker".into(),
                },
                MsgLikeKind::Poll(_) => EventContent::Other {
                    description: "poll".into(),
                },
                MsgLikeKind::LiveLocation(_) => EventContent::Other {
                    description: "live location".into(),
                },
                MsgLikeKind::Other(other) => EventContent::Other {
                    description: other.event_type().to_string(),
                },
            },
            TimelineItemContent::MembershipChange(change) => EventContent::Membership {
                user: change.user_id().to_owned(),
                name: change.display_name(),
                change: change.change().map(membership_change)?,
            },
            TimelineItemContent::ProfileChange(change) => EventContent::DisplayNameChange {
                user: change.user_id().to_owned(),
                new_name: change.displayname_change()?.new.clone(),
            },
            TimelineItemContent::OtherState(state) => {
                let event_type = state.content().event_type();
                let raw = event.original_json();
                if let Some(url) = calls::call_link_change(&event_type, raw) {
                    EventContent::CallLink { url }
                } else if let Some((old, new)) = room_links::topic_change(&event_type, raw) {
                    EventContent::Topic { old, new }
                } else if let Some(change) = pins::pins_change(&event_type, raw) {
                    EventContent::Pins(change)
                } else {
                    EventContent::OtherState {
                        event_type: event_type.to_string(),
                    }
                }
            }
            _ => return None,
        };

        let send_state = match event.send_state() {
            None | Some(EventSendState::Sent { .. }) => SendState::Sent,
            Some(EventSendState::NotSentYet { .. }) => SendState::Sending,
            Some(EventSendState::SendingFailed {
                is_recoverable: true,
                ..
            }) => SendState::Waiting,
            Some(EventSendState::SendingFailed { error, .. }) => {
                SendState::Failed(error.to_string())
            }
        };
        let (sender_name, sender_avatar) = match event.sender_profile() {
            TimelineDetails::Ready(profile) => {
                (profile.display_name.clone(), profile.avatar_url.clone())
            }
            _ => (None, None),
        };
        if sender_name.is_some() || sender_avatar.is_some() {
            self.profiles.insert(
                event.sender().to_owned(),
                (sender_name.clone(), sender_avatar.clone()),
            );
        }
        let own = self.own.clone();
        let read_by = event
            .read_receipts()
            .keys()
            .filter(|user| **user != own)
            .map(|user| self.reader(user))
            .collect();

        Some(Event {
            id: id.to_owned(),
            key: EventKey::of(event),
            event_id: event.event_id().map(ToOwned::to_owned),
            sender: event.sender().to_owned(),
            sender_name,
            sender_avatar,
            timestamp: system_time(event.timestamp()),
            own: event.is_own(),
            editable: event.is_editable(),
            content,
            send_state,
            read_by,
            utd_cause,
        })
    }

    fn reader(&mut self, user: &OwnedUserId) -> Reader {
        let (name, avatar) = match self.profiles.get(user) {
            Some(profile) => profile.clone(),
            None => {
                self.missing_profiles.push(user.clone());
                (None, None)
            }
        };
        Reader {
            id: user.clone(),
            name,
            avatar,
        }
    }

    /// The message `reply` answers, from the reply `event_id` (`None` while
    /// it is being sent).
    fn reply(&mut self, reply: &InReplyToDetails, event_id: Option<&EventId>) -> Reply {
        let mut out = Reply {
            event_id: reply.event_id.clone(),
            sender: None,
            sender_name: None,
            preview: None,
            kind: None,
            failed: false,
        };
        match &reply.event {
            TimelineDetails::Ready(original) => {
                out.sender = Some(original.sender.clone());
                if let TimelineDetails::Ready(profile) = &original.sender_profile {
                    out.sender_name = profile.display_name.clone();
                }
                out.preview = Some(match &original.content {
                    TimelineItemContent::MsgLike(m) => match &m.kind {
                        // The formatted text, without Markdown syntax.
                        MsgLikeKind::Message(message) => {
                            out.kind = Some(message_kind(message.msgtype()));
                            preview(&match html_body(message.msgtype()) {
                                Some(html) => Document::from_html(html).to_plain_text(),
                                None => message.body().to_owned(),
                            })
                        }
                        MsgLikeKind::Redacted => "Message deleted".into(),
                        _ => "…".into(),
                    },
                    _ => "…".into(),
                });
            }
            // The SDK fetches the original for the reply's ID.
            TimelineDetails::Unavailable => {
                self.missing_replies.extend(event_id.map(ToOwned::to_owned))
            }
            TimelineDetails::Error(_) => out.failed = true,
            TimelineDetails::Pending => {}
        }
        out
    }
}

/// The text of a message on one line, shortened.
fn preview(body: &str) -> String {
    // More than a few lines of a wide window.
    const MAX: usize = 600;
    let text = body.split_whitespace().collect::<Vec<_>>().join(" ");
    if text.chars().count() > MAX {
        format!("{}…", text.chars().take(MAX).collect::<String>())
    } else {
        text
    }
}

fn message_kind(msgtype: &MessageType) -> MessageKind {
    match msgtype {
        MessageType::Text(_) => MessageKind::Text,
        MessageType::Notice(_) => MessageKind::Notice,
        MessageType::Emote(_) => MessageKind::Emote,
        MessageType::Image(_) => MessageKind::Image,
        MessageType::Video(_) => MessageKind::Video,
        MessageType::Audio(_) => MessageKind::Audio,
        MessageType::File(_) => MessageKind::File,
        MessageType::Location(_) => MessageKind::Location,
        _ => MessageKind::Other,
    }
}

fn media(msgtype: &MessageType) -> Option<Media> {
    let media = |kind, filename: &str, source: &MediaSource, thumbnail| Media {
        kind,
        filename: filename.to_owned(),
        size: None,
        mimetype: None,
        width: None,
        height: None,
        source: MediaRef::new(source, thumbnail),
    };
    let pixels = |n: Option<UInt>| n.and_then(|n| u32::try_from(n).ok());
    Some(match msgtype {
        MessageType::Image(c) => {
            let info = c.info.as_deref();
            Media {
                size: info.and_then(|i| i.size).map(u64::from),
                mimetype: info.and_then(|i| i.mimetype.clone()),
                width: pixels(info.and_then(|i| i.width)),
                height: pixels(info.and_then(|i| i.height)),
                ..media(
                    MediaKind::Image,
                    c.filename(),
                    &c.source,
                    info.and_then(|i| i.thumbnail_source.as_ref()),
                )
            }
        }
        MessageType::Video(c) => {
            let info = c.info.as_deref();
            Media {
                size: info.and_then(|i| i.size).map(u64::from),
                mimetype: info.and_then(|i| i.mimetype.clone()),
                width: pixels(info.and_then(|i| i.width)),
                height: pixels(info.and_then(|i| i.height)),
                ..media(
                    MediaKind::Video,
                    c.filename(),
                    &c.source,
                    info.and_then(|i| i.thumbnail_source.as_ref()),
                )
            }
        }
        MessageType::Audio(c) => {
            let info = c.info.as_deref();
            Media {
                size: info.and_then(|i| i.size).map(u64::from),
                mimetype: info.and_then(|i| i.mimetype.clone()),
                ..media(MediaKind::Audio, c.filename(), &c.source, None)
            }
        }
        MessageType::File(c) => {
            let info = c.info.as_deref();
            Media {
                size: info.and_then(|i| i.size).map(u64::from),
                mimetype: info.and_then(|i| i.mimetype.clone()),
                ..media(
                    MediaKind::File,
                    c.filename(),
                    &c.source,
                    info.and_then(|i| i.thumbnail_source.as_ref()),
                )
            }
        }
        _ => return None,
    })
}

fn html_body(msgtype: &MessageType) -> Option<&str> {
    let formatted = match msgtype {
        MessageType::Text(c) => c.formatted.as_ref(),
        MessageType::Notice(c) => c.formatted.as_ref(),
        MessageType::Emote(c) => c.formatted.as_ref(),
        _ => None,
    }?;
    (formatted.format.as_str() == "org.matrix.custom.html").then_some(formatted.body.as_str())
}

fn membership_change(change: SdkMembershipChange) -> MembershipChange {
    match change {
        SdkMembershipChange::Joined => MembershipChange::Joined,
        SdkMembershipChange::Left => MembershipChange::Left,
        SdkMembershipChange::Invited => MembershipChange::Invited,
        SdkMembershipChange::InvitationAccepted => MembershipChange::InvitationAccepted,
        SdkMembershipChange::InvitationRejected => MembershipChange::InvitationRejected,
        SdkMembershipChange::InvitationRevoked => MembershipChange::InvitationRevoked,
        SdkMembershipChange::Kicked => MembershipChange::Kicked,
        SdkMembershipChange::Banned | SdkMembershipChange::KickedAndBanned => {
            MembershipChange::Banned
        }
        SdkMembershipChange::Unbanned => MembershipChange::Unbanned,
        SdkMembershipChange::Knocked => MembershipChange::Knocked,
        _ => MembershipChange::Other,
    }
}

#[cfg(test)]
mod tests {
    use matrix_sdk::ruma::owned_user_id;

    use super::*;

    fn reader(id: &str) -> Reader {
        Reader {
            id: id.try_into().unwrap(),
            name: None,
            avatar: None,
        }
    }

    fn event(id: &str, state: bool, readers: &[&str]) -> TimelineEntry {
        let sender = owned_user_id!("@alice:server.org");
        let mut event = Event::test_message(id, sender.clone(), "hi");
        if state {
            event.content = EventContent::Membership {
                user: sender,
                name: None,
                change: MembershipChange::Joined,
            };
        }
        event.read_by = readers.iter().map(|r| reader(r)).collect();
        TimelineEntry::Event(event)
    }

    /// Each event's ID and readers.
    fn readers(entries: &[TimelineEntry]) -> Vec<(&str, Vec<&str>)> {
        entries
            .iter()
            .filter_map(|entry| match entry {
                TimelineEntry::Event(e) => Some((
                    e.id.as_str(),
                    e.read_by.iter().map(|r| r.id.as_str()).collect(),
                )),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn receipts_on_state_events_move_to_the_message_before() {
        let (bob, carol, dave) = ("@bob:server.org", "@carol:server.org", "@dave:server.org");
        let mut entries = vec![
            // No earlier message: they stay.
            event("join", true, &[dave]),
            event("a", false, &[]),
            event("b", false, &[bob]),
            TimelineEntry::ReadMarker,
            TimelineEntry::DayDivider(SystemTime::UNIX_EPOCH),
            event("join2", true, &[carol, bob]),
            event("topic", true, &[dave]),
        ];
        receipts_off_state(&mut entries);
        assert_eq!(
            readers(&entries),
            [
                ("join", vec![dave]),
                ("a", vec![]),
                ("b", vec![bob, carol, dave]),
                ("join2", vec![]),
                ("topic", vec![]),
            ]
        );
    }

    #[test]
    fn mentions_by_id_and_by_unique_name() {
        let members = [
            (owned_user_id!("@nils:server.org"), Some("Nils".to_owned())),
            (owned_user_id!("@anna:server.org"), Some("Anna".to_owned())),
            (owned_user_id!("@anna:other.org"), Some("Anna".to_owned())),
            (owned_user_id!("@bot:server.org"), None),
        ];
        let targets = mention_targets(members.iter().map(|(id, name)| (&**id, name.as_deref())));
        let shown: Vec<_> = targets
            .iter()
            .map(|(target, user)| {
                let link = match &target.mention {
                    Mention::User { href, name } => format!("{href} {name}"),
                    Mention::Room => "room".to_owned(),
                };
                (
                    target.text.as_str(),
                    link,
                    user.as_ref().map(|u| u.as_str()),
                )
            })
            .collect();
        let nils = "https://matrix.to/#/@nils:server.org Nils";
        assert_eq!(
            shown,
            [
                ("nils:server.org", nils.to_owned(), Some("@nils:server.org")),
                ("Nils", nils.to_owned(), Some("@nils:server.org")),
                (
                    "anna:server.org",
                    "https://matrix.to/#/@anna:server.org Anna".to_owned(),
                    Some("@anna:server.org")
                ),
                (
                    "anna:other.org",
                    "https://matrix.to/#/@anna:other.org Anna".to_owned(),
                    Some("@anna:other.org")
                ),
                (
                    "bot:server.org",
                    "https://matrix.to/#/@bot:server.org @bot:server.org".to_owned(),
                    Some("@bot:server.org")
                ),
                ("room", "room".to_owned(), None),
            ]
        );
    }
}
