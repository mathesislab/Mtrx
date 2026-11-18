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
