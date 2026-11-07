//! The account's state: this device verified, the connection, scheduled
//! messages, and notifications.

use mtrx_core::{Connection, Notification, OwnedRoomId, RecoveryStep, ScheduledMessage};

#[derive(Debug, Clone)]
pub enum Message {
    RecoveryKey(String),
    Recover,
    RecoveryProgress(RecoveryStep),
    Recovered(Result<bool, String>),
    /// The server became reachable or not.
    Connection(Connection),
    /// Try the server now.
    Reconnect,
    Scheduled(Vec<ScheduledMessage>),
    /// Refreshes times shown relative to now.
    Tick,
    Notified(Notification),
    /// A desktop notification about the room was clicked.
    NotificationClicked(OwnedRoomId),
}
