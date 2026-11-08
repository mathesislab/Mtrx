//! Fetching a message's file to disk, for downloads and exports.
//!
//! The file streams to a `.part` file next to where it goes, and is renamed
//! when complete. An encrypted file's ciphertext goes to the profile's
//! cache first, and is decrypted from there (checking its hash), so no
//! ciphertext lands in the destination folder. Nothing holds a whole file
//! in memory, except a file the SDK's media store already holds.

use std::{
    borrow::Cow,
    fs::File,
    io::{BufReader, BufWriter, Read, Write},
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};

use matrix_sdk::{
    Client,
    media::{MediaFormat, MediaRequestParameters},
    ruma::{
        MxcUri,
        api::{
            Metadata, OutgoingRequestExt,
            auth_scheme::SendAccessToken,
            client::{authenticated_media, media},
        },
        events::room::MediaSource,
    },
};
use matrix_sdk_crypto::{AttachmentDecryptor, MediaEncryptionInfo};
use tokio::io::AsyncWriteExt;

/// Progress is reported at most this often (10 times a second).
pub(crate) const STEP: Duration = Duration::from_millis(100);
/// A server that sends nothing for this long has stopped.
const STALL: Duration = Duration::from_secs(60);
/// Bytes written to disk at once.
pub(crate) const BUFFER: usize = 256 * 1024;
/// Times a request answered "too many requests" is tried again.
const RETRIES: u32 = 3;
/// The longest wait a server's `Retry-After` gets.
const MAX_RETRY_AFTER: Duration = Duration::from_secs(60);

pub(crate) const DAMAGED: &str = "The file is damaged";
pub(crate) const UNREACHABLE: &str = "Couldn't reach the server";
const STALLED: &str = "The server stopped sending";
const TOO_LARGE: &str = "Larger than the size limit";

/// A message's file to fetch, and where to.
pub(crate) struct Fetch<'a> {
    pub(crate) source: &'a MediaSource,
    /// As sent, in bytes; the progress's total if the server gives none.
    pub(crate) size: Option<u64>,
    pub(crate) target: &'a Path,
    /// Where an encrypted file's ciphertext goes while it downloads: a
    /// place in the cache of its own.
    pub(crate) ciphertext: PathBuf,
    /// Pick another name if the target is taken when done.
    pub(crate) unique: bool,
    /// Stop if the file turns out larger than this.
    pub(crate) limit: Option<u64>,
}

/// Get `fetch`'s file to its target, reporting progress (bytes so far, and
/// the total if known) at most every [`STEP`]; where it went. Partial files
/// go if it fails or the future is dropped.
pub(crate) async fn fetch_file(
    client: &Client,
    fetch: Fetch<'_>,
    mut on_progress: impl FnMut(u64, Option<u64>),
) -> Result<PathBuf, String> {
    if let Some(folder) = fetch.target.parent() {
        tokio::fs::create_dir_all(folder)
            .await
            .map_err(write_failed)?;
    }
    let part = Part::new(part_path(fetch.target));
    if let Some(bytes) = stored(client, fetch.source).await {
        if fetch.limit.is_some_and(|limit| bytes.len() as u64 > limit) {
            return Err(TOO_LARGE.into());
        }
        tokio::fs::write(part.path(), bytes)
            .await
            .map_err(write_failed)?;
    } else {
        match fetch.source {
            MediaSource::Plain(uri) => {
                stream(client, &fetch, uri, part.path(), &mut on_progress).await?
            }
            MediaSource::Encrypted(file) => {
                let ciphertext = Part::new(fetch.ciphertext.clone());
                if let Some(folder) = ciphertext.path().parent() {
                    tokio::fs::create_dir_all(folder)
                        .await
                        .map_err(write_failed)?;
                }
                stream(
                    client,
                    &fetch,
                    &file.url,
                    ciphertext.path(),
                    &mut on_progress,
                )
                .await?;
                decrypt(ciphertext.path(), part.path(), (**file).clone().into()).await?;
            }
        }
    }
    place(part, fetch.target, fetch.unique)
}

/// The file from the SDK's media store, if it has it (a viewer showed it,
/// say). Decrypted.
async fn stored(client: &Client, source: &MediaSource) -> Option<Vec<u8>> {
    let request = MediaRequestParameters {
        source: source.clone(),
        format: MediaFormat::File,
    };
    let store = client.media_store().lock().await.ok()?;
    store.get_media_content(&request).await.ok().flatten()
}

/// Download `uri` to `path`, reporting progress.
async fn stream(
    client: &Client,
    fetch: &Fetch<'_>,
    uri: &MxcUri,
    path: &Path,
    on_progress: &mut impl FnMut(u64, Option<u64>),
) -> Result<(), String> {
    let mut response = response(client, uri).await?;
    let total = response.content_length().or(fetch.size);
    let over = |bytes: u64| fetch.limit.is_some_and(|limit| bytes > limit);
    if total.is_some_and(over) {
        return Err(TOO_LARGE.into());
    }
    let file = tokio::fs::File::create(path).await.map_err(write_failed)?;
    let mut file = tokio::io::BufWriter::with_capacity(BUFFER, file);
    let mut received = 0;
    on_progress(received, total);
    let mut steps = Throttle::new(STEP, Instant::now());
    while let Some(chunk) = tokio::time::timeout(STALL, response.chunk())
        .await
        .map_err(|_| STALLED)?
        .map_err(|_| UNREACHABLE)?
    {
        file.write_all(&chunk).await.map_err(write_failed)?;
        received += chunk.len() as u64;
        if over(received) {
            return Err(TOO_LARGE.into());
        }
        if steps.due(Instant::now()) {
            on_progress(received, total);
        }
    }
    file.flush().await.map_err(write_failed)?;
    on_progress(received, total);
    Ok(())
}

/// The server's answer for `uri`'s content. An expired token is refreshed
/// once; "too many requests" waits as the server asks, a few times.
async fn response(client: &Client, uri: &MxcUri) -> Result<reqwest::Response, String> {
    let (mut refreshed, mut retries) = (false, 0);
    loop {
        let request = content_request(client, uri).await?;
        let response = (client.http_client().execute(request).await).map_err(|_| UNREACHABLE)?;
        let status = response.status();
        if status.is_success() {
            return Ok(response);
        }
        if status == reqwest::StatusCode::TOO_MANY_REQUESTS && retries < RETRIES {
            retries += 1;
            tokio::time::sleep(retry_after(&response)).await;
            continue;
        }
        if status == reqwest::StatusCode::UNAUTHORIZED
            && !refreshed
            && unknown_token(response).await
        {
            refreshed = true;
            client
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

/// How long a "too many requests" answer asks to wait: its `Retry-After`
/// seconds, else a second; at most [`MAX_RETRY_AFTER`].
fn retry_after(response: &reqwest::Response) -> Duration {
    let seconds = (response.headers().get(reqwest::header::RETRY_AFTER))
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.trim().parse::<u64>().ok())
        .unwrap_or(1);
    Duration::from_secs(seconds).min(MAX_RETRY_AFTER)
}

/// The request for `uri`'s content: authenticated if the server has it, as
/// the SDK chooses.
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
/// if this future is dropped.
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
pub(crate) fn decrypt_file(
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

/// Move the complete `part` to `target`; if `unique` and `target` was taken
/// meanwhile, to a free name next to it.
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

pub(crate) fn write_failed(e: std::io::Error) -> String {
    format!("Couldn't write the file: {e}")
}

/// A file being written: deleted when dropped (the fetch failed or was
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
pub(crate) fn part_path(path: &Path) -> PathBuf {
    let mut part = path.as_os_str().to_owned();
    part.push(".part");
    part.into()
}

/// Whether a file is at `path`, or being fetched there.
pub(crate) fn taken(path: &Path) -> bool {
    path.exists() || part_path(path).exists()
}

/// Reports at most once every `every`.
pub(crate) struct Throttle {
    every: Duration,
    last: Instant,
}

impl Throttle {
    /// The last report at `start`.
    pub(crate) fn new(every: Duration, start: Instant) -> Self {
        Self { every, last: start }
    }

    /// Whether to report at `now`.
    pub(crate) fn due(&mut self, now: Instant) -> bool {
        let due = now.duration_since(self.last) >= self.every;
        if due {
            self.last = now;
        }
        due
    }
}

/// The user's Downloads folder, else `~/Downloads`.
#[cfg(not(any(target_os = "ios", target_os = "android")))]
pub(crate) fn downloads_folder() -> Option<PathBuf> {
    let dirs = directories::UserDirs::new()?;
    Some(match dirs.download_dir() {
        Some(folder) => folder.to_owned(),
        None => dirs.home_dir().join("Downloads"),
    })
}

/// The phones keep downloads in the cache.
#[cfg(any(target_os = "ios", target_os = "android"))]
pub(crate) fn downloads_folder() -> Option<PathBuf> {
    None
}

/// A file name from a sender, safe to save under: no folders, no leading
/// dots, control or reserved characters, no names Windows reserves; with an
/// extension from `mimetype` if it has none. "file" if nothing is left.
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
pub(crate) fn unique_path(folder: &Path, name: &str, taken: impl Fn(&Path) -> bool) -> PathBuf {
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
