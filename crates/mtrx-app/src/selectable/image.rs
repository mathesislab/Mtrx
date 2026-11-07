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
        widget::{Operation, Tree, tree},
    },
};

use super::geometry::{self, Geometry, Handlers, Pointer};
use crate::{
    find::Found,
    message::TextMap,
    selection::{Gesture, Pos, unit_range},
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
}

/// Its paragraphs are its units; it steps a line up or down by paragraph
/// (see `docs/text-selection.md`), so it has no lines of its own.
impl Geometry for Text {
    /// On the line under `point` (else the nearest line), before or after
    /// the piece there.
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

    fn link_at(&self, point: Point) -> Option<&str> {
        self.pieces
            .iter()
            .find(|p| p.rect.contains(point))
            .and_then(|p| p.link.as_deref())
    }

    fn word_at(&self, pos: Pos) -> Range<usize> {
        let formula = self.pieces.iter().find(|p| {
            p.whole && p.unit == pos.unit && (p.range.start..=p.range.end).contains(&pos.offset)
        });
        match formula {
            Some(formula) => formula.range.clone(),
            None => geometry::word_at(
                self.units.get(pos.unit).map_or("", String::as_str),
                pos.offset,
            ),
        }
    }

    fn unit_len(&self, unit: usize) -> usize {
        self.units.get(unit).map_or(0, String::len)
    }

    /// The rectangles of the pieces in it.
    fn highlight(&self, from: Pos, to: Pos) -> Vec<Rectangle> {
        (self.pieces.iter())
            .filter(|piece| {
                unit_range((from, to), piece.unit, self.unit_len(piece.unit)).is_some_and(|range| {
                    range.start < piece.range.end && piece.range.start < range.end
                })
            })
            .map(|piece| piece.rect)
            .collect()
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
    handlers: Handlers<'a, Message>,
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
        handlers: Handlers::default(),
    }
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

    /// A selection is being dragged: report where the cursor is.
    pub fn dragging(mut self, dragging: bool) -> Self {
        self.handlers.dragging = dragging;
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

    pub fn on_gesture(mut self, on_gesture: Rc<dyn Fn(Gesture) -> Message + 'a>) -> Self {
        self.handlers.on_gesture = Some(on_gesture);
        self
    }
}

impl<Message> Widget<Message, Theme, Renderer> for SelectableImage<'_, Message> {
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<Pointer>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(Pointer::default())
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
        let at = layout.bounds().position() - Point::ORIGIN;
        let text = &*self.text;
        geometry::draw_highlights(renderer, theme, text, self.selected, &self.found, at);
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
        let pointer = tree.state.downcast_mut::<Pointer>();
        let text = &*self.text;
        pointer.update(text, &self.handlers, event, layout.bounds(), cursor, shell);
    }

    fn operate(
        &mut self,
        _tree: &mut Tree,
        layout: Layout<'_>,
        _renderer: &Renderer,
        operation: &mut dyn Operation,
    ) {
        geometry::report_found(operation, &*self.text, &self.found, layout.bounds());
    }

    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut Tree,
        _layout: Layout<'b>,
        _renderer: &Renderer,
        _viewport: &Rectangle,
        _translation: Vector,
    ) -> Option<overlay::Element<'b, Message, Theme, Renderer>> {
        tree.state.downcast_mut::<Pointer>().overlay(&self.handlers)
    }

    fn mouse_interaction(
        &self,
        _tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        _viewport: &Rectangle,
        _renderer: &Renderer,
    ) -> mouse::Interaction {
        geometry::interaction(&*self.text, &self.handlers, cursor, layout.bounds())
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
    use crate::style;

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
        // Just after a word, and at a word's start after a space.
        assert_eq!(text.word_at(pos(0, 4)), 0..4);
        assert_eq!(text.word_at(pos(0, 9)), 9..11);
    }

    #[test]
    fn highlights_the_selected_pieces() {
        let text = text();
        let lit: Vec<f32> = text
            .highlight(pos(0, 3), pos(1, 2))
            .iter()
            .map(|rect| rect.x)
            .collect();
        assert_eq!(lit, [30.0, 40.0, 45.0, 75.0, 80.0, 90.0, 0.0]);
    }

    #[test]
    fn code_in_a_typeset_message_is_selected_by_character() {
        use crate::math::{Math, Params, Rendered, Request};
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
        math.render_now(Request {
            document: document.clone(),
            params,
        });
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
