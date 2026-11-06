//! Native rendering of a message [`Document`], with selectable text: each
//! paragraph, heading and code block is a unit of [`crate::selection`].
//! Messages with math use a typeset image instead (see [`crate::math`]);
//! this view is the fallback while that image is not ready, showing math as
//! LaTeX code.

use std::rc::Rc;

use iced::{
    Element, Font, Length, Pixels, Theme,
    widget::{column, container, row, rule, span, text, text::Span},
};
use mtrx_core::{OwnedUserId, UserId};
use mtrx_format::{Block, Document, Inline, Style};

use crate::{
    clipboard::Copied,
    emoji,
    find::Found,
    fonts,
    selectable::text::{Selectable, selectable},
    selection::{Gesture, Pos, Unit, unit_range, units_text},
    style,
};

#[derive(Clone, Copy, Default)]
struct Flags {
    bold: bool,
    italic: bool,
    underline: bool,
    strike: bool,
    code: bool,
}

/// How a message's text, or another part of the timeline's text (see
/// [`crate::rows::Part`]), can be selected.
pub struct Select<'a, M> {
    /// The part of this text that is selected.
    pub range: Option<(Pos, Pos)>,
    /// Where in this text the selection ends, if it does.
    pub focus: Option<Pos>,
    /// A selection is being dragged, so moving over the text extends it.
    pub dragging: bool,
    pub on_gesture: Rc<dyn Fn(Gesture) -> M + 'a>,
    /// Matches of the find bar in this message, drawn as a selection is.
    pub found: Vec<Found>,
}

/// What clicking a link sends, and right-clicking one.
pub struct Links<'a, M> {
    pub open: fn(String) -> M,
    /// Opens the link's menu.
    pub menu: Rc<dyn Fn(String) -> M + 'a>,
}

/// `document`, its mentions of `own` (us) in bold; marked when `edited`.
pub fn view<'a, M: Clone + 'a>(
    document: &Document,
    theme: &Theme,
    own: &UserId,
    links: Links<'a, M>,
    select: Select<'a, M>,
    edited: bool,
) -> Element<'a, M> {
    let mut walk = Walk {
        theme,
        own: Some(own),
        links: Some(links),
        select: Some(select),
        edited,
        map: TextMap::default(),
    };
    let blocks = walk.blocks(&document.blocks, "", "\n\n", true);
    // Not ending in text (e.g. in a rule): marked on a line of its own.
    if walk.edited {
        column![blocks, edited_line()].spacing(6).into()
    } else {
        blocks
    }
}

/// The mark of an edited message, after its last line of text so that an
/// edit arriving does not make it taller.
const EDITED: &str = " (edited)";
const EDITED_SIZE: f32 = 11.0;

/// The mark of an edited message on a line of its own, for one that does
/// not end in text, such as a picture.
pub fn edited_line<'a, M: 'a>() -> Element<'a, M> {
    text(EDITED.trim_start())
        .size(EDITED_SIZE)
        .style(text::secondary)
        .into()
}

/// The message's text as shown, in units, and where each text leaf is in
/// them.
#[derive(Clone, Debug, Default)]
pub struct TextMap {
    pub units: Vec<Unit>,
    /// Each text, inline code and inline math, and each code block and
    /// displayed math, in reading order (as [`mtrx_format::Piece::leaf`]
    /// counts them).
    pub leaves: Vec<Leaf>,
}

/// Where a text leaf is shown: its unit, and its bytes there; and the link
/// it is in, if any (a mention's too).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Leaf {
    pub unit: usize,
    pub start: usize,
    pub len: usize,
    pub link: Option<String>,
}

pub fn text_map(document: &Document) -> TextMap {
    let mut walk: Walk<'_, '_, ()> = Walk {
        theme: &Theme::Light,
        own: None,
        links: None,
        select: None,
        edited: false,
        map: TextMap::default(),
    };
    let _ = walk.blocks(&document.blocks, "", "\n\n", true);
    walk.map
}

/// What copying `document` from `from` to `to` puts on the clipboard: the
/// text as shown, and that part of the message as HTML.
pub fn copied(document: &Document, from: Pos, to: Pos) -> Copied {
    let map = text_map(document);
    let part = document.clipped(|leaf| {
        let leaf = map.leaves.get(leaf)?;
        let unit = unit_range((from, to), leaf.unit, map.units[leaf.unit].text.len())?;
        let start = unit.start.max(leaf.start);
        let end = unit.end.min(leaf.start + leaf.len);
        (start < end).then(|| start - leaf.start..end - leaf.start)
    });
    Copied {
        text: units_text(&map.units, from, to),
        html: part.to_html(),
    }
}

/// Builds the view of blocks and records their units and leaves, in the
/// same order.
struct Walk<'t, 'a, M> {
    theme: &'t Theme,
    own: Option<&'t UserId>,
    links: Option<Links<'a, M>>,
    select: Option<Select<'a, M>>,
    /// The message is edited and its mark not yet shown.
    edited: bool,
    map: TextMap,
}

impl<'a, M: Clone + 'a> Walk<'_, 'a, M> {
    /// `prefix` goes before each unit's text when copied (quote marks and
    /// list indents); `gap` between blocks. `last` when they end the
    /// message.
    fn blocks(
        &mut self,
        list: &[Block],
        prefix: &str,
        gap: &'static str,
        last: bool,
    ) -> Element<'a, M> {
        let mut out = column![].spacing(6);
        for (i, block) in list.iter().enumerate() {
            out = out.push(self.block(block, prefix, gap, last && i + 1 == list.len()));
        }
        out.into()
    }

    fn block(
        &mut self,
        block: &Block,
        prefix: &str,
        gap: &'static str,
        last: bool,
    ) -> Element<'a, M> {
        match block {
            Block::Paragraph(content) => self.paragraph(content, None, prefix, gap, last),
            Block::Heading { content, .. } => {
                self.paragraph(content, Some(16.0), prefix, gap, last)
            }
            Block::Quote(inner) => row![
                container(text(""))
                    .width(3)
                    .height(Length::Fill)
                    .style(style::quote_bar),
                self.blocks(inner, &format!("{prefix}> "), gap, last),
            ]
            .spacing(8)
            .height(Length::Shrink)
            .into(),
            Block::List { start, items } => {
                let mut list = column![].spacing(2);
                for (i, item) in items.iter().enumerate() {
                    let (shown, copied) = match start {
                        Some(start) => {
                            let marker = format!("{}.", *start + i as u64);
                            (marker.clone(), format!("{marker} "))
                        }
                        None => ("•".to_owned(), "- ".to_owned()),
                    };
                    // The item's first unit gets the marker, the rest its
                    // indent; items are a line apart.
                    let first = self.map.units.len();
                    let indent = format!("{prefix}{}", " ".repeat(copied.len()));
                    let content = self.blocks(item, &indent, "\n", last && i + 1 == items.len());
                    if let Some(unit) = self.map.units.get_mut(first) {
                        unit.prefix = format!("{prefix}{copied}");
                        unit.gap = if i == 0 { gap } else { "\n" };
                    }
                    list = list.push(row![text(shown).width(20), content]);
                }
                list.into()
            }
            Block::CodeBlock { code, .. } => {
                let code = code.trim_end().to_owned();
                self.whole_leaf(code.len());
                container(self.unit(
                    vec![span(code).font(Font::MONOSPACE)],
                    Some(13.0),
                    prefix,
                    gap,
                    last,
                ))
                .padding(8)
                .width(Length::Fill)
                .style(style::code_block)
                .into()
            }
            Block::Math(tex) => {
                let shown = format!("$${tex}$$");
                self.whole_leaf(shown.len());
                self.unit(
                    vec![span(shown).font(Font::MONOSPACE)],
                    Some(13.0),
                    prefix,
                    gap,
                    last,
                )
            }
            Block::Rule => rule::horizontal(1).into(),
        }
    }

    fn paragraph(
        &mut self,
        content: &[Inline],
        size: Option<f32>,
        prefix: &str,
        gap: &'static str,
        last: bool,
    ) -> Element<'a, M> {
        let mut out = Vec::new();
        let flags = Flags {
            bold: size.is_some(),
            ..Flags::default()
        };
        let mut leaves = Vec::new();
        let look = Look {
            theme: self.theme,
            own: self.own,
        };
        spans(content, flags, None, &look, &mut out, &mut leaves);
        let unit = self.map.units.len();
        self.map
            .leaves
            .extend(leaves.into_iter().map(|(start, len, link)| Leaf {
                unit,
                start,
                len,
                link,
            }));
        self.unit(out, size, prefix, gap, last)
    }

    /// A leaf that is all of the next unit's text.
    fn whole_leaf(&mut self, len: usize) {
        self.map.leaves.push(Leaf {
            unit: self.map.units.len(),
            start: 0,
            len,
            link: None,
        });
    }

    /// One selectable unit of text; the mark of an edited message after it
    /// when `last`.
    fn unit(
        &mut self,
        spans: Vec<Span<'static, String, Font>>,
        size: Option<f32>,
        prefix: &str,
        gap: &'static str,
        last: bool,
    ) -> Element<'a, M> {
        let index = self.map.units.len();
        let text: String = spans.iter().map(|s| s.text.as_ref()).collect();
        let gap = if index == 0 { "" } else { gap };
        self.map.units.push(Unit {
            text,
            prefix: prefix.to_owned(),
            gap,
        });
        let mut unit = selectable(spans, index);
        if last && std::mem::take(&mut self.edited) {
            let color = text::secondary(self.theme).color;
            unit = unit.after(span(EDITED).size(EDITED_SIZE).color_maybe(color));
        }
        if let Some(size) = size {
            unit = unit.size(Pixels(size));
        }
        if let Some(links) = &self.links {
            unit = unit.on_link(links.open).on_link_menu(links.menu.clone());
        }
        if let Some(select) = &self.select {
            unit = select.apply(unit, index);
        }
        unit.into()
    }
}

impl<'a, M: 'a> Select<'a, M> {
    /// `unit`, unit `index` of the text, selected and with the find bar's
    /// matches as this says.
    pub fn apply(&self, unit: Selectable<'a, M>, index: usize) -> Selectable<'a, M> {
        unit.selected(self.range)
            .focus(self.focus.is_some_and(|focus| focus.unit == index))
            .dragging(self.dragging)
            .on_gesture(self.on_gesture.clone())
            .found(
                (self.found.iter())
                    .filter(|found| found.unit == index)
                    .cloned()
                    .collect(),
            )
    }
}

/// What spans look like besides their text.
struct Look<'t> {
    theme: &'t Theme,
    /// Us, whose mentions stand out.
    own: Option<&'t UserId>,
}

/// Styled spans of `content` into `out`; each leaf's start and length in
/// their text, and the link each is in, into `leaves`.
fn spans(
    content: &[Inline],
    flags: Flags,
    link: Option<&str>,
    look: &Look<'_>,
    out: &mut Vec<Span<'static, String, Font>>,
    leaves: &mut Vec<(usize, usize, Option<String>)>,
) {
    let theme = look.theme;
    for inline in content {
        let start: usize = out.iter().map(|s| s.text.len()).sum();
        match inline {
            Inline::Text(t) => {
                leaves.push((start, t.len(), link.map(str::to_owned)));
                for (is_emoji, run) in emoji::runs(t) {
                    let run = styled(span(run), flags, link, theme);
                    out.push(if is_emoji {
                        run.font(fonts::EMOJI)
                    } else {
                        run
                    });
                }
            }
            Inline::Styled { style, content } => {
                let mut f = flags;
                match style {
                    Style::Bold => f.bold = true,
                    Style::Italic => f.italic = true,
                    Style::Strikethrough => f.strike = true,
                    Style::Underline => f.underline = true,
                    // No good native equivalent; keep the text.
                    Style::Superscript | Style::Subscript | Style::Spoiler => {}
                }
                spans(content, f, link, look, out, leaves);
            }
            Inline::Code(code) => {
                leaves.push((start, code.len(), link.map(str::to_owned)));
                out.push(styled(
                    span(code.clone()),
                    Flags {
                        code: true,
                        ..flags
                    },
                    link,
                    theme,
                ));
            }
            Inline::Link { url, content } => {
                spans(content, flags, Some(url), look, out, leaves);
            }
            Inline::Mention { user_id, name } => {
                let own = look.own.is_some_and(|own| own.as_str() == user_id);
                let url = mention_url(user_id);
                leaves.push((start, name.len(), Some(url.clone())));
                for (is_emoji, run) in emoji::runs(name) {
                    let run = mention(span(run), flags, own, theme).link(url.clone());
                    out.push(if is_emoji {
                        run.font(fonts::EMOJI)
                    } else {
                        run
                    });
                }
            }
            Inline::Math(tex) => {
                let shown = format!("${tex}$");
                leaves.push((start, shown.len(), link.map(str::to_owned)));
                out.push(styled(
                    span(shown),
                    Flags {
                        code: true,
                        ..flags
                    },
                    link,
                    theme,
                ));
            }
            Inline::LineBreak => out.push(span("\n".to_owned())),
        }
    }
}

/// A mention: the name in the link colour, bold when it is us.
fn mention(
    s: Span<'static, String, Font>,
    flags: Flags,
    own: bool,
    theme: &Theme,
) -> Span<'static, String, Font> {
    s.font(fonts::styled(flags.bold || own, flags.italic))
        .color(theme.extended_palette().primary.base.color)
        .strikethrough(flags.strike)
}

/// What a click on a mention of `user_id` sends: a `matrix.to` link.
fn mention_url(user_id: &str) -> String {
    format!("{MATRIX_TO}{user_id}")
}

const MATRIX_TO: &str = "https://matrix.to/#/";

/// The user a clicked link is to, if it is a mention's (see
/// [`mention_url`]); links to rooms are not.
pub fn mentioned_user(url: &str) -> Option<OwnedUserId> {
    OwnedUserId::try_from(url.strip_prefix(MATRIX_TO)?).ok()
}

fn styled(
    s: Span<'static, String, Font>,
    flags: Flags,
    link: Option<&str>,
    theme: &Theme,
) -> Span<'static, String, Font> {
    let palette = theme.extended_palette();
    let mut s = s
        .font(if flags.code {
            Font::MONOSPACE
        } else {
            fonts::styled(flags.bold, flags.italic)
        })
        .underline(flags.underline || link.is_some())
        .strikethrough(flags.strike);
    if flags.code {
        s = s.background(palette.background.strong.color.scale_alpha(0.5));
    }
    if let Some(url) = link {
        s = s.color(palette.primary.base.color).link(url.to_owned());
    }
    s
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use iced::{Event, Size, mouse};
    use iced_test::simulator::Simulator;

    use super::*;
    use crate::selection::{Message, Selection};

    /// Drives `document`'s view (marked if `edited`), in a scrollable
    /// `height` high, with mouse `steps` (a position and an event there);
    /// returns the gestures reported.
    fn gestures(
        document: &Document,
        edited: bool,
        height: f32,
        dragging: bool,
        steps: &[((f32, f32), mouse::Event)],
    ) -> Vec<Gesture> {
        let own = OwnedUserId::try_from("@me:example.org").unwrap();
        let link = |_| Gesture::Drag {
            unit: usize::MAX,
            offset: 0,
        };
        let view = view(
            document,
            &Theme::Light,
            &own,
            Links {
                open: link,
                menu: Rc::new(link),
            },
            Select {
                range: None,
                focus: None,
                dragging,
                on_gesture: Rc::new(|gesture| gesture),
                found: Vec::new(),
            },
            edited,
        );
        let view = iced::widget::scrollable(view).height(height);
        let mut ui = Simulator::with_size(fonts::settings(), Size::new(800.0, 400.0), view);
        for (at, event) in steps {
            ui.point_at(*at);
            let _ = ui.simulate([Event::Mouse(*event)]);
        }
        ui.into_messages().collect()
    }

    /// What copying the selection made by `steps` (see [`gestures`])
    /// copies.
    fn select_in(
        document: &Document,
        height: f32,
        dragging: bool,
        steps: &[((f32, f32), mouse::Event)],
    ) -> Copied {
        let mut selection = None;
        for gesture in gestures(document, false, height, dragging, steps) {
            Selection::update(&mut selection, Message::Gesture("$e".into(), gesture));
        }
        let order = HashMap::from([("$e", 0)]);
        let Some(bounds) = selection.and_then(|s| s.ordered(&order)) else {
            return Copied::default();
        };
        let (from, to) = bounds.in_message(0).unwrap();
        copied(document, from, to)
    }

    /// The text selected by `steps`.
    fn select(document: &Document, dragging: bool, steps: &[((f32, f32), mouse::Event)]) -> String {
        select_in(document, 400.0, dragging, steps).text
    }

    #[test]
    fn selects_where_the_cursor_is_when_scrolled() {
        use mouse::{Button::Left, Event::*};
        let html: String = (0..30).map(|i| format!("<p>Line {i}</p>")).collect();
        let document = Document::from_html(&html);
        let selected = select_in(
            &document,
            100.0,
            true,
            &[
                (
                    (2.0, 50.0),
                    WheelScrolled {
                        delta: mouse::ScrollDelta::Pixels { x: 0.0, y: -300.0 },
                    },
                ),
                ((2.0, 8.0), ButtonPressed(Left)),
                (
                    (700.0, 35.0),
                    CursorMoved {
                        position: iced::Point::new(700.0, 35.0),
                    },
                ),
            ],
        )
        .text;
        // Two neighbouring lines well below the first.
        let lines: Vec<u32> = selected
            .split("\n\n")
            .map(|line| line.trim_start_matches("Line ").parse().unwrap())
            .collect();
        assert_eq!(lines.len(), 2, "{selected:?}");
        assert!(lines[0] > 5 && lines[1] == lines[0] + 1, "{selected:?}");
    }

    #[test]
    fn selects_by_dragging_and_clicking() {
        use mouse::{Button::Left, Event::*};
        let document = Document::from_html("<p>Hello you there</p><p>Second line</p>");
        let moved = |x, y| CursorMoved {
            position: iced::Point::new(x, y),
        };
        // From the start of the first paragraph to right of the second.
        let dragged = select(
            &document,
            true,
            &[
                ((2.0, 8.0), ButtonPressed(Left)),
                ((700.0, 35.0), moved(700.0, 35.0)),
            ],
        );
        assert_eq!(dragged, "Hello you there\n\nSecond line");
        // Not dragging: moving selects nothing.
        assert_eq!(
            select(
                &document,
                false,
                &[
                    ((2.0, 8.0), ButtonPressed(Left)),
                    ((700.0, 35.0), moved(700.0, 35.0))
                ],
            ),
            ""
        );
        // A double click selects the word, a triple click the paragraph.
        let click = [
            ((2.0, 8.0), ButtonPressed(Left)),
            ((2.0, 8.0), ButtonReleased(Left)),
        ];
        assert_eq!(select(&document, false, &click.repeat(2)), "Hello");
        assert_eq!(
            select(&document, false, &click.repeat(3)),
            "Hello you there"
        );
    }

    #[derive(Clone, Debug, PartialEq)]
    enum Clicked {
        Link(String),
        LinkMenu(String),
        Menu,
        Selecting,
    }

    #[test]
    fn right_click_on_a_link_opens_its_menu() {
        let document =
            Document::from_html(r#"<p><a href="https://example.org/a">Data</a> here</p>"#);
        let own = OwnedUserId::try_from("@me:example.org").unwrap();
        // The messages sent by right-clicking at `x`, in a message that
        // has a menu of its own.
        let right_click = |x: f32| -> Vec<Clicked> {
            let view = view(
                &document,
                &Theme::Light,
                &own,
                Links {
                    open: Clicked::Link,
                    menu: Rc::new(Clicked::LinkMenu),
                },
                Select {
                    range: None,
                    focus: None,
                    dragging: false,
                    on_gesture: Rc::new(|_| Clicked::Selecting),
                    found: Vec::new(),
                },
                false,
            );
            let view = crate::context_menu::context_menu(view, None, Clicked::Menu);
            let mut ui = Simulator::with_size(fonts::settings(), Size::new(400.0, 100.0), view);
            ui.point_at((x, 8.0));
            let _ = ui.simulate([Event::Mouse(mouse::Event::ButtonPressed(
                mouse::Button::Right,
            ))]);
            ui.into_messages().collect()
        };
        assert_eq!(
            right_click(10.0),
            [Clicked::LinkMenu("https://example.org/a".into())]
        );
        assert_eq!(right_click(60.0), [Clicked::Menu]);
    }

    #[test]
    fn reports_where_the_current_match_is() {
        use iced::{
            Rectangle,
            advanced::{
                renderer::Headless,
                widget::{Id, Operation, operation::black_box},
            },
        };
        use iced_runtime::user_interface::{Cache, UserInterface};

        /// Where the current match was reported.
        struct Current(Option<Rectangle>);

        impl Operation for Current {
            fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation)) {
                operate(self);
            }

            fn custom(&mut self, id: Option<&Id>, bounds: Rectangle, _: &mut dyn std::any::Any) {
                if id == Some(&Id::new(crate::find::CURRENT)) {
                    self.0 = Some(bounds);
                }
            }
        }

        let mut renderer = iced::futures::executor::block_on(iced::Renderer::new(
            fonts::INTER,
            Pixels(style::TEXT_SIZE),
            None,
        ))
        .unwrap();
        let own = OwnedUserId::try_from("@me:example.org").unwrap();
        let document = Document::from_html("<p>Ice</p><p>Sea ice and more ice</p>");
        let mut current = |found| {
            let select = Select {
                range: None,
                focus: None,
                dragging: false,
                on_gesture: Rc::new(|_| ()),
                found,
            };
            let links = Links {
                open: |_| (),
                menu: Rc::new(|_| ()),
            };
            let view = view(&document, &Theme::Light, &own, links, select, false);
            let mut ui = UserInterface::build(
                view,
                Size::new(400.0, 400.0),
                Cache::default(),
                &mut renderer,
            );
            let mut operation = Current(None);
            ui.operate(&renderer, &mut black_box(&mut operation));
            operation.0
        };
        let found = |range, current| Found {
            unit: 1,
            range,
            current,
        };
        let at = current(vec![found(4..7, false), found(17..20, true)]).unwrap();
        // The second line's last word.
        assert!(at.y > 15.0 && at.x > 80.0, "{at:?}");
        assert!(at.width > 10.0 && at.width < 40.0, "{at:?}");
        let before = current(vec![found(4..7, true), found(17..20, false)]).unwrap();
        assert!(before.x < at.x && before.y == at.y, "{before:?}");
        assert_eq!(current(vec![found(4..7, false)]), None);
    }

    #[test]
    fn copies_formatting_as_html_too() {
        use mouse::{Button::Left, Event::*};
        let document = Document::from_html(
            r#"<p>Hello <b>you</b> there</p><ul><li>See <a href="https://example.org">docs</a></li><li>and $x$</li></ul>"#,
        );
        // Dragged from the start to right of the last item.
        let copied = select_in(
            &document,
            400.0,
            true,
            &[
                ((2.0, 8.0), ButtonPressed(Left)),
                (
                    (700.0, 58.0),
                    CursorMoved {
                        position: iced::Point::new(700.0, 58.0),
                    },
                ),
            ],
        );
        assert_eq!(copied.text, "Hello you there\n\n- See docs\n- and $x$");
        assert_eq!(
            copied.html,
            r#"<p>Hello <strong>you</strong> there</p><ul><li>See <a href="https://example.org">docs</a></li><li>and $x$</li></ul>"#
        );
        // From within "you" to within "docs".
        let part = super::copied(
            &document,
            Pos { unit: 0, offset: 7 },
            Pos { unit: 1, offset: 6 },
        );
        assert_eq!(part.text, "ou there\n\n- See do");
        assert_eq!(
            part.html,
            r#"<p><strong>ou</strong> there</p><ul><li>See <a href="https://example.org">do</a></li></ul>"#
        );
    }

    #[test]
    fn edited_mark_is_not_part_of_the_text() {
        use mouse::{Button::Left, Event::*};
        let document = Document::from_html("<p>Hello you there</p>");
        let len = "Hello you there".len();
        let press = |x| ((x, 8.0), ButtonPressed(Left));
        let release = |x| ((x, 8.0), ButtonReleased(Left));
        let moved = |x| {
            (
                (x, 8.0),
                CursorMoved {
                    position: iced::Point::new(x, 8.0),
                },
            )
        };
        // Dragged onto and past the mark, a double click on it, and a
        // triple click: all within the text.
        let steps = [
            vec![press(2.0), moved(130.0), moved(700.0)],
            vec![press(130.0), release(130.0), press(130.0)],
            vec![
                press(2.0),
                release(2.0),
                press(2.0),
                release(2.0),
                press(2.0),
            ],
        ];
        for steps in steps {
            let gestures = gestures(&document, true, 400.0, true, &steps);
            assert!(!gestures.is_empty());
            for gesture in gestures {
                match gesture {
                    Gesture::Press { from, to, .. } => assert!(from <= len && to <= len),
                    Gesture::Drag { offset, .. } => assert!(offset <= len),
                }
            }
        }
        // Its text, as copied, is without it.
        assert_eq!(text_map(&document).units[0].text, "Hello you there");
    }

    #[test]
    fn mentions_link_to_their_user() {
        let url = mention_url("@nils:server.org");
        assert_eq!(url, "https://matrix.to/#/@nils:server.org");
        assert_eq!(
            mentioned_user(&url).as_deref().map(|u| u.as_str()),
            Some("@nils:server.org")
        );
        // Rooms and other links open as before.
        assert_eq!(mentioned_user("https://matrix.to/#/!room:server.org"), None);
        assert_eq!(mentioned_user("https://example.org/@nils"), None);
        // A mention is one leaf of text, with its link, as other links.
        let document = Document::from_html(
            r#"Ask <a href="https://matrix.to/#/@nils:server.org">Nils</a> or <a href="https://x.org">here</a>"#,
        );
        let map = text_map(&document);
        assert_eq!(map.units[0].text, "Ask Nils or here");
        let links: Vec<_> = map.leaves.iter().map(|l| l.link.as_deref()).collect();
        assert_eq!(
            links,
            [None, Some(url.as_str()), None, Some("https://x.org")]
        );
    }

    #[test]
    fn units_follow_the_blocks_as_shown() {
        let document = Document::from_html(
            "<p>Hello <b>you</b><br>there</p><blockquote><p>quoted</p></blockquote>\
             <ol><li>one</li><li><p>two</p><p>more</p></li></ol><pre><code>let x = 1;\n</code></pre>",
        );
        let units = text_map(&document).units;
        let texts: Vec<_> = units.iter().map(|u| u.text.as_str()).collect();
        assert_eq!(
            texts,
            [
                "Hello you\nthere",
                "quoted",
                "one",
                "two",
                "more",
                "let x = 1;"
            ]
        );
        assert_eq!(
            units_text(&units, Pos::default(), Pos::END),
            "Hello you\nthere\n\n> quoted\n\n1. one\n2. two\n   more\n\nlet x = 1;"
        );
    }
}
