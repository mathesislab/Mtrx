//! Files in messages: images inline, with previews downloaded in the
//! background and cached; cards for other files, opening PDFs once checked
//! that they can be read; the image viewer; saving and sending files.

use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};

use iced::{
    Alignment, Element, Length, Padding, Size, Subscription, Task, Theme, event, mouse,
    widget::{
        button, column, container, image, mouse_area, opaque, responsive, row, scrollable, text,
    },
    window,
};
use mtrx_core::{Attachment, Media, MediaKind, MediaRef, OwnedRoomId, Session};
use tokio::sync::Semaphore;

use crate::{
    app::Message,
    clipboard,
    context_menu::context_menu,
    ellipsis::ellipsis,
    fonts, frame,
    hint::{Position, hint},
    icons, message_menu, pdf, room, style,
};

/// Where saving or copying a file was asked from, to show there how it
/// went.
#[derive(Debug, Clone, PartialEq)]
pub enum Origin {
    /// The image or PDF viewer.
    Viewer,
    /// A message's file in the room.
    Room(OwnedRoomId),
}

/// Largest size of an image in the timeline, in logical pixels.
const MAX_INLINE: Size = Size::new(360.0, 300.0);
/// An image's size in the timeline while its dimensions are unknown.
const PLACEHOLDER: Size = Size::new(240.0, 160.0);
/// Longest side of an image in the viewer, in pixels (GPUs limit texture
/// sizes).
const MAX_FULL: u32 = 4096;
/// Space between the viewer and the window's edges.
const VIEWER_MARGIN: f32 = 32.0;
/// Height of the viewer's bar with the filename and buttons.
const VIEWER_BAR: f32 = 44.0;
/// The viewer's bar is at least this wide, even below a narrow image.
const VIEWER_BAR_MIN_WIDTH: f32 = 360.0;
/// A failed download is tried again only after this long.
const RETRY_AFTER: Duration = Duration::from_secs(5 * 60);
/// Downloads running at once.
const PARALLEL_DOWNLOADS: usize = 3;

/// A decoded image.
#[derive(Clone, Debug)]
pub struct Picture {
    handle: image::Handle,
    /// Of the downloaded image, before downscaling.
    width: u32,
    height: u32,
}

/// What a file shows in the timeline.
#[derive(Clone, Debug)]
pub enum Preview {
    /// An image's preview.
    Picture(Picture),
    /// A PDF the viewer can show.
    Readable,
    /// A PDF the viewer cannot show.
    Unreadable,
}

enum Loaded {
    Ready(Preview),
    Failed(Instant),
}

/// Previews of the images in messages, and whether their PDFs can be read.
pub struct Previews {
    loaded: HashMap<MediaRef, Loaded>,
    pending: HashSet<MediaRef>,
    downloads: Arc<Semaphore>,
}

impl Previews {
    pub fn new() -> Self {
        Self {
            loaded: HashMap::new(),
            pending: HashSet::new(),
            downloads: Arc::new(Semaphore::new(PARALLEL_DOWNLOADS)),
        }
    }

    fn ready(&self, media: &MediaRef) -> Option<&Picture> {
        match self.loaded.get(media)? {
            Loaded::Ready(Preview::Picture(picture)) => Some(picture),
            _ => None,
        }
    }

    /// Whether a PDF can be read: `None` until checked.
    fn readable(&self, media: &MediaRef) -> Option<bool> {
        match self.loaded.get(media)? {
            Loaded::Ready(Preview::Readable) => Some(true),
            Loaded::Ready(Preview::Unreadable) => Some(false),
            _ => None,
        }
    }

    fn failed(&self, media: &MediaRef) -> bool {
        matches!(self.loaded.get(media), Some(Loaded::Failed(_)))
    }

    /// Start downloading unless it is cached, running, or failed recently:
    /// an image's preview, or a PDF to check it can be read.
    pub fn request<M: Send + 'static>(
        &mut self,
        session: &Arc<Session>,
        media: &Media,
        done: fn(MediaRef, Option<Preview>) -> M,
    ) -> Task<M> {
        let is_pdf = pdf::is_pdf(media);
        let media = &media.source;
        let wanted = match self.loaded.get(media) {
            Some(Loaded::Ready(_)) => false,
            Some(Loaded::Failed(at)) => at.elapsed() >= RETRY_AFTER,
            None => true,
        };
        if !wanted || !self.pending.insert(media.clone()) {
            return Task::none();
        }
        let (session, downloads, media) = (session.clone(), self.downloads.clone(), media.clone());
        Task::perform(
            async move {
                let picture = async {
                    // Never closed, so this only waits.
                    let _permit = downloads.acquire().await.ok()?;
                    if is_pdf {
                        // The download is cached for the viewer.
                        let bytes = session.media_content(&media).await.ok()?;
                        return tokio::task::spawn_blocking(move || check_pdf(bytes))
                            .await
                            .ok();
                    }
                    let bytes = session.media_preview(&media).await.ok()?;
                    tokio::task::spawn_blocking(move || decode_preview(&bytes))
                        .await
                        .ok()?
                        .map(Preview::Picture)
                }
                .await;
                (media, picture)
            },
            move |(media, picture)| done(media, picture),
        )
    }

    /// Store a download's result; `None` if it failed.
    pub fn insert(&mut self, media: MediaRef, preview: Option<Preview>) {
        self.pending.remove(&media);
        let loaded = match preview {
            Some(preview) => Loaded::Ready(preview),
            None => Loaded::Failed(Instant::now()),
        };
        self.loaded.insert(media, loaded);
    }
}

#[cfg(test)]
impl Previews {
    /// Download and decode, or check a PDF, right away (tests).
    pub async fn load_now(&mut self, session: &Session, media: &Media) {
        let preview = if pdf::is_pdf(media) {
            Some(check_pdf(
                session.media_content(&media.source).await.unwrap(),
            ))
        } else {
            let bytes = session.media_preview(&media.source).await.unwrap();
            decode_preview(&bytes).map(Preview::Picture)
        };
        assert!(preview.is_some(), "decoding failed");
        self.insert(media.source.clone(), preview);
    }
}

fn check_pdf(bytes: Vec<u8>) -> Preview {
    match pdf::Document::open(bytes) {
        Some(_) => Preview::Readable,
        None => Preview::Unreadable,
    }
}

/// Whether a message's file is looked at in the background: images for
/// their previews, and PDFs that are not too large to check.
pub fn previewed(media: &Media) -> bool {
    media.kind == MediaKind::Image
        || (pdf::is_pdf(media) && media.size.is_none_or(|size| size <= pdf::MAX_BYTES))
}

/// Decoded at up to twice the size shown, for HiDPI screens.
fn decode_preview(bytes: &[u8]) -> Option<Picture> {
    decode(
        bytes,
        (2.0 * MAX_INLINE.width) as u32,
        (2.0 * MAX_INLINE.height) as u32,
    )
}

/// Decode, shrunk to fit `max_width`×`max_height` pixels.
fn decode(bytes: &[u8], max_width: u32, max_height: u32) -> Option<Picture> {
    let picture = ::image::load_from_memory(bytes).ok()?;
    let (width, height) = (picture.width(), picture.height());
    let picture = if width > max_width || height > max_height {
        picture.resize(
            max_width,
            max_height,
            ::image::imageops::FilterType::Triangle,
        )
    } else {
        picture
    };
    let rgba = picture.into_rgba8();
    Some(Picture {
        handle: image::Handle::from_rgba(rgba.width(), rgba.height(), rgba.into_raw()),
        width,
        height,
    })
}

/// `width`×`height` shrunk to fit `bounds`, keeping its aspect ratio.
fn fit(width: u32, height: u32, bounds: Size) -> Size {
    let (width, height) = (width.max(1) as f32, height.max(1) as f32);
    let scale = (bounds.width / width).min(bounds.height / height).min(1.0);
    Size::new((width * scale).round(), (height * scale).round())
}

/// Pixel size of an image: from its message, else from the picture.
fn dimensions(media: &Media, picture: Option<&Picture>) -> Option<(u32, u32)> {
    media
        .width
        .zip(media.height)
        .or(picture.map(|p| (p.width, p.height)))
}

/// A message's file: an image (a placeholder until loaded), else a card;
/// a PDF's card opens the viewer once known to be readable. `body` shows
/// below it if it is a caption rather than the filename.
/// A message's file in `room`; an image has a download button beside it
/// while the message is `hovered`.
pub fn view<'a>(
    room: &OwnedRoomId,
    media: &'a Media,
    body: &'a str,
    previews: &Previews,
    hovered: bool,
) -> Element<'a, Message> {
    let from = Origin::Room(room.clone());
    let content = match media.kind {
        MediaKind::Image if !previews.failed(&media.source) => {
            let picture = inline_image(media, previews.ready(&media.source));
            if hovered {
                row![picture, save_button(media, from)]
                    .spacing(8)
                    .align_y(Alignment::End)
                    .into()
            } else {
                picture
            }
        }
        _ if pdf::is_pdf(media) => {
            if !previewed(media) {
                card(media, from, None, Some("Too large to preview here"))
            } else {
                match previews.readable(&media.source) {
                    Some(true) => card(media, from, Some(Message::OpenPdf(media.clone())), None),
                    Some(false) => card(
                        media,
                        from,
                        None,
                        Some("Preview not available for this document"),
                    ),
                    None => card(media, from, None, None),
                }
            }
        }
        _ => card(media, from, None, None),
    };
    if body == media.filename {
        content
    } else {
        column![content, text(body)].spacing(4).into()
    }
}

/// An image in the timeline; clicking it opens the viewer.
fn inline_image<'a>(media: &'a Media, picture: Option<&Picture>) -> Element<'a, Message> {
    let size = dimensions(media, picture).map_or(PLACEHOLDER, |(w, h)| fit(w, h, MAX_INLINE));
    let content: Element<'a, Message> = match picture {
        Some(picture) => image(picture.handle.clone())
            .width(size.width)
            .height(size.height)
            .border_radius(style::MEDIA_RADIUS)
            .into(),
        None => placeholder(size),
    };
    mouse_area(content)
        .on_press(Message::OpenImage(media.clone()))
        .interaction(mouse::Interaction::Pointer)
        .into()
}

fn placeholder<'a>(size: Size) -> Element<'a, Message> {
    container(text("Loading image…").size(12).style(text::secondary))
        .center_x(size.width)
        .center_y(size.height)
        .style(style::media_placeholder)
        .into()
}

/// A file: icon, name, size and type (and `note`), and a download
/// button; clicking it sends `open` if given.
fn card<'a>(
    media: &'a Media,
    from: Origin,
    open: Option<Message>,
    note: Option<&'a str>,
) -> Element<'a, Message> {
    let mut details = match media.size {
        Some(size) => format!("{} · {}", human_size(size), kind_label(media)),
        None => kind_label(media),
    };
    if let Some(note) = note {
        details = format!("{details} · {note}");
    }
    let content = row![
        container(icons::file(16.0, on_primary))
            .center(32)
            .style(style::call_icon),
        column![
            ellipsis(&media.filename).font(fonts::SEMIBOLD).size(14),
            text(details).size(12).style(text::secondary),
        ]
        .spacing(1)
        .width(Length::Fill),
        save_button(media, from),
    ]
    .spacing(10)
    .align_y(Alignment::Center);
    let card: Element<'a, Message> = match open {
        Some(open) => hint(
            button(content)
                .padding([8, 10])
                .width(Length::Fill)
                .style(style::card_button)
                .on_press(open),
            "Open in the viewer",
            Position::Top,
        ),
        None => container(content)
            .padding([8, 10])
            .style(style::card)
            .into(),
    };
    container(card).max_width(460).into()
}

/// Saves the file, saying how it went where asked `from`.
pub fn save_button<'a>(media: &Media, from: Origin) -> Element<'a, Message> {
    hint(
        button(icons::download(18.0, |theme| {
            theme
                .extended_palette()
                .background
                .base
                .text
                .scale_alpha(0.75)
        }))
        .padding([6, 8])
        .style(style::secondary_button)
        .on_press(Message::SaveMedia(media.clone(), from)),
        "Save to your computer",
        Position::Top,
    )
}

/// "Video", "Audio", or for other files their extension, e.g. "PDF".
fn kind_label(media: &Media) -> String {
    match media.kind {
        MediaKind::Image => "Image".into(),
        MediaKind::Video => "Video".into(),
        MediaKind::Audio => "Audio".into(),
        MediaKind::File => extension_label(Path::new(&media.filename)),
    }
}

/// [`kind_label`] of a file not sent yet.
fn file_label(name: &Path, mimetype: &str) -> String {
    match mimetype.split('/').next() {
        Some("image") => "Image".into(),
        Some("video") => "Video".into(),
        Some("audio") => "Audio".into(),
        _ => extension_label(name),
    }
}

/// The extension, e.g. "PDF", or "File".
fn extension_label(name: &Path) -> String {
    name.extension()
        .and_then(|e| e.to_str())
        .filter(|e| !e.is_empty() && e.len() <= 5)
        .map_or_else(|| "File".into(), str::to_uppercase)
}

/// A file size in decimal units, e.g. "2.4 MB".
fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["kB", "MB", "GB", "TB"];
    if bytes < 1000 {
        return format!("{bytes} B");
    }
    let mut value = bytes as f64 / 1000.0;
    let mut unit = 0;
    while value >= 999.95 && unit < UNITS.len() - 1 {
        value /= 1000.0;
        unit += 1;
    }
    if value < 99.95 {
        format!("{value:.1} {}", UNITS[unit])
    } else {
        format!("{value:.0} {}", UNITS[unit])
    }
}

fn on_primary(theme: &Theme) -> iced::Color {
    theme.extended_palette().primary.base.text
}

/// An image shown large over the window.
pub struct Viewer {
    pub media: Media,
    /// The whole image, once loaded; until then the preview shows.
    full: Option<Picture>,
    pub error: Option<String>,
    /// The image's menu is open (right-click).
    pub menu: bool,
}

impl Viewer {
    pub fn new(media: Media) -> Self {
        Self {
            media,
            full: None,
            error: None,
            menu: false,
        }
    }

    /// Download and decode the whole image.
    pub fn load(&self, session: Arc<Session>) -> Task<Message> {
        let media = self.media.source.clone();
        Task::perform(
            {
                let media = media.clone();
                async move {
                    let bytes = session
                        .media_content(&media)
                        .await
                        .map_err(|e| e.to_string())?;
                    tokio::task::spawn_blocking(move || decode(&bytes, MAX_FULL, MAX_FULL))
                        .await
                        .ok()
                        .flatten()
                        .ok_or_else(|| "not an image we can show".to_owned())
                }
            },
            move |result| Message::ImageLoaded(media.clone(), result),
        )
    }

    /// The whole image of `media` was loaded, or not.
    pub fn loaded(&mut self, media: &MediaRef, result: Result<Picture, String>) {
        if *media != self.media.source {
            return;
        }
        match result {
            Ok(picture) => self.full = Some(picture),
            Err(e) => self.error = Some(format!("Could not load the image: {e}")),
        }
    }

    /// The image as large as fits (at most its own size), with its name,
    /// Save and Close below. Clicking around it closes it.
    pub fn view<'a>(&'a self, previews: &'a Previews) -> Element<'a, Message> {
        let picture = self.full.as_ref().or(previews.ready(&self.media.source));
        let content = responsive(move |available| {
            let bounds = Size::new(
                available.width,
                (available.height - VIEWER_BAR - 8.0).max(0.0),
            );
            let size =
                dimensions(&self.media, picture).map_or(PLACEHOLDER, |(w, h)| fit(w, h, bounds));
            let shown: Element<'a, Message> = match picture {
                Some(picture) => image(picture.handle.clone())
                    .width(size.width)
                    .height(size.height)
                    .border_radius(style::MEDIA_RADIUS)
                    .into(),
                None => placeholder(size),
            };
            let menu = self.menu.then(|| {
                message_menu::view(message_menu::media_items(&self.media, Origin::Viewer))
            });
            let shown = context_menu(shown, menu, Message::ViewerMenu(true))
                .on_dismiss(Message::ViewerMenu(false));
            let bar_width = size.width.max(VIEWER_BAR_MIN_WIDTH).min(available.width);
            container(
                column![opaque(shown), opaque(self.bar(bar_width))]
                    .spacing(8)
                    .align_x(Alignment::Center),
            )
            .center(Length::Fill)
            .into()
        });
        mouse_area(
            container(content)
                .padding(VIEWER_MARGIN)
                .style(style::viewer_backdrop),
        )
        .on_press(Message::CloseImage)
        .into()
    }

    fn bar<'a>(&'a self, width: f32) -> Element<'a, Message> {
        let mut name = column![
            ellipsis(&self.media.filename)
                .font(fonts::SEMIBOLD)
                .size(14)
        ]
        .width(Length::Fill);
        if let Some(error) = &self.error {
            name = name.push(ellipsis(error).size(12).style(text::danger));
        }
        container(
            row![
                name,
                save_button(&self.media, Origin::Viewer),
                frame::close_button(Message::CloseImage),
            ]
            .spacing(8)
            .align_y(Alignment::Center),
        )
        .padding([0, 12])
        .center_y(VIEWER_BAR)
        .width(width)
        .style(style::dialog)
        .into()
    }
}

#[cfg(test)]
impl Viewer {
    /// Load the whole image right away (tests).
    pub async fn load_now(&mut self, session: &Session) {
        let bytes = session.media_content(&self.media.source).await.unwrap();
        self.full = decode(&bytes, MAX_FULL, MAX_FULL);
        assert!(self.full.is_some(), "decoding failed");
    }
}

/// Put a message's image on the clipboard.
pub async fn copy_image(session: Arc<Session>, media: Media) -> Result<(), String> {
    let bytes = session
        .media_content(&media.source)
        .await
        .map_err(|e| format!("Could not download {}: {e}", media.filename))?;
    tokio::task::spawn_blocking(move || {
        let picture = ::image::load_from_memory(&bytes)
            .map_err(|e| format!("Could not read {}: {e}", media.filename))?;
        clipboard::write_picture(picture.to_rgba8())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Ask where to save a message's file, then download it there. Cancelling
/// is fine.
pub async fn save(session: Arc<Session>, media: Media) -> Result<(), String> {
    let Some(file) = rfd::AsyncFileDialog::new()
        .set_file_name(&media.filename)
        .save_file()
        .await
    else {
        return Ok(());
    };
    let bytes = session
        .media_content(&media.source)
        .await
        .map_err(|e| format!("Could not download {}: {e}", media.filename))?;
    file.write(&bytes)
        .await
        .map_err(|e| format!("Could not save {}: {e}", file.file_name()))
}

/// A file attached to the message being written, sent after its text.
#[derive(Clone, Debug)]
pub struct Attached {
    pub file: Attachment,
    /// A small picture of an image.
    preview: Option<image::Handle>,
}

/// Height of an attached file's chip in the composer.
pub const CHIP_HEIGHT: f32 = 44.0;

/// Ask for files to attach.
pub async fn pick_files() -> Vec<PathBuf> {
    rfd::AsyncFileDialog::new()
        .set_title("Attach files")
        .pick_files()
        .await
        .unwrap_or_default()
        .iter()
        .map(|file| file.path().to_owned())
        .collect()
}

/// Read files to attach, in order.
pub async fn read_attached(paths: Vec<PathBuf>) -> Vec<Result<Attached, String>> {
    let mut attached = Vec::with_capacity(paths.len());
    for path in paths {
        attached.push(read_one(&path).await);
    }
    attached
}

async fn read_one(path: &Path) -> Result<Attached, String> {
    let file = Attachment::read(path)
        .await
        .map_err(|e| format!("Could not attach {}: {e}", path.display()))?;
    attached(file).await
}

/// `file` to attach, with a small picture of it if it is an image.
pub async fn attached(file: Attachment) -> Result<Attached, String> {
    if !file.mimetype.starts_with("image/") {
        return Ok(Attached {
            file,
            preview: None,
        });
    }
    let filename = file.filename.clone();
    let side = (2.0 * CHIP_HEIGHT) as u32;
    tokio::task::spawn_blocking(move || {
        let preview = decode(&file.bytes, side, side).map(|picture| picture.handle);
        Attached { file, preview }
    })
    .await
    .map_err(|e| format!("Could not attach {filename}: {e}"))
}

/// The attached files in a row, each with an × sending `remove` with its
/// index.
pub fn attached_view<'a>(
    attached: &'a [Attached],
    remove: impl Fn(usize) -> Message,
) -> Element<'a, Message> {
    let chips = attached.iter().enumerate().map(|(index, attached)| {
        let thumb = CHIP_HEIGHT - 12.0;
        let picture: Element<'a, Message> = match &attached.preview {
            Some(handle) => image(handle.clone())
                .width(thumb)
                .height(thumb)
                .content_fit(iced::ContentFit::Cover)
                .border_radius(4)
                .into(),
            None => container(icons::file(16.0, on_primary))
                .center(thumb)
                .style(style::call_icon)
                .into(),
        };
        let name = Path::new(&attached.file.filename);
        container(
            row![
                picture,
                container(
                    column![
                        ellipsis(&attached.file.filename).size(13),
                        text(format!(
                            "{} · {}",
                            human_size(attached.file.bytes.len() as u64),
                            file_label(name, &attached.file.mimetype)
                        ))
                        .size(11)
                        .style(text::secondary),
                    ]
                    .spacing(1),
                )
                .max_width(160),
                hint(
                    button(text("×").size(14))
                        .padding([0, 6])
                        .style(style::text_button)
                        .on_press(remove(index)),
                    "Remove",
                    Position::Top,
                ),
            ]
            .spacing(8)
            .align_y(Alignment::Center),
        )
        .height(CHIP_HEIGHT)
        .padding(Padding::new(6.0).right(4.0))
        .align_y(Alignment::Center)
        .style(style::card)
        .into()
    });
    scrollable(row(chips).spacing(6))
        .direction(scrollable::Direction::Horizontal(
            scrollable::Scrollbar::new().width(3).scroller_width(3),
        ))
        .into()
}

/// Files dropped onto the window, for the room shown.
pub fn dropped_files() -> Subscription<Message> {
    event::listen_with(|event, _status, _window| match event {
        iced::Event::Window(window::Event::FileDropped(path)) => {
            Some(Message::Shown(room::Message::FileDropped(path)))
        }
        _ => None,
    })
}

#[cfg(test)]
mod tests {
    use iced::advanced::{clipboard, renderer::Headless};
    use iced_runtime::user_interface::{Cache, UserInterface};
    use iced_test::simulator::{Simulator, click as left_click};

    use super::*;

    #[test]
    fn sizes() {
        assert_eq!(human_size(0), "0 B");
        assert_eq!(human_size(999), "999 B");
        assert_eq!(human_size(1000), "1.0 kB");
        assert_eq!(human_size(2_400_000), "2.4 MB");
        assert_eq!(human_size(99_960), "100 kB");
        assert_eq!(human_size(999_960), "1.0 MB");
        assert_eq!(human_size(345_000_000), "345 MB");
    }

    /// The messages sent by right-clicking `view` at `at`.
    fn right_click(view: Element<'_, Message>, at: (f32, f32)) -> Vec<Message> {
        let mut ui = Simulator::with_size(fonts::settings(), Size::new(800.0, 600.0), view);
        ui.point_at(at);
        let _ = ui.simulate([iced::Event::Mouse(mouse::Event::ButtonPressed(
            mouse::Button::Right,
        ))]);
        ui.into_messages().collect()
    }

    #[test]
    fn right_clicking_a_picture_offers_its_menu() {
        let media = Media {
            kind: MediaKind::Image,
            filename: "ice.png".into(),
            size: None,
            mimetype: None,
            width: Some(200),
            height: Some(100),
            source: MediaRef::plain("mxc://example.org/a"),
        };
        let previews = Previews::new();
        // In the timeline, the image leaves the click to its message's menu.
        let inline = context_menu(
            view(
                &"!room:example.org".try_into().unwrap(),
                &media,
                &media.filename,
                &previews,
                false,
            ),
            None,
            Message::CloseImage,
        );
        assert!(matches!(
            right_click(inline.into(), (50.0, 50.0))[..],
            [Message::CloseImage]
        ));
        // Shown large, it has a menu of its own, not around it.
        let mut viewer = Viewer::new(media.clone());
        assert!(right_click(viewer.view(&previews), (100.0, 100.0)).is_empty());
        let mut renderer = iced::futures::executor::block_on(iced::Renderer::new(
            fonts::INTER,
            iced::Pixels(style::TEXT_SIZE),
            None,
        ))
        .unwrap();
        let window = Size::new(800.0, 600.0);
        let mut ui = UserInterface::build(
            viewer.view(&previews),
            window,
            Cache::default(),
            &mut renderer,
        );
        let right = [iced::Event::Mouse(mouse::Event::ButtonPressed(
            mouse::Button::Right,
        ))];
        let sent = send(&mut ui, &mut renderer, &right, (400.0, 270.0));
        assert!(matches!(sent[..], [Message::ViewerMenu(true)]));
        // Its first item, just below where it was right-clicked.
        let cache = ui.into_cache();
        viewer.menu = true;
        let mut ui = UserInterface::build(viewer.view(&previews), window, cache, &mut renderer);
        let sent = send(
            &mut ui,
            &mut renderer,
            &left_click().collect::<Vec<_>>(),
            (420.0, 285.0),
        );
        assert!(
            matches!(&sent[..], [Message::CopyImage(copied, Origin::Viewer)] if *copied == media),
            "{sent:?}"
        );
    }

    /// The messages `events` at `at` send.
    fn send(
        ui: &mut UserInterface<'_, Message, Theme, iced::Renderer>,
        renderer: &mut iced::Renderer,
        events: &[iced::Event],
        at: (f32, f32),
    ) -> Vec<Message> {
        let mut sent = Vec::new();
        let _ = ui.update(
            events,
            mouse::Cursor::Available(at.into()),
            renderer,
            &mut clipboard::Null,
            &mut sent,
        );
        sent
    }

    #[test]
    fn fitting() {
        assert_eq!(fit(640, 400, MAX_INLINE), Size::new(360.0, 225.0));
        assert_eq!(fit(400, 800, MAX_INLINE), Size::new(150.0, 300.0));
        // Small images keep their size.
        assert_eq!(fit(48, 32, MAX_INLINE), Size::new(48.0, 32.0));
    }
}
