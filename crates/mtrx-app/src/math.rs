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

    /// Start rendering unless it is cached or already running.
    pub fn request<M: Send + 'static>(
        &mut self,
        document: &Document,
        params: &Params,
        done: fn(Key, Rendered) -> M,
    ) -> Task<M> {
        let key = Self::key(document, params);
        if self.images.contains_key(&key) || !self.pending.insert(key) {
            return Task::none();
        }
        let renderer = self.renderer.clone();
        let document = document.clone();
        let params = *params;
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
    pub fn render_now(&mut self, document: &Document, params: &Params) {
        let key = Self::key(document, params);
        let rendered = render(&self.renderer, document, key, params);
        assert!(matches!(rendered, Rendered::Image { .. }), "render failed");
        self.insert(key, rendered);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
