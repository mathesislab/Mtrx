//! Whether the server is reachable and still accepts the session. Sync
//! goes offline on network errors and checks the server until it answers
//! again; meanwhile messages wait in the send queue, which is re-enabled
//! once the server is back. A session the server rejected stays signed out.

use std::{sync::Arc, time::Duration};

use matrix_sdk::{Client, SessionChange};
use matrix_sdk_ui::sync_service::{State, SyncService};
use tokio::{
    sync::{broadcast, watch},
    time::Instant,
};
use tracing::warn;

use crate::{AbortOnDrop, Live, Session};

/// After an unexpected sync error, how long until sync starts again.
const RESTART: Duration = Duration::from_secs(5);
/// After a send failed although sync runs, how long until it is retried.
const RESEND: Duration = Duration::from_secs(10);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Connection {
    #[default]
    Online,
    /// The server is unreachable; sync checks it until it answers.
    Offline,
    /// The server no longer accepts the session (it expired or was signed
    /// out elsewhere): only logging in again helps.
    SignedOut,
}

/// Follow sync's state as a [`Connection`] until dropped, restarting sync
/// after errors and resuming sends once the server is reachable.
pub(crate) fn watch(
    client: Client,
    sync: Arc<SyncService>,
) -> (watch::Receiver<Connection>, AbortOnDrop) {
    let (tx, rx) = watch::channel(Connection::Online);
    let task = tokio::spawn(async move {
        let mut states = sync.state();
        let mut errors = client.send_queue().subscribe_errors();
        let mut sessions = client.subscribe_to_session_changes();
        let mut resend: Option<Instant> = None;
        loop {
            tokio::select! {
                change = sessions.recv() => match change {
                    Ok(SessionChange::UnknownToken(_)) => {
                        warn!("the server no longer accepts this session");
                        tx.send_replace(Connection::SignedOut);
                        // Retrying cannot help; and states no longer matter.
                        sync.stop().await;
                        return;
                    }
                    Ok(SessionChange::TokensRefreshed)
                    | Err(broadcast::error::RecvError::Lagged(_)) => {}
                    Err(broadcast::error::RecvError::Closed) => return,
                },
                state = states.next() => match state {
                    Some(State::Running) => {
                        tx.send_replace(Connection::Online);
                        // Sends stop on network errors; the server is back.
                        client.send_queue().set_enabled(true).await;
                    }
                    Some(State::Offline) => {
                        tx.send_replace(Connection::Offline);
                    }
                    Some(State::Error(e)) => {
                        warn!("sync failed, restarting: {e}");
                        tx.send_replace(Connection::Offline);
                        tokio::time::sleep(RESTART).await;
                        sync.start().await;
                    }
                    Some(State::Idle | State::Terminated) => {}
                    None => return,
                },
                error = errors.recv() => match error {
                    // A send failed while sync works: retry it a bit later.
                    Ok(error) if error.is_recoverable => {
                        resend.get_or_insert(Instant::now() + RESEND);
                    }
                    Ok(_) | Err(broadcast::error::RecvError::Lagged(_)) => {}
                    Err(broadcast::error::RecvError::Closed) => return,
                },
                _ = tokio::time::sleep_until(resend.unwrap_or_else(Instant::now)),
                    if resend.is_some() =>
                {
                    resend = None;
                    if *tx.borrow() == Connection::Online {
                        client.send_queue().set_enabled(true).await;
                    }
                }
            }
        }
    });
    (rx, AbortOnDrop(task))
}

impl Session {
    /// Whether the server is reachable, as far as sync knows.
    pub fn connection(&self) -> Connection {
        *self.connection.borrow()
    }

    /// The connection, then every change, while the session lives.
    pub fn connection_updates(&self) -> Live<Connection> {
        Live::from_watch(self.connection.clone())
    }

    /// Try the server now instead of waiting for the next check.
    pub async fn reconnect(&self) {
        self.sync.start().await;
    }
}
