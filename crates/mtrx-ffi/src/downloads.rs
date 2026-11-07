//! Downloads: a message's file to the cache, to open or share from there,
//! with its progress.

use std::sync::Arc;

use mtrx_core::{self as core, OwnedEventId};

use crate::{
    Media, MediaKind, MtrxError, Room, Session, TaskHandle,
    runtime::{enter, follow, run},
};

#[derive(Clone, Debug, PartialEq, uniffi::Enum)]
pub enum DownloadState {
    /// `total` is the file's length, if known.
    Downloading {
        received: u64,
        total: Option<u64>,
    },
    Done {
        path: String,
    },
    /// Why, in English.
    Failed {
        message: String,
    },
    Cancelled,
}

/// A file being downloaded, or downloaded.
#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct Download {
    pub id: u64,
    /// As sent.
    pub file_name: String,
    pub kind: MediaKind,
    /// As sent, in bytes.
    pub size: Option<u64>,
    /// The `Media.source` it was started from: a message's file finds its
    /// download by it.
    pub source: String,
    pub room_id: String,
    /// When it started.
    pub room_name: String,
    pub event_id: Option<String>,
    pub state: DownloadState,
    /// From 0 to 1, while downloading with a known length.
    pub fraction: Option<f32>,
    /// A line about it: "3.2 of 10.0 MB", "Starting…", "Failed: …".
    pub status: String,
}

impl From<core::Download> for Download {
    fn from(download: core::Download) -> Self {
        Self {
            id: download.id,
            fraction: download.fraction(),
            status: download.status(),
            source: serde_json::to_string(download.source()).expect("media sources serialize"),
            file_name: download.file_name,
            kind: download.kind.into(),
            size: download.size,
            room_id: download.room_id.to_string(),
            room_name: download.room_name,
            event_id: download.event_id.map(|id| id.to_string()),
            state: match download.state {
                core::DownloadState::Downloading { received, total } => {
                    DownloadState::Downloading { received, total }
                }
                core::DownloadState::Done { path } => DownloadState::Done {
                    path: path.to_string_lossy().into_owned(),
                },
                core::DownloadState::Failed(message) => DownloadState::Failed { message },
                core::DownloadState::Cancelled => DownloadState::Cancelled,
            },
        }
    }
}

fn downloads(list: Vec<core::Download>) -> Vec<Download> {
    list.into_iter().map(Download::from).collect()
}

/// Receives the downloads, now and after every change (progress at most
/// ten times a second).
#[uniffi::export(with_foreign)]
pub trait DownloadsListener: Send + Sync {
    fn on_update(&self, downloads: Vec<Download>);
}

#[uniffi::export]
impl Room {
    /// Download a message's file to the cache, to open or share from
    /// there. The same file again gives the same download.
    pub fn download(&self, media: Media, event_id: Option<String>) -> Result<u64, MtrxError> {
        let media = media.to_core()?;
        let event_id = event_id
            .map(|id| OwnedEventId::try_from(id.as_str()))
            .transpose()
            .map_err(MtrxError::invalid)?;
        let _runtime = enter();
        Ok(self
            .core()
            .download(&media, event_id.as_deref(), core::SaveTo::Cache))
    }
}

#[uniffi::export]
impl Session {
    /// The downloads, newest first.
    pub fn downloads(&self) -> Vec<Download> {
        downloads(self.core().downloads().get())
    }

    /// Call `listener` with the downloads, now and after every change.
    pub fn subscribe_downloads(&self, listener: Arc<dyn DownloadsListener>) -> Arc<TaskHandle> {
        follow(self.core().downloads(), downloads, move |list| {
            listener.on_update(list)
        })
    }

    /// The path of download `id` once done; an error if it failed, was
    /// cancelled or removed.
    pub async fn download_finished(&self, id: u64) -> Result<String, MtrxError> {
        let session = self.core();
        run(async move {
            let path = session.finished(id).await?;
            Ok(path.to_string_lossy().into_owned())
        })
        .await
    }

    /// Stop a download and delete what it wrote.
    pub fn cancel_download(&self, id: u64) {
        let _runtime = enter();
        self.core().cancel_download(id);
    }

    /// Start a failed or cancelled download again.
    pub fn retry_download(&self, id: u64) {
        let _runtime = enter();
        self.core().retry_download(id);
    }

    /// Take a download off the list; its file stays.
    pub fn remove_download(&self, id: u64) {
        let _runtime = enter();
        self.core().remove_download(id);
    }

    /// Take every download that doesn't run off the list.
    pub fn clear_downloads(&self) {
        let _runtime = enter();
        self.core().clear_downloads();
    }
}
