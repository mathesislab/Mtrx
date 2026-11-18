//! Messages' audio as files the platform's player can open: fetched into
//! the profile's audio cache and, where the player can't play Ogg Opus
//! (feature `opus`: the desktop, iOS), decoded to WAV. Playing is the
//! apps'. It isn't a download, so it isn't in the downloads list.

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, Weak},
    time::{Duration, SystemTime},
};

use crate::{
    Error, Media, Result, Session,
    downloads::path_safe,
    fetch::{Fetch, fetch_file, safe_name},
};

#[cfg(feature = "opus")]
pub(crate) mod opus;

/// Bars in a voice message's waveform ([`Media::waveform`]).
pub const WAVEFORM_BARS: usize = 40;
/// The audio cache's size, beyond which the least recently played files go.
const CACHE_LIMIT: u64 = 200 * 1000 * 1000;
/// A partial file not written for this long was left behind.
const STALE: Duration = Duration::from_secs(60 * 60);

/// A time as a player shows it, rounded down: "0:07", "12:34", "1:02:03".
pub fn playback_time(time: Duration) -> String {
    let seconds = time.as_secs();
    let (hours, minutes, seconds) = (seconds / 3600, seconds / 60 % 60, seconds % 60);
    if hours > 0 {
        format!("{hours}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes}:{seconds:02}")
    }
}

/// A sender's waveform (MSC1767: values from 0 to 1024) as
/// [`WAVEFORM_BARS`] values from 0 to 1, the loudest 1; empty if it has
/// none.
pub(crate) fn waveform(samples: &[u64]) -> Vec<f32> {
    let n = samples.len();
    if n == 0 {
        return Vec::new();
    }
    let bars: Vec<u64> = (0..WAVEFORM_BARS)
        .map(|bar| {
            if n > WAVEFORM_BARS {
                // The loudest of its stretch, so peaks stay.
                let (start, end) = (bar * n / WAVEFORM_BARS, (bar + 1) * n / WAVEFORM_BARS);
                samples[start..end].iter().copied().max().unwrap_or(0)
            } else {
                // The nearest sample.
                samples[((2 * bar + 1) * n / (2 * WAVEFORM_BARS)).min(n - 1)]
            }
        })
        .collect();
    let loudest = bars.iter().copied().max().unwrap_or(0);
    bars.into_iter()
        .map(|bar| match loudest {
            0 => 0.0,
            loudest => bar as f32 / loudest as f32,
        })
        .collect()
}

/// The profile's audio cache, `<cache>/audio/`. It goes with the cache at
/// sign-out.
pub(crate) struct AudioCache {
    folder: PathBuf,
    /// A lock per file, so two calls never write the same one.
    locks: Mutex<HashMap<String, Weak<tokio::sync::Mutex<()>>>>,
}

impl AudioCache {
    pub(crate) fn new(cache: &Path) -> Self {
        Self {
            folder: cache.join("audio"),
            locks: Default::default(),
        }
    }

    /// The lock of the file named `name`.
    fn lock(&self, name: &str) -> Arc<tokio::sync::Mutex<()>> {
        let mut locks = self.locks.lock().expect("not poisoned");
        locks.retain(|_, lock| lock.strong_count() > 0);
        if let Some(lock) = locks.get(name).and_then(Weak::upgrade) {
            return lock;
        }
        let lock = Arc::new(tokio::sync::Mutex::new(()));
        locks.insert(name.to_owned(), Arc::downgrade(&lock));
        lock
    }
}

impl Session {
    /// A message's audio as a file this platform's player can open, in the
    /// profile's audio cache: fetched (`progress` from 0 to 1; None while
    /// the length is unknown or while it converts), and Ogg Opus decoded
    /// to WAV where the player can't play it (feature `opus`). At once if
    /// it's there. Dropping the future stops it and deletes partial files.
    pub async fn audio_file(
        &self,
        media: &Media,
        mut progress: impl FnMut(Option<f32>) + Send,
    ) -> Result<PathBuf> {
        let cache = &self.audio;
        let (name, wav_name) = file_names(media);
        let lock = cache.lock(&name);
        let _turn = lock.lock().await;
        let (original, wav) = (cache.folder.join(&name), cache.folder.join(wav_name));
        for path in [&wav, &original] {
            if path.exists() {
                touch(path);
                return Ok(path.clone());
            }
        }
        let (file, fetched) = match self.downloads.in_cache(&media.source) {
            Some(path) => (path, false),
            None => {
                let fetch = Fetch {
                    source: &media.source.file,
                    size: media.size,
                    target: &original,
                    ciphertext: cache.folder.join(format!("{name}.encrypted.part")),
                    unique: false,
                    limit: None,
                };
                let path = fetch_file(&self.client, fetch, |received, total| {
                    progress(fraction(received, total))
                })
                .await
                .map_err(Error::Audio)?;
                (path, true)
            }
        };
        let file = playable(file, fetched, &wav, &mut progress).await?;
        let (folder, keep) = (cache.folder.clone(), file.clone());
        let _ = tokio::task::spawn_blocking(move || trim(&folder, CACHE_LIMIT, &keep)).await;
        Ok(file)
    }
}

/// The names of a message's audio in the cache, as fetched and as WAV:
/// `<server>_<media id>`, with the extension of the sender's name or of
/// the MIME type, or `.wav`.
fn file_names(media: &Media) -> (String, String) {
    let uri = media.source.uri();
    let (server, media_id) = match uri.parts() {
        Ok((server, media_id)) => (server.as_str(), media_id),
        Err(_) => ("invalid", uri.as_str()),
    };
    let stem = format!("{}_{}", path_safe(server), path_safe(media_id));
    let sent = safe_name(&media.filename, media.mimetype.as_deref());
    let original = match Path::new(&sent).extension().and_then(|e| e.to_str()) {
        Some(extension) => format!("{stem}.{}", path_safe(extension)),
        None => stem.clone(),
    };
    (original, format!("{stem}.wav"))
}

/// From 0 to 1, if the total is known.
fn fraction(received: u64, total: Option<u64>) -> Option<f32> {
    let total = total.filter(|&total| total > 0)?;
    Some((received as f64 / total as f64).min(1.0) as f32)
}

/// `file` as the player can open it: Ogg Opus decoded to `wav` (the file
/// goes if it was `fetched` into the cache), the rest as it is.
#[cfg(feature = "opus")]
async fn playable(
    file: PathBuf,
    fetched: bool,
    wav: &Path,
    progress: &mut impl FnMut(Option<f32>),
) -> Result<PathBuf> {
    if !self::opus::is_ogg_opus(&file).await {
        return Ok(file);
    }
    progress(None);
    self::opus::to_wav(&file, wav).await.map_err(Error::Audio)?;
    if fetched {
        let _ = tokio::fs::remove_file(&file).await;
    }
    Ok(wav.to_owned())
}

/// `file` as the player can open it: as it is, since it plays Ogg Opus.
#[cfg(not(feature = "opus"))]
async fn playable(
    file: PathBuf,
    _fetched: bool,
    _wav: &Path,
    _progress: &mut impl FnMut(Option<f32>),
) -> Result<PathBuf> {
    Ok(file)
}

/// Mark `path` as played now, for the limit.
fn touch(path: &Path) {
    let file = std::fs::File::options().write(true).open(path);
    let _ = file.and_then(|file| file.set_modified(SystemTime::now()));
}

/// Remove the least recently played files of `folder` until they take at
/// most `limit` bytes, but never `keep`; and partial files left behind.
fn trim(folder: &Path, limit: u64, keep: &Path) {
    let Ok(entries) = std::fs::read_dir(folder) else {
        return;
    };
    let now = SystemTime::now();
    let mut files = Vec::new();
    for entry in entries.flatten() {
        let (path, Ok(metadata)) = (entry.path(), entry.metadata()) else {
            continue;
        };
        if !metadata.is_file() {
            continue;
        }
        let modified = metadata.modified().unwrap_or(now);
        if path.extension().is_some_and(|e| e == "part") {
            if now.duration_since(modified).is_ok_and(|age| age > STALE) {
                let _ = std::fs::remove_file(&path);
            }
            continue;
        }
        files.push((modified, metadata.len(), path));
    }
    let mut total: u64 = files.iter().map(|(_, size, _)| size).sum();
    files.sort_by_key(|(modified, _, _)| *modified);
    for (_, size, path) in files {
        if total <= limit {
            break;
        }
        if path != keep && std::fs::remove_file(&path).is_ok() {
            total -= size;
        }
    }
}

#[cfg(test)]
mod tests {
    use matrix_sdk::ruma::events::room::message::RoomMessageEventContent;
    use serde_json::json;

    use super::*;
    use crate::timeline::media;

    #[test]
    fn times_show_as_players_show_them() {
        let time = |seconds: f64| playback_time(Duration::from_secs_f64(seconds));
        assert_eq!(time(0.0), "0:00");
        assert_eq!(time(7.9), "0:07");
        assert_eq!(time(59.999), "0:59");
        assert_eq!(time(60.0), "1:00");
        assert_eq!(time(754.0), "12:34");
        assert_eq!(time(3599.5), "59:59");
        assert_eq!(time(3723.0), "1:02:03");
        assert_eq!(time(36_000.0), "10:00:00");
    }

    #[test]
    fn a_waveform_keeps_its_peaks_and_its_shape() {
        // More samples than bars: each bar the loudest of its stretch.
        let mut samples = vec![100; 400];
        samples[15] = 1024;
        samples[399] = 512;
        let bars = waveform(&samples);
        assert_eq!(bars.len(), WAVEFORM_BARS);
        assert_eq!(bars[1], 1.0);
        assert_eq!(bars[39], 0.5);
        assert_eq!(bars[0], 100.0 / 1024.0);

        // Fewer: each bar the nearest sample, scaled to the loudest.
        let bars = waveform(&[10, 20, 40, 20]);
        assert_eq!(bars.len(), WAVEFORM_BARS);
        assert_eq!(bars[..10], [0.25; 10]);
        assert_eq!(bars[10..20], [0.5; 10]);
        assert_eq!(bars[20..30], [1.0; 10]);
        assert_eq!(bars[30..], [0.5; 10]);

        // As many: as they are.
        let samples: Vec<u64> = (1..=40).collect();
        let bars = waveform(&samples);
        assert_eq!(bars[39], 1.0);
        assert_eq!(bars[19], 0.5);

        assert_eq!(waveform(&[0; 100]), [0.0; WAVEFORM_BARS]);
        assert!(waveform(&[]).is_empty());
    }

    fn audio(content: serde_json::Value) -> Media {
        let content: RoomMessageEventContent = serde_json::from_value(content).unwrap();
        media(&content.msgtype).unwrap()
    }

    #[test]
    fn voice_messages_bring_their_duration_and_waveform() {
        // As Element Web sends it.
        let waveform: Vec<u64> = (0..100).map(|i| i * 10).collect();
        let voice = audio(json!({
            "msgtype": "m.audio",
            "body": "Voice message",
            "url": "mxc://example.org/voice",
            "info": { "mimetype": "audio/ogg", "size": 18_045, "duration": 4_210 },
            "org.matrix.msc1767.text": "Voice message",
            "org.matrix.msc1767.file": {
                "url": "mxc://example.org/voice",
                "name": "Voice message.ogg",
                "mimetype": "audio/ogg",
                "size": 18_045,
            },
            "org.matrix.msc1767.audio": { "duration": 4_210, "waveform": waveform },
            "org.matrix.msc3245.voice": {},
        }));
        assert!(voice.voice);
        assert_eq!(voice.duration, Some(Duration::from_millis(4_210)));
        assert_eq!(voice.waveform.len(), WAVEFORM_BARS);
        assert_eq!(voice.waveform[39], 1.0);
        assert!(voice.waveform.windows(2).all(|w| w[0] <= w[1]));

        // `info.duration` wins.
        let both = audio(json!({
            "msgtype": "m.audio",
            "body": "talk.ogg",
            "url": "mxc://example.org/talk",
            "info": { "duration": 1_000 },
            "org.matrix.msc1767.audio": { "duration": 2_000 },
        }));
        assert_eq!(both.duration, Some(Duration::from_secs(1)));
        assert!(!both.voice);
        assert!(both.waveform.is_empty());

        let plain = audio(json!({
            "msgtype": "m.audio",
            "body": "song.mp3",
            "url": "mxc://example.org/song",
            "info": { "mimetype": "audio/mpeg" },
        }));
        assert_eq!(plain.duration, None);
        assert!(!plain.voice);
        assert!(plain.waveform.is_empty());

        let only_msc1767 = audio(json!({
            "msgtype": "m.audio",
            "body": "note.ogg",
            "url": "mxc://example.org/note",
            "org.matrix.msc1767.audio": { "duration": 2_500 },
        }));
        assert_eq!(only_msc1767.duration, Some(Duration::from_millis(2_500)));
    }

    #[test]
    fn audio_is_cached_under_its_uri() {
        let voice = audio(json!({
            "msgtype": "m.audio",
            "body": "Voice message",
            "filename": "Voice message.ogg",
            "url": "mxc://example.org/abc-123",
        }));
        assert_eq!(
            file_names(&voice),
            (
                "example.org_abc-123.ogg".into(),
                "example.org_abc-123.wav".into()
            )
        );
        let unnamed = audio(json!({
            "msgtype": "m.audio",
            "body": "",
            "url": "mxc://example.org/x",
            "info": { "mimetype": "audio/ogg" },
        }));
        assert_eq!(file_names(&unnamed).0, "example.org_x.ogg");
    }

    #[test]
    fn the_limit_removes_the_least_recently_played() {
        let dir = tempfile::tempdir().unwrap();
        let now = SystemTime::now();
        let file = |name: &str, size: usize, minutes_ago: u64| {
            let path = dir.path().join(name);
            std::fs::write(&path, vec![0; size]).unwrap();
            let file = std::fs::File::options().write(true).open(&path).unwrap();
            file.set_modified(now - Duration::from_secs(60 * minutes_ago))
                .unwrap();
            path
        };
        let oldest = file("a.wav", 400, 30);
        let older = file("b.ogg", 400, 20);
        let newer = file("c.wav", 400, 10);
        let stale = file("d.wav.part", 10, 120);
        let running = file("e.wav.part", 10, 0);

        // Over by 300 bytes: the oldest goes.
        trim(dir.path(), 900, &newer);
        assert!(!oldest.exists());
        assert!(older.exists() && newer.exists());
        assert!(!stale.exists());
        assert!(running.exists());

        // The file just returned stays, even if it's the oldest.
        trim(dir.path(), 100, &older);
        assert!(older.exists());
        assert!(!newer.exists());
    }

    #[test]
    fn progress_needs_a_length() {
        assert_eq!(fraction(5, Some(10)), Some(0.5));
        assert_eq!(fraction(12, Some(10)), Some(1.0));
        assert_eq!(fraction(5, None), None);
        assert_eq!(fraction(0, Some(0)), None);
    }
}
