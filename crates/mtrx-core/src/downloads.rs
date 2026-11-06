//! Downloads: the files of messages the user asked for, to keep them or to
//! open them in another app, with their progress.
//!
//! A download streams the file to a `.part` file next to where it goes, and
//! renames it when complete. An encrypted file's ciphertext goes to the
//! profile's cache first, and is decrypted from there (checking its hash),
//! so no ciphertext lands in the destination folder. Nothing holds a whole
//! file in memory, except a file the SDK's media store already holds.
//!
//! The list is kept in the store, with the running downloads and the
//! [`KEEP`] newest others. Downloads that were running when the app quit
//! come back as failed, to retry.

use std::{
    borrow::Cow,
    collections::HashMap,
    fs::File,
    io::{BufReader, BufWriter, Read, Write},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant, SystemTime},
};

use matrix_sdk::{
    Client,
    media::{MediaFormat, MediaRequestParameters},
    ruma::{
        EventId, MxcUri, OwnedEventId, OwnedRoomId,
        api::{
            Metadata, OutgoingRequestExt,
            auth_scheme::SendAccessToken,
            client::{authenticated_media, media},
        },
        events::room::MediaSource,
    },
};
use matrix_sdk_crypto::{AttachmentDecryptor, MediaEncryptionInfo};
use serde::{Deserialize, Serialize};
use tokio::{io::AsyncWriteExt, sync::watch};
use tracing::warn;

use crate::{AbortOnDrop, Error, Live, Media, MediaKind, MediaRef, Result, RoomHandle, Session};

/// Store key of the list.
const STORE_KEY: &[u8] = b"mtrx.downloads";
/// The finished, failed and cancelled downloads the list keeps, newest
/// first; it keeps every running one too.
const KEEP: usize = 20;
/// Progress is reported at most this often (10 times a second).
const STEP: Duration = Duration::from_millis(100);
/// A server that sends nothing for this long has stopped.
const STALL: Duration = Duration::from_secs(60);
/// Bytes written to disk at once.
const BUFFER: usize = 256 * 1024;

const QUIT: &str = "Stopped when Mtrx quit";
const GONE: &str = "Moved or deleted";
const DAMAGED: &str = "The file is damaged";
const UNREACHABLE: &str = "Couldn't reach the server";
const STALLED: &str = "The server stopped sending";
const NO_DOWNLOADS_FOLDER: &str = "There is no Downloads folder";

pub type DownloadId = u64;

/// Where a download goes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SaveTo {
    /// The user's Downloads folder (desktop), under the sender's name made
    /// safe and unique: "report.pdf", "report (1).pdf".
    Downloads,
    /// This path, replaced if there (a save dialog asked).
    File(PathBuf),
    /// The profile's cache, to open or share from (phones): one file per
    /// URI, so a file already there is done at once.
    Cache,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DownloadState {
    /// `total` is the server's length, else the size the sender gave.
    Downloading {
        received: u64,
        total: Option<u64>,
    },
    Done {
        path: PathBuf,
    },
    /// Why, in English: "The file is damaged", "Stopped when Mtrx quit".
    Failed(String),
    Cancelled,
}

/// A file the user downloaded or is downloading ([`Session::downloads`]).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Download {
    pub id: DownloadId,
    /// As sent.
    pub file_name: String,
    pub kind: MediaKind,
    /// As sent, in bytes.
    pub size: Option<u64>,
    pub room_id: OwnedRoomId,
    /// When it started.
    pub room_name: String,
    pub event_id: Option<OwnedEventId>,
    pub to: SaveTo,
    pub started: SystemTime,
    pub state: DownloadState,
    /// To retry; holds an encrypted file's key (the store is encrypted).
    source: MediaRef,
    /// The file name it is saved under: [`Download::file_name`] made safe.
    name: String,
    /// Where it goes, while it runs.
    target: Option<PathBuf>,
}

impl Download {
    fn new(
        id: DownloadId,
        media: &Media,
        room_id: OwnedRoomId,
        room_name: String,
        event_id: Option<OwnedEventId>,
        to: SaveTo,
    ) -> Self {
        Self {
            id,
            file_name: media.filename.clone(),
            kind: media.kind,
            size: media.size,
            room_id,
            room_name,
            event_id,
            to,
            started: SystemTime::now(),
            state: DownloadState::Downloading {
                received: 0,
                total: media.size,
            },
            source: media.source.clone(),
            name: safe_name(&media.filename, media.mimetype.as_deref()),
            target: None,
        }
    }

    /// A download of `file_name` in a room, running, for frontends' tests.
    #[cfg(feature = "test-support")]
    pub fn example(id: DownloadId, file_name: &str) -> Self {
        let media = Media {
            kind: MediaKind::File,
            filename: file_name.into(),
            size: None,
            mimetype: None,
            width: None,
            height: None,
            source: MediaRef::plain("mxc://example.org/file"),
        };
        let room = "!room:example.org".try_into().expect("a room ID");
        Self::new(id, &media, room, "Room".into(), None, SaveTo::Downloads)
    }

    pub fn is_running(&self) -> bool {
        matches!(self.state, DownloadState::Downloading { .. })
    }

    /// From 0 to 1, while it runs and its length is known.
    pub fn fraction(&self) -> Option<f32> {
        match self.state {
            DownloadState::Downloading {
                received,
                total: Some(total),
            } if total > 0 => Some((received as f64 / total as f64).min(1.0) as f32),
            _ => None,
        }
    }

    /// A line about it: "3.2 of 10.0 MB", "Starting…", "10.0 MB · Ice
    /// sheets", "Failed: …", "Cancelled".
    pub fn status(&self) -> String {
        match &self.state {
            DownloadState::Downloading { received: 0, .. } => "Starting…".into(),
            DownloadState::Downloading {
                received,
                total: Some(total),
            } if *total > 0 => {
                let unit = unit_of(*total);
                format!(
                    "{} of {} {}",
                    in_unit(*received, unit),
                    in_unit(*total, unit),
                    UNITS[unit]
                )
            }
            DownloadState::Downloading { received, .. } => human_size(*received),
            DownloadState::Done { .. } => match self.size {
                Some(size) => format!("{} · {}", human_size(size), self.room_name),
                None => self.room_name.clone(),
            },
            DownloadState::Failed(message) => format!("Failed: {message}"),
            DownloadState::Cancelled => "Cancelled".into(),
        }
    }

    /// The file's URI: a message's file and its download have the same.
    pub fn uri(&self) -> &MxcUri {
        self.source.uri()
    }

    /// The message's file it downloads.
    pub fn source(&self) -> &MediaRef {
        &self.source
    }
}

const UNITS: [&str; 5] = ["B", "kB", "MB", "GB", "TB"];

/// A file size in decimal units, e.g. "2.4 MB".
pub fn human_size(bytes: u64) -> String {
    let unit = unit_of(bytes);
    format!("{} {}", in_unit(bytes, unit), UNITS[unit])
}

/// The index in [`UNITS`] that `bytes` is shown in.
fn unit_of(bytes: u64) -> usize {
    if bytes < 1000 {
        return 0;
    }
    let mut value = bytes as f64 / 1000.0;
    let mut unit = 1;
    // What rounds to 1000 shows as 1.0 of the next unit.
    while value >= 999.95 && unit < UNITS.len() - 1 {
        value /= 1000.0;
        unit += 1;
    }
    unit
}

/// `bytes` as a number of `unit`s: "3.2", "120", "512" (bytes).
fn in_unit(bytes: u64, unit: usize) -> String {
    if unit == 0 {
        return bytes.to_string();
    }
    let value = bytes as f64 / 1000f64.powi(unit as i32);
    if value < 99.95 {
        format!("{value:.1}")
    } else {
        format!("{value:.0}")
    }
}

impl RoomHandle {
    /// Start downloading a message's file. It's in [`Session::downloads`]
    /// at once. The same file already downloading to the same place (or in
    /// the cache) gives that download.
    pub fn download(&self, media: &Media, event: Option<&EventId>, to: SaveTo) -> DownloadId {
        let room_name = self
            .room
            .cached_display_name()
            .map(|n| n.to_string())
            .unwrap_or_else(|| self.id().to_string());
        self.session.downloads.start(
            media,
            self.id().to_owned(),
            room_name,
            event.map(ToOwned::to_owned),
            to,
        )
    }
}

impl Session {
    /// The downloads, newest first.
    pub fn downloads(&self) -> Live<Vec<Download>> {
        Live::from_watch(self.downloads.shared.list.subscribe())
    }

    /// Where download `id` is once done; an error if it failed, was
    /// cancelled or removed.
    pub async fn finished(&self, id: DownloadId) -> Result<PathBuf> {
        self.downloads.finished(id).await
    }

    /// Stop a download and delete what it wrote.
    pub fn cancel_download(&self, id: DownloadId) {
        self.downloads.cancel(id);
    }

    /// Start a failed or cancelled download again, from the start.
    pub fn retry_download(&self, id: DownloadId) {
        self.downloads.retry(id);
    }

    /// Take a download off the list (stopping it if it runs); its file stays.
    pub fn remove_download(&self, id: DownloadId) {
        self.downloads.remove(id);
    }

    /// Take every download that doesn't run off the list.
    pub fn clear_downloads(&self) {
        self.downloads.clear();
    }
}

/// A session's downloads: the list, and a task per running download.
pub(crate) struct Downloads {
    shared: Arc<Shared>,
    /// Each download's latest task, aborted when dropped.
    tasks: Mutex<HashMap<DownloadId, AbortOnDrop>>,
}

/// What download tasks share.
struct Shared {
    client: Client,
    places: Places,
    list: watch::Sender<Vec<Download>>,
    next_id: AtomicU64,
    /// One save at a time, so the last one saves the latest list.
    saving: tokio::sync::Mutex<()>,
}

/// Where downloads go.
#[derive(Clone, Debug)]
pub(crate) struct Places {
    /// The user's Downloads folder, if the platform has one.
    pub(crate) downloads: Option<PathBuf>,
    /// The profile's cache directory.
    pub(crate) cache: PathBuf,
}

impl Places {
    pub(crate) fn new(cache: &Path) -> Self {
        Self {
            downloads: downloads_folder(),
            cache: cache.to_owned(),
        }
    }

    /// Where an encrypted file's ciphertext goes while it downloads.
    fn ciphertext(&self, id: DownloadId) -> PathBuf {
        self.cache.join("downloads").join(format!("{id}.part"))
    }

    /// A file's place in the cache: a folder per URI, so other apps get
    /// the sender's name.
    fn cached(&self, uri: &MxcUri, name: &str) -> PathBuf {
        let (server, media_id) = match uri.parts() {
            Ok((server, media_id)) => (server.as_str(), media_id),
            Err(_) => ("invalid", uri.as_str()),
        };
        self.cache
            .join("media")
            .join(path_safe(server))
            .join(path_safe(media_id))
            .join(name)
    }

    /// Where `download` goes, given the others.
    fn target(&self, download: &Download, list: &[Download]) -> Result<PathBuf, String> {
        match &download.to {
            SaveTo::File(path) => Ok(path.clone()),
            SaveTo::Cache => Ok(self.cached(download.uri(), &download.name)),
            SaveTo::Downloads => {
                let folder = self.downloads.as_ref().ok_or(NO_DOWNLOADS_FOLDER)?;
                Ok(unique_path(folder, &download.name, |path| {
                    taken(path)
                        || (list.iter())
                            .any(|d| d.is_running() && d.target.as_deref() == Some(path))
                }))
            }
        }
    }

    /// Set `download` going: where it goes, and its state now. The job to
    /// run, unless it's done (in the cache) or can't start.
    fn prepare(&self, download: &mut Download, list: &[Download]) -> Option<Job> {
        download.target = None;
        match self.target(download, list) {
            Err(message) => {
                download.state = DownloadState::Failed(message);
                None
            }
            Ok(path) if download.to == SaveTo::Cache && path.exists() => {
                download.state = DownloadState::Done { path };
                None
            }
            Ok(path) => {
                download.state = DownloadState::Downloading {
                    received: 0,
                    total: download.size,
                };
                download.target = Some(path.clone());
                Some(Job {
                    id: download.id,
                    source: download.source.clone(),
                    size: download.size,
                    target: path,
                    unique: download.to == SaveTo::Downloads,
                })
            }
        }
    }
}

/// The user's Downloads folder, else `~/Downloads`.
#[cfg(not(any(target_os = "ios", target_os = "android")))]
fn downloads_folder() -> Option<PathBuf> {
    let dirs = directories::UserDirs::new()?;
    Some(match dirs.download_dir() {
        Some(folder) => folder.to_owned(),
        None => dirs.home_dir().join("Downloads"),
    })
}

/// The phones keep downloads in the cache.
#[cfg(any(target_os = "ios", target_os = "android"))]
fn downloads_folder() -> Option<PathBuf> {
    None
}

/// What a download task needs.
struct Job {
    id: DownloadId,
    source: MediaRef,
    size: Option<u64>,
    target: PathBuf,
    /// Pick another name if the target is taken when done (Downloads).
    unique: bool,
}

impl Downloads {
    /// The list kept in the store: what ran when the app quit failed, and
    /// files that are gone need downloading again.
    pub(crate) async fn load(client: Client, places: Places) -> Self {
        let stored = match client.state_store().get_custom_value(STORE_KEY).await {
            Ok(Some(bytes)) => serde_json::from_slice(&bytes).unwrap_or_else(|e| {
                warn!("could not read the downloads: {e}");
                Vec::new()
            }),
            Ok(None) => Vec::new(),
            Err(e) => {
                warn!("could not load the downloads: {e}");
                Vec::new()
            }
        };
        let (list, changed) = reopened(stored, &places);
        let next_id = list.iter().map(|d| d.id + 1).max().unwrap_or(1);
        let shared = Arc::new(Shared {
            client,
            places,
            list: watch::Sender::new(list),
            next_id: AtomicU64::new(next_id),
            saving: Default::default(),
        });
        if changed {
            shared.clone().save().await;
        }
        Self {
            shared,
            tasks: Default::default(),
        }
    }

    fn start(
        &self,
        media: &Media,
        room_id: OwnedRoomId,
        room_name: String,
        event_id: Option<OwnedEventId>,
        to: SaveTo,
    ) -> DownloadId {
        let mut tasks = self.tasks.lock().expect("not poisoned");
        let (mut id, mut job) = (0, None);
        let added = self.shared.list.send_if_modified(|list| {
            let same = list.iter().find(|d| {
                d.source == media.source
                    && d.to == to
                    && (d.is_running()
                        || (to == SaveTo::Cache
                            && matches!(&d.state, DownloadState::Done { path } if path.exists())))
            });
            if let Some(same) = same {
                id = same.id;
                return false;
            }
            id = self.shared.next_id.fetch_add(1, Ordering::Relaxed);
            let mut download = Download::new(id, media, room_id, room_name, event_id, to);
            job = self.shared.places.prepare(&mut download, list);
            list.insert(0, download);
            trim(list);
            true
        });
        if added {
            self.shared.changed();
        }
        if let Some(job) = job {
            self.spawn(&mut tasks, job);
        }
        id
    }

    /// Run `job`, after the download's previous task stopped (it may still
    /// be deleting its files).
    fn spawn(&self, tasks: &mut HashMap<DownloadId, AbortOnDrop>, job: Job) {
        tasks.retain(|_, task| !task.0.is_finished());
        let id = job.id;
        let previous = tasks.remove(&id);
        let shared = self.shared.clone();
        let task = tokio::spawn(async move {
            if let Some(mut previous) = previous {
                previous.0.abort();
                let _ = (&mut previous.0).await;
            }
            let result = shared.fetch(&job).await;
            shared.end(id, result);
        });
        tasks.insert(id, AbortOnDrop(task));
    }

    async fn finished(&self, id: DownloadId) -> Result<PathBuf> {
        let mut list = self.shared.list.subscribe();
        let list = list
            .wait_for(|list| list.iter().all(|d| d.id != id || !d.is_running()))
            .await
            .map_err(|_| Error::Download("Mtrx is closing".into()))?;
        match list.iter().find(|d| d.id == id).map(|d| &d.state) {
            Some(DownloadState::Done { path }) => Ok(path.clone()),
            Some(DownloadState::Failed(message)) => Err(Error::Download(message.clone())),
            Some(DownloadState::Cancelled) => Err(Error::Download("Cancelled".into())),
            Some(DownloadState::Downloading { .. }) | None => {
                Err(Error::Download("Removed from the list".into()))
            }
        }
    }

    fn cancel(&self, id: DownloadId) {
        let tasks = self.tasks.lock().expect("not poisoned");
        // Its files go as the task stops; a retry waits for that.
        if let Some(task) = tasks.get(&id) {
            task.0.abort();
        }
        let cancelled = self.shared.update(id, |d| {
            d.is_running().then(|| {
                d.state = DownloadState::Cancelled;
                d.target = None;
            })
        });
        if cancelled.is_some() {
            self.shared.changed();
        }
    }

    fn retry(&self, id: DownloadId) {
        let mut tasks = self.tasks.lock().expect("not poisoned");
        let mut job = None;
        let retried = self.shared.list.send_if_modified(|list| {
            let Some(index) = list.iter().position(|d| d.id == id) else {
                return false;
            };
            if list[index].is_running() || matches!(list[index].state, DownloadState::Done { .. }) {
                return false;
            }
            let mut download = list[index].clone();
            job = self.shared.places.prepare(&mut download, list);
            list[index] = download;
            true
        });
        if retried {
            self.shared.changed();
        }
        if let Some(job) = job {
            self.spawn(&mut tasks, job);
        }
    }

    fn remove(&self, id: DownloadId) {
        // Aborts it; its files go as the task stops.
        drop(self.tasks.lock().expect("not poisoned").remove(&id));
        let removed = self.shared.list.send_if_modified(|list| {
            let before = list.len();
            list.retain(|d| d.id != id);
            list.len() != before
        });
        if removed {
            self.shared.changed();
        }
    }

    fn clear(&self) {
        let cleared = self.shared.list.send_if_modified(|list| {
            let before = list.len();
            list.retain(Download::is_running);
            list.len() != before
        });
        if cleared {
            self.shared.changed();
        }
    }
}

impl Shared {
    /// Apply `change` to download `id`, if it's there and `change` gives
    /// something.
    fn update<R>(
        &self,
        id: DownloadId,
        change: impl FnOnce(&mut Download) -> Option<R>,
    ) -> Option<R> {
        let mut result = None;
        self.list.send_if_modified(|list| {
            result = list.iter_mut().find(|d| d.id == id).and_then(change);
            result.is_some()
        });
        result
    }

    /// Save the list, after the changes made so far.
    fn changed(self: &Arc<Self>) {
        tokio::spawn(self.clone().save());
    }

    async fn save(self: Arc<Self>) {
        let _turn = self.saving.lock().await;
        let list = self.list.borrow().clone();
        let saved = match serde_json::to_vec(&list) {
            Ok(bytes) => (self.client.state_store())
                .set_custom_value(STORE_KEY, bytes)
                .await
                .map(drop)
                .map_err(|e| e.to_string()),
            Err(e) => Err(e.to_string()),
        };
        if let Err(e) = saved {
            warn!("could not save the downloads: {e}");
        }
    }

    fn progress(&self, id: DownloadId, received: u64, total: Option<u64>) {
        self.update(id, |d| {
            d.is_running().then(|| {
                d.state = DownloadState::Downloading { received, total };
            })
        });
    }

    /// The end of download `id`'s task. If it was cancelled or removed
    /// meanwhile, the file it just placed goes.
    fn end(self: &Arc<Self>, id: DownloadId, result: Result<PathBuf, String>) {
        let ended = self.update(id, |d| {
            d.is_running().then(|| {
                d.target = None;
                d.state = match &result {
                    Ok(path) => DownloadState::Done { path: path.clone() },
                    Err(message) => DownloadState::Failed(message.clone()),
                };
            })
        });
        match (ended, result) {
            (Some(()), _) => self.changed(),
            (None, Ok(path)) => {
                let _ = std::fs::remove_file(path);
            }
            (None, Err(_)) => {}
        }
    }

    /// Get the file to its target; where it went.
    async fn fetch(&self, job: &Job) -> Result<PathBuf, String> {
        if let Some(folder) = job.target.parent() {
            tokio::fs::create_dir_all(folder)
                .await
                .map_err(write_failed)?;
        }
        let part = Part::new(part_path(&job.target));
        if let Some(bytes) = self.stored(&job.source.file).await {
            tokio::fs::write(part.path(), bytes)
                .await
                .map_err(write_failed)?;
        } else {
            match &job.source.file {
                MediaSource::Plain(uri) => self.stream(job, uri, part.path()).await?,
                MediaSource::Encrypted(file) => {
                    let ciphertext = Part::new(self.places.ciphertext(job.id));
                    if let Some(folder) = ciphertext.path().parent() {
                        tokio::fs::create_dir_all(folder)
                            .await
                            .map_err(write_failed)?;
                    }
                    self.stream(job, &file.url, ciphertext.path()).await?;
                    decrypt(ciphertext.path(), part.path(), (**file).clone().into()).await?;
                }
            }
        }
        place(part, &job.target, job.unique)
    }

    /// The file from the SDK's media store, if it has it (a viewer showed
    /// it, say). Decrypted.
    async fn stored(&self, source: &MediaSource) -> Option<Vec<u8>> {
        let request = MediaRequestParameters {
            source: source.clone(),
            format: MediaFormat::File,
        };
        let store = self.client.media_store().lock().await.ok()?;
        store.get_media_content(&request).await.ok().flatten()
    }

    /// Download `uri` to `path`, reporting progress.
    async fn stream(&self, job: &Job, uri: &MxcUri, path: &Path) -> Result<(), String> {
        let mut response = self.response(uri).await?;
        let total = response.content_length().or(job.size);
        let file = tokio::fs::File::create(path).await.map_err(write_failed)?;
        let mut file = tokio::io::BufWriter::with_capacity(BUFFER, file);
        let mut received = 0;
        self.progress(job.id, received, total);
        let mut steps = Throttle::new(STEP, Instant::now());
        while let Some(chunk) = tokio::time::timeout(STALL, response.chunk())
            .await
            .map_err(|_| STALLED)?
            .map_err(|_| UNREACHABLE)?
        {
            file.write_all(&chunk).await.map_err(write_failed)?;
            received += chunk.len() as u64;
            if steps.due(Instant::now()) {
                self.progress(job.id, received, total);
            }
        }
        file.flush().await.map_err(write_failed)?;
        self.progress(job.id, received, total);
        Ok(())
    }

    /// The server's answer for `uri`'s content. An expired token is
    /// refreshed once.
    async fn response(&self, uri: &MxcUri) -> Result<reqwest::Response, String> {
        let mut refreshed = false;
        loop {
            let request = content_request(&self.client, uri).await?;
            let response =
                (self.client.http_client().execute(request).await).map_err(|_| UNREACHABLE)?;
            let status = response.status();
            if status.is_success() {
                return Ok(response);
            }
            if status == reqwest::StatusCode::UNAUTHORIZED
                && !refreshed
                && unknown_token(response).await
            {
                refreshed = true;
                self.client
                    .refresh_access_token()
                    .await
                    .map_err(|_| "The server no longer accepts this session".to_owned())?;
                continue;
            }
            return Err(match status {
                reqwest::StatusCode::NOT_FOUND => "The file is no longer on the server".into(),
                status => format!("The server answered {status}"),
            });
        }
    }
}

/// The request for `uri`'s content: authenticated if the server has it,
/// as the SDK chooses.
async fn content_request(client: &Client, uri: &MxcUri) -> Result<reqwest::Request, String> {
    let versions = client.supported_versions().await.map_err(|_| UNREACHABLE)?;
    let homeserver = client.homeserver().to_string();
    let token = client.access_token();
    let access = token
        .as_deref()
        .map_or(SendAccessToken::None, SendAccessToken::IfRequired);
    let invalid = |e: &dyn std::fmt::Display| format!("Invalid file address: {e}");
    let request = if authenticated_media::get_content::v1::Request::PATH_BUILDER
        .is_supported(&versions)
    {
        let request = authenticated_media::get_content::v1::Request::from_uri(uri)
            .map_err(|e| invalid(&e))?;
        request.try_into_http_request::<Vec<u8>>(&homeserver, access, Cow::Owned(versions))
    } else {
        #[allow(deprecated)]
        let request = media::get_content::v3::Request::from_url(uri).map_err(|e| invalid(&e))?;
        request.try_into_http_request::<Vec<u8>>(&homeserver, access, Cow::Owned(versions))
    };
    let request = request.map_err(|e| invalid(&e))?;
    let mut request = reqwest::Request::try_from(request).map_err(|e| invalid(&e))?;
    // Instead of the client's timeout for the whole request: big files take
    // long, and a stall is caught on its own.
    *request.timeout_mut() = Some(Duration::from_secs(24 * 60 * 60));
    Ok(request)
}

/// Whether a 401 says the access token expired.
async fn unknown_token(response: reqwest::Response) -> bool {
    let Ok(body) = response.bytes().await else {
        return false;
    };
    serde_json::from_slice::<serde_json::Value>(&body)
        .is_ok_and(|error| error["errcode"] == "M_UNKNOWN_TOKEN")
}

/// Decrypt the ciphertext at `from` to `to`, in a blocking thread. It stops
/// if the download is cancelled (this future dropped).
async fn decrypt(from: &Path, to: &Path, key: MediaEncryptionInfo) -> Result<(), String> {
    let running = Arc::new(());
    let still = Arc::downgrade(&running);
    let (from, to) = (from.to_owned(), to.to_owned());
    tokio::task::spawn_blocking(move || decrypt_file(&from, &to, key, || still.strong_count() > 0))
        .await
        .map_err(|e| e.to_string())?
}

/// Decrypt the file at `from` to `to`, while `go_on`. Fails if its hash
/// isn't the one sent.
fn decrypt_file(
    from: &Path,
    to: &Path,
    key: MediaEncryptionInfo,
    go_on: impl Fn() -> bool,
) -> Result<(), String> {
    let mut input = BufReader::with_capacity(BUFFER, File::open(from).map_err(write_failed)?);
    let mut decrypted = AttachmentDecryptor::new(&mut input, key).map_err(|_| DAMAGED)?;
    let mut output = BufWriter::with_capacity(BUFFER, File::create(to).map_err(write_failed)?);
    let mut buffer = vec![0; BUFFER];
    loop {
        if !go_on() {
            return Err("Cancelled".into());
        }
        // The hash is checked at the end.
        let read = decrypted.read(&mut buffer).map_err(|_| DAMAGED)?;
        if read == 0 {
            break;
        }
        output.write_all(&buffer[..read]).map_err(write_failed)?;
    }
    output.flush().map_err(write_failed)
}

/// Move the complete `part` to `target`; if `unique` and `target` was
/// taken meanwhile, to a free name next to it.
fn place(part: Part, target: &Path, unique: bool) -> Result<PathBuf, String> {
    let target = match (unique, target.parent(), target.file_name()) {
        (true, Some(folder), Some(name)) if target.exists() => {
            unique_path(folder, &name.to_string_lossy(), taken)
        }
        _ => target.to_owned(),
    };
    std::fs::rename(part.path(), &target).map_err(write_failed)?;
    part.keep();
    Ok(target)
}

fn write_failed(e: std::io::Error) -> String {
    format!("Couldn't write the file: {e}")
}

/// A file being written: deleted when dropped (the download failed or was
/// cancelled), unless kept.
struct Part(Option<PathBuf>);

impl Part {
    fn new(path: PathBuf) -> Self {
        Self(Some(path))
    }

    fn path(&self) -> &Path {
        self.0.as_deref().expect("present until kept")
    }

    fn keep(mut self) {
        self.0 = None;
    }
}

impl Drop for Part {
    fn drop(&mut self) {
        if let Some(path) = self.0.take() {
            let _ = std::fs::remove_file(path);
        }
    }
}

/// `path` with `.part` added.
fn part_path(path: &Path) -> PathBuf {
    let mut part = path.as_os_str().to_owned();
    part.push(".part");
    part.into()
}

/// Whether a file is at `path`, or being downloaded there.
fn taken(path: &Path) -> bool {
    path.exists() || part_path(path).exists()
}

/// Reports at most once every `every`.
struct Throttle {
    every: Duration,
    last: Instant,
}

impl Throttle {
    /// The last report at `start`.
    fn new(every: Duration, start: Instant) -> Self {
        Self { every, last: start }
    }

    /// Whether to report at `now`.
    fn due(&mut self, now: Instant) -> bool {
        let due = now.duration_since(self.last) >= self.every;
        if due {
            self.last = now;
        }
        due
    }
}

/// The running downloads and the [`KEEP`] newest others, of a list
/// newest first.
fn trim(list: &mut Vec<Download>) {
    let mut others = 0;
    list.retain(|d| {
        if d.is_running() {
            return true;
        }
        others += 1;
        others <= KEEP
    });
}

/// The stored list as the app starts: downloads running when it quit
/// failed (their partial files deleted), and done ones whose file is gone
/// need downloading again. Whether anything changed.
fn reopened(mut list: Vec<Download>, places: &Places) -> (Vec<Download>, bool) {
    let mut changed = false;
    for download in &mut list {
        let failed = match &download.state {
            DownloadState::Downloading { .. } => {
                if let Some(target) = &download.target {
                    let _ = std::fs::remove_file(part_path(target));
                }
                let _ = std::fs::remove_file(places.ciphertext(download.id));
                QUIT
            }
            DownloadState::Done { path } if !path.exists() => GONE,
            _ => continue,
        };
        download.state = DownloadState::Failed(failed.into());
        download.target = None;
        changed = true;
    }
    (list, changed)
}

/// A file name from a sender, safe to save under: no folders, no leading
/// dots, control or reserved characters, no names Windows reserves; with
/// an extension from `mimetype` if it has none. "file" if nothing is left.
pub(crate) fn safe_name(name: &str, mimetype: Option<&str>) -> String {
    const MAX_LEN: usize = 200;
    let last = name.rsplit(['/', '\\']).next().unwrap_or_default();
    let cleaned: String = last
        .chars()
        .map(|c| match c {
            c if c.is_control() => '_',
            '<' | '>' | ':' | '"' | '|' | '?' | '*' => '_',
            c => c,
        })
        .collect();
    let trimmed = cleaned
        .trim()
        .trim_start_matches('.')
        .trim_end_matches(['.', ' ']);
    let mut name = if trimmed.is_empty() {
        "file".to_owned()
    } else {
        trimmed.to_owned()
    };
    let (stem, extension) = split_name(&name);
    let (reserved, has_extension) = (is_reserved(stem), extension.is_some());
    if reserved {
        name = format!("_{name}");
    }
    if !has_extension && let Some(extension) = mimetype.and_then(extension_of) {
        name = format!("{name}.{extension}");
    }
    if name.len() > MAX_LEN {
        let (stem, extension) = split_name(&name);
        let extension = extension.map(|e| format!(".{e}")).unwrap_or_default();
        let mut end = MAX_LEN.saturating_sub(extension.len()).min(stem.len());
        while !stem.is_char_boundary(end) {
            end -= 1;
        }
        name = format!("{}{extension}", &stem[..end]);
    }
    name
}

/// A file name's stem and extension: "report", "pdf".
fn split_name(name: &str) -> (&str, Option<&str>) {
    match name.rsplit_once('.') {
        Some((stem, extension)) if !stem.is_empty() && !extension.is_empty() => {
            (stem, Some(extension))
        }
        _ => (name, None),
    }
}

/// Names Windows keeps for devices, with any extension.
fn is_reserved(stem: &str) -> bool {
    let stem = stem.to_ascii_uppercase();
    matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || ((stem.starts_with("COM") || stem.starts_with("LPT"))
            && stem.len() == 4
            && stem.as_bytes()[3].is_ascii_digit())
}

/// The usual extension of a MIME type: "pdf", "jpg".
fn extension_of(mimetype: &str) -> Option<&'static str> {
    let extensions = mime_guess::get_mime_extensions_str(mimetype)?;
    let subtype = mimetype.split(['/', ';']).nth(1)?.trim();
    (extensions.iter().find(|e| e.eq_ignore_ascii_case(subtype)))
        .or_else(|| extensions.iter().find(|e| **e == "jpg"))
        .or(extensions.first())
        .copied()
}

/// `name` in `folder`, or "name (1).ext", "name (2).ext", ... if `taken`.
fn unique_path(folder: &Path, name: &str, taken: impl Fn(&Path) -> bool) -> PathBuf {
    let path = folder.join(name);
    if !taken(&path) {
        return path;
    }
    let (stem, extension) = split_name(name);
    let extension = extension.map(|e| format!(".{e}")).unwrap_or_default();
    (1..)
        .map(|n| folder.join(format!("{stem} ({n}){extension}")))
        .find(|path| !taken(path))
        .expect("a free name")
}

/// A URI part as a folder name: letters, digits, `-`, `_` and `.` only.
fn path_safe(part: &str) -> String {
    let safe: String = part
        .chars()
        .map(|c| match c {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '-' | '_' | '.' => c,
            _ => '_',
        })
        .collect();
    if safe.is_empty() || safe.starts_with('.') {
        format!("_{safe}")
    } else {
        safe
    }
}

#[cfg(test)]
mod tests;
