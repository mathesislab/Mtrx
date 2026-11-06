//! Rich text whose text can be selected, like iced's `rich_text` (whose
//! layout and drawing it follows) plus hit-testing and highlighting through
//! the paragraph's cosmic-text layout. In the timeline the selection lives
//! in the room ([`crate::selection`]): the widget reports gestures and draws
//! the part it is given, and the find bar's matches ([`crate::find`]) the
//! same way. Elsewhere ([`selectable_text`]) it keeps its own
//! selection and copies it with ⌘C.

use std::{any::Any, borrow::Cow, ops::Range, rc::Rc};

use iced::{
    Color, Element, Event, Font, Length, Pixels, Point, Rectangle, Renderer, Size, Task, Theme,
    Vector,
    advanced::{
        Clipboard, Shell, Widget, clipboard, layout, mouse, overlay, renderer,
        text::{self as core_text, Paragraph as _, Span},
        widget::{
            Id, Operation, operate,
            operation::Outcome,
            text as core_widget_text,
            tree::{self, Tree},
        },
    },
    alignment, keyboard,
    widget::text::{LineHeight, Shaping, Wrapping},
};
use iced_graphics::text::{
    Paragraph,
    cosmic_text::{Buffer, Cursor},
};

use super::geometry::{self, Geometry, Handlers, Pointer, Press};
use crate::{
    ellipsis,
    find::Found,
    selection::{Gesture, Pos, unit_range},
};

/// The id of the unit holding the end of the selection, for [`line_step`].
const FOCUS: &str = "selection-focus";

pub struct Selectable<'a, Message> {
    spans: Vec<Span<'static, String, Font>>,
    /// The bytes of `spans` that are its text, when spans after them are
    /// only shown (see [`Selectable::after`]).
    text_len: Option<usize>,
    /// Its index among the message's units.
    unit: usize,
    size: Option<Pixels>,
    /// The selected part of the message.
    selected: Option<(Pos, Pos)>,
    /// Matches of the find bar in its text.
    found: Vec<Found>,
    /// The selection ends in it.
    focus: bool,
    handlers: Handlers<'a, Message>,
    /// Keeps its own selection, not the room's.
    own: bool,
    wrapping: Wrapping,
    style: Option<fn(&Theme) -> core_widget_text::Style>,
    /// At most this many lines, ending in "…" when cut.
    lines: Option<usize>,
    /// Reports the text shown (see [`Selectable::id`]).
    id: Option<Id>,
}

/// Text with its own selection, copied with ⌘C, e.g. in info cards.
pub fn selectable_text<'a, Message>(content: impl Into<String>) -> Selectable<'a, Message> {
    Selectable {
        own: true,
        ..selectable(vec![Span::new(content.into())], 0)
    }
}

/// Unit `unit` of a message, showing `spans`.
pub fn selectable<'a, Message>(
    spans: Vec<Span<'static, String, Font>>,
    unit: usize,
) -> Selectable<'a, Message> {
    Selectable {
        spans,
        text_len: None,
        unit,
        size: None,
        selected: None,
        found: Vec::new(),
        focus: false,
        handlers: Handlers::default(),
        own: false,
        wrapping: Wrapping::default(),
        style: None,
        lines: None,
        id: None,
    }
}

impl<'a, Message> Selectable<'a, Message> {
    pub fn size(mut self, size: impl Into<Pixels>) -> Self {
        self.size = Some(size.into());
        self
    }

    /// The font of all its text.
    pub fn font(mut self, font: Font) -> Self {
        for span in &mut self.spans {
            span.font = Some(font);
        }
        self
    }

    /// Shown after its text, without being part of it: never selected or
    /// copied, e.g. the "(edited)" of a message.
    pub fn after(mut self, span: Span<'static, String, Font>) -> Self {
        let len = self.spans.iter().map(|s| s.text.len()).sum();
        self.text_len.get_or_insert(len);
        self.spans.push(span);
        self
    }

    pub fn wrapping(mut self, wrapping: Wrapping) -> Self {
        self.wrapping = wrapping;
        self
    }

    /// The colour of text without its own.
    pub fn style(mut self, style: fn(&Theme) -> core_widget_text::Style) -> Self {
        self.style = Some(style);
        self
    }

    /// Sent when a link is clicked (pressed and released without moving).
    pub fn on_link(mut self, on_link: fn(String) -> Message) -> Self {
        self.handlers.on_link = Some(on_link);
        self
    }

    /// Sent when a link is right-clicked, for its menu.
    pub fn on_link_menu(mut self, on_link_menu: Rc<dyn Fn(String) -> Message + 'a>) -> Self {
        self.handlers.on_link_menu = Some(on_link_menu);
        self
    }

    /// The part of the message that is selected.
    pub fn selected(mut self, selected: Option<(Pos, Pos)>) -> Self {
        self.selected = selected;
        self
    }

    /// Matches of the find bar in its text.
    pub fn found(mut self, found: Vec<Found>) -> Self {
        self.found = found;
        self
    }

    /// The selection ends in it, so Shift with ↑ or ↓ moves from it.
    pub fn focus(mut self, focus: bool) -> Self {
        self.focus = focus;
        self
    }

    /// A selection is being dragged: report where the cursor is.
    pub fn dragging(mut self, dragging: bool) -> Self {
        self.handlers.dragging = dragging;
        self
    }

    pub fn on_gesture(mut self, on_gesture: Rc<dyn Fn(Gesture) -> Message + 'a>) -> Self {
        self.handlers.on_gesture = Some(on_gesture);
        self
    }

    /// Sent when it is clicked, as a link is: pressed and released without
    /// moving. Dragging still selects.
    pub fn on_click(mut self, message: Message) -> Self
    where
        Message: Clone + 'a,
    {
        self.handlers.on_click = Some(Rc::new(move || message.clone()));
        self
    }

    /// At most `lines` lines, the text cut to end in "…" when it does not
    /// fit, as [`crate::ellipsis`] does; for text of one span. What is
    /// shown is then its text.
    pub fn lines(mut self, lines: usize) -> Self {
        self.lines = Some(lines.max(1));
        self
    }

    /// Reports the text it shows to [`Operation::text`] under `id`, e.g.
    /// as cut to fit.
    pub fn id(mut self, id: Id) -> Self {
        self.id = Some(id);
        self
    }
}

#[derive(Default)]
struct State {
    layout: Layout,
    pointer: Pointer,
    /// With its own selection: where it starts and ends, and whether it is
    /// being dragged.
    own: Option<(usize, usize)>,
    own_dragging: bool,
}

/// Its text as laid out, a unit of a message.
#[derive(Default)]
struct Layout {
    unit: usize,
    /// The spans given, and the width their text was cut to fit.
    spans: Vec<Span<'static, String, Font>>,
    fitted_width: Option<f32>,
    text_len: Option<usize>,
    paragraph: Paragraph,
    /// Its text as shown (the spans' text but what is only shown after it),
    /// and where each line of the spans' text starts.
    text: String,
    line_starts: Vec<usize>,
    /// The size it is laid out in, and of its text.
    size: Size,
    font_size: f32,
}

type Spans = [Span<'static, String, Font>];

/// `text`, which lays out the spans it is given (its type said here, as a
/// closure cannot say its result borrows from its argument).
fn laid_out<F>(text: F) -> F
where
    F: for<'s> Fn(&'s Spans) -> core_text::Text<&'s Spans>,
{
    text
}

/// `spans`, or if they do not `fit`, their text cut to end in "…", styled as
/// the first span.
fn cut(
    spans: &[Span<'static, String, Font>],
    fits: impl Fn(&[Span<'static, String, Font>]) -> bool,
) -> Vec<Span<'static, String, Font>> {
    let Some(first) = spans.first() else {
        return Vec::new();
    };
    let span = |text: &str| Span {
        text: text.to_owned().into(),
        ..first.clone()
    };
    let full: String = spans.iter().map(|s| s.text.as_ref()).collect();
    match ellipsis::shorten(&full, |text| fits(&[span(text)])) {
        Cow::Borrowed(_) => spans.to_vec(),
        Cow::Owned(text) => vec![span(&text)],
    }
}

impl State {
    /// Its own selection, if not empty.
    fn own_range(&self) -> Option<Range<usize>> {
        let (anchor, focus) = self.own?;
        (anchor != focus).then(|| anchor.min(focus)..anchor.max(focus))
    }
}

impl Layout {
    /// Laid out as `shown`, the `spans` given (cut to fit, say).
    fn set_spans(
        &mut self,
        spans: &[Span<'static, String, Font>],
        shown: &[Span<'static, String, Font>],
        text_len: Option<usize>,
    ) {
        self.spans = spans.to_vec();
        self.text_len = text_len;
        let mut shown: String = shown.iter().map(|s| s.text.as_ref()).collect();
        self.line_starts = std::iter::once(0)
            .chain(shown.match_indices('\n').map(|(i, _)| i + 1))
            .collect();
        if let Some(len) = text_len {
            shown.truncate(len);
        }
        self.text = shown;
    }

    fn buffer(&self) -> &Buffer {
        self.paragraph.buffer()
    }

    /// `offset` in its text.
    fn at(&self, offset: usize) -> Pos {
        Pos {
            unit: self.unit,
            offset,
        }
    }

    /// The text offset nearest to `point` (relative to the text).
    fn offset_at(&self, point: Point) -> usize {
        let x = point.x.clamp(0.0, self.size.width);
        let y = point.y.clamp(0.0, (self.size.height - 0.5).max(0.0));
        match self.buffer().hit(x, y) {
            Some(cursor) => (self.line_starts.get(cursor.line).copied().unwrap_or(0)
                + cursor.index)
                .min(self.text.len()),
            None if point.y <= 0.0 => 0,
            None => self.text.len(),
        }
    }

    /// Rectangles covering `range` of its text.
    fn highlight_range(&self, range: &Range<usize>) -> Vec<Rectangle> {
        let runs: Vec<_> = self.buffer().layout_runs().collect();
        let mut rects = Vec::new();
        for (i, run) in runs.iter().enumerate() {
            let start = self.line_starts.get(run.line_i).copied().unwrap_or(0);
            let len = run.text.len();
            if range.end <= start || range.start > start + len {
                continue;
            }
            let from = range.start.saturating_sub(start).min(len);
            let to = (range.end - start).min(len);
            let line_ends = runs.get(i + 1).is_none_or(|next| next.line_i != run.line_i);
            // The line break is selected too: show a little of it.
            let newline = line_ends && range.end > start + len && start + len < self.text.len();
            let Some((x, width)) = run
                .highlight(Cursor::new(run.line_i, from), Cursor::new(run.line_i, to))
                .or_else(|| newline.then_some((run.line_w, 0.0)))
            else {
                continue;
            };
            let width = if newline {
                width + self.font_size * 0.3
            } else {
                width
            };
            rects.push(Rectangle::new(
                Point::new(x, run.line_top),
                Size::new(width, run.line_height),
            ));
        }
        rects
    }
}

/// A paragraph: one unit.
impl Geometry for Layout {
    fn hit(&self, point: Point) -> Option<Pos> {
        Some(self.at(self.offset_at(point)))
    }

    fn link_at(&self, point: Point) -> Option<&str> {
        let span = self.paragraph.hit_span(point)?;
        self.spans.get(span)?.link.as_deref()
    }

    fn word_at(&self, pos: Pos) -> Range<usize> {
        geometry::word_at(&self.text, pos.offset)
    }

    fn unit_len(&self, _unit: usize) -> usize {
        self.text.len()
    }

    fn highlight(&self, from: Pos, to: Pos) -> Vec<Rectangle> {
        unit_range((from, to), self.unit, self.text.len())
            .map_or_else(Vec::new, |range| self.highlight_range(&range))
    }

    fn line_step(&self, pos: Pos, up: bool) -> Option<Pos> {
        let runs: Vec<_> = self.buffer().layout_runs().collect();
        let (at, x) = runs.iter().enumerate().find_map(|(i, run)| {
            let start = self.line_starts.get(run.line_i).copied().unwrap_or(0);
            let local = pos.offset.checked_sub(start)?;
            let (from, to) = match (run.glyphs.first(), run.glyphs.last()) {
                (Some(first), Some(last)) => (first.start, last.end),
                // An empty line.
                _ => (0, 0),
            };
            (from..=to).contains(&local).then(|| {
                let x = (run.glyphs.iter())
                    .find(|glyph| glyph.start >= local)
                    .map_or(run.line_w, |glyph| glyph.x);
                (i, x)
            })
        })?;
        let run = runs.get(if up { at.checked_sub(1)? } else { at + 1 })?;
        self.hit(Point::new(x, run.line_top + run.line_height / 2.0))
    }

    /// An offset where one line ends and the next starts (a word broken
    /// across them) is the end of the first going forward and the start of
    /// the second going back, so pressing again stays.
    fn line_edge(&self, pos: Pos, forward: bool) -> Option<Pos> {
        let mut lines = self.buffer().layout_runs().filter_map(|run| {
            let start = self.line_starts.get(run.line_i).copied().unwrap_or(0);
            // In either direction of writing.
            let from = run.glyphs.iter().map(|g| g.start).min().unwrap_or(0);
            let to = run.glyphs.iter().map(|g| g.end).max().unwrap_or(0);
            let (from, to) = (start + from, start + to);
            (from..=to).contains(&pos.offset).then_some((from, to))
        });
        let offset = if forward {
            lines.next().map(|(_, to)| to)
        } else {
            lines.last().map(|(from, _)| from)
        };
        offset.map(|offset| self.at(offset))
    }
}

impl<Message> Widget<Message, Theme, Renderer> for Selectable<'_, Message> {
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<State>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(State::default())
    }

    fn size(&self) -> Size<Length> {
        Size::new(Length::Shrink, Length::Shrink)
    }

    fn layout(
        &mut self,
        tree: &mut Tree,
        renderer: &Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        use iced::advanced::text::Renderer as _;
        let laid = &mut tree.state.downcast_mut::<State>().layout;
        let size = self.size.unwrap_or_else(|| renderer.default_size());
        let node = layout::sized(limits, Length::Shrink, Length::Shrink, |limits| {
            let text = laid_out(|spans| core_text::Text {
                content: spans,
                bounds: limits.max(),
                size,
                line_height: LineHeight::default(),
                font: renderer.default_font(),
                align_x: core_text::Alignment::Default,
                align_y: alignment::Vertical::Top,
                shaping: Shaping::Advanced,
                wrapping: self.wrapping,
            });
            // Cut to fit again at another width.
            let changed = laid.spans != self.spans
                || laid.text_len != self.text_len
                || self.lines.is_some() && laid.fitted_width != Some(limits.max().width);
            let difference = if changed {
                core_text::Difference::Shape
            } else {
                laid.paragraph.compare(text(&[]).with_content(()))
            };
            match difference {
                core_text::Difference::None => {}
                core_text::Difference::Bounds => laid.paragraph.resize(limits.max()),
                core_text::Difference::Shape => {
                    let shown = match self.lines {
                        Some(lines) => {
                            let line = LineHeight::default().to_absolute(size).0;
                            cut(&self.spans, |spans| {
                                let height = Paragraph::with_spans(text(spans)).min_bounds().height;
                                height <= lines as f32 * line + 0.5
                            })
                        }
                        None => self.spans.clone(),
                    };
                    laid.paragraph = Paragraph::with_spans(text(&shown));
                    laid.set_spans(&self.spans, &shown, self.text_len);
                    laid.fitted_width = Some(limits.max().width);
                }
            }
            laid.paragraph.min_bounds()
        });
        laid.unit = self.unit;
        laid.size = node.size();
        laid.font_size = size.0;
        node
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut Renderer,
        theme: &Theme,
        defaults: &renderer::Style,
        layout: layout::Layout<'_>,
        _cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        use iced::advanced::{Renderer as _, text::Renderer as _};
        let bounds = layout.bounds();
        if !bounds.intersects(viewport) {
            return;
        }
        let state = tree.state.downcast_ref::<State>();
        let laid = &state.layout;
        let translation = bounds.position() - Point::ORIGIN;
        let size = self.size.unwrap_or_else(|| renderer.default_size());

        let selected = if self.own {
            (state.own_range()).map(|range| (laid.at(range.start), laid.at(range.end)))
        } else {
            self.selected
        };
        geometry::draw_highlights(renderer, theme, laid, selected, &self.found, translation);

        // Backgrounds, underlines and strikethroughs, as `rich_text` does.
        let hovered_link = state.pointer.hovered_link();
        for (index, span) in self.spans.iter().enumerate() {
            let hovered = span.link.is_some() && span.link.as_deref() == hovered_link;
            if span.highlight.is_none() && !span.underline && !span.strikethrough && !hovered {
                continue;
            }
            let regions = laid.paragraph.span_bounds(index);
            if let Some(highlight) = span.highlight {
                for region in &regions {
                    renderer.fill_quad(
                        renderer::Quad {
                            bounds: *region + translation,
                            border: highlight.border,
                            ..Default::default()
                        },
                        highlight.background,
                    );
                }
            }
            let span_size = span.size.unwrap_or(size);
            let line_height = span.line_height.unwrap_or_default().to_absolute(span_size);
            let color: Color = span.color.unwrap_or(defaults.text_color);
            let baseline =
                translation + Vector::new(0.0, span_size.0 + (line_height.0 - span_size.0) / 2.0);
            let mut line = |offset: f32| {
                for region in &regions {
                    renderer.fill_quad(
                        renderer::Quad {
                            bounds: Rectangle::new(
                                region.position() + baseline - Vector::new(0.0, offset),
                                Size::new(region.width, 1.0),
                            ),
                            ..Default::default()
                        },
                        color,
                    );
                }
            };
            if span.underline || hovered {
                line(span_size.0 * 0.08);
            }
            if span.strikethrough {
                line(span_size.0 / 2.0);
            }
        }

        core_widget_text::draw(
            renderer,
            defaults,
            bounds,
            &laid.paragraph,
            self.style
                .map_or_else(Default::default, |style| style(theme)),
            viewport,
        );
    }

    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: layout::Layout<'_>,
        cursor: mouse::Cursor,
        _renderer: &Renderer,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
        _viewport: &Rectangle,
    ) {
        let state = tree.state.downcast_mut::<State>();
        let bounds = layout.bounds();
        if self.own {
            // A menu's Copy as the key press it stands for: macOS's Edit
            // menu takes ⌘C before the window.
            let key_press;
            let event = match event {
                Event::StandardAction(action) => {
                    key_press = Event::Keyboard(action.key_press());
                    &key_press
                }
                event => event,
            };
            update_own(state, event, bounds, cursor, clipboard, shell);
        } else {
            (state.pointer).update(&state.layout, &self.handlers, event, bounds, cursor, shell);
        }
    }

    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: layout::Layout<'_>,
        _renderer: &Renderer,
        operation: &mut dyn Operation,
    ) {
        let state = tree.state.downcast_mut::<State>();
        geometry::report_found(operation, &state.layout, &self.found, layout.bounds());
        if let Some(id) = &self.id {
            operation.text(Some(id), layout.bounds(), &state.layout.text);
        }
        if self.focus {
            let laid: &mut dyn Any = &mut state.layout;
            operation.custom(Some(&Id::new(FOCUS)), layout.bounds(), laid);
        }
    }

    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut Tree,
        _layout: layout::Layout<'b>,
        _renderer: &Renderer,
        _viewport: &Rectangle,
        _translation: Vector,
    ) -> Option<overlay::Element<'b, Message, Theme, Renderer>> {
        let state = tree.state.downcast_mut::<State>();
        state.pointer.overlay(&self.handlers)
    }

    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: layout::Layout<'_>,
        cursor: mouse::Cursor,
        _viewport: &Rectangle,
        _renderer: &Renderer,
    ) -> mouse::Interaction {
        let state = tree.state.downcast_ref::<State>();
        let bounds = layout.bounds();
        if !self.own {
            geometry::interaction(&state.layout, &self.handlers, cursor, bounds)
        } else if state.own_dragging || cursor.is_over(bounds) {
            mouse::Interaction::Text
        } else {
            mouse::Interaction::None
        }
    }
}

/// Selecting by itself: pressing, dragging, and ⌘C; a press elsewhere
/// clears the selection.
fn update_own<Message>(
    state: &mut State,
    event: &Event,
    bounds: Rectangle,
    cursor: mouse::Cursor,
    clipboard: &mut dyn Clipboard,
    shell: &mut Shell<'_, Message>,
) {
    state.pointer.track_modifiers(event);
    match event {
        Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
            let Some(at) = cursor.position_in(bounds) else {
                if state.own.take().is_some() {
                    shell.request_redraw();
                }
                return;
            };
            let position = cursor.position().unwrap_or_default();
            let Some(Press { range, extend, .. }) =
                state.pointer.press(&state.layout, at, position)
            else {
                return;
            };
            state.own = match state.own {
                Some((anchor, _)) if extend => Some((anchor, range.end)),
                _ => Some((range.start, range.end)),
            };
            state.own_dragging = true;
            shell.request_redraw();
            shell.capture_event();
        }
        Event::Mouse(mouse::Event::CursorMoved { .. }) if state.own_dragging => {
            if let Some(position) = cursor.land().position()
                && let Some((anchor, _)) = state.own
            {
                let focus = state.layout.offset_at(position - translation(bounds));
                state.own = Some((anchor, focus));
                shell.request_redraw();
            }
        }
        Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
            state.own_dragging = false;
        }
        Event::Keyboard(keyboard::Event::KeyPressed { key, modifiers, .. })
            if modifiers.command() && matches!(key.as_ref(), keyboard::Key::Character("c")) =>
        {
            if let Some(range) = state.own_range() {
                clipboard.write(
                    clipboard::Kind::Standard,
                    state.layout.text[range].to_owned(),
                );
                shell.capture_event();
            }
        }
        _ => {}
    }
}

/// Where the end of the selection, at `offset` in the unit holding it,
/// moves a line up or down: `Some(None)` past the unit's first or last line,
/// `None` if no text unit holds it (e.g. in a typeset message).
pub fn line_step(offset: usize, up: bool) -> Task<Option<Option<usize>>> {
    operate(AtFocus::new(move |laid| {
        let moved = laid.line_step(laid.at(offset), up);
        moved.map(|pos| pos.offset)
    }))
}

/// Where the end of the selection, at `offset` in the unit holding it, is
/// at the start or end (`forward`) of its line; `None` if no text unit holds
/// it.
pub fn line_edge(offset: usize, forward: bool) -> Task<Option<usize>> {
    operate(AtFocus::new(move |laid| {
        let edge = laid.line_edge(laid.at(offset), forward);
        edge.map(|pos| pos.offset)
    }))
    .map(Option::flatten)
}

/// What `find` makes of the layout of the unit holding the end of the
/// selection: `None` if no text unit holds it.
struct AtFocus<T, F> {
    find: F,
    found: Option<T>,
}

impl<T, F: Fn(&Layout) -> T> AtFocus<T, F> {
    fn new(find: F) -> Self {
        Self { find, found: None }
    }
}

impl<T: Clone + Send + 'static, F: Fn(&Layout) -> T + Send + 'static> Operation<Option<T>>
    for AtFocus<T, F>
{
    fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation<Option<T>>)) {
        operate(self);
    }

    fn custom(&mut self, id: Option<&Id>, _bounds: Rectangle, state: &mut dyn Any) {
        if id == Some(&Id::new(FOCUS))
            && let Some(laid) = state.downcast_ref::<Layout>()
        {
            self.found = Some((self.find)(laid));
        }
    }

    fn finish(&self) -> Outcome<Option<T>> {
        Outcome::Some(self.found.clone())
    }
}

fn translation(bounds: Rectangle) -> Vector {
    bounds.position() - Point::ORIGIN
}

impl<'a, Message: 'a> From<Selectable<'a, Message>> for Element<'a, Message> {
    fn from(selectable: Selectable<'a, Message>) -> Self {
        Element::new(selectable)
    }
}

#[cfg(test)]
mod tests {
    use iced::{event::Status, keyboard::Key};
    use iced_test::simulator::{Simulator, press_key};

    use super::*;
    use crate::{fonts, style};

    /// ⌘C, as a key press.
    fn command_c() -> Event {
        let Event::Keyboard(keyboard::Event::KeyPressed {
            key,
            modified_key,
            physical_key,
            location,
            text,
            repeat,
            ..
        }) = press_key(Key::Character("c".into()), None)
        else {
            unreachable!()
        };
        Event::Keyboard(keyboard::Event::KeyPressed {
            key,
            modified_key,
            physical_key,
            location,
            modifiers: keyboard::Modifiers::COMMAND,
            text,
            repeat,
        })
    }

    #[test]
    fn own_selection_is_dragged_and_copied() {
        let view: Element<'_, ()> = selectable_text("Ice sheet").into();
        let mut ui = Simulator::with_size(fonts::settings(), Size::new(400.0, 100.0), view);
        // Nothing selected: ⌘C is left to others.
        assert_eq!(ui.simulate([command_c()]), [Status::Ignored]);
        ui.point_at((2.0, 8.0));
        let _ = ui.simulate([Event::Mouse(mouse::Event::ButtonPressed(
            mouse::Button::Left,
        ))]);
        ui.point_at((300.0, 8.0));
        let _ = ui.simulate([
            Event::Mouse(mouse::Event::CursorMoved {
                position: Point::new(300.0, 8.0),
            }),
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)),
        ]);
        assert_eq!(ui.simulate([command_c()]), [Status::Captured]);
        // Copy from the Edit menu, which takes ⌘C on macOS.
        assert_eq!(
            ui.simulate([Event::StandardAction(
                iced_core::event::StandardAction::Copy
            )]),
            [Status::Captured]
        );
        // A click elsewhere clears it.
        ui.point_at((350.0, 90.0));
        let _ = ui.simulate([Event::Mouse(mouse::Event::ButtonPressed(
            mouse::Button::Left,
        ))]);
        assert_eq!(ui.simulate([command_c()]), [Status::Ignored]);
    }

    #[test]
    fn a_click_does_its_action_and_a_drag_selects() {
        #[derive(Clone, Debug, PartialEq)]
        enum Did {
            Open,
            Selecting,
        }
        let clicked = |to: f32| -> Vec<Did> {
            let view: Element<'_, Did> = selectable(vec![Span::new("Alice Smith".to_owned())], 0)
                .on_click(Did::Open)
                .on_gesture(Rc::new(|_| Did::Selecting))
                .dragging(true)
                .into();
            let mut ui = Simulator::with_size(fonts::settings(), Size::new(400.0, 100.0), view);
            ui.point_at((5.0, 8.0));
            let _ = ui.simulate([Event::Mouse(mouse::Event::ButtonPressed(
                mouse::Button::Left,
            ))]);
            ui.point_at((to, 8.0));
            let _ = ui.simulate([
                Event::Mouse(mouse::Event::CursorMoved {
                    position: Point::new(to, 8.0),
                }),
                Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)),
            ]);
            ui.into_messages().filter(|did| *did == Did::Open).collect()
        };
        assert_eq!(clicked(6.0), [Did::Open]);
        assert!(clicked(40.0).is_empty(), "dragged: selecting");
    }

    #[test]
    fn cut_to_its_lines_and_reports_what_it_shows() {
        use iced::advanced::{renderer::Headless, widget::operation::black_box};
        use iced_runtime::user_interface::{Cache, UserInterface};

        /// The text reported under `SHOWN`.
        struct Shown(Option<String>);

        impl Operation for Shown {
            fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation)) {
                operate(self);
            }

            fn text(&mut self, id: Option<&Id>, _bounds: Rectangle, text: &str) {
                if id == Some(&Id::new("shown")) {
                    self.0 = Some(text.to_owned());
                }
            }
        }

        let mut renderer = iced::futures::executor::block_on(iced::Renderer::new(
            fonts::INTER,
            Pixels(style::TEXT_SIZE),
            None,
        ))
        .unwrap();
        let full = "Ice sheets flow towards the ocean, where they calve into icebergs.";
        let mut shown = |width: f32| {
            let view: Element<'_, ()> = iced::widget::container(
                selectable::<()>(vec![Span::new(full.to_owned())], 0)
                    .lines(2)
                    .id(Id::new("shown")),
            )
            .width(width)
            .into();
            let mut ui = UserInterface::build(
                view,
                Size::new(width, 400.0),
                Cache::default(),
                &mut renderer,
            );
            let mut operation = Shown(None);
            ui.operate(&renderer, &mut black_box(&mut operation));
            operation.0.expect("reported")
        };
        // Wide enough: all of it.
        assert_eq!(shown(1000.0), full);
        // Narrow: its start and "…".
        let cut = shown(120.0);
        let start = cut.strip_suffix('…').expect("ends in …");
        assert!(full.starts_with(start) && start.len() > 10, "{cut}");
    }

    #[test]
    fn hovered_link_shows_its_address_in_the_corner() {
        const URL: &str = "https://example.org/data";
        let spans = vec![
            Span::new("See ".to_owned()),
            Span::new("the data".to_owned()).link(URL.to_owned()),
        ];
        let view: Element<'_, String> = selectable(spans, 0).on_link(|url| url).into();
        let window = Size::new(400.0, 300.0);
        let mut ui = Simulator::with_size(fonts::settings(), window, view);
        let move_to = |ui: &mut Simulator<'_, String>, x: f32| {
            ui.point_at((x, 8.0));
            let _ = ui.simulate([Event::Mouse(mouse::Event::CursorMoved {
                position: Point::new(x, 8.0),
            })]);
        };
        assert!(ui.find(URL).is_err(), "not before hovering");
        move_to(&mut ui, 50.0);
        let bounds = ui.find(URL).expect("shown while hovered").bounds();
        assert_eq!(bounds.x, 0.0);
        assert_eq!(bounds.y + bounds.height, window.height);
        // Over the plain text, or beside it: gone.
        move_to(&mut ui, 5.0);
        assert!(ui.find(URL).is_err());
        move_to(&mut ui, 50.0);
        move_to(&mut ui, 350.0);
        assert!(ui.find(URL).is_err());
    }

    /// Wrapped over a few lines, the last after a line break.
    const WRAPPED: &str = "Ice sheets flow towards the ocean, where they calve.\nMelt";

    fn renderer() -> Renderer {
        use iced::advanced::renderer::Headless;
        iced::futures::executor::block_on(Renderer::new(
            fonts::INTER,
            Pixels(style::TEXT_SIZE),
            None,
        ))
        .unwrap()
    }

    /// What `find` makes of [`WRAPPED`] laid out 150 wide, as the unit
    /// holding the end of the selection.
    fn at_focus<T: Clone + Send + 'static>(
        renderer: &mut Renderer,
        find: impl Fn(&Layout) -> T + Send + 'static,
    ) -> T {
        use iced::advanced::widget::operation::black_box;
        use iced_runtime::user_interface::{Cache, UserInterface};

        let view: Element<'_, ()> = iced::widget::container(
            selectable::<()>(vec![Span::new(WRAPPED.to_owned())], 0).focus(true),
        )
        .width(150)
        .into();
        let mut ui =
            UserInterface::build(view, Size::new(150.0, 400.0), Cache::default(), renderer);
        let mut operation = AtFocus::new(find);
        ui.operate(renderer, &mut black_box(&mut operation));
        operation.found.expect("the focused unit reports itself")
    }

    #[test]
    fn shift_up_and_down_move_by_line() {
        let mut renderer = renderer();
        let mut step = |offset, up| {
            at_focus(&mut renderer, move |laid| {
                let moved = laid.line_step(laid.at(offset), up);
                moved.map(|pos| pos.offset)
            })
        };
        let text = WRAPPED;
        assert_eq!(step(2, true), None);
        let below = step(2, false).unwrap();
        assert!(below > 2 && below < text.find('\n').unwrap(), "{below}");
        assert_eq!(step(below, true), Some(2));
        let last = text.len() - 1;
        assert_eq!(step(last, false), None);
        assert!(step(last, true).unwrap() < text.find('\n').unwrap());
    }

    #[test]
    fn shift_command_left_and_right_go_to_the_line_edges() {
        let mut renderer = renderer();
        let mut edge = |offset, forward| {
            at_focus(&mut renderer, move |laid| {
                let edge = laid.line_edge(laid.at(offset), forward);
                edge.map(|pos| pos.offset)
            })
            .unwrap()
        };
        let text = WRAPPED;
        let first_end = edge(2, true);
        assert!(
            first_end > 2 && first_end < text.find('\n').unwrap(),
            "{first_end}"
        );
        assert_eq!(edge(2, false), 0);
        // Pressed again, it stays; back from there is the same line's start.
        assert_eq!(edge(first_end, true), first_end);
        assert_eq!(edge(first_end, false), 0);
        // After the line break.
        let last = text.len() - 1;
        assert_eq!(edge(last, true), text.len());
        assert_eq!(edge(last, false), text.find('\n').unwrap() + 1);
    }
}
