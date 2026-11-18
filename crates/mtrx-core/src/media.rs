//! Media: avatar pictures, and files in messages (images, videos, ...);
//! photos and videos made smaller for sending (`docs/reducing-media.md`).

use std::{
    fmt,
    hash::{Hash, Hasher},
    io::Cursor,
    path::Path,
    time::Duration,
};

use image::{
    DynamicImage, ExtendedColorType, ImageDecoder, ImageEncoder, ImageFormat, ImageReader,
    ImageResult, Rgb, RgbImage,
    codecs::jpeg::JpegEncoder,
    imageops::{self, FilterType},
};
use matrix_sdk::{
    attachment::{
        AttachmentConfig, AttachmentInfo, BaseAudioInfo, BaseFileInfo, BaseImageInfo,
        BaseVideoInfo, Thumbnail,
    },
    media::{MediaFormat, MediaRequestParameters, MediaThumbnailSettings},
    ruma::{MxcUri, OwnedMxcUri, UInt, events::room::MediaSource, media::Method},
};
use mime_guess::mime::{self, Mime};
use serde::{Deserialize, Serialize};

use crate::{Attachment, Error, Result, Session, duration_words, timeline};

/// Images bigger than this (in pixels) are sent with a thumbnail of this size.
const THUMBNAIL_SIZE: (u32, u32) = (800, 600);

/// How photos and videos are sent: a setting of each device, since data
/// plans differ.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum MediaQuality {
    /// Photos at most 1 MB and 2048 px ([`reduce_photo`]); videos at 720p,
    /// sized to their length ([`video_plan`]).
    Reduced,
    /// As taken (on iOS still JPEG and H.264, so every app shows them).
    Original,
}

impl MediaQuality {
    /// Both, in the order offered.
    pub const ALL: [Self; 2] = [Self::Reduced, Self::Original];
    /// The phones': their data plans are limited.
    pub const PHONE_DEFAULT: Self = Self::Reduced;
    /// The desktop's, which reduces photos only.
    pub const DESKTOP_DEFAULT: Self = Self::Original;

    pub fn title(self) -> &'static str {
        match self {
            Self::Reduced => "Reduced",
            Self::Original => "Original",
        }
    }

    /// What it does, below the phones' setting "Photos and videos".
    pub fn explanation(self) -> &'static str {
        match self {
            Self::Reduced => "Photos up to 1 MB, videos at 720p. Location data is left out.",
            Self::Original => "As taken, with the photo's details such as its location.",
        }
    }

    /// What it does, below the desktop's setting "Photos": the desktop
    /// sends videos as they are.
    pub fn photo_explanation(self) -> &'static str {
        match self {
            Self::Reduced => "Photos up to 1 MB. Location data is left out.",
            Self::Original => Self::Original.explanation(),
        }
    }
}

impl fmt::Display for MediaQuality {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.title())
    }
}

/// A reduced photo is at most this many pixels on its longer side...
pub const PHOTO_MAX_EDGE: u32 = 2048;
/// ...and at most this many bytes.
pub const PHOTO_MAX_BYTES: usize = 1_000_000;
/// The JPEG qualities tried in turn at each size.
const PHOTO_QUALITIES: [u8; 3] = [80, 70, 60];

/// A photo made small enough to send: at most [`PHOTO_MAX_EDGE`] px on its
/// longer side and [`PHOTO_MAX_BYTES`], as JPEG, turned upright, without
/// its metadata (its colour profile is kept). JPEG, PNG and WebP are
/// reduced; GIFs, other files, and photos already within both limits and
/// without metadata are unchanged.
///
/// Shrunk to the edge, then encoded at quality 80, 70, 60, until it fits;
/// still too large, shrunk by a quarter and tried again. Its name keeps
/// its stem: `IMG_1234.PNG` becomes `IMG_1234.jpg`. Slow for big photos:
/// call it off the async threads.
pub fn reduce_photo(file: Attachment) -> Result<Attachment> {
    let format = ImageReader::new(Cursor::new(&file.bytes))
        .with_guessed_format()?
        .format();
    let readable = matches!(
        format,
        Some(ImageFormat::Jpeg | ImageFormat::Png | ImageFormat::WebP)
    );
    if !file.mimetype.starts_with("image/") || !readable {
        return Ok(file);
    }
    let reduced = reduced_jpeg(&file.bytes).map_err(|e| Error::PhotoNotReduced {
        filename: file.filename.clone(),
        reason: e.to_string(),
    })?;
    Ok(match reduced {
        Some(bytes) => Attachment {
            filename: jpeg_name(&file.filename),
            mimetype: mime::IMAGE_JPEG.to_string(),
            bytes,
            video: None,
        },
        None => file,
    })
}

/// `photo` (JPEG, PNG or WebP) as a JPEG within the limits, upright; `None`
/// if it is within them already and has no metadata (EXIF, XMP).
fn reduced_jpeg(photo: &[u8]) -> ImageResult<Option<Vec<u8>>> {
    let mut decoder = ImageReader::new(Cursor::new(photo))
        .with_guessed_format()?
        .into_decoder()?;
    let (width, height) = decoder.dimensions();
    let metadata = decoder.exif_metadata()?.is_some() || decoder.xmp_metadata()?.is_some();
    if width.max(height) <= PHOTO_MAX_EDGE && photo.len() <= PHOTO_MAX_BYTES && !metadata {
        return Ok(None);
    }
    let orientation = decoder.orientation()?;
    let colours = decoder.icc_profile()?;
    let mut image = DynamicImage::from_decoder(decoder)?;
    image.apply_orientation(orientation);
    let image = on_white(image);
    let mut edge = PHOTO_MAX_EDGE;
    loop {
        let (width, height) = fit_within(image.width(), image.height(), edge);
        let resized = ((width, height) != image.dimensions())
            .then(|| imageops::resize(&image, width, height, FilterType::Triangle));
        let sized = resized.as_ref().unwrap_or(&image);
        for quality in PHOTO_QUALITIES {
            let jpeg = encode_jpeg(sized, quality, colours.as_deref())?;
            if jpeg.len() <= PHOTO_MAX_BYTES {
                return Ok(Some(jpeg));
            }
        }
        // A quarter smaller. A few hundred pixels always fit.
        edge = edge * 3 / 4;
    }
}

/// `image` in RGB, as JPEG takes it: transparent parts on white.
fn on_white(image: DynamicImage) -> RgbImage {
    if !image.color().has_alpha() {
        return image.into_rgb8();
    }
    let rgba = image.into_rgba8();
    RgbImage::from_fn(rgba.width(), rgba.height(), |x, y| {
        let [r, g, b, a] = rgba.get_pixel(x, y).0;
        let a = u16::from(a);
        let blend = |c: u8| ((u16::from(c) * a + 255 * (255 - a)) / 255) as u8;
        Rgb([blend(r), blend(g), blend(b)])
    })
}

fn encode_jpeg(image: &RgbImage, quality: u8, colours: Option<&[u8]>) -> ImageResult<Vec<u8>> {
    let mut bytes = Vec::new();
    let mut encoder = JpegEncoder::new_with_quality(&mut bytes, quality);
    if let Some(colours) = colours {
        // JPEG takes a colour profile; without it, wide-gamut photos (an
        // iPhone's) look dull.
        let _ = encoder.set_icc_profile(colours.to_vec());
    }
    encoder.write_image(
        image.as_raw(),
        image.width(),
        image.height(),
        ExtendedColorType::Rgb8,
    )?;
    Ok(bytes)
}

/// `width`×`height` scaled to at most `edge` on the longer side, if larger.
fn fit_within(width: u32, height: u32, edge: u32) -> (u32, u32) {
    let longer = width.max(height);
    if longer <= edge {
        return (width, height);
    }
    let scale = f64::from(edge) / f64::from(longer);
    let scaled = |length: u32| ((f64::from(length) * scale).round() as u32).max(1);
    (scaled(width), scaled(height))
}

/// `IMG_1234.HEIC` → `IMG_1234.jpg`.
fn jpeg_name(filename: &str) -> String {
    let stem = Path::new(filename)
        .file_stem()
        .and_then(|stem| stem.to_str())
        .filter(|stem| !stem.is_empty())
        .unwrap_or("Photo");
    format!("{stem}.jpg")
}

/// `files` with their photos reduced if `quality` says so, off the async
/// threads.
pub(crate) async fn reduce_photos(
    files: Vec<Attachment>,
    quality: MediaQuality,
) -> Result<Vec<Attachment>> {
    if quality == MediaQuality::Original {
        return Ok(files);
    }
    let mut reduced = Vec::with_capacity(files.len());
    for file in files {
        let file = tokio::task::spawn_blocking(move || reduce_photo(file))
            .await
            .map_err(|e| Error::Timeline(format!("reducing a photo: {e}")))??;
        reduced.push(file);
    }
    Ok(reduced)
}

/// A converted video is at most this many pixels on its longer side (720p)...
const VIDEO_MAX_EDGE: u32 = 1280;
/// ...at most this many frames a second...
const VIDEO_MAX_FPS: f32 = 30.0;
/// ...at this many bits per pixel and frame (720p30: about 1.9 Mbit/s)...
const VIDEO_BITS_PER_PIXEL: f64 = 0.07;
/// ...with its sound in AAC at this rate: about 15 MB a minute in all.
const AUDIO_BITRATE: u32 = 128_000;
/// Fitted to the server's limit, a video gets as small as this (480p)...
const VIDEO_FLOOR_EDGE: u32 = 854;
/// ...at this bit rate, and no lower.
const VIDEO_FLOOR_BITRATE: u32 = 500_000;
/// What the container (MP4) adds to the streams, at most.
const CONTAINER_SHARE: f64 = 0.05;
/// A video's thumbnail is at most this many pixels on its longer side.
pub const VIDEO_THUMBNAIL_EDGE: u32 = 800;

/// How a video is converted for sending: its size (as stored, like the
/// video it is made from), frame rate, and bit rates in bits a second.
/// Bits per pixel per frame make its size follow its length.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VideoPlan {
    pub width: u32,
    pub height: u32,
    pub fps: f32,
    pub video_bitrate: u32,
    pub audio_bitrate: u32,
}

impl VideoPlan {
    /// About how many bytes a video of `duration` takes, without its
    /// container.
    fn bytes(&self, duration: Duration) -> u64 {
        let rate = f64::from(self.video_bitrate) + f64::from(self.audio_bitrate);
        (rate * duration.as_secs_f64() / 8.0) as u64
    }
}

/// How to convert a video picked to send: `width`×`height` px as stored,
/// `fps` frames a second (0 if not known), `duration` long, `size` bytes.
/// `None` if it is within the plan already (size, frame rate and bytes):
/// it is sent as it is.
///
/// At most 1280 px on the longer side and 30 fps, at 0.07 bits per pixel
/// per frame, with 128 kbit/s AAC. When that won't fit `max_upload` (the
/// server's limit, less 5% for the container), the bit rate is lowered to
/// fit, at 854 px (480p) once it is below what that size takes, down to
/// 0.5 Mbit/s; below that, [`Error::VideoTooLong`].
pub fn video_plan(
    width: u32,
    height: u32,
    fps: f32,
    duration: Duration,
    size: u64,
    max_upload: Option<u64>,
) -> Result<Option<VideoPlan>> {
    let rate = if fps.is_finite() && fps > 0.0 {
        fps.min(VIDEO_MAX_FPS)
    } else {
        VIDEO_MAX_FPS
    };
    let (plan_width, plan_height) = video_size(width, height, VIDEO_MAX_EDGE);
    let mut plan = VideoPlan {
        width: plan_width,
        height: plan_height,
        fps: rate,
        video_bitrate: video_bitrate(plan_width, plan_height, rate),
        audio_bitrate: AUDIO_BITRATE,
    };
    let seconds = duration.as_secs_f64();
    if let Some(max) = max_upload
        && seconds > 0.0
    {
        let budget = max as f64 * (1.0 - CONTAINER_SHARE) * 8.0;
        let fitted = budget / seconds - f64::from(AUDIO_BITRATE);
        if fitted < f64::from(plan.video_bitrate) {
            if fitted < f64::from(VIDEO_FLOOR_BITRATE) {
                let longest = budget / f64::from(VIDEO_FLOOR_BITRATE + AUDIO_BITRATE);
                return Err(Error::VideoTooLong {
                    max,
                    longest: Duration::from_secs_f64(longest),
                });
            }
            let (small_width, small_height) = video_size(width, height, VIDEO_FLOOR_EDGE);
            if fitted < f64::from(video_bitrate(small_width, small_height, rate)) {
                (plan.width, plan.height) = (small_width, small_height);
            }
            plan.video_bitrate = fitted as u32;
        }
    }
    let within = (plan.width, plan.height) == (width, height)
        && fps <= plan.fps
        && size <= plan.bytes(duration);
    Ok((!within).then_some(plan))
}

/// The size a video is converted to: at most `edge` on the longer side,
/// in even numbers of pixels, as encoders take them.
fn video_size(width: u32, height: u32, edge: u32) -> (u32, u32) {
    let even = |side: u32| (side & !1).max(2);
    let (width, height) = fit_within(width, height, edge);
    (even(width), even(height))
}

fn video_bitrate(width: u32, height: u32, fps: f32) -> u32 {
    (VIDEO_BITS_PER_PIXEL * f64::from(width) * f64::from(height) * f64::from(fps)).round() as u32
}

/// About how long, for [`Error::VideoTooLong`]: whole minutes ("10
/// minutes"), or seconds below one.
pub(crate) fn length_words(length: Duration) -> String {
    let seconds = length.as_secs();
    let rounded = if seconds >= 60 {
        seconds / 60 * 60
    } else {
        seconds
    };
    duration_words(Duration::from_secs(rounded))
}

/// A video's details, sent with it so that apps show a still of it and
/// its length before it is opened.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VideoInfo {
    /// In pixels, as shown.
    pub width: u32,
    pub height: u32,
    pub duration: Duration,
    /// A JPEG of a frame near the start, at most [`VIDEO_THUMBNAIL_EDGE`]
    /// px.
    pub thumbnail: Option<Vec<u8>>,
}

/// Where a message's file and its thumbnail are. Two refer to the same media
/// if their files have the same URI. Serializable, for frontends that hand it
/// to another language (the mobile apps).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MediaRef {
    pub(crate) file: MediaSource,
    thumbnail: Option<MediaSource>,
}

impl MediaRef {
    pub(crate) fn new(file: &MediaSource, thumbnail: Option<&MediaSource>) -> Self {
        Self {
            file: file.clone(),
            thumbnail: thumbnail.cloned(),
        }
    }

    /// A plain file at `uri`, for tests of frontends.
    #[cfg(feature = "test-support")]
    pub fn plain(uri: &str) -> Self {
        Self::new(&MediaSource::Plain(uri.into()), None)
    }

    /// The file's URI: the same for the same media.
    pub fn uri(&self) -> &MxcUri {
        match &self.file {
            MediaSource::Plain(uri) => uri,
            MediaSource::Encrypted(file) => &file.url,
        }
    }
}

impl PartialEq for MediaRef {
    fn eq(&self, other: &Self) -> bool {
        self.uri() == other.uri()
    }
}

impl Eq for MediaRef {}

impl Hash for MediaRef {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.uri().hash(state);
    }
}

impl Session {
    /// A thumbnail of `mxc` cropped to about `size`×`size` pixels, as encoded
    /// image bytes (PNG, JPEG, ...). The server may return a bigger one.
    /// Downloads are cached in the store.
    pub async fn thumbnail(&self, mxc: &MxcUri, size: u32) -> Result<Vec<u8>> {
        let size = UInt::from(size);
        self.download(
            MediaSource::Plain(mxc.to_owned()),
            MediaFormat::Thumbnail(MediaThumbnailSettings::with_method(
                Method::Crop,
                size,
                size,
            )),
        )
        .await
    }

    /// The whole file of a message, decrypted. Downloads are cached in the
    /// store.
    pub async fn media_content(&self, media: &MediaRef) -> Result<Vec<u8>> {
        self.download(media.file.clone(), MediaFormat::File).await
    }

    /// The message's thumbnail if it has one, else its whole file. The server
    /// cannot make thumbnails of encrypted files, so senders include them.
    pub async fn media_preview(&self, media: &MediaRef) -> Result<Vec<u8>> {
        let source = media.thumbnail.as_ref().unwrap_or(&media.file);
        self.download(source.clone(), MediaFormat::File).await
    }

    async fn download(&self, source: MediaSource, format: MediaFormat) -> Result<Vec<u8>> {
        let request = MediaRequestParameters { source, format };
        Ok(self
            .client
            .media()
            .get_media_content(&request, true)
            .await?)
    }

    /// How to convert a video picked to send ([`video_plan`]), fitted to the
    /// server's upload limit: asked once per session; not fitted while it
    /// cannot be had (offline, say).
    pub async fn video_plan(
        &self,
        width: u32,
        height: u32,
        fps: f32,
        duration: Duration,
        size: u64,
    ) -> Result<Option<VideoPlan>> {
        let max_upload = timeline::upload_limit(&self.client).await;
        video_plan(width, height, fps, duration, size, max_upload)
    }

    /// Set our display name, shown instead of the user ID.
    pub async fn set_display_name(&self, name: &str) -> Result<()> {
        self.client.account().set_display_name(Some(name)).await?;
        Ok(())
    }

    /// Upload `data` (of MIME type `content_type`, e.g. `image/png`) and make
    /// it our avatar.
    pub async fn set_avatar(&self, content_type: &str, data: Vec<u8>) -> Result<OwnedMxcUri> {
        let content_type = content_type
            .parse()
            .map_err(|_| Error::InvalidMediaType(content_type.to_owned()))?;
        Ok(self
            .client
            .account()
            .upload_avatar(&content_type, data)
            .await?)
    }
}

/// The MIME type of a file, from its extension.
pub(crate) fn guess_mimetype(filename: &str) -> Mime {
    mime_guess::from_path(filename).first_or_octet_stream()
}

/// How to send `data` of type `mimetype`: its size; for images their
/// dimensions and, if big, a thumbnail; for videos what `video` says (its
/// dimensions, length and still). Decodes images, so slow for big ones.
pub(crate) fn attachment_config(
    mimetype: &Mime,
    data: &[u8],
    video: Option<&VideoInfo>,
) -> AttachmentConfig {
    let size = UInt::new(data.len() as u64);
    let info = match mimetype.type_() {
        mime::IMAGE => {
            let (width, height) = image_dimensions(data).unzip();
            AttachmentInfo::Image(BaseImageInfo {
                width: width.map(UInt::from),
                height: height.map(UInt::from),
                size,
                ..Default::default()
            })
        }
        mime::VIDEO => AttachmentInfo::Video(BaseVideoInfo {
            duration: video.map(|video| video.duration),
            width: video.map(|video| UInt::from(video.width)),
            height: video.map(|video| UInt::from(video.height)),
            size,
            ..Default::default()
        }),
        mime::AUDIO => AttachmentInfo::Audio(BaseAudioInfo {
            size,
            ..Default::default()
        }),
        _ => AttachmentInfo::File(BaseFileInfo { size }),
    };
    let thumbnail = match mimetype.type_() {
        mime::IMAGE => thumbnail(data),
        mime::VIDEO => video
            .and_then(|video| video.thumbnail.as_deref())
            .and_then(still),
        _ => None,
    };
    AttachmentConfig::new().info(info).thumbnail(thumbnail)
}

/// A video's still (a JPEG the app made) as its thumbnail.
fn still(jpeg: &[u8]) -> Option<Thumbnail> {
    let (width, height) = image_dimensions(jpeg)?;
    Some(Thumbnail {
        content_type: mime::IMAGE_JPEG,
        width: width.into(),
        height: height.into(),
        size: UInt::new(jpeg.len() as u64)?,
        data: jpeg.to_vec(),
    })
}

/// Width and height in pixels, from the image's header.
fn image_dimensions(data: &[u8]) -> Option<(u32, u32)> {
    ImageReader::new(Cursor::new(data))
        .with_guessed_format()
        .ok()?
        .into_dimensions()
        .ok()
}

/// A thumbnail of an image bigger than [`THUMBNAIL_SIZE`]: PNG if it has
/// transparency, else JPEG.
fn thumbnail(data: &[u8]) -> Option<Thumbnail> {
    let (width, height) = image_dimensions(data)?;
    if width <= THUMBNAIL_SIZE.0 && height <= THUMBNAIL_SIZE.1 {
        return None;
    }
    let small = image::load_from_memory(data)
        .ok()?
        .thumbnail(THUMBNAIL_SIZE.0, THUMBNAIL_SIZE.1);
    let (small, format, content_type) = if small.color().has_alpha() {
        (small, ImageFormat::Png, mime::IMAGE_PNG)
    } else {
        // The JPEG encoder takes 8-bit RGB only.
        let rgb = DynamicImage::ImageRgb8(small.to_rgb8());
        (rgb, ImageFormat::Jpeg, mime::IMAGE_JPEG)
    };
    let mut bytes = Vec::new();
    small.write_to(&mut Cursor::new(&mut bytes), format).ok()?;
    Some(Thumbnail {
        content_type,
        width: small.width().into(),
        height: small.height().into(),
        size: UInt::new(bytes.len() as u64)?,
        data: bytes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png(width: u32, height: u32, alpha: bool) -> Vec<u8> {
        let image = image::RgbaImage::from_fn(width, height, |x, _| {
            image::Rgba([(x % 256) as u8, 90, 200, if alpha { 128 } else { 255 }])
        });
        let image = if alpha {
            DynamicImage::ImageRgba8(image)
        } else {
            DynamicImage::ImageRgb8(DynamicImage::ImageRgba8(image).to_rgb8())
        };
        let mut bytes = Vec::new();
        image
            .write_to(&mut Cursor::new(&mut bytes), ImageFormat::Png)
            .unwrap();
        bytes
    }

    #[test]
    fn small_image_has_dimensions_and_no_thumbnail() {
        let data = png(64, 40, false);
        let config = attachment_config(&mime::IMAGE_PNG, &data, None);
        let Some(AttachmentInfo::Image(info)) = config.info else {
            panic!("no image info");
        };
        assert_eq!(info.width, Some(UInt::from(64u32)));
        assert_eq!(info.height, Some(UInt::from(40u32)));
        assert_eq!(info.size, UInt::new(data.len() as u64));
        assert!(config.thumbnail.is_none());
    }

    #[test]
    fn big_image_gets_a_thumbnail() {
        let config = attachment_config(&mime::IMAGE_PNG, &png(1600, 900, false), None);
        let thumbnail = config.thumbnail.expect("thumbnail");
        assert_eq!(thumbnail.content_type, mime::IMAGE_JPEG);
        assert_eq!(
            (thumbnail.width, thumbnail.height),
            (UInt::from(800u32), UInt::from(450u32))
        );
        let decoded = image::load_from_memory(&thumbnail.data).unwrap();
        assert_eq!((decoded.width(), decoded.height()), (800, 450));

        let transparent = attachment_config(&mime::IMAGE_PNG, &png(900, 900, true), None);
        assert_eq!(transparent.thumbnail.unwrap().content_type, mime::IMAGE_PNG);
    }

    #[test]
    fn media_refs_round_trip_through_json() {
        let file: MediaSource = serde_json::from_value(serde_json::json!({
            "file": {
                "url": "mxc://example.org/abc",
                "key": {
                    "kty": "oct",
                    "key_ops": ["encrypt", "decrypt"],
                    "alg": "A256CTR",
                    "k": "qcHVMSgYg-71CauWBezXI5qkaRb0LuIy-Wx5kIaHMIA",
                    "ext": true
                },
                "iv": "X85+XgHN+HEAAAAAAAAAAA",
                "hashes": {"sha256": "5qG4fFnbbVdlAB1Q72JDKwCagV6Dbkx9uds4rSak37c"},
                "v": "v2"
            }
        }))
        .unwrap();
        let thumbnail = MediaSource::Plain("mxc://example.org/thumb".into());
        let media = MediaRef::new(&file, Some(&thumbnail));
        let json = serde_json::to_string(&media).unwrap();
        let back: MediaRef = serde_json::from_str(&json).unwrap();
        assert_eq!(back.uri().as_str(), "mxc://example.org/abc");
        assert_eq!(serde_json::to_string(&back).unwrap(), json);
    }

    /// A JPEG of `width`×`height`: a red block at the top left, the rest a
    /// gradient with fine noise (which JPEG keeps large); with an EXIF
    /// `orientation`, if given.
    fn photo(width: u32, height: u32, orientation: Option<u8>) -> Vec<u8> {
        let image = RgbImage::from_fn(width, height, |x, y| {
            if x < width / 5 && y < height / 5 {
                return Rgb([220, 30, 30]);
            }
            let noise = (x.wrapping_mul(7919) ^ y.wrapping_mul(104_729)) % 64;
            Rgb([
                (x * 190 / width + noise) as u8,
                (y * 190 / height + noise) as u8,
                (noise * 3) as u8,
            ])
        });
        let mut bytes = Vec::new();
        let mut encoder = JpegEncoder::new_with_quality(&mut bytes, 95);
        if let Some(orientation) = orientation {
            encoder
                .set_exif_metadata(exif_orientation(orientation))
                .unwrap();
        }
        encoder
            .write_image(image.as_raw(), width, height, ExtendedColorType::Rgb8)
            .unwrap();
        bytes
    }

    /// An EXIF chunk (TIFF, big-endian) with only an orientation.
    fn exif_orientation(orientation: u8) -> Vec<u8> {
        // The header, its one directory at 8.
        let mut exif = b"MM\0\x2a\0\0\0\x08".to_vec();
        // One entry: the orientation (0x0112), a SHORT, one of them.
        exif.extend_from_slice(&[0, 1, 0x01, 0x12, 0, 3, 0, 0, 0, 1, 0, orientation, 0, 0]);
        // No next directory.
        exif.extend_from_slice(&[0, 0, 0, 0]);
        exif
    }

    fn exif_of(jpeg: &[u8]) -> Option<Vec<u8>> {
        ImageReader::new(Cursor::new(jpeg))
            .with_guessed_format()
            .unwrap()
            .into_decoder()
            .unwrap()
            .exif_metadata()
            .unwrap()
    }

    #[test]
    fn a_large_photo_is_reduced_upright_without_metadata() {
        // Stored landscape, shown portrait: turned 90° clockwise.
        let taken = photo(3000, 2000, Some(6));
        assert!(exif_of(&taken).is_some());
        let original = Attachment {
            filename: "IMG_1234.JPEG".into(),
            mimetype: "image/jpeg".into(),
            bytes: taken,
            video: None,
        };
        let reduced = reduce_photo(original).unwrap();
        assert_eq!(reduced.filename, "IMG_1234.jpg");
        assert_eq!(reduced.mimetype, "image/jpeg");
        assert!(reduced.bytes.len() <= PHOTO_MAX_BYTES);
        assert_eq!(exif_of(&reduced.bytes), None);
        let image = image::load_from_memory(&reduced.bytes).unwrap().into_rgb8();
        let (width, height) = image.dimensions();
        assert!(
            width < height && height <= PHOTO_MAX_EDGE,
            "{width}×{height}"
        );
        // The red block, at the top left as stored, is at the top right
        // once upright.
        let red = |x: u32, y: u32| {
            let [r, g, _] = image.get_pixel(x, y).0;
            r > 180 && g < 80
        };
        assert!(red(width - 10, 10));
        assert!(!red(10, 10));
    }

    #[test]
    fn small_photos_gifs_and_other_files_are_unchanged() {
        let small = Attachment {
            filename: "small.jpg".into(),
            mimetype: "image/jpeg".into(),
            bytes: photo(640, 480, None),
            video: None,
        };
        assert_eq!(reduce_photo(small.clone()).unwrap(), small);
        // With metadata, which may say where it was taken: without it.
        let located = Attachment {
            bytes: photo(640, 480, Some(1)),
            ..small.clone()
        };
        let reduced = reduce_photo(located).unwrap();
        assert_eq!(exif_of(&reduced.bytes), None);
        assert_eq!(image_dimensions(&reduced.bytes), Some((640, 480)));

        let mut wide = Vec::new();
        DynamicImage::ImageRgb8(RgbImage::from_pixel(2100, 10, Rgb([10, 20, 30])))
            .write_to(&mut Cursor::new(&mut wide), ImageFormat::Gif)
            .unwrap();
        let gif = Attachment {
            filename: "wide.gif".into(),
            mimetype: "image/gif".into(),
            bytes: wide,
            video: None,
        };
        assert_eq!(reduce_photo(gif.clone()).unwrap(), gif);
        let notes = Attachment {
            filename: "notes.txt".into(),
            mimetype: "text/plain".into(),
            bytes: b"hello".to_vec(),
            video: None,
        };
        assert_eq!(reduce_photo(notes.clone()).unwrap(), notes);
    }

    const MB: u64 = 1_000_000;

    #[test]
    fn videos_are_planned_at_720p_and_30_fps() {
        let minute = Duration::from_secs(60);
        // 4K at 60 fps: 720p at 30.
        let plan = video_plan(3840, 2160, 60.0, minute, 400 * MB, Some(100 * MB))
            .unwrap()
            .unwrap();
        assert_eq!(
            plan,
            VideoPlan {
                width: 1280,
                height: 720,
                fps: 30.0,
                video_bitrate: 1_935_360,
                audio_bitrate: 128_000,
            }
        );
        // About 15 MB a minute.
        assert_eq!(plan.bytes(minute) / MB, 15);
        // Portrait, as stored.
        let portrait = video_plan(1080, 1920, 30.0, minute, 100 * MB, None)
            .unwrap()
            .unwrap();
        assert_eq!((portrait.width, portrait.height), (720, 1280));
        // 720p at 30 fps but more bytes than planned: converted at the
        // plan's rate.
        let large = video_plan(1280, 720, 30.0, minute, 60 * MB, None)
            .unwrap()
            .unwrap();
        assert_eq!(
            (large.width, large.height, large.video_bitrate),
            (1280, 720, 1_935_360)
        );
        // Within the plan: sent as it is.
        assert_eq!(
            video_plan(1280, 720, 30.0, minute, 10 * MB, None).unwrap(),
            None
        );
        // 480p keeps its size.
        let small = video_plan(854, 480, 30.0, minute, 60 * MB, None)
            .unwrap()
            .unwrap();
        assert_eq!(
            (small.width, small.height, small.video_bitrate),
            (854, 480, 860_832)
        );
    }

    #[test]
    fn long_videos_are_fitted_to_the_upload_limit() {
        let limit = Some(50 * MB);
        let fits = |plan: &VideoPlan, length| plan.bytes(length) <= 50 * MB * 95 / 100;
        // Six minutes: 720p at a lower rate.
        let six = Duration::from_secs(6 * 60);
        let plan = video_plan(1920, 1080, 30.0, six, 500 * MB, limit)
            .unwrap()
            .unwrap();
        assert_eq!((plan.width, plan.height), (1280, 720));
        assert!(plan.video_bitrate < 1_935_360 && fits(&plan, six));
        // Nine: 480p.
        let nine = Duration::from_secs(9 * 60);
        let plan = video_plan(1920, 1080, 30.0, nine, 500 * MB, limit)
            .unwrap()
            .unwrap();
        assert_eq!((plan.width, plan.height), (854, 480));
        assert!(plan.video_bitrate >= 500_000 && fits(&plan, nine));
        // Eleven: too long even at the lowest rate.
        let eleven = Duration::from_secs(11 * 60);
        let too_long = video_plan(1920, 1080, 30.0, eleven, 500 * MB, limit).unwrap_err();
        assert_eq!(
            too_long.to_string(),
            "Too long to send: the server takes at most 50.0 MB (about 10 minutes)"
        );
    }

    #[test]
    fn media_quality_words() {
        assert_eq!(
            MediaQuality::ALL.map(MediaQuality::title),
            ["Reduced", "Original"]
        );
        assert_eq!(
            MediaQuality::Reduced.explanation(),
            "Photos up to 1 MB, videos at 720p. Location data is left out."
        );
        assert_eq!(
            MediaQuality::Original.explanation(),
            "As taken, with the photo's details such as its location."
        );
        assert_eq!(
            MediaQuality::Reduced.photo_explanation(),
            "Photos up to 1 MB. Location data is left out."
        );
        assert_eq!(MediaQuality::PHONE_DEFAULT, MediaQuality::Reduced);
        assert_eq!(MediaQuality::DESKTOP_DEFAULT, MediaQuality::Original);
    }

    #[test]
    fn videos_carry_their_length_size_and_still() {
        let still = photo(800, 450, None);
        let video = VideoInfo {
            width: 1280,
            height: 720,
            duration: Duration::from_millis(12_500),
            thumbnail: Some(still.clone()),
        };
        let mp4: Mime = "video/mp4".parse().unwrap();
        let config = attachment_config(&mp4, b"video", Some(&video));
        let Some(AttachmentInfo::Video(info)) = config.info else {
            panic!("no video info");
        };
        assert_eq!(info.duration, Some(Duration::from_millis(12_500)));
        assert_eq!(info.width, Some(UInt::from(1280u32)));
        assert_eq!(info.height, Some(UInt::from(720u32)));
        let thumbnail = config.thumbnail.expect("thumbnail");
        assert_eq!(thumbnail.content_type, mime::IMAGE_JPEG);
        assert_eq!(
            (thumbnail.width, thumbnail.height),
            (UInt::from(800u32), UInt::from(450u32))
        );
        assert_eq!(thumbnail.data, still);
    }

    #[test]
    fn other_files_have_their_size() {
        let mimetype = guess_mimetype("notes.txt");
        assert_eq!(mimetype, mime::TEXT_PLAIN);
        let config = attachment_config(&mimetype, b"hello", None);
        assert!(matches!(
            config.info,
            Some(AttachmentInfo::File(BaseFileInfo { size })) if size == UInt::new(5)
        ));
        assert_eq!(
            guess_mimetype("data.unknownext"),
            mime::APPLICATION_OCTET_STREAM
        );
    }
}
