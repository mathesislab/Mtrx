//! What shows over the whole window, above the card: Settings, a dialog,
//! joining, or an image or PDF shown large.

use mtrx_core::MediaRef;

use crate::{dialog, downloads, join, media, pdf_viewer, settings};

#[derive(Debug, Clone)]
pub enum Message {
    ShowSettings(bool),
    Settings(settings::Message),
    OpenDialog(dialog::Kind),
    Dialog(dialog::Message),
    /// Join a space or room from elsewhere.
    OpenJoin,
    Join(join::Message),
    /// Show an image large.
    OpenImage(downloads::File),
    CloseImage,
    /// Open the menu of the image in the viewer.
    OpenViewerMenu,
    /// The whole image for the viewer.
    ImageLoaded(MediaRef, Result<media::Picture, String>),
    /// Show a PDF large.
    OpenPdf(downloads::File),
    Pdf(pdf_viewer::Message),
    /// Fast ticks while something is in progress.
    Progress,
}
