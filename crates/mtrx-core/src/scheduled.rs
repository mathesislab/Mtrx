//! Send later: messages kept on this device and sent when their time comes.
//!
//! Matrix has no server-side scheduling for ordinary messages, so scheduled
//! messages live in the local (encrypted) store, and a scheduler task hands
//! them to the send queue when due. That only happens while a scheduler runs
//! (the app, `mtrx watch`); a message whose time passed while none ran is
//! held as overdue until the user sends or cancels it. Attached files are
//! kept in the store with the message, each under its own key.

use std::{
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use matrix_sdk::{
    Client,
    ruma::{EventId, OwnedEventId, OwnedRoomId},
};
use serde::{Deserialize, Serialize};
use tokio::sync::{Mutex, watch};
use tracing::warn;

use crate::{
    AbortOnDrop, Attachment, Error, Live, PendingSend, Result, RoomHandle, Session,
    timeline::send_message,
};

/// Store key of the list of scheduled messages.
const STORE_KEY: &[u8] = b"mtrx.scheduled_messages";
/// Store key prefix of the contents of attached files.
const FILE_KEY: &str = "mtrx.scheduled_file";
/// A message found this late is held instead of sent: the app was probably
/// closed or the computer asleep at its time.
const GRACE: Duration = Duration::from_secs(5 * 60);
/// Longest wait between checks. Timers may not count time the computer
/// sleeps, so the wall clock is checked regularly.
const MAX_WAIT: Duration = Duration::from_secs(30);

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScheduledMessage {
    pub id: String,
    pub room_id: OwnedRoomId,
    /// Markdown, as written.
    pub text: String,
    #[serde(with = "millis")]
    pub send_at: SystemTime,
    /// The message this answers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub in_reply_to: Option<OwnedEventId>,
    /// Its time passed while no scheduler ran; it waits for the user to
    /// send, reschedule or cancel it.
    pub overdue: bool,
    /// Sent after the text, each as a message of its own.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub files: Vec<ScheduledFile>,
}

/// A file sent with a scheduled message.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScheduledFile {
    pub filename: String,
    pub mimetype: String,
    /// In bytes.
    pub size: u64,
}

fn file_key(message_id: &str, index: usize) -> Vec<u8> {
    format!("{FILE_KEY}.{message_id}.{index}").into_bytes()
}

/// The scheduled messages of a session, persisted on every change.
pub(crate) struct Schedule {
    client: Client,
    send_lock: Arc<Mutex<()>>,
    messages: watch::Sender<Vec<ScheduledMessage>>,
    /// Serializes changes so each is persisted before the next starts.
    write: Mutex<()>,
}

impl Schedule {
    pub(crate) async fn load(client: Client, send_lock: Arc<Mutex<()>>) -> Result<Self> {
        let messages = match client.state_store().get_custom_value(STORE_KEY).await? {
            Some(bytes) => serde_json::from_slice(&bytes)?,
            None => Vec::new(),
        };
        Ok(Self {
            client,
            send_lock,
            messages: watch::Sender::new(messages),
            write: Mutex::new(()),
        })
    }

    /// Apply `change` to the list and persist it if it changed; nothing
    /// changes if `change` fails.
    async fn modify<R>(
        &self,
        change: impl AsyncFnOnce(&mut Vec<ScheduledMessage>) -> Result<R>,
    ) -> Result<R> {
        let _guard = self.write.lock().await;
        let mut messages = self.messages.borrow().clone();
        let result = change(&mut messages).await?;
        messages.sort_by_key(|m| m.send_at);
        if messages != *self.messages.borrow() {
            self.client
                .state_store()
                .set_custom_value(STORE_KEY, serde_json::to_vec(&messages)?)
                .await?;
            self.messages.send_replace(messages);
        }
        Ok(result)
    }

    /// Send what is due, hold what is late. Returns when the next message is due.
    async fn dispatch(&self, now: SystemTime, grace: Duration) -> Result<Option<SystemTime>> {
        self.modify(async |messages| {
            let mut kept = Vec::new();
            for mut message in std::mem::take(messages) {
                if message.overdue || message.send_at > now {
                    kept.push(message);
                } else if now.duration_since(message.send_at).unwrap_or_default() > grace {
                    message.overdue = true;
                    kept.push(message);
                } else if let Err(e) = self.send(&message).await {
                    warn!("could not send scheduled message: {e}");
                    message.overdue = true;
                    kept.push(message);
                } else {
                    self.remove_files(&message).await;
                }
            }
            *messages = kept;
            Ok(messages
                .iter()
                .filter(|m| !m.overdue)
                .map(|m| m.send_at)
                .min())
        })
        .await
    }

    /// Hand a message to the send queue, which persists it and sends it as
    /// soon as the server is reachable. Its files stay stored.
    async fn send(&self, message: &ScheduledMessage) -> Result<PendingSend> {
        let room = self
            .client
            .get_room(&message.room_id)
            .ok_or_else(|| Error::RoomNotFound(message.room_id.to_string()))?;
        send_message(
            &room,
            &self.send_lock,
            &message.text,
            self.load_files(message).await?,
            message.in_reply_to.as_deref(),
        )
        .await
    }

    async fn store_files(&self, message_id: &str, files: Vec<Attachment>) -> Result<()> {
        for (index, file) in files.into_iter().enumerate() {
            self.client
                .state_store()
                .set_custom_value(&file_key(message_id, index), file.bytes)
                .await?;
        }
        Ok(())
    }

    async fn load_files(&self, message: &ScheduledMessage) -> Result<Vec<Attachment>> {
        let mut files = Vec::with_capacity(message.files.len());
        for (index, file) in message.files.iter().enumerate() {
            let bytes = self
                .client
                .state_store()
                .get_custom_value(&file_key(&message.id, index))
                .await?
                .ok_or_else(|| Error::Timeline(format!("{} is missing", file.filename)))?;
            files.push(Attachment {
                filename: file.filename.clone(),
                mimetype: file.mimetype.clone(),
                bytes,
            });
        }
        Ok(files)
    }

    /// Forget the contents of a message's files, once sent or cancelled.
    async fn remove_files(&self, message: &ScheduledMessage) {
        for index in 0..message.files.len() {
            if let Err(e) = self
                .client
                .state_store()
                .remove_custom_value(&file_key(&message.id, index))
                .await
            {
                warn!("could not remove a scheduled file: {e}");
            }
        }
    }

    /// Take a message off the list, keeping its files.
    async fn take(&self, id: &str) -> Result<ScheduledMessage> {
        self.modify(async |messages| {
            let index = position(messages, id)?;
            Ok(messages.remove(index))
        })
        .await
    }
}

fn position(messages: &[ScheduledMessage], id: &str) -> Result<usize> {
    messages
        .iter()
        .position(|m| m.id == id)
        .ok_or_else(|| Error::ScheduledNotFound(id.to_owned()))
}

impl Session {
    /// Scheduled messages of all rooms, earliest first.
    pub fn scheduled(&self) -> Vec<ScheduledMessage> {
        self.schedule.messages.borrow().clone()
    }

    /// The scheduled messages, then every change.
    pub fn scheduled_updates(&self) -> Live<Vec<ScheduledMessage>> {
        Live::from_watch(self.schedule.messages.subscribe())
    }

    /// Change a scheduled message's text, keeping its time.
    pub async fn edit_scheduled(&self, id: &str, text: &str) -> Result<()> {
        self.schedule
            .modify(async |messages| {
                let index = position(messages, id)?;
                messages[index].text = text.to_owned();
                Ok(())
            })
            .await
    }

    /// Change a scheduled message's time; `send_at` must be in the future.
    /// An overdue message waits again.
    pub async fn reschedule(&self, id: &str, send_at: SystemTime) -> Result<()> {
        let send_at = future_time(send_at)?;
        self.schedule
            .modify(async |messages| {
                let index = position(messages, id)?;
                let message = &mut messages[index];
                message.send_at = send_at;
                message.overdue = false;
                Ok(())
            })
            .await
    }

    /// Remove a scheduled message without sending it.
    pub async fn unschedule(&self, id: &str) -> Result<ScheduledMessage> {
        let message = self.schedule.take(id).await?;
        self.schedule.remove_files(&message).await;
        Ok(message)
    }

    /// Send a scheduled message now, e.g. an overdue one.
    pub async fn send_scheduled_now(&self, id: &str) -> Result<PendingSend> {
        let message = self.schedule.take(id).await?;
        match self.schedule.send(&message).await {
            Ok(pending) => {
                self.schedule.remove_files(&message).await;
                Ok(pending)
            }
            Err(e) => {
                // Keep it rather than lose it.
                self.schedule
                    .modify(async |messages| {
                        messages.push(message);
                        Ok(())
                    })
                    .await?;
                Err(e)
            }
        }
    }

    /// Send scheduled messages when they are due, until the session is
    /// dropped. Messages whose time already passed are held as overdue.
    pub fn start_scheduler(&self) {
        self.start_scheduler_with_grace(GRACE);
    }

    /// [`Session::start_scheduler`] with a different lateness limit, for tests.
    #[cfg(feature = "test-support")]
    pub fn start_scheduler_for_test(&self, grace: Duration) {
        self.start_scheduler_with_grace(grace);
    }

    fn start_scheduler_with_grace(&self, grace: Duration) {
        let schedule = self.schedule.clone();
        let task = tokio::spawn(async move {
            let mut changes = schedule.messages.subscribe();
            loop {
                changes.mark_unchanged();
                let now = SystemTime::now();
                let wait = match schedule.dispatch(now, grace).await {
                    Ok(next) => next
                        .map(|at| at.duration_since(now).unwrap_or_default())
                        .map_or(MAX_WAIT, |wait| wait.min(MAX_WAIT)),
                    Err(e) => {
                        warn!("scheduler: {e}");
                        MAX_WAIT
                    }
                };
                tokio::select! {
                    _ = tokio::time::sleep(wait) => {}
                    _ = changes.changed() => {}
                }
            }
        });
        *self.scheduler.lock().expect("not poisoned") = Some(AbortOnDrop(task));
    }
}

impl RoomHandle {
    /// Schedule a Markdown message and files for `send_at`, which must be
    /// in the future; they are sent as by [`RoomTimeline::send`].
    ///
    /// [`RoomTimeline::send`]: crate::RoomTimeline::send
    pub async fn schedule(
        &self,
        text: &str,
        files: Vec<Attachment>,
        send_at: SystemTime,
        in_reply_to: Option<&EventId>,
    ) -> Result<ScheduledMessage> {
        let schedule = &self.session.schedule;
        let send_at = future_time(send_at)?;
        if text.trim().is_empty() && files.is_empty() {
            return Err(Error::Timeline("nothing to send".into()));
        }
        let message = ScheduledMessage {
            id: random_id(),
            room_id: self.id().to_owned(),
            text: text.to_owned(),
            send_at,
            in_reply_to: in_reply_to.map(ToOwned::to_owned),
            overdue: false,
            files: files
                .iter()
                .map(|file| ScheduledFile {
                    filename: file.filename.clone(),
                    mimetype: file.mimetype.clone(),
                    size: file.bytes.len() as u64,
                })
                .collect(),
        };
        // The files first: the scheduler may send the message once listed.
        let listed = match schedule.store_files(&message.id, files).await {
            Ok(()) => {
                schedule
                    .modify(async |messages| {
                        messages.push(message.clone());
                        Ok(())
                    })
                    .await
            }
            Err(e) => Err(e),
        };
        if let Err(e) = listed {
            schedule.remove_files(&message).await;
            return Err(e);
        }
        Ok(message)
    }
}

/// `send_at` to the stored precision, if it is in the future.
fn future_time(send_at: SystemTime) -> Result<SystemTime> {
    if send_at <= SystemTime::now() {
        return Err(Error::SendTimeInPast);
    }
    let millis = send_at
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    Ok(UNIX_EPOCH + Duration::from_millis(millis as u64))
}

pub(crate) fn random_id() -> String {
    let bytes: [u8; 4] = rand::random();
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// `SystemTime` as milliseconds since the Unix epoch.
mod millis {
    use super::*;
    use serde::{Deserializer, Serializer};

    pub fn serialize<S: Serializer>(time: &SystemTime, s: S) -> Result<S::Ok, S::Error> {
        let millis = time
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        s.serialize_u64(millis as u64)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<SystemTime, D::Error> {
        Ok(UNIX_EPOCH + Duration::from_millis(u64::deserialize(d)?))
    }
}
