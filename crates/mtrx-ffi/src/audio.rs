//! Audio messages: a message's audio as a file the platform's player can
//! open, and times as players show them. Playing is the app's.

use std::{sync::Arc, time::Duration};

use crate::{Media, MtrxError, Session, runtime::run};

/// Receives how fetching an audio file goes.
#[uniffi::export(with_foreign)]
pub trait AudioProgress: Send + Sync {
    /// From 0 to 1; none while the length is unknown or while it converts.
    fn on_progress(&self, fraction: Option<f32>);
}

#[uniffi::export]
impl Session {
    /// A message's audio as a file this platform's player can open (iOS:
    /// Ogg Opus as WAV), in the cache. At once if it's there. Cancelling
    /// the call stops the fetch.
    pub async fn audio_file(
        &self,
        media: Media,
        progress: Arc<dyn AudioProgress>,
    ) -> Result<String, MtrxError> {
        let (session, media) = (self.core(), media.to_core()?);
        run(async move {
            let path = session
                .audio_file(&media, |fraction| progress.on_progress(fraction))
                .await?;
            Ok(path.to_string_lossy().into_owned())
        })
        .await
    }
}

/// A time as a player shows it, rounded down: "0:07", "12:34", "1:02:03".
#[uniffi::export]
pub fn playback_time(time: Duration) -> String {
    mtrx_core::playback_time(time)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn times_are_the_library_s() {
        assert_eq!(playback_time(Duration::from_millis(7_900)), "0:07");
        assert_eq!(playback_time(Duration::from_secs(3723)), "1:02:03");
    }
}
