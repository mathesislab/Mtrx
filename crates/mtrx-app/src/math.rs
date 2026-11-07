//! Messages containing math are typeset with Typst in the background and
//! cached as images.

use std::{
    collections::{HashMap, HashSet},
    hash::{DefaultHasher, Hash, Hasher},
    io::Cursor,
    sync::Arc,
};

use ::image::{DynamicImage, ImageFormat, RgbImage};
use iced::{Task, widget::image};
use mtrx_format::{Document, RenderStyle, Renderer};

use crate::{clipboard, fonts, message, selectable, style};

/// Identifies one rendering: same content, width and colours give the same image.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Key {
    content: u64,
    width: u32,
    scale_milli: u32,
    color: [u8; 4],
}

#[derive(Clone, Debug)]
pub enum Rendered {
    Image {
        handle: image::Handle,
        width: f32,
        height: f32,
        /// Where its text is, for selecting it.
        text: Arc<selectable::image::Text>,
    },
    /// Typst or LaTeX error; the message is shown natively instead.
    Failed,
}

/// A message's math to typeset for the room as it is laid out.
#[derive(Clone, Debug)]
pub struct Request {
    pub document: Document,
    pub params: Params,
}

/// Layout inputs that affect every rendering.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Params {
    pub max_width: f32,
    pub scale: f32,
    pub text: iced::Color,
    pub link: iced::Color,
    pub muted: iced::Color,
}

pub struct Math {
    renderer: Arc<Renderer>,
    images: HashMap<Key, Rendered>,
    pending: HashSet<Key>,
}

impl Math {
    pub fn new() -> Self {
        let fonts = fonts::FILES.iter().copied();
        Self {
            renderer: Arc::new(Renderer::new(fonts)),
            images: HashMap::new(),
            pending: HashSet::new(),
        }
    }

    pub fn key(document: &Document, params: &Params) -> Key {
        let mut hasher = DefaultHasher::new();
        document.hash(&mut hasher);
        Key {
            content: hasher.finish(),
            // Re-render in steps while the window is resized.
            width: (params.max_width / 40.0).floor() as u32 * 40,
            scale_milli: (params.scale * 1000.0) as u32,
            color: style::to_rgba8(params.text),
        }
    }

    pub fn get(&self, key: &Key) -> Option<&Rendered> {
        self.images.get(key)
    }

    /// Whether to render it: not rendered (nor failed), nor rendering.
    pub fn wanted(&self, key: &Key) -> bool {
        !self.images.contains_key(key) && !self.pending.contains(key)
    }

    /// Start rendering if [wanted](Self::wanted).
    pub fn request<M: Send + 'static>(
        &mut self,
        request: Request,
        done: fn(Key, Rendered) -> M,
    ) -> Task<M> {
        let key = Self::key(&request.document, &request.params);
        if !self.wanted(&key) {
            return Task::none();
        }
        self.pending.insert(key);
        let renderer = self.renderer.clone();
        let Request { document, params } = request;
        Task::perform(
            async move {
                tokio::task::spawn_blocking(move || render(&renderer, &document, key, &params))
                    .await
                    .unwrap_or(Rendered::Failed)
            },
            move |rendered| done(key, rendered),
        )
    }

    pub fn insert(&mut self, key: Key, rendered: Rendered) {
        self.pending.remove(&key);
        self.images.insert(key, rendered);
    }

    /// The message typeset as a PNG file: dark text on white, whatever the
    /// theme, and sharper than on screen.
    pub fn png(
        &self,
        document: &Document,
    ) -> impl Future<Output = Result<Vec<u8>, String>> + use<> {
        let renderer = self.renderer.clone();
        let document = document.clone();
        blocking(move || {
            let mut png = Vec::new();
            picture(&renderer, &document)?
                .write_to(&mut Cursor::new(&mut png), ImageFormat::Png)
                .map_err(|e| e.to_string())?;
            Ok(png)
        })
    }

    /// Put the message, typeset as for [`Math::png`], on the clipboard.
    pub fn copy_image(
        &self,
        document: &Document,
    ) -> impl Future<Output = Result<(), String>> + use<> {
        let renderer = self.renderer.clone();
        let document = document.clone();
        blocking(move || {
            clipboard::write_picture(
                DynamicImage::ImageRgb8(picture(&renderer, &document)?).to_rgba8(),
            )
        })
    }
}

/// Saved pictures: their width in logical pixels, and pixels per logical
/// pixel.
const EXPORT_WIDTH: f32 = 680.0;
const EXPORT_SCALE: f32 = 3.0;

async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|e| e.to_string())?
}

fn picture(renderer: &Renderer, document: &Document) -> Result<RgbImage, String> {
    let style = RenderStyle {
        font_family: fonts::FAMILY.into(),
        font_size: style::TEXT_SIZE,
        max_width: EXPORT_WIDTH,
        scale: EXPORT_SCALE,
        text_color: [0x1d, 0x1d, 0x1f, 0xff],
        link_color: [0x1a, 0x5f, 0xb4, 0xff],
        muted_color: [0xd0, 0xd0, 0xd0, 0xff],
    };
    let rendered = renderer
        .render(document, &style)
        .map_err(|e| format!("Could not typeset the message: {e}"))?;
    let on_white = rendered
        .rgba
        .as_chunks::<4>()
        .0
        .iter()
        .flat_map(|p| {
            let alpha = u32::from(p[3]);
            [0, 1, 2].map(|i| ((u32::from(p[i]) * alpha + 255 * (255 - alpha)) / 255) as u8)
        })
        .collect();
    RgbImage::from_raw(rendered.width, rendered.height, on_white)
        .ok_or_else(|| "The typeset message is empty".to_owned())
}

fn render(renderer: &Renderer, document: &Document, key: Key, params: &Params) -> Rendered {
    let style = RenderStyle {
        font_family: fonts::FAMILY.into(),
        font_size: style::TEXT_SIZE,
        max_width: key.width as f32,
        scale: params.scale,
        text_color: style::to_rgba8(params.text),
        link_color: style::to_rgba8(params.link),
        muted_color: style::to_rgba8(params.muted),
    };
    match renderer.render(document, &style) {
        Ok(img) => Rendered::Image {
            width: img.width as f32 / params.scale,
            height: img.height as f32 / params.scale,
            text: Arc::new(selectable::image::Text::new(
                &img.pieces,
                &message::text_map(document),
                params.scale,
            )),
            handle: image::Handle::from_rgba(img.width, img.height, img.rgba),
        },
        Err(_) => Rendered::Failed,
    }
}

#[cfg(test)]
impl Math {
    /// Render synchronously (tests).
    pub fn render_now(&mut self, request: Request) {
        let key = Self::key(&request.document, &request.params);
        let rendered = render(&self.renderer, &request.document, key, &request.params);
        assert!(matches!(rendered, Rendered::Image { .. }), "render failed");
        self.insert(key, rendered);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Asked for twice, a message is typeset once; laid out wider, anew.
    #[test]
    fn asked_twice_renders_once() {
        let mut math = Math::new();
        let request = Request {
            document: Document::from_html(r#"<span data-mx-maths="x^2"><code>x^2</code></span>"#),
            params: Params {
                max_width: 400.0,
                scale: 2.0,
                text: iced::Color::BLACK,
                link: iced::Color::BLACK,
                muted: iced::Color::BLACK,
            },
        };
        let key = Math::key(&request.document, &request.params);
        let done: fn(Key, Rendered) = |_, _| ();
        assert!(math.wanted(&key));
        assert_eq!(math.request(request.clone(), done).units(), 1);
        assert!(!math.wanted(&key));
        assert_eq!(math.request(request.clone(), done).units(), 0);
        let wider = Request {
            params: Params {
                max_width: 600.0,
                ..request.params
            },
            ..request
        };
        assert_eq!(math.request(wider, done).units(), 1);
    }

    #[test]
    fn saves_math_as_a_png_on_white() {
        let math = Math::new();
        let document = Document::from_html(
            r#"Melt <span data-mx-maths="\dot m = \frac{G}{\rho L}"><code>m</code></span>"#,
        );
        let picture = super::picture(&math.renderer, &document).unwrap();
        assert!(picture.width() > 100 && picture.height() > 30);
        assert_eq!(
            picture.get_pixel(0, 0).0,
            [255, 255, 255],
            "white background"
        );
        assert!(
            picture.pixels().any(|p| p.0.iter().all(|&c| c < 80)),
            "dark text"
        );
    }
}
