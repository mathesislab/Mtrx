# Downloads

Goal: one list of the files the user downloaded, with progress, kept in core. The desktop shows it behind a small icon. The phones get files at all (today they can't), with progress on the file's bubble.

Rules from steps 2–3 that apply:
- Shared state lives in core and reaches the apps as finished state (`Live<T>` in core; listener + `TaskHandle` via `follow` in FFI).
- Apps keep no parallel copies.
- Core returns English text. FFI labels and derived values are record fields, not functions.
- iOS behaviour is canonical, unless it's a bug. Here iOS has nothing, so the gap is a missing feature, not a choice.

Compatible with step 4 (`docs/app-structure.md`), but not dependent on it: the desktop part is one component module with its own `Message`, and its menu is a plain item list.

Paths: `core/` = `crates/mtrx-core/src/`, `ffi/` = `crates/mtrx-ffi/src/`, `app/` = `crates/mtrx-app/src/`, `ios/` = `apps/ios/Mtrx/`, `android/` = `apps/android/app/src/main/kotlin/org/mtrx/app/`, `sdk/` = `~/.cargo/registry/src/index.crates.io-*/matrix-sdk-0.19.1/src/`.

---

## Today

**Core**
- `core/media.rs:88-108`: `media_content`, `media_preview` and `thumbnail` all call `get_media_content(&request, true)`.
  - The whole file comes back as one `Vec<u8>`, decrypted, and has no progress.
  - `use_cache = true` also puts every whole file in the SDK's media store, a saved 1 GB video included.
- `core/timeline.rs:265-277`: `Media{kind, filename, size, mimetype, width, height, source: MediaRef}`. It doesn't say which event or room it came from.
- `core/timeline.rs:391-430`: `RoomTimeline::uploads` reports upload progress from the send queue. There's nothing like it for downloads.

**FFI**
- `ffi/session.rs:117-131`: `media_file(media) -> path` writes the decrypted file to `<cache>/media/<sha256(uri)>.<ext>` (`:158-182`), once. Signing out deletes it.
- Neither phone calls it. It's only in the stale generated Swift (`apps/ios/MtrxFFI/.../mtrx.swift:3946`).
- `docs/mobile.md:460, 784` lists "opening files" as not done on both phones.

**Desktop**
- Places that save:
  - A download button beside a hovered image (`app/media.rs:257-301`).
  - The button on every file card (`app/media.rs:329-373`, `save_button` `:375-391`).
  - The image viewer's bar (`app/media.rs:552`) and the PDF viewer's (`app/pdf_viewer.rs:396`).
  - The message menu's "Save…" / "Save image…" (`app/message_menu.rs:81-97`).
- All of them send `Message::SaveMedia(Media, Origin)` (`app/app.rs:2168-2175`), which runs `media::save` (`app/media.rs:593-607`):
  - an `rfd` save dialog every time;
  - then `media_content`, so the whole file is held in memory with no progress;
  - then it writes the file.
  - Errors show where the save was asked from (`app.rs:2186-2204`, `MediaDone`).
- Showing, not saving:
  - The image viewer (`media.rs:465-483`) and the PDF viewer (`pdf_viewer.rs:105-121`) load the whole file through `media_content`.
  - Previews (`media.rs:121-162`) also check PDFs up to 30 MB.
- A non-PDF file card does nothing when clicked. There's no "open with another app".
- Precedents:
  - The update installer streams with `reqwest` `chunk()` and reports progress (`app/update/install.rs:50-68`).
  - It reveals a file in Finder with `open -R` (`install.rs:242-252`).
  - `open::that_detached` opens URLs (`menu_bar.rs:247`, `app.rs:2039`).
- `human_size` lives in `app/media.rs:422-438`.

**iOS**
- `ios/Views/MediaViews.swift:6-60` `MediaImageView` shows `mediaPreview` through `ImageCache` (`ios/Models/ImageCache.swift`). A video shows a play symbol, but tapping it does nothing.
- `MediaViews.swift:62-90` `FileView` shows the name and size only.
- `ios/Views/TimelineRowView.swift:183-200` puts no tap or menu item on media.
- There's no QuickLook, share sheet or Files export anywhere. Deployment target is iOS 16.

**Android**
- `android/timeline/TimelineScreen.kt:1087-1093`: an image or video shows `MediaPreview` (`:1167-1190`, via `model/Images.kt:32`). Any other file shows as italic text, "name (size)".
- There's no `FileProvider`, `ACTION_VIEW`, share or MediaStore code. `androidx.core` (which has `FileProvider`) is already a dependency. `minSdk` is 26.

**matrix-sdk 0.19.1: no download progress**
- `sdk/media.rs:431-490` `get_media_content` returns `Vec<u8>`. The fetcher (`:806-895`) does `client.send(request)` and decrypts the whole buffer with `AttachmentDecryptor` (`:842-851`).
- `TransmissionProgress` is for sending only. It wraps the request body (`sdk/http_client/native.rs:243-275`), and the response is buffered.
- `MediaFetcher` can be replaced (`Client::set_media_fetcher`, `sdk/client/mod.rs:3936`), but it still returns a whole `Vec<u8>` and is global.
- What can be used:
  - `Client::http_client() -> &reqwest::Client` (`sdk/client/mod.rs:579`): the SDK's own client, with the same TLS and user agent.
  - `access_token()` (`:830`), `refresh_access_token()` (`:1627`), `supported_versions()` (`:2361`).
  - `media_store()` (`:907`).
  - `matrix_sdk_crypto::AttachmentDecryptor`. `matrix-sdk-crypto` is already a core dependency.

## Where they disagree

| | Desktop | iOS | Android |
|---|---|---|---|
| Get a file | Save dialog, every time | not possible | not possible |
| Progress | none (whole file in memory) | — | — |
| Open with another app | no | no | no |
| Recent files | no | no | no |

- Only the desktop can get a file at all. The phones' gap is a missing feature, so the design follows the platforms' conventions there.

---

## What counts as a download

- **Counts:** the user asked for the file itself, either to keep it ("Download", "Save as…") or to hand it to another app ("Open" on a phone or on a desktop file card).
- **Doesn't count:** anything shown inside Mtrx: avatars, thumbnails, previews, the PDF check, the image and PDF viewers, "Copy image". These keep `media_content` and the SDK cache as they do now.
- Rendered math saved as PNG (`app.rs:3509-3528`) isn't a message's file, so it stays as it is.

## Core: new `core/downloads.rs`

```rust
pub type DownloadId = u64;

/// Where a download goes.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum SaveTo {
    /// The user's Downloads folder (desktop; `directories::UserDirs`, else ~/Downloads).
    /// The sender's name, made safe and unique: "report.pdf", "report (2).pdf".
    Downloads,
    /// This path, replaced if there (a save dialog asked).
    File(PathBuf),
    /// The profile's media cache, to open or share from (phones). Named by the URI as
    /// `media_file` names it now, so files already there count as done.
    Cache,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum DownloadState {
    Downloading { received: u64, total: Option<u64> },
    Done { path: PathBuf },
    Failed(String),            // English, e.g. "The file is damaged", "Stopped when Mtrx quit"
    Cancelled,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Download {
    pub id: DownloadId,
    pub file_name: String,     // as sent
    pub kind: MediaKind,
    pub size: Option<u64>,     // as sent
    pub room_id: OwnedRoomId,
    pub room_name: String,     // when started
    pub event_id: Option<OwnedEventId>,
    pub to: SaveTo,
    pub started: SystemTime,
    pub state: DownloadState,
    pub(crate) source: MediaRef,   // to retry; holds an encrypted file's key
}
impl Download {
    pub fn fraction(&self) -> Option<f32>;   // 0..1 while downloading with a known total
    pub fn status(&self) -> String;          // "3.2 of 10 MB", "Starting…", "10 MB · Ice sheets",
                                             // "Failed: …", "Cancelled"
    pub fn uri(&self) -> &MxcUri;            // to find a bubble's download
}

pub fn human_size(bytes: u64) -> String;     // moved from app/media.rs:422

impl RoomHandle {
    /// Start downloading a message's file. It's in `downloads()` at once.
    /// The same file already downloading to the same place returns that download.
    pub fn download(&self, media: &Media, event: Option<&EventId>, to: SaveTo) -> DownloadId;
}
impl Session {
    pub fn downloads(&self) -> Live<Vec<Download>>;            // newest first
    pub async fn finished(&self, id: DownloadId) -> Result<PathBuf>;  // waits for Done (Failed/Cancelled → Err)
    pub fn cancel_download(&self, id: DownloadId);             // stops it, deletes partial files
    pub fn retry_download(&self, id: DownloadId);              // Failed or Cancelled: from the start
    pub fn remove_download(&self, id: DownloadId);             // from the list; the file stays
    pub fn clear_downloads(&self);                             // every one not downloading
}
```

**Session**
- Gains `downloads: Arc<Downloads>`, like `schedule` (`core/session.rs:56`).
- `Downloads` holds a `watch::Sender<Vec<Download>>` (read through `Live::from_watch`) and one `AbortOnDrop` task per running download, keyed by ID.

**How a download runs (no new crate; see Q1)**
1. If the SDK's media store already has the file (the viewer loaded it, say), write it from there. It's done at once.
2. Otherwise:
   - Build the request with ruma: `authenticated_media::get_content::v1::Request::from_uri(uri)`, or the v3 one when `supported_versions()` lacks v1, as the SDK's fetcher chooses (`sdk/media.rs:823-824`).
   - Turn it into an HTTP request with `try_into_http_request(homeserver, access_token, versions)`.
   - Send it with `client.http_client()` and read it with `response.chunk()`.
   - `total` is `Content-Length`, else `media.size`.
3. Write the chunks to `<final>.part`:
   - in the destination folder for a plain file, so the rename is atomic;
   - in the profile cache for an encrypted file's ciphertext, so no ciphertext lands in Downloads.
4. Encrypted files:
   - When all of it has come, decrypt `.part` to `<final>.part` in `spawn_blocking` with `AttachmentDecryptor` (stream to stream, constant memory).
   - The decryptor checks the SHA-256 at the end. A mismatch is `Failed("The file is damaged")`.
5. Rename to the final name. With `Downloads`, the name is picked again if a file took it meanwhile.
6. A 401 `M_UNKNOWN_TOKEN` calls `refresh_access_token()` once, then starts over (OAuth sessions).

Details:
- Progress steps are sent at most 10 times a second. The list is written to the store on start and on each end state, not on progress steps.
- File names from the sender are made safe: no path separators, no leading dots or control characters, no Windows reserved names; empty becomes "file". The extension comes from the MIME type if the name has none.
- Cancelling aborts the task and deletes the `.part` files.
- There's no resume (Range). Retry starts over.
- The list keeps every running download and the 20 newest others. Older ones drop at each change.

**Persistence** (Q4)
- The list is stored as the state-store custom value `mtrx.downloads`, the way `core/scheduled.rs:82-107` stores scheduled messages.
- At start:
  - downloads still marked `Downloading` become `Failed("Stopped when Mtrx quit")` (retry offered);
  - `Done` files that are gone become `Failed("Moved or deleted")`, so retry downloads them again.
- The store is encrypted, so an encrypted file's key (in `source`) stays encrypted at rest.

**Open and show**
- These stay in the apps. They're OS actions, and core only hands over `Done{path}`.

**Tests**
- Unit:
  - name sanitising and uniqueness;
  - `status()` and `fraction()`;
  - round trip through the store (a running download comes back as failed);
  - cancel removes `.part`.
- Integration (local Synapse, `test-support`):
  - a plain and an encrypted file arrive with increasing progress;
  - a corrupted ciphertext fails;
  - the media-store shortcut.

## FFI

```rust
#[derive(uniffi::Enum)]
pub enum DownloadState { Downloading { received: u64, total: Option<u64> }, Done { path: String },
                         Failed { message: String }, Cancelled }

#[derive(uniffi::Record)]
pub struct Download {
    pub id: u64, pub file_name: String, pub kind: MediaKind, pub size: Option<u64>,
    pub source: String,          // the `Media.source` it was started from: a bubble finds its download
    pub room_id: String, pub room_name: String, pub event_id: Option<String>,
    pub state: DownloadState,
    pub fraction: Option<f32>,   // derived, filled when built
    pub status: String,          // core's English line
}

#[uniffi::export] impl Room {
    /// Download a message's file to the cache (phones open or share it from there).
    pub fn download(&self, media: Media, event_id: Option<String>) -> Result<u64, MtrxError>;
}
#[uniffi::export] impl Session {
    pub fn downloads(&self) -> Vec<Download>;
    pub fn subscribe_downloads(&self, listener: Arc<dyn DownloadsListener>) -> Arc<TaskHandle>;  // via `follow`
    pub async fn download_finished(&self, id: u64) -> Result<String, MtrxError>;               // the path
    pub fn cancel_download(&self, id: u64);
    pub fn retry_download(&self, id: u64);
    pub fn remove_download(&self, id: u64);
    pub fn clear_downloads(&self);
}
```
- `SaveTo` isn't exported. The phones always use `Cache`.
- `source` is matched as a string. The same event always gives the same string.

## Desktop

**Actions**
- "Download" (the image's hover button, the file card's button, both viewers' bars) → `SaveTo::Downloads`. Tooltip: "Download to your Downloads folder".
- The message menu's "Save…" / "Save image…" become "Save as…": the `rfd` dialog, then `SaveTo::File(path)`.
- Clicking a file card that isn't an image or a readable PDF opens it (Q5):
  - a finished download of the same file whose path still exists is opened at once;
  - otherwise it downloads to Downloads and opens when finished (`session.finished(id)`, then `open::that_detached`).
- The viewers keep the room and event they were opened from. `Message::OpenImage` and `OpenPdf` carry them, so their Download button has them.

**Icon** (`app/sidebar.rs:313-339`)
- `icons::download` (`app/icons.rs:787`) at the bottom of the space bar, above the Settings gear.
- It's shown only while the list isn't empty.
- While anything downloads, a ring around it shows the overall fraction.
- Tooltip: "Downloads", or "Downloading 2 files…".

**Popover**
- Opens above the icon, extending right (`popover`, `Side::Above`, `Edge::Left`). Width about 340.
- One row per download, newest first:
  - the file icon, the name (ellipsis) and `status` (secondary);
  - a thin progress bar while downloading (indeterminate without a total).
- Row actions:
  - Downloading: ✕ cancels.
  - Done: a click opens the file (`open::that_detached`). A folder button shows it in the folder.
  - Failed / Cancelled: ↻ retries. The message shows in pink.
- Right-click on a row: "Open", "Show in folder", "Go to message" (the existing jump to an event, when `event_id` is known), "Remove from list".
- Footer: "Clear" (removes all that aren't downloading).
- An open or show that fails (the file is gone) shows its error in the row.

**Show in folder** (new `app/files.rs`; no new crate)
- macOS: `open -R <path>`.
- Windows: `explorer /select,<path>`.
- Linux: `gdbus call … org.freedesktop.FileManager1.ShowItems`, else open the parent folder.
- `update/install.rs:242-252` uses the same helper.

**Structure**
- New `app/downloads.rs`: `enum Message {Toggle, Open(Id), Show(Id), Cancel(Id), Retry(Id), Remove(Id), Clear, GoTo(..)}`, `fn update(&Session, Message) -> Task`, and `fn view(&[Download], open: bool) -> Element`.
- The list comes from one subscription to `session.downloads().updates()`, like `uploads` (`app.rs:4304-4312`).
- Step 4 (`docs/app-structure.md`, item 2): the open list is the window's popup, `Popup::Downloads { menu }`, with a download's menu in it. One popup at a time, so another one replaces it. Esc closes the download's menu, then the list.
- `app/media.rs` uses core's `human_size`.

## iOS (recommended, Q6)

- **Tap a file, video or audio bubble** (`FileView`, and `MediaImageView` for videos):
  - `room.download(media, eventId)`, then `downloadFinished(id)`, then `.quickLookPreview($url)`.
  - QuickLook shows or plays the file. Its share button covers Save to Files, Photos and AirDrop.
- **Tap an image:** the same, full size in QuickLook. This is also the "open images full size" item in `docs/mobile.md:459-460`.
- **While downloading:** the bubble shows a small progress ring with ✕ (cancel). A failure shows "Couldn't download" with a retry tap.
- **Context menu:** "Share…" (`ShareLink` on the downloaded URL; it downloads first if needed).
- **No downloads icon:** iOS has no app-level downloads list. Files keep them.
- `SessionModel` holds one `Subscriptions` entry for `subscribeDownloads` and publishes `downloads`. A bubble reads `downloads.first { $0.source == media.source }`. No other copy.

## Android (recommended, Q6, Q7)

- **The file row** replaces the italic text (`TimelineScreen.kt:1093`): icon, name, `status`, and a `LinearProgressIndicator` while downloading.
- **Tap a file, video, audio or image bubble:**
  - `room.download` → `downloadFinished`;
  - then `Intent.ACTION_VIEW` with a `FileProvider` URI and `FLAG_GRANT_READ_URI_PERMISSION`, in a chooser;
  - "No app can open this file" if none can.
- **Long-press sheet (existing):**
  - "Share…" (`ACTION_SEND`).
  - "Save to Downloads": copies the finished cache file into `MediaStore.Downloads` on API 29+, or uses `ACTION_CREATE_DOCUMENT` on 26–28. A snackbar says "Saved to Downloads".
- **Manifest:**
  - an `androidx.core.content.FileProvider` with `res/xml/file_paths.xml` (`<cache-path>` for `profiles/default/media/`);
  - no new permission.
- **No downloads icon:** Android has its Files › Downloads.
- The timeline model holds one `TaskHandle` for `subscribeDownloads`. JVM tests build `Download` records directly.

---

## Deleted

- FFI:
  - `Session.media_file`, `media_file_name` and `write_atomically` (`ffi/session.rs:117-131, 158-182`). They move into core as `SaveTo::Cache`, with the same file names, so existing cache files count as done.
  - Their test moves too.
- Desktop:
  - `media::save` (`app/media.rs:593-607`) and `Message::SaveMedia`.
  - The save branch of `MediaDone` (`MediaDone` stays for "Copy image").
  - `media::human_size` (now in core).
  - The `open -R` in `update/install.rs:247` (now the shared helper).
- iOS: nothing exists to delete. The stale `mediaFile` in the generated Swift goes when it's regenerated.
- Android: the italic file text (`TimelineScreen.kt:1093`).
- Not deleted: `media_content`, `media_preview`, `thumbnail` (they show things inside Mtrx).

## PR split

One PR, four commits, each building (CI only):
1. Core and FFI: `downloads.rs`, `human_size`, `Session.downloads`, `Room.download`, unit and integration tests. FFI `media_file` is removed here, since no app calls it.
2. Desktop: the icon, the popover, Download / Save as… / open-on-click, the viewers' room and event, `files.rs`.
3. iOS: QuickLook on tap, the progress ring, Share…, the subscription.
4. Android: the file row, `FileProvider`, the chooser, Share…, Save to Downloads, the subscription.

Size: M–L. About 500 lines of core, 300 desktop and 150 per phone. One PR is enough: core and FFI are additive except for the unused `media_file`.

---

## Open questions

1. **How core gets progress.** The SDK has none for downloads.
   - (a) Stream it in core:
     - ruma builds the request, the SDK's `http_client()` sends it, `chunk()` reads it, and `AttachmentDecryptor` decrypts the file afterwards.
     - It needs `reqwest` made non-optional in `mtrx-core/Cargo.toml`. It's already a workspace dependency and optional in core (`test-support`). It's the same 0.13 that matrix-sdk builds with, so nothing new goes into `Cargo.lock`.
   - (b) The same without naming `reqwest`: only call methods on `http_client()` and map its errors to text. No manifest change, but a little awkward.
   - (c) No progress: `get_media_content(use_cache = false)` in memory, with a spinner.
   - Recommend (a), and it needs your OK as a manifest change. (c) fails the feature on large files.

2. **What counts as a download.**
   - (a) Download, Save as…, and Open (a phone's tap, a desktop file card). Viewers, previews and Copy image don't count.
   - (b) Also the desktop's image and PDF viewers.
   - (c) Only Download and Save as….
   - Recommend (a). What's shown inside Mtrx isn't something the user keeps.

3. **Where the desktop saves.**
   - (a) Download → the Downloads folder at once. "Save as…" in the message menu keeps the dialog.
   - (b) Always the dialog, as today, with progress after it.
   - (c) A setting, "Ask where to save each file".
   - Recommend (a): the browser convention, and the list makes the file easy to find.

4. **Whether the list persists.**
   - (a) In the state store (`mtrx.downloads`), the 20 newest. Interrupted downloads come back as failed, with retry.
   - (b) In memory, for the session only.
   - (c) A JSON file in the profile. It would keep encrypted files' keys in the clear.
   - Recommend (a). It's the same as scheduled messages, and encrypted at rest.

5. **Clicking a desktop file card** (not an image or readable PDF).
   - (a) Opens it: it downloads to Downloads, then opens in the default app, or reuses a finished download.
   - (b) Stays inert. Only the button downloads.
   - Recommend (a). It's the desktop's "open with another app".

6. **Phones' UI.**
   - (a) No icon. Progress on the bubble. Tap opens (QuickLook / the Android chooser). Share… and Save to Downloads in the menu.
   - (b) (a) plus a "Downloads" list in Settings.
   - (c) An icon in the toolbar like the desktop.
   - Recommend (a). Each OS keeps downloaded files in its own place (Files, the Downloads app), and phone users rarely run several downloads.

7. **Android "Save to Downloads".**
   - (a) `MediaStore.Downloads` on API 29+. `ACTION_CREATE_DOCUMENT` on 26–28.
   - (b) `ACTION_CREATE_DOCUMENT` always (a save dialog, like the desktop's Save as…).
   - (c) `DownloadManager`. It can't send our token or decrypt, so not possible.
   - Recommend (a). One tap on the API levels most phones run, and no storage permission on old ones.

---

## Decisions (2026-11-06)

All recommendations taken: 1a (`reqwest` non-optional in core, same version as the SDK's), 2a, 3a, 4a, 5a, 6a, 7a. The user asked for performant, robust and conceptually clean code and left the choices to the recommendations.

## As built (PR 84)

Differences from the design above:

- **Cache names on the phones:** `<cache>/media/<server>/<media-id>/<safe sender name>`, not a SHA-256 name. QuickLook and share sheets show the real file name, and core needs no `sha2`.
- **A cleared cache file** (the OS removed it) drops its finished download from the list at start, instead of showing "Moved or deleted".
- **Stalls:** a download fails with "The server stopped sending" after 60 s without data. The request's own timeout is 24 h, since the SDK client's 30 s default would cut off large files.
- **tokio** in core gains the `io-util` feature for buffered writes.
- **iOS:** a separate `DownloadsModel`, owned by `SessionModel` and injected like `presence`, holds the one subscription, so progress redraws only file bubbles.
- **Android:** open uses `ACTION_VIEW` (the system still asks when several apps can), so "No app can open this file" needs no `<queries>` entry; share uses a chooser. Images and videos also get a progress ring.
- **Desktop:** a file of unknown length shows an empty progress bar, not an animated one. "Go to message" opens the room, then jumps to the event (`Main.go_to`).
