//! A signed-in account: sync, the room list, media, signing out.

use std::sync::Arc;

use mtrx_core::{self as core, OwnedMxcUri, OwnedRoomId, OwnedUserId};

use crate::{
    ConnectionState, Media, MtrxError, TaskHandle, Timeline, User,
    runtime::{InRuntime, follow, run},
};

/// Receives the connection state, now and after every change.
#[uniffi::export(with_foreign)]
pub trait ConnectionListener: Send + Sync {
    fn on_update(&self, state: ConnectionState);
}

/// A signed-in account.
#[derive(uniffi::Object)]
pub struct Session {
    inner: InRuntime<Arc<core::Session>>,
}

impl Session {
    pub(crate) fn new(session: core::Session) -> Arc<Self> {
        Arc::new(Self {
            inner: InRuntime::new(Arc::new(session)),
        })
    }

    pub(crate) fn core(&self) -> Arc<core::Session> {
        Arc::clone(&self.inner)
    }
}

#[uniffi::export]
impl Session {
    /// `@user:server`.
    pub fn user_id(&self) -> String {
        self.inner.user_id().to_string()
    }

    pub fn device_id(&self) -> String {
        self.inner.device_id().to_string()
    }

    /// The homeserver's URL.
    pub fn homeserver(&self) -> String {
        self.inner.homeserver()
    }

    /// Start syncing in the background (when the app comes to the
    /// foreground). Room lists and timelines only update while sync runs.
    pub async fn start_sync(&self) {
        let session = self.core();
        run(async move { session.start_sync().await }).await
    }

    /// Stop syncing and save the sync position (when the app goes to the
    /// background).
    pub async fn stop_sync(&self) {
        let session = self.core();
        run(async move { session.stop_sync().await }).await
    }

    /// Try the server now instead of waiting for the next retry, e.g. when
    /// the device's network comes back.
    pub async fn reconnect(&self) {
        let session = self.core();
        run(async move { session.reconnect().await }).await
    }

    pub fn connection(&self) -> ConnectionState {
        self.inner.connection().into()
    }

    /// Call `listener` with the connection state, now and on every change.
    pub fn subscribe_connection(&self, listener: Arc<dyn ConnectionListener>) -> Arc<TaskHandle> {
        let connection = self.inner.connection_updates();
        follow(connection, ConnectionState::from, move |c| {
            listener.on_update(c)
        })
    }

    /// The live timeline of a room (`RoomSummary.id`).
    pub async fn timeline(&self, room_id: String) -> Result<Arc<Timeline>, MtrxError> {
        let room_id = OwnedRoomId::try_from(room_id.as_str()).map_err(MtrxError::invalid)?;
        let session = self.core();
        run(async move { Ok(Timeline::new(session.timeline(&room_id).await?)) }).await
    }

    /// A thumbnail of the picture at `mxc_uri` (an avatar), cropped to
    /// about `size`×`size` pixels, as encoded image bytes (PNG, JPEG, ...).
    /// Downloads are cached.
    pub async fn avatar(&self, mxc_uri: String, size: u32) -> Result<Vec<u8>, MtrxError> {
        let uri = OwnedMxcUri::from(mxc_uri);
        uri.validate().map_err(MtrxError::invalid)?;
        let session = self.core();
        run(async move { Ok(session.thumbnail(&uri, size).await?) }).await
    }

    /// What to show for a message's image or video: its thumbnail if it has
    /// one, else the whole file, decrypted, as encoded image bytes. Downloads
    /// are cached.
    pub async fn media_preview(&self, media: Media) -> Result<Vec<u8>, MtrxError> {
        let source = media.media_ref()?;
        let session = self.core();
        run(async move { Ok(session.media_preview(&source).await?) }).await
    }

    /// Someone's name and picture from their profile on the server.
    pub async fn user_profile(&self, user_id: String) -> Result<User, MtrxError> {
        let user = OwnedUserId::try_from(user_id.as_str()).map_err(MtrxError::invalid)?;
        let session = self.core();
        run(async move { Ok(User::from(&session.user_profile(&user).await?)) }).await
    }

    /// Sign out on the server and delete all local data of the account,
    /// even if the server is unreachable. Do not use the session afterwards.
    pub async fn logout(&self) -> Result<(), MtrxError> {
        let session = self.core();
        run(async move { Ok(session.logout().await?) }).await
    }

    /// Delete all local data of a session the server no longer accepts
    /// ([`ConnectionState::SignedOut`]), to sign in again. Do not use the
    /// session afterwards.
    pub async fn forget(&self) -> Result<(), MtrxError> {
        let session = self.core();
        run(async move { Ok(session.forget().await?) }).await
    }
}
