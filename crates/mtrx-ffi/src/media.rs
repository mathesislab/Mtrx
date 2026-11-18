//! Reducing photos and videos before sending (see
//! `docs/reducing-media.md`): the setting, and how a phone converts a
//! video. Photos are reduced by `Timeline.send`.

use std::time::Duration;

use mtrx_core as core;

use crate::{MtrxError, Session, runtime::run};

/// How photos and videos are sent: a setting of each device.
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum MediaQuality {
    /// Photos at most 1 MB and 2048 px; videos at 720p, sized to their
    /// length (`Session.videoPlan`).
    Reduced,
    /// As taken.
    Original,
}

impl From<MediaQuality> for core::MediaQuality {
    fn from(quality: MediaQuality) -> Self {
        match quality {
            MediaQuality::Reduced => Self::Reduced,
            MediaQuality::Original => Self::Original,
        }
    }
}

impl From<core::MediaQuality> for MediaQuality {
    fn from(quality: core::MediaQuality) -> Self {
        match quality {
            core::MediaQuality::Reduced => Self::Reduced,
            core::MediaQuality::Original => Self::Original,
        }
    }
}

/// A choice of Settings' "Photos and videos", with its words.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct MediaQualityChoice {
    pub quality: MediaQuality,
    /// "Reduced", "Original".
    pub title: String,
    /// Below the setting while chosen: "Photos up to 1 MB, videos at
    /// 720p. Location data is left out."
    pub explanation: String,
}

/// Reduced, then Original, as Settings offers them.
#[uniffi::export]
pub fn media_quality_choices() -> Vec<MediaQualityChoice> {
    (core::MediaQuality::ALL.into_iter())
        .map(|quality| MediaQualityChoice {
            quality: quality.into(),
            title: quality.title().to_owned(),
            explanation: quality.explanation().to_owned(),
        })
        .collect()
}

/// The setting until it is changed on this device: Reduced.
#[uniffi::export]
pub fn default_media_quality() -> MediaQuality {
    core::MediaQuality::PHONE_DEFAULT.into()
}

/// The longer side of a video's still at most, in pixels
/// (`VideoInfo.thumbnail_path`).
#[uniffi::export]
pub fn video_thumbnail_edge() -> u32 {
    core::VIDEO_THUMBNAIL_EDGE
}

/// How to convert a video for sending: its size in pixels (as stored, like
/// the video it is made from), frames a second, and the bit rates of its
/// H.264 video and AAC sound, in bits a second.
#[derive(Clone, Copy, Debug, PartialEq, uniffi::Record)]
pub struct VideoPlan {
    pub width: u32,
    pub height: u32,
    pub fps: f32,
    pub video_bitrate: u32,
    pub audio_bitrate: u32,
}

impl From<core::VideoPlan> for VideoPlan {
    fn from(plan: core::VideoPlan) -> Self {
        Self {
            width: plan.width,
            height: plan.height,
            fps: plan.fps,
            video_bitrate: plan.video_bitrate,
            audio_bitrate: plan.audio_bitrate,
        }
    }
}

#[uniffi::export]
impl Session {
    /// How to convert a video picked to send while the setting is Reduced:
    /// `width`×`height` px as stored, `fps` frames a second (0 if not
    /// known), `duration_ms` long, `size` bytes. `None`: it is within the
    /// plan already, and is sent as it is. Fitted to the server's upload
    /// limit; a video too long even at the lowest quality fails, saying so
    /// ("Too long to send: the server takes at most 50.0 MB (about 10
    /// minutes)").
    pub async fn video_plan(
        &self,
        width: u32,
        height: u32,
        fps: f32,
        duration_ms: u64,
        size: u64,
    ) -> Result<Option<VideoPlan>, MtrxError> {
        let session = self.core();
        let duration = Duration::from_millis(duration_ms);
        run(async move {
            let plan = session
                .video_plan(width, height, fps, duration, size)
                .await?;
            Ok(plan.map(VideoPlan::from))
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_choices_have_their_words_and_the_phones_reduce() {
        let choices = media_quality_choices();
        assert_eq!(
            choices.iter().map(|c| c.quality).collect::<Vec<_>>(),
            [MediaQuality::Reduced, MediaQuality::Original]
        );
        assert_eq!(choices[0].title, "Reduced");
        assert_eq!(
            choices[1].explanation,
            "As taken, with the photo's details such as its location."
        );
        assert_eq!(default_media_quality(), MediaQuality::Reduced);
    }
}
