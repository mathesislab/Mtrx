//! The PDF viewer: all pages one below the other, scrolling, at a zoom
//! that starts fitting the width. Pages are drawn when they come near the
//! view, and again sharp after zooming.

use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

use iced::{
    Alignment, Element, Length, Padding, Size, Task,
    widget::{
        Id, button, column, container, image, mouse_area, opaque, row, scrollable,
        scrollable::Viewport, space, text,
    },
};
use mtrx_core::Session;

use crate::{
    app,
    downloads::File,
    ellipsis::ellipsis,
    fonts, frame,
    hint::{Position, hint, shortcut},
    media::download_button,
    pdf::Document,
    style,
};

/// The pages' scrollable.
const PAGES: &str = "pdf-pages";
/// Between the viewer and the window's edges.
const MARGIN: f32 = 24.0;
const BAR: f32 = 48.0;
/// Between the bar and the pages, and between pages.
const GAP: f32 = 12.0;
/// Around the pages inside the scrollable.
const PADDING: f32 = 16.0;
/// Zoom steps, as logical pixels per point.
const ZOOMS: [f32; 11] = [0.25, 0.33, 0.5, 0.67, 0.75, 1.0, 1.25, 1.5, 2.0, 3.0, 4.0];

#[derive(Debug, Clone, Copy)]
pub enum Zoom {
    In,
    Out,
    /// Fit the page width, following the window.
    Fit,
}

#[derive(Debug, Clone)]
pub enum Message {
    Opened(Result<Arc<Document>, String>),
    Scrolled(Viewport),
    Zoom(Zoom),
    /// Page `page` was drawn for zoom `key`, or could not be.
    Drawn(usize, u32, Option<image::Handle>),
}

/// A page as drawn, for the zoom it was drawn at; shown stretched while
/// another zoom is drawn.
struct Drawn {
    key: u32,
    handle: image::Handle,
}

pub struct Viewer {
    /// The PDF, and where it was sent.
    pub file: File,
    document: Option<Arc<Document>>,
    pub error: Option<String>,
    /// Logical pixels per point; `None` fits the width.
    zoom: Option<f32>,
    /// The pages' view and how far it is scrolled, logical pixels.
    area: Size,
    scrolled: f32,
    /// Display pixels per logical pixel.
    scale: f32,
    drawn: HashMap<usize, Drawn>,
    drawing: HashSet<(usize, u32)>,
}

/// A zoom as a key: hundredths.
fn key(zoom: f32) -> u32 {
    (zoom * 100.0).round() as u32
}

fn message(message: Message) -> app::Message {
    app::Message::Pdf(message)
}

impl Viewer {
    /// For a window of `window` logical pixels at `scale`.
    pub fn new(file: File, window: Size, scale: f32) -> Self {
        Self {
            file,
            document: None,
            error: None,
            zoom: None,
            area: area(window),
            scrolled: 0.0,
            scale,
            drawn: HashMap::new(),
            drawing: HashSet::new(),
        }
    }

    /// Download and read the PDF.
    pub fn open(&self, session: Arc<Session>) -> Task<app::Message> {
        let source = self.file.media.source.clone();
        Task::perform(
            async move {
                let bytes = session
                    .media_content(&source)
                    .await
                    .map_err(|e| e.to_string())?;
                tokio::task::spawn_blocking(move || Document::open(bytes))
                    .await
                    .ok()
                    .flatten()
                    .map(Arc::new)
                    .ok_or_else(|| "it is not a PDF we can read".to_owned())
            },
            |result| message(Message::Opened(result)),
        )
    }

    pub fn resized(&mut self, window: Size, scale: f32) -> Task<app::Message> {
        self.area = area(window);
        self.scale = scale;
        self.draw_visible()
    }

    pub fn update(&mut self, message: Message) -> Task<app::Message> {
        match message {
            Message::Opened(Ok(document)) => {
                self.document = Some(document);
                self.draw_visible()
            }
            Message::Opened(Err(e)) => {
                self.error = Some(format!("Could not open the PDF: {e}"));
                Task::none()
            }
            Message::Scrolled(viewport) => {
                self.area = viewport.bounds().size();
                self.scrolled = viewport.absolute_offset().y;
                self.draw_visible()
            }
            Message::Zoom(zoom) => self.zoom(zoom),
            Message::Drawn(page, key, handle) => {
                self.drawing.remove(&(page, key));
                // Keep a sharper drawing for the current zoom over an old one.
                if let Some(handle) = handle
                    && self.drawn.get(&page).is_none_or(|d| d.key != self.key())
                {
                    self.drawn.insert(page, Drawn { key, handle });
                }
                Task::none()
            }
        }
    }

    fn pages(&self) -> &[Size] {
        self.document.as_ref().map_or(&[], |d| &d.pages)
    }

    /// The zoom fitting the widest page into the view.
    fn fit(&self) -> f32 {
        let widest = self.pages().iter().map(|p| p.width).fold(1.0, f32::max);
        ((self.area.width - 2.0 * PADDING) / widest).clamp(ZOOMS[0], ZOOMS[ZOOMS.len() - 1])
    }

    fn current_zoom(&self) -> f32 {
        self.zoom.unwrap_or_else(|| self.fit())
    }

    fn key(&self) -> u32 {
        key(self.current_zoom())
    }

    /// Each page's top and height at the current zoom, in the scrolled
    /// content.
    fn layout(&self) -> Vec<(f32, f32)> {
        let zoom = self.current_zoom();
        let mut top = PADDING;
        self.pages()
            .iter()
            .map(|page| {
                let height = page.height * zoom;
                let span = (top, height);
                top += height + GAP;
                span
            })
            .collect()
    }

    /// The pages in or within one view of the view.
    fn near(&self) -> Vec<usize> {
        let (from, to) = (
            self.scrolled - self.area.height,
            self.scrolled + 2.0 * self.area.height,
        );
        self.layout()
            .into_iter()
            .enumerate()
            .filter(|(_, (top, height))| top + height >= from && *top <= to)
            .map(|(page, _)| page)
            .collect()
    }

    /// The page at the top of the view (the first mostly showing), from 0.
    fn current_page(&self) -> usize {
        let mark = self.scrolled + 2.0 * GAP;
        self.layout()
            .iter()
            .position(|(top, height)| top + height > mark)
            .unwrap_or(0)
    }

    /// Draw the pages near the view that are not drawn for this zoom, and
    /// forget those far away.
    fn draw_visible(&mut self) -> Task<app::Message> {
        let Some(document) = self.document.clone() else {
            return Task::none();
        };
        let near = self.near();
        self.drawn.retain(|page, _| near.contains(page));
        let (zoom, key) = (self.current_zoom(), self.key());
        let scale = zoom * self.scale;
        let mut tasks = Vec::new();
        for page in near {
            let current = self.drawn.get(&page).is_some_and(|d| d.key == key);
            if current || !self.drawing.insert((page, key)) {
                continue;
            }
            let document = document.clone();
            tasks.push(Task::perform(
                async move {
                    tokio::task::spawn_blocking(move || document.render(page, scale))
                        .await
                        .ok()
                        .flatten()
                        .map(|p| image::Handle::from_rgba(p.width, p.height, p.rgba))
                },
                move |handle| message(Message::Drawn(page, key, handle)),
            ));
        }
        Task::batch(tasks)
    }

    /// Zoom in or out a step, or fit the width, keeping the middle of the
    /// view in place.
    fn zoom(&mut self, zoom: Zoom) -> Task<app::Message> {
        if self.document.is_none() {
            return Task::none();
        }
        let old = self.current_zoom();
        self.zoom = match zoom {
            Zoom::In => Some(ZOOMS.into_iter().find(|z| *z > old + 0.005).unwrap_or(old)),
            Zoom::Out => Some(
                ZOOMS
                    .into_iter()
                    .rev()
                    .find(|z| *z < old - 0.005)
                    .unwrap_or(old),
            ),
            Zoom::Fit => None,
        };
        let new = self.current_zoom();
        let middle = self.scrolled + self.area.height / 2.0;
        self.scrolled =
            ((middle - PADDING) * new / old + PADDING - self.area.height / 2.0).max(0.0);
        let scroll = iced::widget::operation::scroll_to(
            PAGES,
            iced::widget::operation::AbsoluteOffset {
                x: None,
                y: Some(self.scrolled),
            },
        );
        Task::batch([scroll, self.draw_visible()])
    }

    pub fn view(&self) -> Element<'_, app::Message> {
        let content: Element<'_, app::Message> = match (&self.document, &self.error) {
            (_, Some(error)) => container(text(error).style(text::danger))
                .center(Length::Fill)
                .into(),
            (None, None) => container(text("Opening…").style(|_| text::Style {
                color: Some(iced::Color::WHITE.scale_alpha(0.8)),
            }))
            .center(Length::Fill)
            .into(),
            (Some(_), None) => self.pages_view(),
        };
        mouse_area(
            container(
                column![opaque(self.bar()), content]
                    .spacing(GAP)
                    .align_x(Alignment::Center),
            )
            .padding(MARGIN)
            .style(style::viewer_backdrop),
        )
        .on_press(app::Message::CloseImage)
        .into()
    }

    fn pages_view(&self) -> Element<'_, app::Message> {
        let zoom = self.current_zoom();
        let widest = self.pages().iter().map(|p| p.width).fold(0.0, f32::max) * zoom;
        let pages = self.pages().iter().enumerate().map(|(index, page)| {
            let size = Size::new(page.width * zoom, page.height * zoom);
            let content: Element<'_, app::Message> = match self.drawn.get(&index) {
                Some(drawn) => image(drawn.handle.clone())
                    .width(size.width)
                    .height(size.height)
                    .into(),
                None => space().width(size.width).height(size.height).into(),
            };
            container(content).style(style::pdf_page).into()
        });
        // Centred while narrower than the view; scrolls sideways if wider.
        let width = (widest + 2.0 * PADDING).max(self.area.width);
        let pages = container(
            column(pages)
                .spacing(GAP)
                .padding(PADDING)
                .align_x(Alignment::Center),
        )
        .center_x(width);
        opaque(
            scrollable(pages)
                .id(Id::new(PAGES))
                .direction(scrollable::Direction::Both {
                    vertical: scrollable::Scrollbar::default(),
                    horizontal: scrollable::Scrollbar::default(),
                })
                .on_scroll(|viewport| message(Message::Scrolled(viewport)))
                .width(Length::Fill)
                .height(Length::Fill),
        )
    }

    /// The name, page, zoom, download and Close.
    fn bar(&self) -> Element<'_, app::Message> {
        let mut bar = row![
            ellipsis(&self.file.media.filename)
                .font(fonts::SEMIBOLD)
                .size(14)
                .width(Length::Fill)
        ]
        .spacing(8)
        .align_y(Alignment::Center);
        if self.document.is_some() {
            let tool = |label: &'static str, tip: String, zoom: Zoom| {
                hint(
                    button(text(label).size(15).center())
                        .width(30)
                        .padding([2, 0])
                        .style(style::secondary_button)
                        .on_press(message(Message::Zoom(zoom))),
                    tip,
                    Position::Bottom,
                )
            };
            let fit = hint(
                button(text("Fit width").size(13))
                    .padding([4, 10])
                    .style(if self.zoom.is_none() {
                        style::primary_button
                    } else {
                        style::secondary_button
                    })
                    .on_press(message(Message::Zoom(Zoom::Fit))),
                format!("Fit the page width to the window ({})", shortcut("0")),
                Position::Bottom,
            );
            bar = bar
                .push(
                    text(format!(
                        "Page {} of {}",
                        self.current_page() + 1,
                        self.pages().len()
                    ))
                    .size(13)
                    .style(text::secondary),
                )
                .push(space().width(8))
                .push(tool(
                    "−",
                    format!("Zoom out ({})", shortcut("−")),
                    Zoom::Out,
                ))
                .push(text(format!("{}%", self.key())).size(13).width(44).center())
                .push(tool("+", format!("Zoom in ({})", shortcut("+")), Zoom::In))
                .push(fit);
        }
        container(
            bar.push(download_button(&self.file))
                .push(frame::close_button(app::Message::CloseImage)),
        )
        .padding(Padding::from([0, 12]))
        .center_y(BAR)
        .width(Length::Fill)
        .style(style::dialog)
        .into()
    }
}

/// The pages' view in a window of `window` logical pixels.
fn area(window: Size) -> Size {
    Size::new(
        (window.width - 2.0 * MARGIN).max(1.0),
        (window.height - 2.0 * MARGIN - BAR - GAP).max(1.0),
    )
}

#[cfg(test)]
impl Viewer {
    /// Read the PDF and draw the pages in view right away (tests).
    pub async fn open_now(&mut self, session: &Session) {
        let bytes = session
            .media_content(&self.file.media.source)
            .await
            .unwrap();
        let document = Arc::new(Document::open(bytes).expect("a PDF"));
        let scale = self.current_zoom_for(&document) * self.scale;
        for page in 0..document.pages.len() {
            let drawn = document.render(page, scale).unwrap();
            self.drawn.insert(
                page,
                Drawn {
                    key: 0,
                    handle: image::Handle::from_rgba(drawn.width, drawn.height, drawn.rgba),
                },
            );
        }
        self.document = Some(document);
    }

    fn current_zoom_for(&self, document: &Document) -> f32 {
        let widest = document.pages.iter().map(|p| p.width).fold(1.0, f32::max);
        self.zoom
            .unwrap_or(((self.area.width - 2.0 * PADDING) / widest).clamp(ZOOMS[0], ZOOMS[10]))
    }

    pub fn set_zoom(&mut self, zoom: f32) {
        self.zoom = Some(zoom);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pdf;
    use mtrx_core::{Media, MediaRef};

    fn viewer(pages: usize) -> Viewer {
        let media = Media {
            kind: mtrx_core::MediaKind::File,
            filename: "a.pdf".into(),
            size: None,
            mimetype: Some("application/pdf".into()),
            width: None,
            height: None,
            source: MediaRef::plain("mxc://example.org/a"),
        };
        let file = File {
            media,
            room: "!room:example.org".try_into().unwrap(),
            event: None,
        };
        let mut viewer = Viewer::new(file, Size::new(800.0, 700.0), 2.0);
        let mut document = Document::open(pdf::sample()).unwrap();
        document.pages = vec![Size::new(300.0, 400.0); pages];
        viewer.document = Some(Arc::new(document));
        viewer
    }

    #[test]
    fn fits_the_width_and_finds_pages_in_view() {
        let mut viewer = viewer(10);
        // 800 − 2·24 margin − 2·16 padding = 720 for 300 points: 2.4.
        assert_eq!(viewer.key(), 240);
        // Pages are 960 high; the view (700 − 48 − 48 − 12 = 592) shows the
        // first; one view more either way also counts.
        assert_eq!(viewer.near(), [0, 1]);
        viewer.scrolled = 3000.0;
        assert_eq!(viewer.current_page(), 3);
        assert_eq!(viewer.near(), [2, 3, 4]);
        let _ = viewer.zoom(Zoom::Out);
        assert_eq!(viewer.key(), 200);
        let _ = viewer.zoom(Zoom::In);
        assert_eq!(viewer.key(), 300);
        let _ = viewer.zoom(Zoom::Fit);
        assert_eq!(viewer.key(), 240);
    }
}
