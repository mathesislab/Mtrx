# Audio messages

Goal: audio messages play in the bubble on all three apps, with a play/pause button, a scrubber and the time. Element's voice messages (Ogg Opus) play everywhere. Today no app plays audio inline, and a tester on macOS 0.8.7 couldn't play a voice message at all.

Rules that apply (`docs/core-state.md`, `docs/downloads.md`):
- Shared state lives in core and reaches the apps as finished state. Apps keep no parallel copies.
- Core returns English text. FFI labels and derived values are record fields, not functions.
- iOS behaviour is canonical, unless it's a bug. Here no app plays audio, so the gap is a missing feature.
- What Mtrx shows itself isn't a download ("What counts as a download"). Playing is showing.

Compatible with step 4 (`docs/app-structure.md`): the desktop part is one component module with its own `Message`.

Paths: as in `docs/downloads.md`, plus `ruma/` = `~/.cargo/registry/src/index.crates.io-*/ruma-events-0.35.1/src/`.

Recording and sending voice messages is a later step (end of this document).

---

## Today

**Core**
- `core/timeline.rs:1560-1567`: an `m.audio` message gives `Media{kind: Audio, filename, size, mimetype, source}`. `info.duration` is dropped, and so are Element's voice-message fields.
- `core/timeline.rs:280-293`: `Media` has no duration. `width` and `height` are for images and videos.
- ruma already parses what's needed (`ruma/room/message/audio.rs`):
  - `info.duration` (`:131`), milliseconds on the wire.
  - `audio` (`:50`), the event's `org.matrix.msc1767.audio`: `duration` (`:162`) and `waveform` (`:168`), each value 0–1024 (`:201-209`).
  - `voice` (`:58`), the event's `org.matrix.msc3245.voice`: the event is a voice message.
  - Both sit behind ruma's `unstable-msc3245-v1-compat`, which matrix-sdk 0.19.1 turns on (`matrix-sdk-0.19.1/Cargo.toml:349`).
- `core/fetch.rs:65-111` `fetch_file` streams a message's file to disk with progress, decrypting encrypted ones. It takes the SDK media store's copy if there is one (`:113-122`).
- `core/downloads.rs:47-56` `SaveTo::Cache` puts the phones' downloads in `<cache>/media/<server>/<media id>/<name>`.

**FFI**
- `ffi/types.rs:491-502`: the `Media` record mirrors core's, so no duration, voice flag or waveform.
- `ffi/downloads.rs:95` `Room.download` is how the phones get a file.

**Desktop**
- `app/media.rs:304-349` `view`: audio falls through to `open_card` (`:338`, `:378-384`). The card says "Audio" (`kind_label`, `:451-458`).
- Clicking it (`app/downloads.rs:193-214`, `OpenFile`) downloads to the Downloads folder and opens the file in its default app.
- macOS has no app for Ogg Opus (below), so Element's voice messages don't play. That's the 0.8.7 report. On Linux and Windows it depends on the players installed.

**iOS**
- `ios/Views/MediaViews.swift:65-102` `FileView`: a "waveform" symbol (`:94`), the name and the size.
- A tap (`TimelineRowView.swift:211-213`) downloads to the cache and opens QuickLook (`RoomView.swift:135, 158-169`). QuickLook plays MP3 and M4A, not Ogg Opus.

**Android**
- `android/timeline/EventRows.kt:410` `FileRow` (`:461-486`): the name, the size, download progress.
- A tap (`TimelineScreen.kt:371-372`, `TimelineViewModel.kt:209-230`) downloads to the cache and hands the file to another app (`Files.kt:46-57`, `ACTION_VIEW`).

**Ogg Opus on Apple systems**
- AVFoundation plays Opus in CAF since iOS 11 and macOS 10.13. It plays Opus in MP4 from iOS 17 (an Apple engineer on the developer forums). Apple's documented formats don't list Opus.
- There was no Ogg container before iOS 18.4 and macOS 15.4. Since then Safari plays Ogg Opus and Ogg Vorbis, and QuickTime and Quick Look play Ogg Vorbis (reports, not documentation). Nothing documents Ogg for `AVAudioPlayer`.
- The iOS deployment target is 16, so iOS needs Ogg Opus converted whatever newer versions do.
- The desktop doesn't use AVFoundation. It plays with rodio (below).

**Ogg Opus on Android**
- Android's format table lists the Opus decoder from Android 5.0, and the Ogg extractor with no version limit. `MediaPlayer` plays Element's voice messages at `minSdk` 26.

**Crates** (crates.io, 18 Nov 2026)
- `rodio` 0.23.0 (11 Oct 2026): playback through `cpal` 0.18.2, decoding through `symphonia` 0.5.5. `Sink` is now called `Player`, and `OutputStream` is now `MixerDeviceSink`.
- `symphonia` 0.6.1 (13 Aug 2026) is out, but rodio 0.23 still uses 0.5.5.
- Symphonia has no Opus decoder. Its README gives Opus no status, and `symphonia-codec-opus` isn't published. `symphonia-adapter-libopus` 0.3.0 wraps libopus for symphonia 0.6 only, so it doesn't fit rodio.
- `opus` 0.4.0 (23 Aug 2026): safe bindings over `opusic-sys` 0.7.7 (4 Nov 2026), which builds libopus 1.6.1 from source with CMake. Decoder and encoder, no multistream.
- `ogg` 0.9.2 (Jan 2025): a pure-Rust Ogg reader and writer. It depends only on `byteorder`, which is already locked.
- Pure-Rust Opus decoders now exist (Q1):
  - `opus-decoder` 0.1.1 (Mar 2026): no `unsafe`, no FFI, passes all 12 RFC 8251 test vectors, has multistream. It's a decoder only, with one release and one maintainer.
  - `opus-rs` 0.1.42 (10 Nov 2026): a port of libopus 1.6 with an encoder and a decoder, released often. Its first release was in Feb 2026.
  - `ropus` and `mousiki`: little use.

## Where they disagree

| | Desktop | iOS | Android |
|---|---|---|---|
| Audio bubble | file card | file row | file row |
| Click or tap | Downloads folder, then its app | cache, then QuickLook | cache, then another app |
| Element voice message | fails on macOS | fails (QuickLook) | plays in another app |
| Duration, waveform | not parsed | — | — |

- No app plays inline, so the design follows the platforms' conventions, the same on all three.

---

## Metadata

Core reads, for `m.audio`:
- **Duration:** `info.duration`, else the MSC1767 block's `duration`. None if neither. `m.video` gets its `info.duration` too, but nothing shows it yet.
- **Voice message:** the event has `org.matrix.msc3245.voice`. The bubble then shows no file name, since "Voice message.ogg" says nothing.
- **Waveform:** the MSC1767 block's `waveform`, when it has values. Core turns it into `WAVEFORM_BARS` (40) values from 0 to 1:
  - with more samples than bars, each bar is the loudest sample of its stretch, so peaks stay;
  - with fewer, each bar takes the nearest sample;
  - then each is divided by the loudest, so a quiet recording still shows its shape. All zero stays zero.
- The apps draw the bars they get across the width they have. Whether to show a waveform at all is Q5.

## Where playback lives

- **The file:** core fetches the audio into its own cache and hands the app a path the platform's player can open. On the desktop and iOS, Ogg Opus is first decoded to WAV. Android plays it as it is. It isn't a download, so it doesn't show in the desktop's list (Q3).
- **The output:** each app owns its player: rodio on a thread (desktop), `AVAudioPlayer` (iOS), `MediaPlayer` (Android). The player holds the audio device, the OS's audio session or focus, calls that interrupt and headphones that go. Core sees none of these.
- **One at a time:** starting a message pauses the one playing, which keeps its position.
- **Positions:** kept per message (by `Media.source`) while the room is open. A message that ends goes back to the start.
- **Leaving the room** stops playback and forgets the positions (Q6).
  - Phones: going to the background pauses.
  - Desktop: hiding or minimising the window keeps playing, as a browser does.
- **The state is the app's** (Q4):
  - The position belongs to the player, which the app reads ten times a second. A core `Live` would be a mirror the app writes and reads back: a second copy, and FFI calls both ways ten times a second.
  - Nothing in it is shared: not between devices, not stored, not used by another screen.
  - What can be computed once is in core: the playable file, the bars and the time labels.

## Core: new `core/audio.rs`

```rust
/// Bars in a voice message's waveform ([`Media::waveform`]).
pub const WAVEFORM_BARS: usize = 40;

pub struct Media {                       // timeline.rs, gains three fields
    // … as now …
    /// Audio and video: as sent (`info.duration`, else MSC1767's).
    pub duration: Option<Duration>,
    /// A voice message (MSC3245): its file name means nothing.
    pub voice: bool,
    /// Audio: the sender's waveform (MSC1767) as WAVEFORM_BARS values from 0 to 1;
    /// empty if none.
    pub waveform: Vec<f32>,
}

/// A time as a player shows it, rounded down: "0:07", "12:34", "1:02:03".
pub fn playback_time(time: Duration) -> String;

impl Session {
    /// A message's audio as a file this platform's player can open, in the profile's
    /// audio cache: fetched (`progress` from 0 to 1; None while the length is unknown
    /// or while it converts), and Ogg Opus decoded to WAV where the player can't play
    /// it (feature `opus`). At once if it's there. Dropping the future stops it and
    /// deletes partial files.
    pub async fn audio_file(&self, media: &Media, progress: impl FnMut(Option<f32>) + Send)
        -> Result<PathBuf>;
}

pub enum Error {                         // error.rs
    // …
    /// A message's audio can't be fetched or played (why, in English).
    #[error("{0}")]
    Audio(String),
}
```

**Session**
- Gains `audio: Arc<AudioCache>`, like `downloads`: the folder `<cache>/audio/`, and a lock per file so two calls never write the same `.part`.

**How `audio_file` runs**
1. The name: `<server>_<media id>`, made path-safe as downloads do (`path_safe`), with the extension from the sender's name or the MIME type.
2. Already there (as `.wav` or as the original): its modification time is set to now, for the limit, and it's returned.
3. A finished `SaveTo::Cache` download of the same file (phones) is used as the source. Otherwise `fetch_file` gets it to `<cache>/audio/`, reporting progress.
4. With the `opus` feature, core reads the first Ogg page. If its packet starts `OpusHead`, the file is decoded to `<name>.wav` (below) and the fetched file goes. Ogg Vorbis, MP3, M4A and the rest come back as they are.
5. The cache is trimmed to `CACHE_LIMIT` (200 MB), least recently used first. The file just returned always stays.

**Ogg Opus to WAV** (`core/audio/opus.rs`, feature `opus`)
- Runs in `spawn_blocking`. It stops if the future is dropped.
- Packets come from `ogg::PacketReader`.
- `OpusHead` gives the channels, the pre-skip, the output gain and the mapping family. Only family 0 (mono or stereo) is read. Others fail with "This audio can't be played".
- `OpusTags` is skipped.
- Each packet goes through `opus::Decoder` (48 kHz, the head's channels) into 16-bit samples.
- The first pre-skip samples are dropped. The last page's granule position cuts the end (RFC 7845, section 4).
- The output gain goes through the decoder's `set_gain`.
- A packet libopus rejects fails with "The file is damaged".
- The WAV is 16-bit PCM at 48 kHz, with 1 or 2 channels. Its header's sizes are written at the end. It's written to `.part`, then renamed.
- libopus decodes hundreds of times faster than real time, so a few minutes of voice take a fraction of a second. A minute of mono is 5.8 MB.

**The cache**
- `<cache>/audio/` goes with the profile's cache at sign-out, as the downloads' cache does.
- A WAV larger than the limit (an hour of Ogg Opus is 345 MB) still plays: the file just returned stays. Ogg Opus that long is rare in Matrix.

**`media()`** (`core/timeline.rs:1518-1590`)
- The audio arm reads `c.info`, `c.audio` and `c.voice`. The video arm reads `info.duration`.
- Other arms set `duration: None`, `voice: false` and an empty `waveform`.

## FFI

```rust
#[derive(uniffi::Record)]
pub struct Media {                         // types.rs, gains
    // … as now …
    pub duration: Option<Duration>,        // uniffi: TimeInterval (Swift), java.time.Duration (Kotlin)
    pub duration_label: Option<String>,    // "0:42", filled when built
    pub voice: bool,
    pub waveform: Vec<f32>,                // WAVEFORM_BARS values from 0 to 1, or none
}

/// Receives how fetching an audio file goes.
#[uniffi::export(with_foreign)]
pub trait AudioProgress: Send + Sync {
    fn on_progress(&self, fraction: Option<f32>);
}

#[uniffi::export]
impl Session {
    /// A message's audio as a file this platform's player can open (iOS: Ogg Opus as WAV).
    pub async fn audio_file(&self, media: Media, progress: Arc<dyn AudioProgress>)
        -> Result<String, MtrxError>;
}

/// "0:07", "12:34", "1:02:03".
#[uniffi::export]
pub fn playback_time(time: Duration) -> String;
```
- `playback_time` is a function, not a field. The position belongs to the app's player, so there's no record to put its label in. One function keeps the format the same on all three apps.
- `audio_file` runs through `run`, so cancelling the Swift `Task` or the Kotlin coroutine stops the fetch.
- `Error::Audio` reaches the apps as `MtrxError::Other`, with core's text.
- `Media::to_core` maps the new fields back. `duration_label` is ignored on the way back.
- `mtrx-ffi` turns on core's `opus` for iOS only:
  ```toml
  [target.'cfg(target_os = "ios")'.dependencies]
  mtrx-core = { workspace = true, features = ["math", "opus"] }
  ```
  The workspace's resolver (3) applies it only when building for iOS. Android and the host bindings build without libopus. The exported API is the same either way.

## Dependencies

| Crate | Version and features | In | Brings |
|---|---|---|---|
| `rodio` | 0.23.0, no default features; `playback`, `mp3`, `mp4`, `flac`, `vorbis`, `wav` | `mtrx-app` | `cpal` 0.18.2; `symphonia` 0.5.5 for MP3, AAC-LC in MP4/M4A, FLAC, Ogg Vorbis and WAV; `dasp_sample` 0.11 |
| `opus` | 0.4.0 | `mtrx-core`, feature `opus` (desktop, iOS) | `opusic-sys` 0.7.7: libopus 1.6.1 from source, built with CMake |
| `ogg` | 0.9.2 | `mtrx-core`, feature `opus` | nothing new |

- Licences: rodio and opus are MIT/Apache-2.0, symphonia is MPL-2.0, libopus and ogg are BSD-3. All fit the AGPL.
- `cpal` on each platform:
  - macOS: Core Audio, through `coreaudio-rs` 0.14 and `objc2-core-audio` and `objc2-audio-toolbox`. `objc2` 0.6 is already locked. Playing needs no permission.
  - Windows: WASAPI, through `windows` 0.62. The app already uses that version, with fewer features.
  - Linux: ALSA, through `alsa` 0.11 and `alsa-sys`, which link `libasound`. PipeWire and PulseAudio route ALSA through their plugins.
- Left out: rodio's `recording` (no microphone code, no prompt) and `dither`.

**CI**
- Linux: `libasound2-dev` joins both apt lists (`.github/workflows/linux.yml:43, 115`).
  - The `.deb` gains `libasound2` through `dpkg-shlibdeps` by itself (`crates/xtask/src/linux.rs:186`).
  - The AppImage uses the system's `libasound.so.2`, as AppImages do.
- CMake for libopus is on every GitHub runner (macOS, Windows, Ubuntu). It builds for `aarch64-apple-ios` and `aarch64-apple-ios-sim` through the `cmake` crate, which is already locked (0.1.58, for `aws-lc-sys`).
- This laptop has no CMake. A local desktop build or iOS library build would need `brew install cmake`. Builds are CI-only now, so nothing changes day to day.

**Size**
- iOS: libopus's decoder and `ogg`, an estimated 0.3–0.5 MB per binary. The encoder's object files aren't linked. Typst costs about 30 MB, for comparison.
- Android: nothing. The feature is off, and `MediaPlayer` decodes.
- Desktop: an estimated 2–3 MB on 178 MB.
- The PR states the measured sizes from the iOS and Android Sizes steps.

## Desktop

**The bubble** (new `app/audio.rs`; `media::view` calls it for audio)
- Voice message: `[▶] [waveform] 0:42`.
- Other audio: the card's first line (the name, "size · Audio", the download button), then `[▶] [scrubber] 0:42`.
- The same width as the file card (460 at most).
- The button:
  - plays and pauses;
  - while fetching, shows a ring with the fraction, spinning without a total; a click cancels;
  - after a failure, shows ↻, which retries.
- The scrubber:
  - With a waveform: bars, the played ones in the accent colour. A click or a drag seeks.
  - Without: an iced `slider`. Dragging moves the time shown, and releasing seeks (`on_release`).
  - On a message that isn't playing, seeking sets where it will start.
- The time: the position once started, else the duration, else the size. Tabular figures. A failure shows its reason here instead, in pink.
- Tooltips: "Play", "Pause".
- No keyboard shortcut. The composer has the focus, so Space types a space, and iced 0.14's buttons don't take focus. It isn't cheap, so it's left out.

**State and messages**
```rust
pub struct Audio {
    now: Option<Now>,                          // the message playing, paused, fetching or failed
    positions: HashMap<MediaRef, Duration>,    // where others stopped, while the room is open
    output: Option<output::Commands>,          // the audio thread's, once started
    fetch: Option<task::Handle>,               // aborted by another play or Stop
}
struct Now { file: File, state: State, position: Duration, duration: Option<Duration>,
             drag: Option<Duration> }
enum State { Fetching(Option<f32>), Playing, Paused, Failed(String) }

#[derive(Debug, Clone)]
pub enum Message {
    Toggle(File),                       // the button: play, pause, cancel or retry
    Drag(MediaRef, Duration),           // the slider moves
    Seek(MediaRef, Duration),           // released, or a click on the waveform
    Progress(MediaRef, Option<f32>),
    Fetched(MediaRef, Result<PathBuf, String>),
    Output(output::Event),
    Stop,                               // another room opened, or signing out
}

impl Audio {
    pub fn update(&mut self, session: &Arc<Session>, message: Message) -> Task<Message>;
    pub fn view<'a>(&'a self, file: &File) -> Element<'a, app::Message>;
    pub fn subscription(&self) -> Subscription<Message>;
}
```
- `File` is `downloads::File`: the media, the room and the event.
- `Audio` lives in `MediaState` beside `downloads` (`app/media.rs:776`). `room/view.rs:706` passes it to `media::view`.
- `pane::Message::Open` sends `Stop`.
- The fetch is a `Task::sip` over `audio_file`: progress steps, then the path.

**The audio thread** (`app/audio/output.rs`)
```rust
pub enum Command { Play { path: PathBuf, from: Duration }, Pause, Resume, Seek(Duration), Stop }
pub enum Event { Ready(Commands), Started { duration: Option<Duration> }, At(Duration), Ended,
                 Failed(String) }
```
- The subscription (`Subscription::run`) starts one thread and hands the app its command sender in `Ready`, the usual iced worker.
- The thread owns rodio's `MixerDeviceSink` and `Player`. Opening the device and the file can block, so neither happens on the UI thread.
- `Play`:
  - opens the default output (`DeviceSinkBuilder::open_default_sink`, with `log_on_drop(false)`);
  - opens the file with `Decoder::try_from(File)`, then `append` and `try_seek(from)`;
  - sends `Started` with `Decoder::total_duration()`. Without it, the event's duration is used.
- While playing, it sends `At(player.get_pos())` every 100 ms. When `player.empty()`, it sends `Ended`.
- `Stop` and the end drop the player and the device, so nothing holds the audio hardware while idle. Pause keeps them.
- Idle, the thread waits on its channel: no ticks, no redraws.
- No output device: "No sound output". A file rodio can't read: "Can't play this file".

**Formats:** MP3, AAC-LC (M4A, MP4), FLAC, WAV and Ogg Vorbis through symphonia; Ogg Opus through core. ALAC, AIFF, WebM and AMR fail with "Can't play this file". The download button still gets the file.

## iOS

**`AudioModel`** (new `ios/Models/AudioModel.swift`)
```swift
@MainActor
final class AudioModel: NSObject, ObservableObject, AVAudioPlayerDelegate {
    enum State { case fetching(Float?), playing, paused, failed(String) }
    @Published private(set) var current: String?        // the Media.source playing, paused, …
    @Published private(set) var state: State?
    @Published private(set) var position: TimeInterval
    @Published private(set) var duration: TimeInterval?
    func toggle(_ media: Media)
    func seek(_ media: Media, to time: TimeInterval)
    func position(of media: Media) -> TimeInterval       // kept positions too
    func pause()
}
```
- One per room screen. `RoomView` holds it as a `@StateObject` and injects it, like `DownloadsModel`, so ticks redraw only audio bubbles.
- Closing the room screen releases the model and its player, which stops the sound. Room info opened over the room keeps playing.
- `toggle`:
  - `session.audioFile(media:progress:)` in a `Task`, cancelled by another play;
  - a small `AudioProgress` class hops to the main actor;
  - then `AVAudioPlayer(contentsOf:)`, `currentTime` set to the kept position, `play()`.
- A 0.1 s `Timer` reads `currentTime` while playing. The player's `duration` replaces the event's once loaded.
- `audioPlayerDidFinishPlaying` goes back to the start. `audioPlayerDecodeErrorDidOccur` says "Can't play this file".
- **The audio session:**
  - Category `.playback`, so it plays with the silent switch on, as media apps do.
  - Mode `.spokenAudio` for voice messages, `.default` otherwise. No mixing: other audio pauses.
  - Active while playing. Deactivated on pause and at the end with `.notifyOthersOnDeactivation`, so the user's music resumes.
  - An interruption (a call) pauses. A route change with `.oldDeviceUnavailable` (headphones out) pauses.
  - No background audio mode. Leaving the foreground (`scenePhase`) pauses.
- iOS 16 to 18.3 can't play Ogg Vorbis. Core converts only Opus, so those fail with "Can't play this file". They're rare in Matrix.

**`AudioView`** (in `MediaViews.swift`; `TimelineRowView` uses it instead of `FileView` for audio)
- Voice: the play button, `WaveformView` (a `Canvas`; a drag seeks), the time.
- Other audio: the name and size above, then the button, a `Slider` and the time.
- The button is 44 pt. While fetching it shows a ring like `DownloadBadge`'s, and a tap cancels.
- The time is `Text(playbackTime(time:))`, `.monospacedDigit()`.
- VoiceOver: the button reads "Play" or "Pause". The scrubber's value reads "0:12 of 0:42", and it adjusts by 5 s.
- A tap on the rest of the bubble no longer opens QuickLook for audio. The menu keeps Share….

## Android

**`AudioPlayer`** (new `android/timeline/AudioPlayer.kt`)
```kotlin
data class Playback(
    val source: String?,               // the Media.source playing, paused, …
    val state: PlayState?,
    val position: Duration,
    val duration: Duration?,
    val kept: Map<String, Duration>,
)
sealed interface PlayState {
    data class Fetching(val fraction: Float?) : PlayState
    data object Playing : PlayState
    data object Paused : PlayState
    data class Failed(val message: String) : PlayState
}

/** MediaPlayer, behind an interface so tests use a fake. */
interface Player { fun open(path: String, voice: Boolean, onReady: () -> Unit, onEnd: () -> Unit,
                        onError: () -> Unit); fun start(); fun pause(); fun seekTo(ms: Long)
                   val position: Long; val duration: Long?; fun release() }

class AudioPlayer(session: Session, scope: CoroutineScope, context: Context, newPlayer: () -> Player) {
    val playback: StateFlow<Playback>
    fun toggle(media: Media)
    fun seek(media: Media, to: Duration)
    fun pause()
    fun release()
}
```
- `TimelineViewModel` holds it and releases it in `onCleared()`, when the room is left.
- `toggle`:
  - `session.audioFile(media, progress)` in a coroutine, cancelled by another play;
  - then `MediaPlayer`: `setAudioAttributes` (`USAGE_MEDIA`; `CONTENT_TYPE_SPEECH` for voice, `CONTENT_TYPE_MUSIC` otherwise), `setDataSource(path)`, `prepareAsync()`, `seekTo(kept, SEEK_CLOSEST)`, `start()`.
- A coroutine reads `currentPosition` every 100 ms while playing.
- Completion goes back to the start. An error says "Can't play this file".
- Audio focus: an `AudioFocusRequest` with `AUDIOFOCUS_GAIN_TRANSIENT`. Losing focus pauses.
- `ACTION_AUDIO_BECOMING_NOISY` (headphones out) pauses. Its receiver is registered only while playing.
- The lifecycle's `ON_STOP` pauses.

**`AudioRow`** (in `EventRows.kt`, instead of `FileRow` for audio)
- Voice: an `IconButton` (48 dp) that plays and pauses, a waveform `Canvas` (a tap or a drag seeks), the time.
- Other audio: the name and size above, then the button, a Material `Slider` and the time.
- While fetching, a `CircularProgressIndicator` circles the button, and a tap cancels.
- TalkBack: the button reads "Play" or "Pause". The waveform has `progressBarRangeInfo` and `setProgress`.
- A tap on the rest of the bubble no longer opens another app for audio. The long-press sheet keeps Share… and Save to Downloads.

## Tests

**Core**
- Unit:
  - `playback_time`: seconds, minutes, hours, rounding down.
  - The waveform: more samples than bars (peaks kept), fewer (nearest), all zero.
  - Parsing: an Element Web voice message's JSON gives the duration, `voice` and 40 bars. `info.duration` wins over MSC1767's. A plain `m.audio` gives no waveform.
  - Ogg Opus to WAV (feature `opus`):
    - The test encodes one second of a sine with libopus's encoder and writes it as Ogg with `ogg`'s writer, so no binary fixture is needed. The WAV has exactly 48 000 frames (pre-skip and end trim right), and its strongest frequency is the sine's.
    - A real voice message sent from Element Web, kept as a fixture of a few kB, converts.
    - Ogg Vorbis and MP3 come back unchanged.
    - A cut-off file fails with "The file is damaged". A mapping-family-1 head fails with "This audio can't be played".
  - The limit removes the least recently used file. The file just returned stays.
  - Two calls for one file at once fetch it once.
- Integration (local Synapse, `test-support`):
  - An encrypted Ogg Opus file comes back as WAV, with progress rising to 1.
  - A plain MP3 comes back as it is.
  - A finished cache download is used without fetching.

**FFI**
- `Media` from core: `duration`, `duration_label`, `voice`, 40 bars. Back to core unchanged.
- `playback_time`.

**Desktop**
- `Audio`'s update with a fake output (CI has no audio device):
  - playing B pauses A and keeps A's position;
  - `Ended` goes back to the start;
  - `Stop` forgets the positions and aborts a fetch;
  - seeking a message that isn't playing sets where it starts.
- `iced_test`: a voice bubble shows the button and "0:42", and clicking the button sends `Toggle`.

**iOS**
- `AudioModel` with a fake player: the same four cases.
- Screenshot tests: a voice message paused part-way, audio with a slider, fetching, failed.

**Android**
- JVM: `AudioPlayer` with a fake `Player`: the same four cases.
- Paparazzi: the same four bubbles.

---

## Deleted

- Desktop: audio's way to `open_card` (`app/media.rs:338`). Clicking an audio card no longer opens its app.
- iOS: `FileView`'s audio case (`MediaViews.swift:94`), and the tap to QuickLook for audio (`TimelineRowView.swift:211-213`).
- Android: `FileRow` for audio (`EventRows.kt:410`), and the tap to `ACTION_VIEW` for audio.
- Core and FFI: nothing. Downloads stay as they are.

## PR split

One PR, four commits, each building (CI only):
1. Core and FFI: `Media.{duration, voice, waveform}`, `audio.rs`, the `opus` feature, `Error::Audio`, the FFI fields, `audio_file`, `AudioProgress`, `playback_time`, the tests. The PR states the iOS size from the Sizes step.
2. Desktop: rodio, `app/audio.rs` and its thread, the bubble, `libasound2-dev` on Linux CI.
3. iOS: `AudioModel`, `AudioView`, the waveform, the audio session.
4. Android: `AudioPlayer`, `AudioRow`, focus and headphones, the tests.

Size: M–L. About 450 lines of core (with the converter), 400 desktop and 250 per phone. One PR is enough: core and FFI only add.

## Later: recording voice messages

Out of scope. What this design leaves ready:
- libopus's encoder is already linked on the desktop and iOS, and `ogg` writes pages.
- Android can record Ogg Opus with `MediaRecorder` (`OutputFormat.OGG`, `AudioEncoder.OPUS`) from API 29. API 26–28 would need libopus too.
- Sending needs `info.duration`, MSC1767's `duration` and `waveform` from the recording, and the MSC3245 marker.
- It needs microphone permission on all three: usage strings, and an entitlement on macOS.

---

## Open questions

1. **Which Opus decoder.**
   - (a) libopus, through `opus` 0.4.0 and `opusic-sys` 0.7.7 (libopus 1.6.1):
     - the reference decoder, in every browser, fuzzed continuously by OSS-Fuzz;
     - it has the encoder for recording later;
     - it's C, built with CMake. That's fine on CI. This laptop would need `brew install cmake` for local builds, and iOS goes through CMake's cross build (the first CI run tells).
   - (b) `opus-decoder` 0.1.1, pure Rust: no `unsafe`, no C, no CMake, and it passes the RFC 8251 vectors. But it has one release (Mar 2026), one maintainer, and no encoder.
   - (c) `opus-rs` 0.1.42, a pure-Rust port with an encoder. Active, but nine months old.
   - Recommend (a), as approved in principle. (b) is the fallback if the iOS CMake build fights back.

2. **What iOS plays for Ogg Opus.**
   - (a) WAV, decoded in core by the desktop's code. It's certain to play. It takes 5.8 MB a minute in the cache, and puts libopus in the iOS binary (0.3–0.5 MB).
   - (b) CAF with the Opus packets copied, no decoding. Tiny files and no libopus on iOS. AVFoundation reads Opus in CAF since iOS 11, but the details are undocumented: the packet table, the priming frames and packets of varying length must be right, and Opus CAFs made by ffmpeg once stopped playing on iOS. The desktop still needs (a), so there would be two converters.
   - (c) Decoding in Swift with `AVAudioConverter`, core supplying the packets. More Swift, against "core computes once".
   - Recommend (a).

3. **How the file is fetched.**
   - (a) Core's own audio cache, through `fetch_file`, with progress to the caller. It isn't in the downloads list. It's bounded at 200 MB.
   - (b) As a `SaveTo::Cache` download. Progress and retry come free, but the desktop's list would fill with played voice messages and push out real downloads (it keeps 20). The desktop's cache would have no bound.
   - (c) `media_content` in memory, as the viewers do. No progress, and whole files in memory.
   - Recommend (a). By the downloads rule, playing is showing.

4. **Where the playback state lives.**
   - (a) In each app, with its player. Core gives the file, the bars and the time labels.
   - (b) A core `Live<Playback>` that the apps feed from their players' events and draw from.
   - Recommend (a). The position belongs to an OS object. (b) would be a mirror with traffic both ways, and nothing in it is shared.

5. **Waveform.**
   - (a) The sender's waveform for voice messages that have one (Element's do). A plain slider for other audio.
   - (b) A slider everywhere.
   - (c) (a), plus a waveform computed from the decoded file when the event has none. Only the desktop and iOS decode, so Android couldn't.
   - Recommend (a). It's how voice messages look in Element and elsewhere, and it needs no decoding.

6. **Leaving the room.**
   - (a) Playback stops, and the positions are forgotten.
   - (b) It keeps playing, with a small bar (the room, pause, ✕) while the user is elsewhere.
   - (c) It stops, but positions are kept per room until Mtrx quits.
   - Recommend (a). It's simple, and it's what "while the room is open" means. (b) can come later as a mini player.

7. **Android's player.**
   - (a) `MediaPlayer`, in the framework. No new dependency, and it plays Ogg Opus from Android 5.
   - (b) Media3 ExoPlayer 1.11.1 (`androidx.media3:media3-exoplayer`). A cleaner state model and steadier seeking, but a new dependency of about 1 MB.
   - Recommend (a). The files are local and short, and `seekTo(…, SEEK_CLOSEST)` is enough. Revisit if devices misbehave.

## Decided (2026-11-18)

All recommendations taken: 1a (libopus through `opus`; `opus-decoder` the fallback if the iOS build fights back), 2a (WAV for iOS), 3a (core's own audio cache, 200 MB), 4a (playback state in each app), 5a (the sender's waveform for voice messages, a slider otherwise), 6a (leaving the room stops playback), 7a (`MediaPlayer` on Android).

## As built

Differences from the design above.

**Core and FFI**
- Cache names: `<server>_<media id>.<ext>` as fetched, `<server>_<media id>.wav` decoded. Partial files (`.part`) are left alone by the limit; those not written for an hour (a crash) go.
- A finished cache download that needs no converting comes back as its own path, not copied.
- No Element Web fixture yet: no recording was at hand. The sine test (exact length, frequency), a cut-off file, mapping family 1, Ogg Vorbis and MP3 are covered.
- "Two calls fetch once" is in the integration test. `test_support::ogg_opus_sine` makes the Ogg Opus files; core's own tests always build with `opus`.
- FFI `Media` is no longer `Eq` or `Hash` (the waveform is `f32`). Its new fields are `#[uniffi(default)]`, so the apps' existing `Media(…)` calls compile. `audio_file`, `AudioProgress` and `playback_time` are in `ffi/audio.rs`.

**Desktop**
- The fetch is `Task::run` over `iced::stream::channel`, not `Task::sip`: iced's `sipper` feature would add a crate.
- The thread's events carry the number of the `Play` they answer (`Message::Output(u64, Event)`), so late news of the message before is ignored.
- Starting another message sends `Stop` (the output is let go) and keeps the first's position. After the end the message is no longer current; the next play fetches it again (at once, from the cache).
- `Play` seeks the decoder before it plays, so the start doesn't sound first.
- `Pane::set` sends `Stop` whenever the room shown changes (opening another, leaving it, an invite), not only on `pane::Message::Open`.
- `media.rs` draws the bubble (the card, and for other audio the file card's line, now `file_line`); `Audio::view` draws the button, scrubber and time, borrowing nothing from `Audio`.
- Tabular figures: iced 0.14 can't turn on Inter's `tnum`, so the time sits right-aligned in a fixed-width box.
- While the length is unknown the ring is empty, not spinning, as the downloads' (no redraws while fetching).
- Tooltips also say "Cancel" while fetching and "Try again" after a failure. Play, pause and ↻ are drawn icons (`icons.rs`).

**iOS**
- `AudioModel` isn't the `AVAudioPlayerDelegate`. The player is behind `AudioPlaying`, made by a factory (`SystemAudioPlayer` by default, a fake in tests). `SystemAudioPlayer` is the delegate and holds the audio session, so tests have none.
- A finish that isn't successful says "Can't play this file". The ticks are a 0.1 s loop in `Work`, not a `Timer`.
- `pause()` while fetching cancels the fetch. Leaving the foreground is `scenePhase == .background`, so Control Center and banners don't pause.
- After a failure the button is ↻ and the reason is a pink line below the controls, not in place of the time. A voice message without a length shows its size; other audio shows nothing there (its size is above).
- The waveform and the slider both seek on release. VoiceOver also reads "Cancel loading" and "Try again". The controls are at most 260 pt wide.
- `RoomView` takes the model from `OpenRoom` (an autoclosure); `Work` gains `cancel(_:)`. `FileView` always shows "doc".
- Tests: releasing the model stands for `Stop`; one more covers cancel and retry.

**Android**
- `AudioPlayer` takes an `AudioOutput` (focus and headphones) instead of a `Context`; `SystemAudioOutput` is built from the application context in `Navigation.kt` and passed to `TimelineViewModel`. The JVM tests use fakes, without Robolectric.
- `Playback` has defaults and `stateOf`, `positionOf`, `durationOf`. At its end a message is paused at 0, keeping its player.
- `pause()` while fetching cancels the fetch. Refused focus (a call) leaves it paused; any loss of focus pauses (`setWillPauseWhenDucked`).
- After a failure the reason is a line below the row. A voice message without a waveform gets the slider. TalkBack also reads "Cancel loading", "Try again" and "0:12 of 0:42".
- The rows take the time format through `LocalPlaybackTime`, so the screenshots don't load the native library.
- Room info is its own screen, so opening it stops playback (on iOS it keeps playing over the room).
