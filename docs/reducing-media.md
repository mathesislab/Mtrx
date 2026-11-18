# Reducing photos and videos before sending

Goal: photos and videos sent from a phone are small by default; anyone can send originals instead. The desktop can reduce photos too, but sends originals by default.

Builds on `docs/sending-files.md` (phones send files: `FileToSend`, one `Timeline.send`). Rules from `docs/core-state.md` apply: the policy and its words live in core; the apps keep no copies. The OS does what needs its codecs (HEIC, hardware video encoders).

---

## The setting

```rust
/// How photos and videos are sent (a setting of each device).
pub enum MediaQuality {
    /// Photos at most 1 MB and 2048 px; videos at 720p, sized to their length.
    Reduced,
    /// As taken (on iOS still JPEG and H.264, so every app shows them).
    Original,
}
impl MediaQuality {
    pub fn title(self) -> &'static str;        // "Reduced", "Original"
    pub fn explanation(self) -> &'static str;  // "Photos up to 1 MB, videos at 720p. Location data is left out."
                                               // "As taken, with the photo's details such as its location."
}
```

- Per device, since data plans differ: the phones keep it in their own settings, the desktop in its settings file.
- Default: **Reduced on the phones, Original on the desktop.**
- Phones: Settings → "Photos and videos" → Reduced | Original, with the explanation below.
- Desktop: Settings → "Photos" → Reduced | Original. The desktop reduces photos only; its videos go as they are.

## Photos (core, all three apps)

`core/media.rs`, run in `spawn_blocking` inside `send_message` when the quality is `Reduced`:

```rust
/// A photo made small enough to send: at most `PHOTO_MAX_EDGE` px on its
/// longer side and `PHOTO_MAX_BYTES`, as JPEG, turned upright, without its
/// metadata. GIFs, and photos already within both limits, are unchanged.
pub fn reduce_photo(file: Attachment) -> Result<Attachment>;
pub const PHOTO_MAX_EDGE: u32 = 2048;
pub const PHOTO_MAX_BYTES: usize = 1_000_000;
```

- Steps until it fits: shrink to 2048 px, JPEG quality 80, then 70, then 60; still too large: shrink by 0.75 and start again at 80.
- EXIF orientation is applied (`ImageDecoder::orientation`, `DynamicImage::apply_orientation`), so the result is upright without metadata. Re-encoding drops everything else, GPS included.
- The name keeps its stem with `.jpg`: `IMG_1234.HEIC` → `IMG_1234.jpg`.
- Formats core can read: JPEG, PNG, WebP (the `image` crate's features already in core). Anything else is sent as it is, except on the phones, which hand core JPEG (below).
- `RoomTimeline::send` gains `quality: MediaQuality`; FFI `Timeline.send` gains `quality`.

Phones hand core a JPEG it can read:
- iOS: `PhotosPicker` with `.compatible` gives JPEG; the camera's `UIImage` is written as JPEG at quality 0.9.
- Android: a picked HEIF photo is decoded with `ImageDecoder` (API 28+) and written as JPEG at 0.9 first; JPEG, PNG and WebP go to core as they are.

## Videos (the phones; the OS's encoders, core's numbers)

```rust
/// How a video is converted for sending: its size, frame rate and bit
/// rates. Bits per pixel per frame make the size follow its length.
pub struct VideoPlan { pub width: u32, pub height: u32, pub fps: f32,
                       pub video_bitrate: u32, pub audio_bitrate: u32 }
/// `Err` ("Too long to send: …") when even the floor won't fit the server's limit.
pub fn video_plan(width: u32, height: u32, fps: f32, duration: Duration,
                  max_upload: Option<u64>) -> Result<VideoPlan>;
```

- At most 1280 px on the longer side (720p), at most 30 fps.
- Video bit rate: 0.07 bits per pixel per frame (720p30 ≈ 1.9 Mbit/s), audio 128 kbit/s AAC: about 15 MB a minute.
- When that won't fit the server's upload limit (minus 5% for the container), the bit rate is lowered to fit, down to a floor of 0.5 Mbit/s at 854 px (480p). Below that: "Too long to send: the server takes at most 50 MB (about 10 minutes)."
- A video already within the plan (size, rate) is sent as it is.
- FFI: `Session.video_plan(width, height, fps, duration_ms) -> Result<VideoPlan, MtrxError>` (it reads the server's limit itself).
- iOS: `AVAssetReader` → `AVAssetWriter` with `AVVideoAverageBitRateKey`, H.264, AAC; no new dependency.
- Android: Media3 Transformer (`androidx.media3:media3-transformer`, new dependency, approved 2026-11-18) with `VideoEncoderSettings.setBitrate` and a scale to the plan's size.
- Converting shows on the card above the field ("Preparing… 40%"); Send waits for it.

## Video thumbnail and length (the phones)

- `FileToSend` gains `video: Option<VideoInfo>`: `width`, `height`, `duration_ms`, `thumbnail_path` (a JPEG of a frame near the start, at most 800 px).
- Core puts them into `AttachmentInfo::Video` and the attachment's thumbnail, so Element and our apps show a still and the length before it is opened.
- iOS: `AVAssetImageGenerator`; Android: `MediaMetadataRetriever.getScaledFrameAtTime`.
- The desktop sends videos without them, as now.

## Tests

- Core unit: `reduce_photo` on a large generated JPEG (fits both limits, upright from an EXIF orientation 6 input, no EXIF left, `.jpg` name), a small one and a GIF unchanged; `video_plan` for 4K60, 720p30, 480p, a long video fitted to 50 MB, one too long (its words); `MediaQuality` words.
- FFI: `send` with `Reduced` sends a reduced photo (integration, local Synapse).
- Phones: the setting's default and its row; JVM/XCTest for picking the plan's numbers through.

## PR split

One PR after `docs/sending-files.md`'s PR: core+FFI (setting, `reduce_photo`, `video_plan`, `VideoInfo`), desktop (setting), iOS, Android (Transformer).

## Decided (2026-11-18)

Two levels named Reduced | Original; photos ≤ 1 MB and 2048 px; videos 720p at 0.07 bits per pixel per frame, fitted to the server's limit; Android uses Media3 Transformer; videos carry a thumbnail and their length; the desktop gets photo reduction too, but defaults to Original, while the phones default to Reduced.

## As built

As designed, except:

- **`video_plan` takes the file's size** and returns `None` for a video within the plan already (size, frame rate, and bytes at most the plan's): it is sent as it is. The apps do not compare. FFI: `Session.video_plan(width, height, fps, duration_ms, size) -> Option<VideoPlan>`. "Too long to send: the server takes at most 50.0 MB (about 10 minutes)": sizes by `human_size`, as `FileTooLarge`.
- **Photos within both limits** are unchanged only without metadata (EXIF, XMP); with it they are re-encoded, so "Location data is left out" holds. The colour profile is kept (an iPhone's photos are Display P3); transparent parts go on white. `reduce_photo` fails ("IMG.jpg could not be reduced: …") for a JPEG, PNG or WebP it cannot decode. Sizes are checked against the server's limit after reducing.
- **The desktop's words**: `MediaQuality::photo_explanation` ("Photos up to 1 MB. Location data is left out."), as it sends videos as they are. Send later reduces when a message is scheduled (`RoomHandle::schedule` takes the quality) and keeps it so.
- **FFI**: `media_quality_choices()` (records with `title` and `explanation`), `default_media_quality()` (the phones' default, `MediaQuality::PHONE_DEFAULT`), `video_thumbnail_edge()` (800, `VIDEO_THUMBNAIL_EDGE`) for the still the phones make. `VideoInfo.thumbnail_path` is optional; a still that cannot be read is left out. The app deletes the still with its copy.
- **Phones**: files picked from Files are not converted, but their photos are reduced by the library like any other. A converted video's frames above the plan's rate are dropped (iOS by timestamp; Android `EditedMediaItem.setFrameRate`). Its location is left out: AVAssetWriter writes none; on Android the muxer is wrapped to drop `Mp4LocationData`.
- **Android**: a HEIF photo becomes JPEG only when Reduced; with Original it goes as taken, with its details. Media3 1.11.1 (current stable), which brings kotlin-stdlib 2.2.
- **Tests**: core unit (`media.rs`): the reduced photo (upright from orientation 6, no EXIF, ≤ 1 MB and 2048 px, `.jpg`), a small one, one with EXIF, a GIF and a text file; `video_plan` for 4K60, 720p30 within and over the plan, 480p, portrait, 6 and 9 minutes fitted to 50 MB, 11 minutes too long; the words; a video's info and still in the event. FFI: the choices (unit); a 3000×2000 JPEG sent Reduced arrives as at most 2048 px and 1 MB (integration). Desktop: the setting saved and its default. iOS (`ReducingMediaTests`): the default and words, `send` with the setting of the moment, a video's plan asked for as stored and its numbers in the encoders' settings, the still deleted with its copy. Android (`TimelineViewModelTest`): preparing and sending with the setting of the moment, a conversion's progress on its card, "Too long to send", the plan's numbers through to the encoders' settings; the setting's row in a screenshot.
