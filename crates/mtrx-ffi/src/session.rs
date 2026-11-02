//! A signed-in account: sync, the room list, media, signing out.

use std::{
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

use futures_util::StreamExt;
use mtrx_core::{self as core, OwnedMxcUri, OwnedRoomId, OwnedUserId};
use sha2::{Digest, Sha256};

use crate::{
    ConnectionState, Media, MtrxError, TaskHandle, Timeline, UserProfile,
    runtime::{InRuntime, run},
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
        let mut updates = Box::pin(self.inner.connection_updates());
        TaskHandle::spawn(async move {
            while let Some(connection) = updates.next().await {
                listener.on_update(connection.into());
            }
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

    /// The whole file of a message, decrypted, saved in the cache directory
    /// (once); returns its path. Deleted when signing out.
    pub async fn media_file(&self, media: Media) -> Result<String, MtrxError> {
        let source = media.media_ref()?;
        let session = self.core();
        run(async move {
            let dir = session.profile().cache_dir().join("media");
            let path = dir.join(media_file_name(source.uri().as_str(), &media.filename));
            if !tokio::fs::try_exists(&path).await? {
                let bytes = session.media_content(&source).await?;
                write_atomically(&dir, &path, &bytes).await?;
            }
            Ok(path.to_string_lossy().into_owned())
        })
        .await
    }

    /// Someone's name and picture from their profile on the server, e.g.
    /// for a direct chat's `directUserId`.
    pub async fn user_profile(&self, user_id: String) -> Result<UserProfile, MtrxError> {
        let user = OwnedUserId::try_from(user_id.as_str()).map_err(MtrxError::invalid)?;
        let session = self.core();
        run(async move { Ok(UserProfile::from(session.user_profile(&user).await?)) }).await
    }

    /// Invite `user_id` to a room or space we are in (`RoomSummary.id`).
    pub async fn invite(&self, room_id: String, user_id: String) -> Result<(), MtrxError> {
        let room = OwnedRoomId::try_from(room_id.as_str()).map_err(MtrxError::invalid)?;
        let user = OwnedUserId::try_from(user_id.as_str()).map_err(MtrxError::invalid)?;
        let session = self.core();
        run(async move { Ok(session.invite(&room, &user).await?) }).await
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

/// A media file's name in the cache: from its URI (so each is downloaded
/// once), with the extension of its file name (so other apps can open it).
fn media_file_name(uri: &str, filename: &str) -> String {
    let hash: String = Sha256::digest(uri.as_bytes())[..16]
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    let extension = Path::new(filename)
        .extension()
        .and_then(|e| e.to_str())
        .filter(|e| e.len() <= 10 && e.chars().all(|c| c.is_ascii_alphanumeric()));
    match extension {
        Some(extension) => format!("{hash}.{}", extension.to_ascii_lowercase()),
        None => hash,
    }
}

/// Through a temporary file, so a reader never sees half a file.
async fn write_atomically(dir: &Path, path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    static COUNT: AtomicU64 = AtomicU64::new(0);
    tokio::fs::create_dir_all(dir).await?;
    let partial = dir.join(format!(".{}.part", COUNT.fetch_add(1, Ordering::Relaxed)));
    tokio::fs::write(&partial, bytes).await?;
    tokio::fs::rename(&partial, path).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn media_files_are_named_by_uri_with_a_safe_extension() {
        let name = media_file_name("mxc://example.org/abc", "Ice Sheet.PNG");
        assert_eq!(name.len(), 32 + 4);
        assert!(name.ends_with(".png"));
        assert_eq!(name, media_file_name("mxc://example.org/abc", "other.png"));
        assert_ne!(
            name,
            media_file_name("mxc://example.org/abd", "Ice Sheet.PNG")
        );
        assert_eq!(
            media_file_name("mxc://example.org/abc", "no-extension").len(),
            32
        );
        assert_eq!(
            media_file_name("mxc://example.org/abc", "evil.p/../ng").len(),
            32
        );
    }

    #[test]
    fn atomic_writes_replace_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let media = dir.path().join("media");
        let path = media.join("file.txt");
        futures_executor::block_on(crate::runtime::run({
            let (media, path) = (media.clone(), path.clone());
            async move {
                write_atomically(&media, &path, b"one").await.unwrap();
                write_atomically(&media, &path, b"two").await.unwrap();
            }
        }));
        assert_eq!(std::fs::read(&path).unwrap(), b"two");
        assert_eq!(std::fs::read_dir(&media).unwrap().count(), 1);
    }
}
