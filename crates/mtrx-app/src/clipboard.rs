//! The system clipboard beyond iced's plain text: pictures; text copied
//! from the timeline goes as HTML too, so that Mail or a document keeps its
//! formatting, links and lists; pictures and files pasted into the
//! composer are attached, and rich text comes in as Markdown.

use std::{io::Cursor, path::PathBuf, sync::Mutex};

use arboard::ImageData;
use iced::widget::text_editor::{self, Binding, KeyPress};
use mtrx_core::Attachment;
use mtrx_format::{Block, Document, Inline, Style};

/// What copying text puts on the clipboard.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Copied {
    /// Plain text, math as `$…$`.
    pub text: String,
    /// The same as HTML, math as `$…$` too.
    pub html: String,
}

impl Copied {
    /// A line saying who wrote what follows, and when.
    pub fn byline(name: &str, time: &str) -> Self {
        let line = Document {
            blocks: vec![Block::Paragraph(vec![
                Inline::Styled {
                    style: Style::Bold,
                    content: vec![Inline::Text(name.to_owned())],
                },
                Inline::Text(format!(", {time}")),
            ])],
        };
        Self {
            text: format!("{name}, {time}\n"),
            html: line.to_html(),
        }
    }

    /// `next` right after this.
    pub fn then(mut self, next: Copied) -> Self {
        self.text.push_str(&next.text);
        self.html.push_str(&next.html);
        self
    }

    /// `next` after this, on the next line (in HTML, its own paragraph).
    pub fn push_line(&mut self, next: Copied) {
        if !self.text.is_empty() {
            self.text.push('\n');
        }
        self.text.push_str(&next.text);
        self.html.push_str(&next.html);
    }

    /// `next` after this, a paragraph apart.
    pub fn push(&mut self, next: Copied) {
        if !self.text.is_empty() {
            self.text.push_str("\n\n");
        }
        self.text.push_str(&next.text);
        self.html.push_str(&next.html);
    }
}

/// Put `copied` on the clipboard: its HTML, with its text for apps that
/// take only text.
pub async fn write(copied: Copied) -> Result<(), String> {
    tokio::task::spawn_blocking(move || {
        with(|clipboard| clipboard.set().html(copied.html, Some(copied.text)))
            .map_err(|e| format!("Could not copy: {e}"))
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Put a picture on the clipboard (iced's clipboard holds text only).
/// Blocks: run it off the UI thread.
pub fn write_picture(picture: image::RgbaImage) -> Result<(), String> {
    let image = ImageData {
        width: picture.width() as usize,
        height: picture.height() as usize,
        bytes: picture.into_raw().into(),
    };
    with(|clipboard| clipboard.set_image(image))
        .map_err(|e| format!("Could not copy the picture: {e}"))
}

/// What ⌘V pastes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Pasted {
    /// Files copied in the Finder or Explorer: attached, as if dropped.
    Files(Vec<PathBuf>),
    /// A picture, such as a screenshot: attached, as a PNG.
    Image(Attachment),
    /// Text: typed in.
    Text(String),
}

/// How to paste.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Paste {
    /// ⌘V: files, pictures, rich text as Markdown.
    Rich,
    /// ⌥⇧⌘V (or ⇧⌘V, Ctrl+Shift+V elsewhere): the plain text only.
    Plain,
}

/// How `press` pastes into an editor, while focused.
pub fn paste(press: &KeyPress) -> Option<Paste> {
    let focused = matches!(press.status, text_editor::Status::Focused { .. });
    let v = press.key.to_latin(press.physical_key) == Some('v');
    if focused && v && press.modifiers.command() && press.modifiers.shift() {
        return Some(Paste::Plain);
    }
    matches!(
        Binding::<()>::from_key_press(press.clone()),
        Some(Binding::Paste)
    )
    .then_some(Paste::Rich)
}

/// What the clipboard holds, as pasted `how` into a field that takes
/// `files` and pictures too, or only text; `None` for nothing it can take.
pub async fn read(how: Paste, files: bool) -> Option<Pasted> {
    tokio::task::spawn_blocking(move || {
        with(|clipboard| {
            let text = clipboard.get().text().ok();
            if how == Paste::Plain {
                return Ok(text.filter(|text| !text.is_empty()).map(Pasted::Text));
            }
            let html = clipboard.get().html().ok();
            if !files {
                return Ok(pasted(Vec::new(), text, html, || None));
            }
            let files = clipboard.get().file_list().unwrap_or_default();
            Ok(pasted(files, text, html, || clipboard.get().image().ok()))
        })
        .ok()
        .flatten()
    })
    .await
    .ok()
    .flatten()
}

/// What is pasted of the clipboard's `files`, text (`text`, or `html` as
/// Markdown) and `image`, in that order: copied files come with their
/// icon as a picture, and text from some apps (Word) with a picture of it.
fn pasted(
    files: Vec<PathBuf>,
    text: Option<String>,
    html: Option<String>,
    image: impl FnOnce() -> Option<ImageData<'static>>,
) -> Option<Pasted> {
    if !files.is_empty() {
        return Some(Pasted::Files(files));
    }
    let markdown = html.as_deref().and_then(markdown);
    match text.filter(|text| !text.is_empty()) {
        // HTML that is only the text (from a code editor, say) is pasted as
        // that text.
        Some(text) => Some(Pasted::Text(
            markdown
                .filter(|markdown| formatted(markdown, &text))
                .unwrap_or(text),
        )),
        None => markdown
            .map(Pasted::Text)
            .or_else(|| image().and_then(png).map(Pasted::Image)),
    }
}

/// Rich text (from a web page, Mail) as Markdown for the composer: links,
/// lists, emphasis, headings and code. Pictures are left out, and math
/// (`$…$`) stays as it was.
fn markdown(html: &str) -> Option<String> {
    use htmd::{
        Element, HtmlToMarkdown,
        element_handler::Handlers,
        options::{BulletListMarker, HrStyle, LinkStyle, Options},
    };

    let converter = HtmlToMarkdown::builder()
        .options(Options {
            bullet_list_marker: BulletListMarker::Dash,
            ul_bullet_spacing: 1,
            ol_number_spacing: 1,
            hr_style: HrStyle::Dashes,
            link_style: LinkStyle::InlinedPreferAutolinks,
            ..Options::default()
        })
        .skip_tags(vec!["head", "script", "style", "img"])
        // A link whose text is its address: the address alone, as typed.
        .add_handler(vec!["a"], |handlers: &dyn Handlers, element: Element| {
            let link = handlers.fallback(element)?;
            match link
                .content
                .strip_prefix('<')
                .and_then(|s| s.strip_suffix('>'))
            {
                Some(url) => Some(url.to_owned().into()),
                None => Some(link),
            }
        })
        .build();
    let markdown = converter.convert(html).ok()?;
    // Lines end in breaks in the composer, without trailing spaces.
    let markdown: Vec<_> = markdown.lines().map(str::trim_end).collect();
    let markdown = unescape_math(markdown.join("\n").trim());
    (!markdown.is_empty()).then_some(markdown)
}

/// Whether `markdown` says more than `text`, its plain text: formatting,
/// or links. Whitespace and escapes do not count.
fn formatted(markdown: &str, text: &str) -> bool {
    let bare = |s: &str| -> String { unescape(s).chars().filter(|c| !c.is_whitespace()).collect() };
    bare(markdown) != bare(text)
}

/// `markdown` with the escapes htmd puts into math (`$…$` and `$$…$$`)
/// taken out, so that its LaTeX stays as it was.
fn unescape_math(markdown: &str) -> String {
    let mut out = String::with_capacity(markdown.len());
    let mut rest = markdown;
    while let Some(start) = rest.find('$') {
        let fence = if rest[start..].starts_with("$$") {
            "$$"
        } else {
            "$"
        };
        let body = start + fence.len();
        let Some(len) = rest[body..].find(fence) else {
            break;
        };
        out.push_str(&rest[..body]);
        out.push_str(&unescape(&rest[body..body + len]));
        out.push_str(fence);
        rest = &rest[body + len + fence.len()..];
    }
    out.push_str(rest);
    out
}

/// `text` without the backslashes that escape characters of Markdown.
fn unescape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match chars.peek() {
            Some(&next) if c == '\\' && next.is_ascii_punctuation() => {
                out.push(next);
                chars.next();
            }
            _ => out.push(c),
        }
    }
    out
}

/// A picture from the clipboard as a PNG file to attach.
fn png(image: ImageData<'_>) -> Option<Attachment> {
    let picture = image::RgbaImage::from_raw(
        image.width as u32,
        image.height as u32,
        image.bytes.into_owned(),
    )?;
    let mut bytes = Vec::new();
    picture
        .write_to(&mut Cursor::new(&mut bytes), image::ImageFormat::Png)
        .ok()?;
    Some(Attachment {
        filename: "image.png".into(),
        mimetype: "image/png".into(),
        bytes,
    })
}

/// Runs `f` on the app's clipboard. One for as long as the app runs: on
/// X11 what we copied is served by it, and gone once it is dropped.
fn with<T>(
    f: impl FnOnce(&mut arboard::Clipboard) -> Result<T, arboard::Error>,
) -> Result<T, arboard::Error> {
    static CLIPBOARD: Mutex<Option<arboard::Clipboard>> = Mutex::new(None);
    let mut clipboard = CLIPBOARD.lock().unwrap_or_else(|e| e.into_inner());
    let clipboard = match &mut *clipboard {
        Some(clipboard) => clipboard,
        none => none.insert(arboard::Clipboard::new()?),
    };
    f(clipboard)
}

#[cfg(test)]
mod tests {
    use iced::{
        Event, Size,
        keyboard::{self, Key, Modifiers},
        mouse,
        widget::text_editor::{self, Content},
    };
    use iced_test::simulator::{Simulator, press_key};

    use super::*;

    /// `key` pressed with `modifiers`.
    fn press(key: &str, modifiers: Modifiers) -> Event {
        match press_key(Key::Character(key.into()), None) {
            Event::Keyboard(keyboard::Event::KeyPressed {
                key,
                modified_key,
                physical_key,
                location,
                text,
                repeat,
                ..
            }) => Event::Keyboard(keyboard::Event::KeyPressed {
                key,
                modified_key,
                physical_key,
                location,
                modifiers,
                text,
                repeat,
            }),
            _ => unreachable!(),
        }
    }

    #[derive(Clone, Debug, PartialEq)]
    enum Pressed {
        Edit,
        Paste(Paste),
    }

    /// What an editor bound as the composer is (see `room.rs`) sends for
    /// `events`, focused by a click first if `focused`.
    fn editor_sends(focused: bool, events: Vec<Event>) -> Vec<Pressed> {
        let content = Content::<iced::Renderer>::with_text("Melt");
        let editor: iced::Element<'_, Pressed> = iced::widget::text_editor(&content)
            .key_binding(|press| {
                if let Some(how) = paste(&press) {
                    Some(text_editor::Binding::Custom(Pressed::Paste(how)))
                } else {
                    text_editor::Binding::from_key_press(press)
                }
            })
            .on_action(|_| Pressed::Edit)
            .into();
        let mut ui =
            Simulator::with_size(crate::fonts::settings(), Size::new(300.0, 100.0), editor);
        if focused {
            ui.point_at((290.0, 10.0));
            let _ = ui.simulate([Event::Mouse(mouse::Event::ButtonPressed(
                mouse::Button::Left,
            ))]);
        }
        let _ = ui.simulate(events);
        ui.into_messages()
            .filter(|pressed| *pressed != Pressed::Edit)
            .collect()
    }

    #[test]
    fn command_v_pastes_through_us() {
        let command_v = || press("v", Modifiers::COMMAND);
        assert_eq!(
            editor_sends(true, vec![command_v()]),
            [Pressed::Paste(Paste::Rich)]
        );
        // Not while another field has the focus.
        assert!(editor_sends(false, vec![command_v()]).is_empty());
    }

    #[test]
    fn option_shift_command_v_pastes_plain_text() {
        let plain = Modifiers::COMMAND | Modifiers::SHIFT;
        for modifiers in [plain | Modifiers::ALT, plain] {
            assert_eq!(
                editor_sends(true, vec![press("v", modifiers)]),
                [Pressed::Paste(Paste::Plain)]
            );
            assert!(editor_sends(false, vec![press("v", modifiers)]).is_empty());
        }
    }

    /// The Edit menu's Paste and Paste as Plain Text (macOS's menu takes
    /// ⌘V and ⌥⇧⌘V) paste through us, as the keys do.
    #[test]
    fn the_menus_paste_pastes_through_us() {
        use iced_core::event::StandardAction;

        let action = |action| vec![Event::StandardAction(action)];
        assert_eq!(
            editor_sends(true, action(StandardAction::Paste)),
            [Pressed::Paste(Paste::Rich)]
        );
        assert_eq!(
            editor_sends(true, action(StandardAction::PasteAsPlainText)),
            [Pressed::Paste(Paste::Plain)]
        );
        assert!(editor_sends(false, action(StandardAction::Paste)).is_empty());
    }

    fn picture() -> ImageData<'static> {
        ImageData {
            width: 2,
            height: 1,
            bytes: vec![255, 0, 0, 255, 0, 0, 255, 128].into(),
        }
    }

    #[test]
    fn pastes_files_then_text_then_a_picture() {
        let file = PathBuf::from("/tmp/notes.pdf");
        let no_picture = || -> Option<ImageData<'static>> { panic!("not read") };
        assert_eq!(
            pasted(
                vec![file.clone()],
                Some("notes.pdf".into()),
                None,
                no_picture
            ),
            Some(Pasted::Files(vec![file]))
        );
        assert_eq!(
            pasted(Vec::new(), Some("Melt".into()), None, no_picture),
            Some(Pasted::Text("Melt".into()))
        );
        let Some(Pasted::Image(file)) =
            pasted(Vec::new(), Some(String::new()), None, || Some(picture()))
        else {
            panic!("a picture is attached");
        };
        assert_eq!(file.mimetype, "image/png");
        // The same pixels.
        let decoded = image::load_from_memory(&file.bytes).unwrap().to_rgba8();
        assert_eq!(decoded.dimensions(), (2, 1));
        assert_eq!(decoded.into_raw(), picture().bytes.into_owned());
        assert_eq!(pasted(Vec::new(), None, None, || None), None);
    }

    /// What pasting `html`, with `text` as its plain text, types.
    fn pasted_text(text: &str, html: &str) -> String {
        match pasted(Vec::new(), Some(text.into()), Some(html.into()), || None) {
            Some(Pasted::Text(text)) => text,
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn rich_text_pastes_as_markdown() {
        // As Safari puts a selection on the clipboard.
        let html = r#"<html><head><meta charset="utf-8"><style>p { margin: 0 }</style></head><body><p>See <a href="https://example.org/data">the data</a>, <b>now</b> or <i>later</i>: https://example.org</p><ul><li>one_two</li><li>three</li></ul><ol start="2"><li>four</li></ol><p>A line<br>and the next <img src="https://example.org/a.png"></p></body></html>"#;
        let text = "See the data, now or later: https://example.org\none_two\nthree\nfour\nA line\nand the next";
        assert_eq!(
            pasted_text(text, html),
            "See [the data](https://example.org/data), **now** or *later*: https://example.org\n\n\
             - one\\_two\n- three\n\n2. four\n\nA line\nand the next"
        );
        // A link that is its address: the address alone.
        assert_eq!(
            pasted_text(
                "https://example.org",
                r#"<a href="https://example.org">https://example.org</a>"#
            ),
            "https://example.org"
        );
    }

    #[test]
    fn html_of_only_the_text_pastes_the_text() {
        // As a code editor puts code on the clipboard, coloured.
        let html = r#"<div style="color: #333"><div><span style="color: #00f">let</span> x_1 = *y;</div><div>    z</div></div>"#;
        let text = "let x_1 = *y;\n    z";
        assert_eq!(pasted_text(text, html), text);
    }

    #[test]
    fn math_copied_from_mtrx_pastes_as_it_was() {
        let document = Document::from_html(
            r#"<p><b>Flux</b> <span data-mx-maths="q_i=-k\nabla T"><code>q</code></span> and $5</p><div data-mx-maths="\sum_i x_i^*"><code>x</code></div>"#,
        );
        assert_eq!(
            pasted_text("Flux ...", &document.to_html()),
            "**Flux** $q_i=-k\\nabla T$ and $5\n\n$$\\sum_i x_i^*$$"
        );
    }
}
