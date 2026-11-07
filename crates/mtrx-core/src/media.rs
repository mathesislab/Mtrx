//! Media: avatar pictures, and files in messages (images, videos, ...).

use std::{
    hash::{Hash, Hasher},
    io::Cursor,
};

use image::{DynamicImage, ImageFormat, ImageReader};
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

use crate::{Error, Result, Session};

/// Images bigger than this (in pixels) are sent with a thumbnail of this size.
const THUMBNAIL_SIZE: (u32, u32) = (800, 600);

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

/// How to send `data` of type `mimetype`: its size, and for images their
/// dimensions and, if big, a thumbnail. Decodes images, so slow for big ones.
pub(crate) fn attachment_config(mimetype: &Mime, data: &[u8]) -> AttachmentConfig {
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
            size,
            ..Default::default()
        }),
        mime::AUDIO => AttachmentInfo::Audio(BaseAudioInfo {
            size,
            ..Default::default()
        }),
        _ => AttachmentInfo::File(BaseFileInfo { size }),
    };
    let thumbnail = (mimetype.type_() == mime::IMAGE)
        .then(|| thumbnail(data))
        .flatten();
    AttachmentConfig::new().info(info).thumbnail(thumbnail)
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
        let config = attachment_config(&mime::IMAGE_PNG, &data);
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
        let config = attachment_config(&mime::IMAGE_PNG, &png(1600, 900, false));
        let thumbnail = config.thumbnail.expect("thumbnail");
        assert_eq!(thumbnail.content_type, mime::IMAGE_JPEG);
        assert_eq!(
            (thumbnail.width, thumbnail.height),
            (UInt::from(800u32), UInt::from(450u32))
        );
        let decoded = image::load_from_memory(&thumbnail.data).unwrap();
        assert_eq!((decoded.width(), decoded.height()), (800, 450));

        let transparent = attachment_config(&mime::IMAGE_PNG, &png(900, 900, true));
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

    #[test]
    fn other_files_have_their_size() {
        let mimetype = guess_mimetype("notes.txt");
        assert_eq!(mimetype, mime::TEXT_PLAIN);
        let config = attachment_config(&mimetype, b"hello");
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
