//! What the selectable widgets share: hit-testing and highlighting of their
//! laid-out text ([`Geometry`]), and the pointer ([`Pointer`]): clicks and
//! their count, links, and the selection's gestures.

use std::{ops::Range, rc::Rc};

use iced::{
    Border, Color, Event, Point, Rectangle, Renderer, Theme, Vector,
    advanced::{
        Shell, mouse, overlay, renderer,
        widget::{Id, Operation},
    },
    keyboard,
};
use unicode_segmentation::UnicodeSegmentation;

use crate::{
    find::{self, Found},
    link_status,
    selection::{Gesture, Pos},
    style,
};

/// How far the cursor may move between pressing and releasing for it to
/// count as a click rather than the start of a selection.
const CLICK_SLOP: f32 = 4.0;

/// Hit-testing and highlighting of laid-out text, in the units of
/// [`Pos`]. Points are relative to the text.
pub trait Geometry {
    /// The place nearest to `point`.
    fn hit(&self, point: Point) -> Option<Pos>;

    /// The link under `point`, if any.
    fn link_at(&self, point: Point) -> Option<&str>;

    /// The word at `pos`, in its unit; a formula there whole.
    fn word_at(&self, pos: Pos) -> Range<usize>;

    /// The length of the text of unit `unit`.
    fn unit_len(&self, unit: usize) -> usize;

    /// Rectangles covering the text from `from` to `to`.
    fn highlight(&self, from: Pos, to: Pos) -> Vec<Rectangle>;

    /// The place a line above (`up`) or below `pos`, nearest to where it
    /// is across; `None` past the unit's first or last line, or where lines
    /// are not known.
    fn line_step(&self, _pos: Pos, _up: bool) -> Option<Pos> {
        None
    }

    /// The start or end (`forward`) of the line holding `pos`; `None` where
    /// lines are not known.
    fn line_edge(&self, _pos: Pos, _forward: bool) -> Option<Pos> {
        None
    }
}

/// The word at `offset` in `text`, for a double click: the one ending
/// there, so a click just after a word selects it, unless that is spaces or
/// punctuation and a word starts there; else the run of spaces or
/// punctuation there.
pub fn word_at(text: &str, offset: usize) -> Range<usize> {
    let is_word = |range: &Range<usize>| text[range.clone()].chars().any(char::is_alphanumeric);
    let mut near = (text.split_word_bound_indices())
        .map(|(start, word)| start..start + word.len())
        .skip_while(|word| word.end < offset);
    match near.next() {
        Some(ending) if ending.end == offset => match near.next() {
            Some(next) if !is_word(&ending) && is_word(&next) => next,
            _ => ending,
        },
        Some(around) => around,
        None => offset..offset,
    }
}

/// What a text does with the pointer.
pub struct Handlers<'a, M> {
    /// Sent when a link is clicked (pressed and released without moving).
    pub on_link: Option<fn(String) -> M>,
    /// Sent when a link is right-clicked, for its menu.
    pub on_link_menu: Option<Rc<dyn Fn(String) -> M + 'a>>,
    /// Sent for the selection's gestures.
    pub on_gesture: Option<Rc<dyn Fn(Gesture) -> M + 'a>>,
    /// Sent when the text is clicked, as a link is. Dragging still selects.
    pub on_click: Option<Rc<dyn Fn() -> M + 'a>>,
    /// A selection is being dragged: report where the cursor is
    /// ([`Gesture::Drag`]).
    pub dragging: bool,
}

impl<M> Default for Handlers<'_, M> {
    fn default() -> Self {
        Self {
            on_link: None,
            on_link_menu: None,
            on_gesture: None,
            on_click: None,
            dragging: false,
        }
    }
}

/// What a press selects: `range` in `unit`, as a new selection or, with
/// Shift, `extend`ing the selection to its end.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Press {
    pub unit: usize,
    pub range: Range<usize>,
    pub extend: bool,
}

/// The pointer side of a selectable text.
#[derive(Default)]
pub struct Pointer {
    last_click: Option<mouse::Click>,
    modifiers: keyboard::Modifiers,
    /// The link pressed, and where.
    link_pressed: Option<(String, Point)>,
    /// Pressed where a click does something ([`Handlers::on_click`]),
    /// and where.
    click_pressed: Option<Point>,
    hovered_link: Option<String>,
    /// The hovered link's address in the window's corner.
    link_status: link_status::Status,
}

impl Pointer {
    /// `event` over the text `g` laid out in `bounds`: links hovered,
    /// clicked and right-clicked, the text clicked, and the selection's
    /// gestures, sent as `h` says.
    pub fn update<M>(
        &mut self,
        g: &impl Geometry,
        h: &Handlers<'_, M>,
        event: &Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
        shell: &mut Shell<'_, M>,
    ) {
        self.track_modifiers(event);
        let hovered_link = cursor.position_in(bounds).and_then(|at| g.link_at(at));
        if hovered_link != self.hovered_link.as_deref() {
            self.hovered_link = hovered_link.map(str::to_owned);
            shell.request_redraw();
        }
        match event {
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                let Some(at) = cursor.position_in(bounds) else {
                    return;
                };
                let position = cursor.position().unwrap_or_default();
                self.link_pressed = hovered_link.map(|link| (link.to_owned(), position));
                self.click_pressed = h.on_click.is_some().then_some(position);
                let Some(on_gesture) = &h.on_gesture else {
                    if self.link_pressed.is_some() || self.click_pressed.is_some() {
                        shell.capture_event();
                    }
                    return;
                };
                let Some(Press {
                    unit,
                    range,
                    extend,
                }) = self.press(g, at, position)
                else {
                    return;
                };
                shell.publish(on_gesture(Gesture::Press {
                    unit,
                    from: range.start,
                    to: range.end,
                    extend,
                }));
                shell.capture_event();
            }
            // The link's menu rather than whatever the text is in offers.
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Right)) => {
                if let Some(on_link_menu) = &h.on_link_menu
                    && let Some(link) = hovered_link
                {
                    shell.publish(on_link_menu(link.to_owned()));
                    shell.capture_event();
                }
            }
            Event::Mouse(mouse::Event::CursorMoved { .. }) => {
                // The cursor, not the event: inside a scrollable, only the
                // cursor is moved into the content's coordinates.
                let Some(position) = cursor.land().position() else {
                    return;
                };
                if (self.link_pressed.as_ref())
                    .is_some_and(|(_, pressed)| pressed.distance(position) > CLICK_SLOP)
                {
                    self.link_pressed = None;
                }
                if (self.click_pressed)
                    .is_some_and(|pressed| pressed.distance(position) > CLICK_SLOP)
                {
                    self.click_pressed = None;
                }
                // Over its lines, also left or right of the text.
                let beside = position.y >= bounds.y && position.y < bounds.y + bounds.height;
                if h.dragging
                    && beside
                    && let Some(on_gesture) = &h.on_gesture
                    && let Some(pos) = g.hit(position - (bounds.position() - Point::ORIGIN))
                {
                    shell.publish(on_gesture(Gesture::Drag {
                        unit: pos.unit,
                        offset: pos.offset,
                    }));
                }
            }
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
                if let Some((link, _)) = self.link_pressed.take()
                    && hovered_link == Some(link.as_str())
                    && let Some(on_link) = h.on_link
                {
                    shell.publish(on_link(link));
                }
                if self.click_pressed.take().is_some()
                    && cursor.is_over(bounds)
                    && let Some(on_click) = &h.on_click
                {
                    shell.publish(on_click());
                }
            }
            _ => {}
        }
    }

    /// Notes the modifiers held, from `event`.
    pub fn track_modifiers(&mut self, event: &Event) {
        if let Event::Keyboard(keyboard::Event::ModifiersChanged(modifiers)) = event {
            self.modifiers = *modifiers;
        }
    }

    /// What a press at `at` in the text `g` (`position` in the window)
    /// selects: a place, or for a double or triple click the word or the
    /// unit there.
    pub fn press(&mut self, g: &impl Geometry, at: Point, position: Point) -> Option<Press> {
        let pos = g.hit(at)?;
        let click = mouse::Click::new(position, mouse::Button::Left, self.last_click);
        self.last_click = Some(click);
        let range = match click.kind() {
            mouse::click::Kind::Single => pos.offset..pos.offset,
            mouse::click::Kind::Double => g.word_at(pos),
            mouse::click::Kind::Triple => 0..g.unit_len(pos.unit),
        };
        Some(Press {
            unit: pos.unit,
            range,
            extend: self.modifiers.shift(),
        })
    }

    /// The address of the link under the pointer, if any.
    pub fn hovered_link(&self) -> Option<&str> {
        self.hovered_link.as_deref()
    }

    /// The address of the hovered link in the window's corner, if it can
    /// be clicked.
    pub fn overlay<'b, M: 'b>(
        &'b mut self,
        h: &Handlers<'_, M>,
    ) -> Option<overlay::Element<'b, M, Theme, Renderer>> {
        let url = (self.hovered_link.as_deref()).filter(|_| h.on_link.is_some())?;
        Some(link_status::overlay(url, &mut self.link_status))
    }
}

/// The cursor over the text `g` laid out in `bounds`: a pointing hand over
/// what a click opens, else the text cursor where it selects.
pub fn interaction<M>(
    g: &impl Geometry,
    h: &Handlers<'_, M>,
    cursor: mouse::Cursor,
    bounds: Rectangle,
) -> mouse::Interaction {
    let at = cursor.position_in(bounds);
    let over_link = || at.and_then(|at| g.link_at(at)).is_some();
    if h.on_link.is_some() && over_link() || h.on_click.is_some() && !h.dragging && at.is_some() {
        mouse::Interaction::Pointer
    } else if h.on_gesture.is_some() && (h.dragging || at.is_some()) {
        mouse::Interaction::Text
    } else {
        mouse::Interaction::None
    }
}

/// The find bar's matches and the selection over the text `g`, drawn at
/// `at`.
pub fn draw_highlights(
    renderer: &mut Renderer,
    theme: &Theme,
    g: &impl Geometry,
    selected: Option<(Pos, Pos)>,
    found: &[Found],
    at: Vector,
) {
    use iced::advanced::Renderer as _;
    let mut fill = |(from, to): (Pos, Pos), color: Color, border: Border| {
        for rect in g.highlight(from, to) {
            renderer.fill_quad(
                renderer::Quad {
                    bounds: rect + at,
                    border,
                    ..Default::default()
                },
                color,
            );
        }
    };
    for found in found {
        let (color, border) = style::found(theme, found.current);
        fill(found_part(found), color, border);
    }
    if let Some(selected) = selected {
        fill(selected, style::selection(theme), Border::default());
    }
}

/// Reports where the find bar's current match is, if among `found`, for
/// scrolling to it: the text of `g` it covers, else all of `bounds`, where
/// `g` is laid out.
pub fn report_found(
    operation: &mut dyn Operation,
    g: &impl Geometry,
    found: &[Found],
    bounds: Rectangle,
) {
    let Some(current) = found.iter().find(|found| found.current) else {
        return;
    };
    let (from, to) = found_part(current);
    let rect = (g.highlight(from, to).into_iter())
        .reduce(|a, b| a.union(&b))
        .map_or(bounds, |rect| rect + (bounds.position() - Point::ORIGIN));
    operation.custom(Some(&Id::new(find::CURRENT)), rect, &mut ());
}

/// The part of a message a match covers.
fn found_part(found: &Found) -> (Pos, Pos) {
    let at = |offset| Pos {
        unit: found.unit,
        offset,
    };
    (at(found.range.start), at(found.range.end))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_word_is_the_one_clicked_or_just_before() {
        let text = "Ice sheets (flow) melt.";
        let word = |offset| &text[word_at(text, offset)];
        // Inside a word, and at its start after a space.
        assert_eq!(word(1), "Ice");
        assert_eq!(word(4), "sheets");
        assert_eq!(word(0), "Ice");
        // Just after a word.
        assert_eq!(word(3), "Ice");
        assert_eq!(word(text.len() - 1), "melt");
        // Between a space and punctuation: the one before.
        assert_eq!(word(11), " ");
        // At a word's start after punctuation.
        assert_eq!(word(12), "flow");
        // After the end: the last word or punctuation.
        assert_eq!(word(text.len()), ".");
        assert_eq!(word_at("", 0), 0..0);
        assert_eq!(word_at("ok", 5), 5..5);
    }

    /// Two units, "Ice sheet" and "Melt", each a line of 10-wide characters
    /// 20 high, the second below the first; "sheet" links.
    struct Lines;

    const UNITS: [&str; 2] = ["Ice sheet", "Melt"];
    const LINK: &str = "https://example.org";

    impl Geometry for Lines {
        fn hit(&self, point: Point) -> Option<Pos> {
            let unit = if point.y < 20.0 { 0 } else { 1 };
            let offset = ((point.x / 10.0).round().max(0.0) as usize).min(UNITS[unit].len());
            Some(Pos { unit, offset })
        }

        fn link_at(&self, point: Point) -> Option<&str> {
            (point.y < 20.0 && point.x >= 40.0 && point.x < 90.0).then_some(LINK)
        }

        fn word_at(&self, pos: Pos) -> Range<usize> {
            word_at(UNITS[pos.unit], pos.offset)
        }

        fn unit_len(&self, unit: usize) -> usize {
            UNITS[unit].len()
        }

        fn highlight(&self, from: Pos, to: Pos) -> Vec<Rectangle> {
            (from.unit..=to.unit)
                .filter_map(|unit| {
                    let len = UNITS.get(unit)?.len();
                    let range = crate::selection::unit_range((from, to), unit, len)?;
                    Some(Rectangle {
                        x: range.start as f32 * 10.0,
                        y: unit as f32 * 20.0,
                        width: range.len() as f32 * 10.0,
                        height: 20.0,
                    })
                })
                .collect()
        }
    }

    #[test]
    fn presses_count_clicks_into_a_place_a_word_and_the_unit() {
        let mut pointer = Pointer::default();
        let mut press = |x, y| {
            // Clicks closer than the clock's resolution are not counted.
            std::thread::sleep(std::time::Duration::from_millis(1));
            pointer.press(&Lines, Point::new(x, y), Point::new(100.0, 50.0))
        };
        let pressed = |unit, range, extend| {
            Some(Press {
                unit,
                range,
                extend,
            })
        };
        assert_eq!(press(62.0, 5.0), pressed(0, 6..6, false));
        assert_eq!(press(62.0, 5.0), pressed(0, 4..9, false));
        assert_eq!(press(62.0, 5.0), pressed(0, 0..9, false));
        // With Shift, extending; in the unit below.
        pointer.track_modifiers(&Event::Keyboard(keyboard::Event::ModifiersChanged(
            keyboard::Modifiers::SHIFT,
        )));
        let press = pointer.press(&Lines, Point::new(18.0, 25.0), Point::new(300.0, 300.0));
        assert_eq!(press, pressed(1, 2..2, true));
    }

    #[test]
    fn the_cursor_is_a_hand_over_what_a_click_opens_and_text_where_it_selects() {
        use mouse::Interaction as I;
        let bounds = Rectangle::new(Point::new(10.0, 10.0), iced::Size::new(100.0, 40.0));
        let at = |x, y| mouse::Cursor::Available(Point::new(x, y));
        let selects = Handlers::<()> {
            on_link: Some(|_| ()),
            on_gesture: Some(Rc::new(|_| ())),
            ..Handlers::default()
        };
        assert_eq!(
            interaction(&Lines, &selects, at(70.0, 15.0), bounds),
            I::Pointer
        );
        assert_eq!(
            interaction(&Lines, &selects, at(20.0, 15.0), bounds),
            I::Text
        );
        assert_eq!(
            interaction(&Lines, &selects, at(200.0, 15.0), bounds),
            I::None
        );
        let dragging = Handlers {
            dragging: true,
            ..selects
        };
        assert_eq!(
            interaction(&Lines, &dragging, at(200.0, 15.0), bounds),
            I::Text
        );
        let clicks = Handlers::<()> {
            on_click: Some(Rc::new(|| ())),
            ..Handlers::default()
        };
        assert_eq!(
            interaction(&Lines, &clicks, at(20.0, 15.0), bounds),
            I::Pointer
        );
        assert_eq!(
            interaction(&Lines, &Handlers::<()>::default(), at(20.0, 15.0), bounds),
            I::None
        );
    }

    #[test]
    fn the_current_match_is_reported_where_it_is_else_the_whole_text() {
        /// Where the current match was reported.
        struct Current(Option<Rectangle>);

        impl Operation for Current {
            fn traverse(&mut self, _operate: &mut dyn FnMut(&mut dyn Operation)) {}

            fn custom(
                &mut self,
                id: Option<&Id>,
                bounds: Rectangle,
                _state: &mut dyn std::any::Any,
            ) {
                if id == Some(&Id::new(find::CURRENT)) {
                    self.0 = Some(bounds);
                }
            }
        }

        let bounds = Rectangle::new(Point::new(10.0, 100.0), iced::Size::new(200.0, 40.0));
        let report = |found: Vec<Found>| {
            let mut current = Current(None);
            report_found(&mut current, &Lines, &found, bounds);
            current.0
        };
        let found = |unit, range, current| Found {
            unit,
            range,
            current,
        };
        // "sheet", across the matches not current.
        assert_eq!(
            report(vec![found(1, 0..2, false), found(0, 4..9, true)]),
            Some(Rectangle::new(
                Point::new(50.0, 100.0),
                iced::Size::new(50.0, 20.0)
            ))
        );
        assert_eq!(report(vec![found(0, 4..9, false)]), None);
        // Past the text: all of it.
        assert_eq!(report(vec![found(5, 0..2, true)]), Some(bounds));
    }
}
