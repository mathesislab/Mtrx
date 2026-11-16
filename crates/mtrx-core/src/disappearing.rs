//! Disappearing messages (see `docs/disappearing.md`): ours deleted a set
//! time after every other member has read them, and at the latest a set
//! time after sending.
//!
//! Matrix has no expiry for messages, so this runs here, as send later
//! does. A message sent to disappear is listed in the local (encrypted)
//! store until a timer deletes it; the timer runs while the desktop app,
//! `mtrx watch` or a phone app in front runs, and deletes what fell due
//! meanwhile as soon as it starts.
//! Texts carry a marker ([`MARKER`]), so those our other devices send are
//! listed when they sync, and others see that they disappear; files can't
//! carry it, so only the device sending them lists them.

use std::{
    collections::{BTreeMap, HashMap},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, SystemTime},
};

use matrix_sdk::{
    Client, Room, RoomMemberships, RoomState,
    event_handler::EventHandlerDropGuard,
    ruma::{
        EventId, MilliSecondsSinceUnixEpoch, OwnedEventId, OwnedRoomId, OwnedTransactionId,
        OwnedUserId, RoomId, TransactionId, UserId,
        events::{
            AnyMessageLikeEventContent, AnySyncTimelineEvent, RoomAccountDataEvent,
            macros::EventContent,
            receipt::{ReceiptThread, ReceiptType, SyncReceiptEvent},
            room::{member::SyncRoomMemberEvent, message::RoomMessageEventContent},
        },
        serde::Raw,
    },
    send_queue::{RoomSendQueueUpdate, SendQueueUpdate},
};
use serde::{Deserialize, Serialize};
use tokio::sync::{Mutex, Notify, broadcast::error::RecvError, watch};
use tracing::warn;

use crate::{
    AbortOnDrop, Live, Result, RoomHandle, Session, account_data::AccountData, scheduled::millis,
};

/// The content field of a message that disappears, holding its
/// [`Disappear`].
pub const MARKER: &str = "org.mtrx.disappearing";
/// Store key of the list.
const STORE_KEY: &[u8] = b"mtrx.disappearing_messages";
/// Longest wait between checks, as for send later: timers may not count
/// time the computer sleeps.
const MAX_WAIT: Duration = Duration::from_secs(30);
/// A message still in the send queue after this long is dropped from the
/// list; it was probably cancelled while no timer ran.
const UNSENT_KEPT: Duration = Duration::from_secs(7 * 24 * 3600);
const MINUTE: u64 = 60;
const HOUR: u64 = 60 * MINUTE;
const DAY: u64 = 24 * HOUR;

/// The maximum lifetime until one is chosen in Settings.
pub const DEFAULT_AT_MOST: Duration = Duration::from_secs(DAY);

/// The times after read the apps offer, for a message or a room's mode.
pub const AFTER_READ_CHOICES: [Duration; 4] = [
    Duration::from_secs(MINUTE),
    Duration::from_secs(5 * MINUTE),
    Duration::from_secs(HOUR),
    Duration::from_secs(DAY),
];

/// The maximum lifetimes Settings offers.
pub const AT_MOST_CHOICES: [Duration; 4] = [
    Duration::from_secs(HOUR),
    Duration::from_secs(6 * HOUR),
    DEFAULT_AT_MOST,
    Duration::from_secs(7 * DAY),
];

/// `duration` in words, in its largest whole unit: "5 minutes", "1 day".
pub fn duration_words(duration: Duration) -> String {
    let secs = duration.as_secs();
    let (count, unit) = [(DAY, "day"), (HOUR, "hour"), (MINUTE, "minute")]
        .into_iter()
        .find(|(unit, _)| secs >= *unit && secs.is_multiple_of(*unit))
        .map_or((secs, "second"), |(size, unit)| (secs / size, unit));
    plural(count, unit)
}

/// A room's mode in words: "Off", "5 minutes after read".
pub fn disappearing_mode_words(after_read: Option<Duration>) -> String {
    match after_read {
        None => "Off".to_owned(),
        Some(after) => format!("{} after read", duration_words(after)),
    }
}

/// What the composer sends, in words: "Disappears 5 minutes after
/// everyone has read it", "Disappearing messages: off".
pub fn disappear_words(after_read: Option<Duration>) -> String {
    match after_read {
        Some(after) => format!(
            "Disappears {} after everyone has read it",
            duration_words(after)
        ),
        None => "Disappearing messages: off".to_owned(),
    }
}

/// The time left until `due`, rounded up: "3 minutes", "less than a
/// minute".
fn time_left(due: SystemTime, now: SystemTime) -> String {
    let secs = due.duration_since(now).unwrap_or_default().as_secs();
    if secs < MINUTE {
        "less than a minute".to_owned()
    } else if secs < HOUR {
        plural(secs.div_ceil(MINUTE), "minute")
    } else if secs < DAY {
        plural(secs.div_ceil(HOUR), "hour")
    } else {
        plural(secs.div_ceil(DAY), "day")
    }
}

fn plural(count: u64, unit: &str) -> String {
    match count {
        1 => format!("1 {unit}"),
        n => format!("{n} {unit}s"),
    }
}

/// How a message disappears: `after_read` after every other member has
/// read it, and at the latest `at_most` after it was sent. Also the
/// marker's content, in seconds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Disappear {
    #[serde(with = "secs")]
    pub after_read: Duration,
    #[serde(with = "secs")]
    pub at_most: Duration,
}

impl Disappear {
    /// What a message sent so says of itself: ours (`own`) when it goes,
    /// others' only that it does.
    pub fn words(&self, own: bool) -> String {
        if own {
            format!(
                "Disappears {} after everyone has read it, at the latest {} after sending",
                duration_words(self.after_read),
                duration_words(self.at_most),
            )
        } else {
            "Disappears after it was read".to_owned()
        }
    }
}

/// One of our messages waiting to disappear.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Disappearing {
    pub room_id: OwnedRoomId,
    pub disappear: Disappear,
    pub sent: Sent,
    /// When every other member had read it, once seen.
    #[serde(
        default,
        with = "millis::option",
        skip_serializing_if = "Option::is_none"
    )]
    pub read_at: Option<SystemTime>,
    /// Our edits of it, deleted with it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub edits: Vec<OwnedEventId>,
}

/// Whether a disappearing message left the send queue.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Sent {
    /// In the send queue since `since`.
    Queued {
        transaction_id: OwnedTransactionId,
        #[serde(with = "millis")]
        since: SystemTime,
    },
    /// Sent; `at` is the server's time once synced.
    Sent {
        event_id: OwnedEventId,
        #[serde(with = "millis")]
        at: SystemTime,
    },
}

impl Disappearing {
    pub fn event_id(&self) -> Option<&EventId> {
        match &self.sent {
            Sent::Sent { event_id, .. } => Some(event_id),
            Sent::Queued { .. } => None,
        }
    }

    /// When it goes: `after_read` after everyone read it, at the latest
    /// `at_most` after it was sent; `None` until sent.
    pub fn due(&self) -> Option<SystemTime> {
        let Sent::Sent { at, .. } = &self.sent else {
            return None;
        };
        let latest = *at + self.disappear.at_most;
        Some(match self.read_at {
            Some(read) => latest.min(read + self.disappear.after_read),
            None => latest,
        })
    }

    /// What it says of itself at `now`: once everyone has read it, the
    /// time left.
    pub fn words(&self, now: SystemTime) -> String {
        match (self.read_at, self.due()) {
            (Some(_), Some(due)) => format!(
                "Everyone has read it: disappears in {}",
                time_left(due, now)
            ),
            _ => self.disappear.words(true),
        }
    }
}

/// The marker of a message's raw event, if it has one.
pub(crate) fn marker(event: &Raw<AnySyncTimelineEvent>) -> Option<Disappear> {
    let content = event.get_field::<Marked>("content").ok().flatten()?;
    content.disappear
}

/// `content` with the marker, to send raw.
pub(crate) fn marked(
    content: RoomMessageEventContent,
    disappear: Disappear,
) -> Result<Raw<AnyMessageLikeEventContent>> {
    let mut json = serde_json::to_value(&content)?;
    json[MARKER] = serde_json::to_value(disappear)?;
    Ok(Raw::from_json(serde_json::value::to_raw_value(&json)?))
}

#[derive(Deserialize)]
struct Marked {
    #[serde(rename = "org.mtrx.disappearing")]
    disappear: Option<Disappear>,
}

/// What [`Disappearances`] reads of the events we send.
#[derive(Deserialize)]
struct Own {
    sender: OwnedUserId,
    event_id: OwnedEventId,
    origin_server_ts: MilliSecondsSinceUnixEpoch,
    #[serde(default)]
    content: OwnContent,
    #[serde(default)]
    unsigned: OwnUnsigned,
}

#[derive(Default, Deserialize)]
struct OwnContent {
    #[serde(rename = "org.mtrx.disappearing")]
    disappear: Option<Disappear>,
    #[serde(rename = "m.relates_to")]
    relates_to: Option<OwnRelation>,
}

#[derive(Deserialize)]
struct OwnRelation {
    rel_type: Option<String>,
    event_id: Option<OwnedEventId>,
}

/// The server tells the device that sent an event its transaction ID.
#[derive(Default, Deserialize)]
struct OwnUnsigned {
    transaction_id: Option<OwnedTransactionId>,
}

/// A room's mode: our messages there disappear this long after read.
#[derive(Clone, Debug, Default, Serialize, Deserialize, EventContent)]
#[ruma_event(type = "org.mtrx.disappearing", kind = RoomAccountData)]
pub(crate) struct DisappearingModeEventContent {
    /// In seconds; none for off.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    after_read: Option<u64>,
}

/// How long after sending our disappearing messages go at the latest,
/// chosen in Settings and shared by our devices.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DisappearingLimit {
    pub at_most: Duration,
}

impl Default for DisappearingLimit {
    fn default() -> Self {
        Self {
            at_most: DEFAULT_AT_MOST,
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, EventContent)]
#[ruma_event(type = "org.mtrx.disappearing_limit", kind = GlobalAccountData)]
pub(crate) struct DisappearingLimitEventContent {
    /// In seconds; none for the default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    at_most: Option<u64>,
}

impl AccountData for DisappearingLimit {
    type Content = DisappearingLimitEventContent;

    fn from_content(content: Self::Content) -> Self {
        content.at_most.map_or_else(Self::default, |secs| Self {
            at_most: Duration::from_secs(secs),
        })
    }

    fn into_content(self) -> Self::Content {
        DisappearingLimitEventContent {
            at_most: Some(self.at_most.as_secs()),
        }
    }
}

/// Our disappearing messages, persisted on every change, and the rooms'
/// modes.
pub(crate) struct Disappearances {
    client: Client,
    list: watch::Sender<Vec<Disappearing>>,
    /// Serializes changes so each is persisted before the next starts.
    write: Mutex<()>,
    /// Receipts or members changed where a message waits to be read.
    nudge: Notify,
    /// The rooms whose mode is on, with its time after read.
    modes: watch::Sender<BTreeMap<OwnedRoomId, Duration>>,
    /// Changes of modes not yet saved; synced modes are stale meanwhile,
    /// as for [`crate::account_data::Synced`].
    unsaved_modes: AtomicUsize,
}

/// What keeps a [`Disappearances`] up to date: its event handlers and the
/// task following the send queue. Dropped with the session.
pub(crate) struct Following {
    _handlers: [EventHandlerDropGuard; 4],
    _sent: AbortOnDrop,
}

impl Disappearances {
    pub(crate) async fn load(client: Client) -> Result<Arc<Self>> {
        let list = match client.state_store().get_custom_value(STORE_KEY).await? {
            Some(bytes) => serde_json::from_slice(&bytes)?,
            None => Vec::new(),
        };
        let mut modes = BTreeMap::new();
        for room in client.joined_rooms() {
            if let Some(after_read) = stored_mode(&room).await {
                modes.insert(room.room_id().to_owned(), after_read);
            }
        }
        Ok(Arc::new(Self {
            client,
            list: watch::Sender::new(list),
            write: Mutex::new(()),
            nudge: Notify::new(),
            modes: watch::Sender::new(modes),
            unsaved_modes: AtomicUsize::new(0),
        }))
    }

    /// Follow what we send, receipts and members, and room modes.
    pub(crate) fn follow(self: &Arc<Self>) -> Following {
        let client = &self.client;
        let this = self.clone();
        let synced =
            client.add_event_handler(move |event: Raw<AnySyncTimelineEvent>, room: Room| {
                let this = this.clone();
                async move {
                    let Ok(own) = event.deserialize_as_unchecked::<Own>() else {
                        return;
                    };
                    if Some(&*own.sender) == this.client.user_id() {
                        this.synced(room.room_id(), own).await;
                    }
                }
            });
        let this = self.clone();
        let receipts = client.add_event_handler(move |_: SyncReceiptEvent, room: Room| {
            let this = this.clone();
            async move { this.nudge_if_waiting(room.room_id()) }
        });
        let this = self.clone();
        let members = client.add_event_handler(move |_: SyncRoomMemberEvent, room: Room| {
            let this = this.clone();
            async move { this.nudge_if_waiting(room.room_id()) }
        });
        let this = self.clone();
        let modes = client.add_event_handler(
            move |event: RoomAccountDataEvent<DisappearingModeEventContent>, room: Room| {
                let this = this.clone();
                async move {
                    if this.unsaved_modes.load(Ordering::SeqCst) == 0 {
                        let after_read = event.content.after_read.map(Duration::from_secs);
                        this.set_mode(room.room_id(), after_read);
                    }
                }
            },
        );
        let this = self.clone();
        let mut updates = client.send_queue().subscribe();
        let sent = tokio::spawn(async move {
            loop {
                match updates.recv().await {
                    Ok(SendQueueUpdate { update, .. }) => match update {
                        RoomSendQueueUpdate::SentEvent {
                            transaction_id,
                            event_id,
                        } => this.sent(&transaction_id, event_id).await,
                        RoomSendQueueUpdate::CancelledLocalEvent { transaction_id } => {
                            this.cancelled(&transaction_id).await
                        }
                        _ => {}
                    },
                    // Sync tells the transaction IDs too.
                    Err(RecvError::Lagged(_)) => {}
                    Err(RecvError::Closed) => break,
                }
            }
        });
        Following {
            _handlers: [synced, receipts, members, modes]
                .map(|handle| client.event_handler_drop_guard(handle)),
            _sent: AbortOnDrop(sent),
        }
    }

    /// Apply `change` to the list and persist it if it changed.
    async fn modify<R>(&self, change: impl FnOnce(&mut Vec<Disappearing>) -> R) -> Result<R> {
        let _guard = self.write.lock().await;
        let mut list = self.list.borrow().clone();
        let result = change(&mut list);
        if list != *self.list.borrow() {
            self.client
                .state_store()
                .set_custom_value(STORE_KEY, serde_json::to_vec(&list)?)
                .await?;
            self.list.send_replace(list);
        }
        Ok(result)
    }

    /// Messages just queued in `room`, to disappear as `disappear` says.
    pub(crate) async fn queued(
        &self,
        room: &RoomId,
        transaction_ids: &[OwnedTransactionId],
        disappear: Disappear,
    ) -> Result<()> {
        let since = SystemTime::now();
        self.modify(|list| {
            for id in transaction_ids {
                let listed = list.iter().any(|entry| match &entry.sent {
                    Sent::Queued { transaction_id, .. } => transaction_id == id,
                    Sent::Sent { .. } => false,
                });
                if !listed {
                    list.push(Disappearing {
                        room_id: room.to_owned(),
                        disappear,
                        sent: Sent::Queued {
                            transaction_id: id.clone(),
                            since,
                        },
                        read_at: None,
                        edits: Vec::new(),
                    });
                }
            }
        })
        .await
    }

    /// The send queue sent `transaction_id` as `event_id`.
    async fn sent(&self, transaction_id: &TransactionId, event_id: OwnedEventId) {
        let at = SystemTime::now();
        let done = self
            .modify(|list| {
                for entry in list.iter_mut() {
                    if matches!(&entry.sent, Sent::Queued { transaction_id: id, .. } if **id == *transaction_id)
                    {
                        entry.sent = Sent::Sent {
                            event_id: event_id.clone(),
                            at,
                        };
                    }
                }
            })
            .await;
        if let Err(e) = done {
            warn!("could not note a disappearing message as sent: {e}");
        }
    }

    /// Sending `transaction_id` was cancelled.
    async fn cancelled(&self, transaction_id: &TransactionId) {
        let done = self
            .modify(|list| {
                list.retain(|entry| {
                    !matches!(&entry.sent, Sent::Queued { transaction_id: id, .. } if **id == *transaction_id)
                })
            })
            .await;
        if let Err(e) = done {
            warn!("could not forget a cancelled disappearing message: {e}");
        }
    }

    /// One of our events synced in `room`: a message of ours sent here (by
    /// its transaction ID) or marked by another device, or an edit of one.
    async fn synced(&self, room: &RoomId, own: Own) {
        let at = own
            .origin_server_ts
            .to_system_time()
            .unwrap_or_else(SystemTime::now);
        let edited = (own.content.relates_to.as_ref())
            .filter(|relation| relation.rel_type.as_deref() == Some("m.replace"))
            .and_then(|relation| relation.event_id.clone());
        let done = self
            .modify(|list| {
                if let Some(original) = edited {
                    let entry = list.iter_mut().find(|e| e.event_id() == Some(&*original));
                    if let Some(entry) = entry
                        && !entry.edits.contains(&own.event_id)
                    {
                        entry.edits.push(own.event_id);
                    }
                    return;
                }
                let here = list.iter().position(|entry| match &entry.sent {
                    Sent::Sent { event_id, .. } => *event_id == own.event_id,
                    Sent::Queued { transaction_id, .. } => {
                        own.unsigned.transaction_id.as_ref() == Some(transaction_id)
                    }
                });
                match (here, own.content.disappear) {
                    // The server's time, as receipts have.
                    (Some(index), _) => {
                        list[index].sent = Sent::Sent {
                            event_id: own.event_id,
                            at,
                        };
                    }
                    (None, Some(disappear)) => list.push(Disappearing {
                        room_id: room.to_owned(),
                        disappear,
                        sent: Sent::Sent {
                            event_id: own.event_id,
                            at,
                        },
                        read_at: None,
                        edits: Vec::new(),
                    }),
                    (None, None) => {}
                }
            })
            .await;
        if let Err(e) = done {
            warn!("could not list a disappearing message: {e}");
        }
    }

    fn nudge_if_waiting(&self, room: &RoomId) {
        let waiting = (self.list.borrow().iter())
            .any(|entry| entry.room_id == room && entry.read_at.is_none());
        if waiting {
            self.nudge.notify_one();
        }
    }

    fn set_mode(&self, room: &RoomId, after_read: Option<Duration>) {
        self.modes.send_if_modified(|modes| match after_read {
            Some(after_read) => modes.insert(room.to_owned(), after_read) != Some(after_read),
            None => modes.remove(room).is_some(),
        });
    }

    /// Note who has read what (if `reads`), delete what is due, and forget
    /// what can't be sent. Returns when the next one is due.
    async fn pass(&self, now: SystemTime, reads: bool) -> Result<Option<SystemTime>> {
        let list = self.list.borrow().clone();
        let mut read = Vec::new();
        if reads {
            for entry in list.iter().filter(|entry| entry.read_at.is_none()) {
                let Sent::Sent { event_id, at } = &entry.sent else {
                    continue;
                };
                let Some(room) = self.client.get_room(&entry.room_id) else {
                    continue;
                };
                match read_by_all(&room, event_id, *at).await {
                    Ok(true) => read.push(event_id.clone()),
                    Ok(false) => {}
                    Err(e) => warn!("could not tell who read a disappearing message: {e}"),
                }
            }
        }
        let due = list
            .iter()
            .filter(|entry| entry.due().is_some_and(|due| due <= now));
        let mut deleted = Vec::new();
        for entry in due {
            if self.delete(entry).await
                && let Some(id) = entry.event_id()
            {
                deleted.push(id.to_owned());
            }
        }
        self.modify(|list| {
            list.retain(|entry| {
                entry
                    .event_id()
                    .is_none_or(|id| !deleted.iter().any(|d| **d == *id))
            });
            list.retain(|entry| match &entry.sent {
                Sent::Queued { since, .. } => {
                    now.duration_since(*since).unwrap_or_default() < UNSENT_KEPT
                }
                Sent::Sent { .. } => true,
            });
            for entry in list.iter_mut() {
                if entry
                    .event_id()
                    .is_some_and(|id| read.iter().any(|r| **r == *id))
                {
                    entry.read_at = Some(now);
                }
            }
            list.iter().filter_map(Disappearing::due).min()
        })
        .await
    }

    /// Delete a message and its edits; `false` to try again later, e.g.
    /// offline.
    async fn delete(&self, entry: &Disappearing) -> bool {
        let Some(event_id) = entry.event_id() else {
            return false;
        };
        let Some(room) = self.client.get_room(&entry.room_id) else {
            return true;
        };
        if room.state() != RoomState::Joined {
            return true;
        }
        for id in entry.edits.iter().map(|id| &**id).chain([event_id]) {
            if let Err(e) = room.redact(id, None, None).await {
                match e.as_client_api_error() {
                    // Refused (not rate limited): trying again won't help.
                    Some(api) if api.status_code.as_u16() != 429 => {
                        warn!("could not delete a disappearing message: {e}");
                    }
                    _ => return false,
                }
            }
        }
        true
    }

    /// Run [`Self::pass`] whenever something may have changed, until
    /// dropped.
    pub(crate) fn start(self: &Arc<Self>) -> AbortOnDrop {
        let this = self.clone();
        AbortOnDrop(tokio::spawn(async move {
            let mut changes = this.list.subscribe();
            let mut reads = true;
            loop {
                changes.mark_unchanged();
                let now = SystemTime::now();
                let wait = match this.pass(now, reads).await {
                    Ok(next) => next
                        .map(|at| at.duration_since(now).unwrap_or_default())
                        .map_or(MAX_WAIT, |wait| wait.min(MAX_WAIT)),
                    Err(e) => {
                        warn!("disappearing messages: {e}");
                        MAX_WAIT
                    }
                };
                reads = tokio::select! {
                    _ = tokio::time::sleep(wait) => false,
                    _ = changes.changed() => true,
                    _ = this.nudge.notified() => true,
                };
            }
        }))
    }
}

/// The room's mode as stored.
async fn stored_mode(room: &Room) -> Option<Duration> {
    let raw = room
        .account_data_static::<DisappearingModeEventContent>()
        .await
        .inspect_err(|e| warn!("could not read a room's disappearing mode: {e}"))
        .ok()??;
    let event = raw.deserialize().ok()?;
    event.content.after_read.map(Duration::from_secs)
}

/// Whether every other joined member of `room` has read `event_id`, sent
/// at `sent_at`: their read receipt is on it or on a later event.
async fn read_by_all(room: &Room, event_id: &EventId, sent_at: SystemTime) -> Result<bool> {
    let me = room.own_user_id();
    let members = room.members(RoomMemberships::JOIN).await?;
    // Receipts' events' times, fetched once.
    let mut times = HashMap::new();
    for member in members.iter().filter(|m| m.user_id() != me) {
        if !has_read(room, member.user_id(), event_id, sent_at, &mut times).await? {
            return Ok(false);
        }
    }
    Ok(true)
}

async fn has_read(
    room: &Room,
    user: &UserId,
    event_id: &EventId,
    sent_at: SystemTime,
    times: &mut HashMap<OwnedEventId, Option<SystemTime>>,
) -> Result<bool> {
    for thread in [ReceiptThread::Unthreaded, ReceiptThread::Main] {
        let Some((read, receipt)) = room
            .load_user_receipt(ReceiptType::Read, &thread, user)
            .await?
        else {
            continue;
        };
        if read == event_id {
            return Ok(true);
        }
        // A receipt from before the message can't be on it or later.
        let receipt_at = receipt.ts.and_then(|ts| ts.to_system_time());
        if receipt_at.is_some_and(|at| at < sent_at) {
            continue;
        }
        let time = match times.get(&read) {
            Some(time) => *time,
            None => {
                let time = (room.load_or_fetch_event(&read, None).await.ok())
                    .and_then(|event| event.timestamp())
                    .and_then(|ts| ts.to_system_time());
                times.insert(read, time);
                time
            }
        };
        if time.is_some_and(|time| time >= sent_at) {
            return Ok(true);
        }
    }
    Ok(false)
}

impl Session {
    /// Our messages waiting to disappear, in all rooms.
    pub fn disappearing(&self) -> Vec<Disappearing> {
        self.disappearing.list.borrow().clone()
    }

    /// Our messages waiting to disappear, then every change.
    pub fn disappearing_updates(&self) -> Live<Vec<Disappearing>> {
        Live::from_watch(self.disappearing.list.subscribe())
    }

    /// The rooms whose mode makes our messages disappear, with the time
    /// after read, then every change.
    pub fn disappearing_modes_updates(&self) -> Live<BTreeMap<OwnedRoomId, Duration>> {
        Live::from_watch(self.disappearing.modes.subscribe())
    }

    pub fn disappearing_limit(&self) -> DisappearingLimit {
        self.disappearing_limit.get()
    }

    /// The limit, then every change, from this device or others.
    pub fn disappearing_limit_updates(&self) -> Live<DisappearingLimit> {
        self.disappearing_limit.updates()
    }

    /// Change the limit for messages sent from now on.
    pub async fn set_disappearing_limit(&self, at_most: Duration) -> Result<()> {
        (self.disappearing_limit)
            .change(|limit| limit.at_most = at_most)
            .await
    }

    /// Delete our disappearing messages when due, until the session is
    /// dropped; those due while none ran go at once.
    pub fn start_disappearing(&self) {
        *self.disappearer.lock().expect("not poisoned") = Some(self.disappearing.start());
    }

    /// Stop deleting until [`Session::start_disappearing`] (a phone app
    /// went to the background).
    pub fn stop_disappearing(&self) {
        *self.disappearer.lock().expect("not poisoned") = None;
    }
}

impl RoomHandle {
    /// The room's mode: our messages here disappear this long after read.
    pub fn disappearing_mode(&self) -> Option<Duration> {
        let modes = self.session.disappearing.modes.borrow();
        modes.get(self.room.room_id()).copied()
    }

    /// The room's mode, then every change, from here or our other devices.
    pub fn disappearing_mode_updates(&self) -> Live<Option<Duration>> {
        let room = self.room.room_id().to_owned();
        let mut modes = self.session.disappearing.modes.subscribe();
        let first = modes.borrow_and_update().get(&room).copied();
        Live::spawn(first, move |tx| async move {
            while modes.changed().await.is_ok() {
                let mode = modes.borrow_and_update().get(&room).copied();
                tx.send_if_modified(|shown| {
                    let changed = *shown != mode;
                    *shown = mode;
                    changed
                });
            }
        })
    }

    /// Set the room's mode (`None` for off); only ours, on our devices.
    /// Shown at once, then saved on the server; if saving fails, the
    /// server's mode is restored.
    pub async fn set_disappearing_mode(&self, after_read: Option<Duration>) -> Result<()> {
        let disappearing = &self.session.disappearing;
        disappearing.unsaved_modes.fetch_add(1, Ordering::SeqCst);
        disappearing.set_mode(self.room.room_id(), after_read);
        let content = DisappearingModeEventContent {
            after_read: after_read.map(|d| d.as_secs()),
        };
        let saved = self.room.set_account_data(content).await;
        disappearing.unsaved_modes.fetch_sub(1, Ordering::SeqCst);
        if let Err(e) = saved {
            disappearing.set_mode(self.room.room_id(), stored_mode(&self.room).await);
            return Err(e.into());
        }
        Ok(())
    }
}

/// A `Duration` as whole seconds.
mod secs {
    use std::time::Duration;

    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(duration: &Duration, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_u64(duration.as_secs())
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Duration, D::Error> {
        Ok(Duration::from_secs(u64::deserialize(d)?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(sent_at: SystemTime, read_at: Option<SystemTime>) -> Disappearing {
        Disappearing {
            room_id: "!r:example.org".try_into().unwrap(),
            disappear: Disappear {
                after_read: Duration::from_secs(300),
                at_most: Duration::from_secs(3600),
            },
            sent: Sent::Sent {
                event_id: "$e:example.org".try_into().unwrap(),
                at: sent_at,
            },
            read_at,
            edits: Vec::new(),
        }
    }

    #[test]
    fn due_after_read_or_at_most() {
        let t = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000);
        let secs = Duration::from_secs;
        assert_eq!(entry(t, None).due(), Some(t + secs(3600)));
        assert_eq!(entry(t, Some(t + secs(60))).due(), Some(t + secs(360)));
        // Read late: the limit still holds.
        assert_eq!(entry(t, Some(t + secs(3500))).due(), Some(t + secs(3600)));
    }

    #[test]
    fn not_due_until_sent() {
        let mut queued = entry(SystemTime::UNIX_EPOCH, None);
        queued.sent = Sent::Queued {
            transaction_id: "t1".into(),
            since: SystemTime::UNIX_EPOCH,
        };
        assert_eq!(queued.due(), None);
        assert_eq!(queued.event_id(), None);
    }

    #[test]
    fn the_marker_goes_into_the_content_and_is_read_back() {
        let disappear = Disappear {
            after_read: Duration::from_secs(60),
            at_most: Duration::from_secs(86400),
        };
        let content = marked(RoomMessageEventContent::text_plain("hi"), disappear).unwrap();
        let json: serde_json::Value = serde_json::from_str(content.json().get()).unwrap();
        assert_eq!(json["body"], "hi");
        assert_eq!(
            json[MARKER],
            serde_json::json!({ "after_read": 60, "at_most": 86400 })
        );
        let event = serde_json::json!({
            "type": "m.room.message",
            "event_id": "$e:example.org",
            "sender": "@a:example.org",
            "origin_server_ts": 1,
            "content": json,
        });
        let raw = Raw::from_json(serde_json::value::to_raw_value(&event).unwrap());
        assert_eq!(marker(&raw), Some(disappear));
    }

    #[test]
    fn the_list_survives_a_round_trip() {
        let mut list = vec![entry(
            SystemTime::UNIX_EPOCH + Duration::from_millis(1234),
            None,
        )];
        list[0].edits.push("$edit:example.org".try_into().unwrap());
        let bytes = serde_json::to_vec(&list).unwrap();
        let back: Vec<Disappearing> = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(back, list);
    }

    #[test]
    fn durations_in_words() {
        let secs = Duration::from_secs;
        assert_eq!(duration_words(secs(60)), "1 minute");
        assert_eq!(duration_words(secs(300)), "5 minutes");
        assert_eq!(duration_words(secs(3600)), "1 hour");
        assert_eq!(duration_words(secs(6 * 3600)), "6 hours");
        assert_eq!(duration_words(secs(86400)), "1 day");
        assert_eq!(duration_words(secs(7 * 86400)), "7 days");
        assert_eq!(duration_words(secs(90)), "90 seconds");
        assert_eq!(disappearing_mode_words(None), "Off");
        assert_eq!(disappear_words(None), "Disappearing messages: off");
        assert_eq!(
            disappear_words(Some(secs(3600))),
            "Disappears 1 hour after everyone has read it"
        );
        assert_eq!(
            disappearing_mode_words(Some(secs(300))),
            "5 minutes after read"
        );
    }

    #[test]
    fn time_left_rounds_up() {
        let now = SystemTime::UNIX_EPOCH;
        let at = |s| now + Duration::from_secs(s);
        assert_eq!(time_left(at(30), now), "less than a minute");
        assert_eq!(time_left(at(61), now), "2 minutes");
        assert_eq!(time_left(at(3600), now), "1 hour");
        assert_eq!(time_left(at(3601), now), "2 hours");
        assert_eq!(time_left(now, at(5)), "less than a minute", "overdue");
    }

    #[test]
    fn messages_say_when_they_go() {
        let t = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000);
        let unread = entry(t, None);
        assert_eq!(
            unread.words(t),
            "Disappears 5 minutes after everyone has read it, at the latest 1 hour after sending"
        );
        assert_eq!(
            unread.disappear.words(false),
            "Disappears after it was read"
        );
        let read = entry(t, Some(t + Duration::from_secs(60)));
        assert_eq!(
            read.words(t + Duration::from_secs(120)),
            "Everyone has read it: disappears in 4 minutes"
        );
    }
}
