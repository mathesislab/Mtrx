# Sending files from the phones

Goal: the phones send photos, videos and files, as the desktop does: picked with a paperclip left of the message field, waiting above it as cards, sent with Send (with text or alone), with the upload's progress on the message.

Rules from `docs/core-state.md` apply: shared state in core, the apps keep no copies, core returns English text, FFI labels are record fields, iOS behaviour is canonical.

Paths: `core/` = `crates/mtrx-core/src/`, `ffi/` = `crates/mtrx-ffi/src/`, `ios/` = `apps/ios/Mtrx/`, `android/` = `apps/android/app/src/main/kotlin/org/mtrx/app/`.

As built. Photos and videos go as picked; reducing them is `docs/reducing-media.md`, and plugs in where noted below.

---

## Today (before this)

- Core sent files already: `RoomTimeline::send(text, files: Vec<Attachment>, reply_to, disappear)` queues the text, then each file as a message of its own (`core/timeline.rs`). `RoomTimeline::uploads()` reports each upload's bytes from the send queue. The desktop used both.
- FFI had only `Timeline.send_text` and `Timeline.reply`: text only.
- Neither phone had a picker.

## Core

- **Upload limit.** `send_message` checks every file against the server's limit before anything is queued (`check_sizes`, through `Client::load_or_fetch_max_upload_size`, asked once per session). A file over it fails the whole send with `Error::FileTooLarge { filename, size, max }`: "report.pdf is too large: 120 MB; the server takes at most 100 MB" (sizes by `human_size`). The desktop gets it too.
- When the limit cannot be had within 5 s (offline, say), the files are queued unchecked; the server refuses one too large when it is sent, as before.
- `Attachment::read_as(path, filename)`: the file at `path` sent as `filename`, its type guessed from that. A phone's copy has a name of its own choosing (a photo has none).
- `Upload::fraction()`: 0 to 1.

## FFI (`ffi/timeline.rs`)

```rust
#[derive(uniffi::Record)]
pub struct FileToSend { pub path: String, pub filename: String }   // the app's copy in its cache

#[derive(uniffi::Record)]
pub struct Upload { pub item_id: String, pub fraction: f32 }       // the message's row; in hundredths

impl Timeline {
    pub async fn send(&self, text: String, files: Vec<FileToSend>,
                      reply_to: Option<String> /* row id */, disappear_after_read: Option<Duration>)
        -> Result<(), MtrxError>;
    pub fn subscribe_uploads(&self, listener: Arc<dyn UploadsListener>) -> Arc<TaskHandle>;
}
```

- `send` replaces `send_text` and `reply`. It reads every file before it returns, so the app deletes its copies then.
- `subscribe_uploads` follows core's `uploads()` and the rows together: an upload shows once its message's row is there, by the row's `TimelineRow.id`. Fractions are rounded to hundredths and only changes are passed on, so a large file makes at most about a hundred calls.

## iOS

- **Paperclip** (`ios/Views/AttachViews.swift`, `AttachMenu`), left of the field in `Composer` (`ios/Views/RoomView.swift`); off during an edit. Its menu:
  - "Photos": `.photosPicker`, several, images and videos, `preferredItemEncoding: .compatible` (HEIC → JPEG, HEVC → H.264).
  - "Files": `.fileImporter`, several, any type.
  - "Camera" (where there is one): `CameraPicker`, a `UIImagePickerController` for a photo or a video. `NSCameraUsageDescription` and `NSMicrophoneUsageDescription` (a video's sound) are build settings in `project.pbxproj`.
- **Preparing** (`ios/Models/Outgoing.swift`): `Outgoing.prepare(_ picked: Picked) async throws -> FileToSend` is the one place a picked item becomes a file to send. It copies it into `Caches/outgoing/` (a camera video is moved there):
  - a library photo or video is copied while the system's file exists (`LibraryMedia`, a `Transferable`) and named from the date: "Photo 2026-11-18 14.03.jpg", "Video 2026-11-18 14.03.mp4" (the file's own extension);
  - a camera photo is written as JPEG at quality 0.9, named the same way;
  - a file from Files keeps its name.
  - Copies left by a run that ended before sending them go once a day old, the first time the folder is used.
- **Cards** (`PickedFileCards`): above the field, a card per file with its thumbnail (QuickLook thumbnails: photos, videos, PDFs) or a file symbol, its name and ✕. A card shows "Preparing…" with a spinner until its copy is made.
- **Model** (`TimelineModel`): `picked: [PickedFile]`, `attach(_:)`, `detach(_:)`, `canSend`. Send is on with text, or with files alone, once every file is prepared (an edit needs its text and sends no files). `send()` passes the text, the files and the reply to `Timeline.send`, then deletes the copies. If sending fails, the text and the files come back.
- **Progress**: `UploadsModel` (owned by `TimelineModel`, injected into the timeline's views like `DownloadsModel`) holds the one `subscribeUploads`. `UploadRing` on an image or video's bubble, beside a file's, fills with the fraction while it uploads.

## Android

- **Paperclip** (`android/timeline/Attachments.kt`, `AttachButton`), left of the field in `Composer.kt`; off during an edit. Its menu:
  - "Photos and videos": `PickMultipleVisualMedia` (the system's photo picker, no permission).
  - "Files": `OpenMultipleDocuments`, any type.
  - "Take a photo" and "Record a video" (on a device with a camera): `TakePicture` and `CaptureVideo` into a new file in `cache/outgoing/`, handed to the camera app through the existing `FileProvider` (`res/xml/file_paths.xml` gains `outgoing/`). No `CAMERA` permission is declared, so none is asked: the camera app has it.
  - The launchers are only made with an activity, so the screenshots draw the button without them.
- **Preparing** (`android/timeline/Outgoing.kt`): `prepareToSend(context, picked: Picked): FileToSend` is the one place a picked item becomes a file to send:
  - a content URI (picker or Files) is copied to `cache/outgoing/` under its `OpenableColumns.DISPLAY_NAME` (an extension added from its type if it has none; named from the date if it has no name);
  - a camera's file is in place already and is named from the date: "Photo 2026-11-18 14.03.jpg", "Video 2026-11-18 14.03.mp4".
  - Copies left by a run go once a day old, as on iOS. The day matters here: a camera app may outlive the app it writes for.
- **Cards** (`PickedFileCards`): as on iOS; thumbnails from `ThumbnailUtils` (Android 10+), else a sampled `BitmapFactory` for photos and the file symbol for videos.
- **Model** (`TimelineViewModel`): `picked`, `attach(prepare)`, `detach(id)` (in `TimelineActions`); `canSend(draft, composing, picked)` as on iOS. The screen's button hands the model how to prepare (with the application context), so the model needs no context. Sending and failing as on iOS.
- **Progress**: the model holds the one `subscribeUploads` and publishes `uploads` (row → fraction), passed to `EventBubble`; `UploadRing` over an image or video, beside a file.

## Where reducing photos and videos plugs in

- iOS: `Outgoing.prepare` (`ios/Models/Outgoing.swift`), its `.library`, `.cameraPhoto` and `.cameraVideo` cases.
- Android: `prepareToSend` (`android/timeline/Outgoing.kt`), its `Picked.Media` and `Picked.Camera` cases.
- A card already shows "Preparing…" and Send waits for it, so a conversion fits without new states.

## Tests

- Core unit (`core/timeline.rs`): `within_limit` refuses the first file over the limit; `Error::FileTooLarge`'s words; `Upload::fraction`.
- FFI integration (local Synapse, `crates/mtrx-ffi/tests/integration.rs`): the tests send with `Timeline.send` (text, replies, disappearing). A 3 MB file copied to the cache is sent alone under a name of its own, the copy deleted as `send` returns; its message arrives with that name, and its upload is reported by its row until it is sent.
- iOS (`MtrxTests/TimelineModelTests.swift`): files alone and with a reply's text, copies deleted once sent; one taken away is not sent; files back after a failed send; an edit sends none; the date names; uploads by row. The fakes (`Fakes.swift`, `UITesting.swift`) take `send` and `subscribeUploads`.
- Android (`TimelineViewModelTest.kt`): the same, plus a card that waits while preparing (Send off) and one that fails to prepare. The composer's screenshot shows two cards, one preparing.
