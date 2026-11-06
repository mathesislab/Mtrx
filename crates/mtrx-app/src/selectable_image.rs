//! A typeset message (an image) whose text can be selected: the renderer
//! says where each piece of text is drawn ([`mtrx_format::Piece`]), mapped
//! here onto the message's units ([`crate::message::TextMap`]). Formulas are
//! pieces selected whole, and copy as their source.

use std::{ops::Range, rc::Rc, sync::Arc};

use iced::{
    Element, Event, Length, Point, Rectangle, Renderer, Size, Theme, Vector,
    advanced::{
        Clipboard, Layout, Shell, Widget,
        layout::{Limits, Node},
        mouse, overlay, renderer,
        widget::{Id, Operation, Tree, tree},
    },
    keyboard,
};
use unicode_segmentation::UnicodeSegmentation;

use crate::{
    find::{self, Found},
    link_status,
    message::TextMap,
    selectable::geometry::CLICK_SLOP,
    selection::{Gesture, Pos, unit_range},
    style,
};

/// A piece of text in the image: its bytes in a unit, and where it is drawn
/// in logical pixels.
#[derive(Clone, Debug, PartialEq)]
pub struct Piece {
    pub unit: usize,
    pub range: Range<usize>,
    /// A formula, only selected whole.
    pub whole: bool,
    pub rect: Rectangle,
    /// The link it is in, if any.
    pub link: Option<String>,
}

/// What the image shows, for selecting: its pieces in reading order, and
/// the text of each unit.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Text {
    pub pieces: Vec<Piece>,
    pub units: Vec<String>,
}

impl Text {
    /// `pieces` from the renderer (in pixels, `scale` per logical pixel)
    /// placed in the message's units.
    pub fn new(pieces: &[mtrx_format::Piece], map: &TextMap, scale: f32) -> Self {
        let pieces = pieces
            .iter()
            .filter_map(|piece| {
                let leaf = map.leaves.get(piece.leaf)?;
                let range = match &piece.range {
                    Some(range) if range.end <= leaf.len => {
                        leaf.start + range.start..leaf.start + range.end
                    }
                    Some(_) => return None,
                    None => leaf.start..leaf.start + leaf.len,
                };
                let [x, y, width, height] = piece.rect.map(|v| v / scale);
                Some(Piece {
                    unit: leaf.unit,
                    range,
                    whole: piece.range.is_none(),
                    rect: Rectangle::new(Point::new(x, y), Size::new(width, height)),
                    link: leaf.link.clone(),
                })
            })
            .collect();
        let units = map.units.iter().map(|u| u.text.clone()).collect();
        Self { pieces, units }
    }

    /// The place nearest to `point`: on the line under it (else the nearest
    /// line), before or after the piece there.
    fn hit(&self, point: Point) -> Option<Pos> {
        let nearest = self.pieces.iter().min_by(|a, b| {
            vertical_distance(&a.rect, point.y).total_cmp(&vertical_distance(&b.rect, point.y))
        })?;
        let line: Vec<&Piece> = self
            .pieces
            .iter()
            .filter(|p| overlap(&p.rect, &nearest.rect))
            .collect();
        let piece = line
            .iter()
            .find(|p| point.x >= p.rect.x && point.x < p.rect.x + p.rect.width)
            .or_else(|| {
                line.iter().min_by(|a, b| {
                    horizontal_distance(&a.rect, point.x)
                        .total_cmp(&horizontal_distance(&b.rect, point.x))
                })
            })?;
        let before = point.x < piece.rect.center_x();
        Some(Pos {
            unit: piece.unit,
            offset: if before {
                piece.range.start
            } else {
                piece.range.end
            },
        })
    }

    /// The link under `point`, if any.
    fn link_at(&self, point: Point) -> Option<&str> {
        self.pieces
            .iter()
            .find(|p| p.rect.contains(point))
            .and_then(|p| p.link.as_deref())
    }

    /// The word at `pos`, or the formula there.
    fn word_at(&self, pos: Pos) -> Range<usize> {
        if let Some(piece) = self.pieces.iter().find(|p| {
            p.whole && p.unit == pos.unit && (p.range.start..=p.range.end).contains(&pos.offset)
        }) {
            return piece.range.clone();
        }
        let text = self.units.get(pos.unit).map_or("", String::as_str);
        text.split_word_bound_indices()
            .map(|(start, word)| start..start + word.len())
            .find(|word| word.contains(&pos.offset) || word.end == pos.offset)
            .unwrap_or(pos.offset..pos.offset)
    }

    /// Rectangles of the pieces in the selected part.
    fn highlight(&self, selected: (Pos, Pos)) -> impl Iterator<Item = Rectangle> + '_ {
        self.pieces.iter().filter_map(move |piece| {
            let len = self.units.get(piece.unit)?.len();
            let range = unit_range(selected, piece.unit, len)?;
            (range.start < piece.range.end && piece.range.start < range.end).then_some(piece.rect)
        })
    }
}

fn vertical_distance(rect: &Rectangle, y: f32) -> f32 {
    if y < rect.y {
        rect.y - y
    } else if y > rect.y + rect.height {
        y - rect.y - rect.height
    } else {
        0.0
    }
}

fn horizontal_distance(rect: &Rectangle, x: f32) -> f32 {
    if x < rect.x {
        rect.x - x
    } else if x > rect.x + rect.width {
        x - rect.x - rect.width
    } else {
        0.0
    }
}

fn overlap(a: &Rectangle, b: &Rectangle) -> bool {
    a.y < b.y + b.height && b.y < a.y + a.height
}

pub struct SelectableImage<'a, Message> {
    image: Element<'a, Message>,
    text: Arc<Text>,
    selected: Option<(Pos, Pos)>,
    /// Matches of the find bar.
    found: Vec<Found>,
    dragging: bool,
    on_link: Option<fn(String) -> Message>,
    on_link_menu: Option<Rc<dyn Fn(String) -> Message + 'a>>,
    on_gesture: Option<Rc<dyn Fn(Gesture) -> Message + 'a>>,
}

/// `image` showing `text`.
pub fn selectable_image<'a, Message>(
    image: impl Into<Element<'a, Message>>,
    text: Arc<Text>,
) -> SelectableImage<'a, Message> {
    SelectableImage {
        image: image.into(),
        text,
        selected: None,
        found: Vec::new(),
        dragging: false,
        on_link: None,
        on_link_menu: None,
        on_gesture: None,
    }
}

/// The part of a message a match covers.
fn found_part(found: &Found) -> (Pos, Pos) {
    let at = |offset| Pos {
        unit: found.unit,
        offset,
    };
    (at(found.range.start), at(found.range.end))
}

impl<'a, Message> SelectableImage<'a, Message> {
    /// The part of the message that is selected.
    pub fn selected(mut self, selected: Option<(Pos, Pos)>) -> Self {
        self.selected = selected;
        self
    }

    /// Matches of the find bar, drawn over the pieces they are in.
    pub fn found(mut self, found: Vec<Found>) -> Self {
        self.found = found;
        self
    }

    pub fn dragging(mut self, dragging: bool) -> Self {
        self.dragging = dragging;
        self
    }

    /// Sent when a link is clicked (pressed and released without moving).
    pub fn on_link(mut self, on_link: fn(String) -> Message) -> Self {
        self.on_link = Some(on_link);
        self
    }

    /// Sent when a link is right-clicked, for its menu.
    pub fn on_link_menu(mut self, on_link_menu: Rc<dyn Fn(String) -> Message + 'a>) -> Self {
        self.on_link_menu = Some(on_link_menu);
        self
    }

    pub fn on_gesture(mut self, on_gesture: Rc<dyn Fn(Gesture) -> Message + 'a>) -> Self {
        self.on_gesture = Some(on_gesture);
        self
    }
}

#[derive(Default)]
struct State {
    last_click: Option<mouse::Click>,
    modifiers: keyboard::Modifiers,
    /// The link pressed, and where.
    link_pressed: Option<(String, Point)>,
    hovered_link: Option<String>,
    /// The hovered link's address in the window's corner.
    link_status: link_status::Status,
}

impl<Message> Widget<Message, Theme, Renderer> for SelectableImage<'_, Message> {
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<State>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(State::default())
    }

    fn children(&self) -> Vec<Tree> {
        vec![Tree::new(&self.image)]
    }

    fn diff(&self, tree: &mut Tree) {
        tree.diff_children(&[self.image.as_widget()]);
    }

    fn size(&self) -> Size<Length> {
        self.image.as_widget().size()
    }

    fn layout(&mut self, tree: &mut Tree, renderer: &Renderer, limits: &Limits) -> Node {
        self.image
            .as_widget_mut()
            .layout(&mut tree.children[0], renderer, limits)
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut Renderer,
        theme: &Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        use iced::advanced::Renderer as _;
        let bounds = layout.bounds();
        let translation = bounds.position() - Point::ORIGIN;
        for found in &self.found {
            let (color, border) = style::found(theme, found.current);
            for rect in self.text.highlight(found_part(found)) {
                renderer.fill_quad(
                    renderer::Quad {
                        bounds: rect + translation,
                        border,
                        ..Default::default()
                    },
                    color,
                );
            }
        }
        if let Some(selected) = self.selected {
            for rect in self.text.highlight(selected) {
                renderer.fill_quad(
                    renderer::Quad {
                        bounds: rect + translation,
                        ..Default::default()
                    },
                    style::selection(theme),
                );
            }
        }
        self.image.as_widget().draw(
            &tree.children[0],
            renderer,
            theme,
            style,
            layout,
            cursor,
            viewport,
        );
    }

    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        _renderer: &Renderer,
        _clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
        _viewport: &Rectangle,
    ) {
        let state = tree.state.downcast_mut::<State>();
        let bounds = layout.bounds();
        let origin = bounds.position() - Point::ORIGIN;
        let hovered_link = cursor
            .position_in(bounds)
            .and_then(|at| self.text.link_at(at));
        if hovered_link != state.hovered_link.as_deref() {
            state.hovered_link = hovered_link.map(str::to_owned);
            shell.request_redraw();
        }
        match event {
            Event::Keyboard(keyboard::Event::ModifiersChanged(modifiers)) => {
                state.modifiers = *modifiers;
            }
            // The link's menu rather than whatever the image is in offers.
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Right)) => {
                if let Some(on_link_menu) = &self.on_link_menu
                    && let Some(link) = hovered_link
                {
                    shell.publish(on_link_menu(link.to_owned()));
                    shell.capture_event();
                }
            }
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                let Some(at) = cursor.position_in(bounds) else {
                    return;
                };
                let position = cursor.position().unwrap_or_default();
                state.link_pressed =
                    (self.text.link_at(at)).map(|link| (link.to_owned(), position));
                let Some(on_gesture) = &self.on_gesture else {
                    if state.link_pressed.is_some() {
                        shell.capture_event();
                    }
                    return;
                };
                let Some(pos) = self.text.hit(at) else {
                    return;
                };
                let click = mouse::Click::new(position, mouse::Button::Left, state.last_click);
                state.last_click = Some(click);
                let (unit, from, to) = match click.kind() {
                    mouse::click::Kind::Single => (pos.unit, pos.offset, pos.offset),
                    mouse::click::Kind::Double => {
                        let word = self.text.word_at(pos);
                        (pos.unit, word.start, word.end)
                    }
                    mouse::click::Kind::Triple => {
                        let len = self.text.units.get(pos.unit).map_or(0, String::len);
                        (pos.unit, 0, len)
                    }
                };
                shell.publish(on_gesture(Gesture::Press {
                    unit,
                    from,
                    to,
                    extend: state.modifiers.shift(),
                }));
                shell.capture_event();
            }
            Event::Mouse(mouse::Event::CursorMoved { .. }) => {
                // The cursor: inside a scrollable, it is in the content's
                // coordinates.
                let Some(position) = cursor.land().position() else {
                    return;
                };
                if let Some((_, pressed)) = &state.link_pressed
                    && pressed.distance(position) > CLICK_SLOP
                {
                    state.link_pressed = None;
                }
                let beside = position.y >= bounds.y && position.y < bounds.y + bounds.height;
                if self.dragging
                    && beside
                    && let Some(on_gesture) = &self.on_gesture
                    && let Some(pos) = self.text.hit(position - origin)
                {
                    shell.publish(on_gesture(Gesture::Drag {
                        unit: pos.unit,
                        offset: pos.offset,
                    }));
                }
            }
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
                if let Some((link, _)) = state.link_pressed.take()
                    && let Some(on_link) = self.on_link
                    && cursor
                        .position_in(bounds)
                        .and_then(|at| self.text.link_at(at))
                        == Some(link.as_str())
                {
                    shell.publish(on_link(link));
                }
            }
            _ => {}
        }
    }

    fn operate(
        &mut self,
        _tree: &mut Tree,
        layout: Layout<'_>,
        _renderer: &Renderer,
        operation: &mut dyn Operation,
    ) {
        let Some(current) = self.found.iter().find(|found| found.current) else {
            return;
        };
        // The pieces it is in, or else the whole image.
        let bounds = layout.bounds();
        let rect = (self.text.highlight(found_part(current)))
            .reduce(|a, b| a.union(&b))
            .map_or(bounds, |rect| rect + (bounds.position() - Point::ORIGIN));
        operation.custom(Some(&Id::new(find::CURRENT)), rect, &mut ());
    }

    /// The address of the hovered link, if it can be clicked.
    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut Tree,
        _layout: Layout<'b>,
        _renderer: &Renderer,
        _viewport: &Rectangle,
        _translation: Vector,
    ) -> Option<overlay::Element<'b, Message, Theme, Renderer>> {
        let State {
            hovered_link,
            link_status,
            ..
        } = tree.state.downcast_mut::<State>();
        let url = hovered_link.as_deref().filter(|_| self.on_link.is_some())?;
        Some(link_status::overlay(url, link_status))
    }

    fn mouse_interaction(
        &self,
        _tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        _viewport: &Rectangle,
        _renderer: &Renderer,
    ) -> mouse::Interaction {
        let bounds = layout.bounds();
        let over_link = || {
            cursor
                .position_in(bounds)
                .and_then(|at| self.text.link_at(at))
                .is_some()
        };
        if self.on_link.is_some() && over_link() {
            mouse::Interaction::Pointer
        } else if self.on_gesture.is_some() && (self.dragging || cursor.is_over(bounds)) {
            mouse::Interaction::Text
        } else {
            mouse::Interaction::None
        }
    }
}

impl<'a, Message: 'a> From<SelectableImage<'a, Message>> for Element<'a, Message> {
    fn from(widget: SelectableImage<'a, Message>) -> Self {
        Element::new(widget)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn piece(unit: usize, range: Range<usize>, whole: bool, x: f32, y: f32, w: f32) -> Piece {
        Piece {
            unit,
            range,
            whole,
            rect: Rectangle::new(Point::new(x, y), Size::new(w, 20.0)),
            link: None,
        }
    }

    /// "Rate $m$ ok" on one line, "Next" below.
    fn text() -> Text {
        Text {
            pieces: vec![
                piece(0, 0..1, false, 0.0, 0.0, 10.0),
                piece(0, 1..2, false, 10.0, 0.0, 10.0),
                piece(0, 2..3, false, 20.0, 0.0, 10.0),
                piece(0, 3..4, false, 30.0, 0.0, 10.0),
                piece(0, 4..5, false, 40.0, 0.0, 5.0),
                piece(0, 5..8, true, 45.0, -4.0, 30.0),
                piece(0, 8..9, false, 75.0, 0.0, 5.0),
                piece(0, 9..10, false, 80.0, 0.0, 10.0),
                piece(0, 10..11, false, 90.0, 0.0, 10.0),
                piece(1, 0..4, false, 0.0, 30.0, 40.0),
            ],
            units: vec!["Rate $m$ ok".into(), "Next".into()],
        }
    }

    fn pos(unit: usize, offset: usize) -> Pos {
        Pos { unit, offset }
    }

    #[test]
    fn hits_glyphs_lines_and_whole_formulas() {
        let text = text();
        // Left and right half of a glyph.
        assert_eq!(text.hit(Point::new(12.0, 10.0)), Some(pos(0, 1)));
        assert_eq!(text.hit(Point::new(18.0, 10.0)), Some(pos(0, 2)));
        // A formula is before or after, never inside.
        assert_eq!(text.hit(Point::new(50.0, 10.0)), Some(pos(0, 5)));
        assert_eq!(text.hit(Point::new(70.0, 10.0)), Some(pos(0, 8)));
        // Beside the line, and below everything.
        assert_eq!(text.hit(Point::new(500.0, 10.0)), Some(pos(0, 11)));
        assert_eq!(text.hit(Point::new(-5.0, 35.0)), Some(pos(1, 0)));
        assert_eq!(text.hit(Point::new(500.0, 400.0)), Some(pos(1, 4)));
        // Words and formulas for double clicks.
        assert_eq!(text.word_at(pos(0, 2)), 0..4);
        assert_eq!(text.word_at(pos(0, 5)), 5..8);
    }

    #[test]
    fn highlights_the_selected_pieces() {
        let text = text();
        let lit: Vec<f32> = text
            .highlight((pos(0, 3), pos(1, 2)))
            .map(|rect| rect.x)
            .collect();
        assert_eq!(lit, [30.0, 40.0, 45.0, 75.0, 80.0, 90.0, 0.0]);
    }

    #[test]
    fn code_in_a_typeset_message_is_selected_by_character() {
        use crate::math::{Math, Params, Rendered};
        let document = mtrx_format::Document::from_html(
            r#"Run <code>ls -l</code> for <span data-mx-maths="x"><code>x</code></span>"#,
        );
        let params = Params {
            max_width: 600.0,
            scale: 2.0,
            text: iced::Color::BLACK,
            link: iced::Color::BLACK,
            muted: iced::Color::BLACK,
        };
        let mut math = Math::new();
        math.render_now(&document, &params);
        let Some(Rendered::Image { text, .. }) = math.get(&Math::key(&document, &params)) else {
            panic!("not rendered");
        };
        assert_eq!(text.units, ["Run ls -l for $x$"]);
        // The code's characters are pieces of their own; the formula is one.
        let code: Vec<_> = (text.pieces.iter())
            .filter(|p| (4..9).contains(&p.range.start))
            .map(|p| (p.range.clone(), p.whole))
            .collect();
        let each: Vec<_> = (4..9).map(|i| (i..i + 1, false)).collect();
        assert_eq!(code, each);
        assert!(
            text.pieces
                .last()
                .is_some_and(|p| p.whole && p.range == (14..17))
        );
        // A double click selects a word in it.
        assert_eq!(text.word_at(pos(0, 5)), 4..6);
    }

    #[test]
    fn links_show_their_address_and_open_their_menu() {
        use iced::widget::{container, space};
        use iced_test::simulator::Simulator;

        #[derive(Clone, Debug, PartialEq)]
        enum Clicked {
            Link(String),
            LinkMenu(String),
            Menu,
            Selecting,
        }

        const URL: &str = "https://example.org/data";
        // "Next" is a link.
        let mut linked = text();
        linked.pieces[9].link = Some(URL.into());
        let linked = Arc::new(linked);
        let window = Size::new(400.0, 300.0);
        let ui = || {
            let image = selectable_image(space().width(100).height(50), linked.clone())
                .on_link(Clicked::Link)
                .on_link_menu(Rc::new(Clicked::LinkMenu))
                .on_gesture(Rc::new(|_| Clicked::Selecting));
            let view = crate::context_menu::context_menu(
                container(image).padding(10),
                None,
                Clicked::Menu,
            );
            Simulator::with_size(crate::fonts::settings(), window, view)
        };
        let move_to = |ui: &mut Simulator<'_, Clicked>, x: f32, y: f32| {
            ui.point_at((x, y));
            let _ = ui.simulate([Event::Mouse(mouse::Event::CursorMoved {
                position: Point::new(x, y),
            })]);
        };

        // The address in the corner while over the link.
        let mut hovering = ui();
        assert!(hovering.find(URL).is_err(), "not before hovering");
        move_to(&mut hovering, 20.0, 50.0);
        let bounds = hovering.find(URL).expect("shown while hovered").bounds();
        assert_eq!(bounds.x, 0.0);
        assert_eq!(bounds.y + bounds.height, window.height);
        move_to(&mut hovering, 20.0, 20.0);
        assert!(hovering.find(URL).is_err(), "gone over other text");

        // Right-clicked: the link's menu, else the message's.
        let right_click = |x, y| {
            let mut ui = ui();
            move_to(&mut ui, x, y);
            let _ = ui.simulate([Event::Mouse(mouse::Event::ButtonPressed(
                mouse::Button::Right,
            ))]);
            ui.into_messages().collect::<Vec<_>>()
        };
        assert_eq!(right_click(20.0, 50.0), [Clicked::LinkMenu(URL.into())]);
        assert_eq!(right_click(20.0, 20.0), [Clicked::Menu]);
    }

    #[test]
    fn links_open_on_a_click_and_show_a_hand() {
        use iced::{
            advanced::renderer::Headless,
            widget::{container, space},
        };
        use iced_runtime::user_interface::{Cache, State, UserInterface};
        use iced_test::simulator::Simulator;

        const URL: &str = "https://example.org";
        // "Next" is a link.
        let mut linked = text();
        linked.pieces[9].link = Some(URL.into());
        let linked = Arc::new(linked);
        // Links are messages, gestures nothing.
        let view = || -> Element<'_, Option<String>> {
            container(
                selectable_image(space().width(100).height(50), linked.clone())
                    .on_link(Some)
                    .on_gesture(Rc::new(|_| None)),
            )
            .padding(10)
            .into()
        };
        let clicked = |steps: &[(f32, f32, mouse::Event)]| -> Vec<String> {
            let mut ui =
                Simulator::with_size(crate::fonts::settings(), Size::new(200.0, 100.0), view());
            for &(x, y, event) in steps {
                ui.point_at((x, y));
                let _ = ui.simulate([Event::Mouse(event)]);
            }
            ui.into_messages().flatten().collect()
        };
        use mouse::{Button::Left, Event::*};
        let moved = |x, y| CursorMoved {
            position: Point::new(x, y),
        };
        // Pressed and released on it.
        assert_eq!(
            clicked(&[
                (20.0, 50.0, ButtonPressed(Left)),
                (21.0, 50.0, ButtonReleased(Left))
            ]),
            [URL]
        );
        // Dragged from it, or clicked beside it: no link.
        let dragged = [
            (20.0, 50.0, ButtonPressed(Left)),
            (60.0, 50.0, moved(60.0, 50.0)),
            (20.0, 50.0, ButtonReleased(Left)),
        ];
        assert!(clicked(&dragged).is_empty());
        let beside = [
            (20.0, 20.0, ButtonPressed(Left)),
            (20.0, 20.0, ButtonReleased(Left)),
        ];
        assert!(clicked(&beside).is_empty());

        // A pointing hand over the link, the text cursor elsewhere.
        let mut renderer = iced::futures::executor::block_on(iced::Renderer::new(
            crate::fonts::INTER,
            iced::Pixels(style::TEXT_SIZE),
            None,
        ))
        .unwrap();
        let mut ui = UserInterface::build(
            view(),
            Size::new(200.0, 100.0),
            Cache::default(),
            &mut renderer,
        );
        let mut over = |x, y| {
            let (state, _) = ui.update(
                &[],
                mouse::Cursor::Available(Point::new(x, y)),
                &mut renderer,
                &mut iced::advanced::clipboard::Null,
                &mut Vec::new(),
            );
            match state {
                State::Updated {
                    mouse_interaction, ..
                } => mouse_interaction,
                State::Outdated => panic!("outdated"),
            }
        };
        assert_eq!(over(20.0, 50.0), mouse::Interaction::Pointer);
        assert_eq!(over(20.0, 20.0), mouse::Interaction::Text);
    }
}
