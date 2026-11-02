//! Finding text in the room's loaded messages (⌘F), as Safari and Mail
//! do: case and accents ignored, every match highlighted where a selection
//! would be ([`crate::selectable`], [`crate::selectable_image`]), the
//! current one distinct and scrolled into view. Matches are counted from the
//! newest message, where the timeline sits, and the next one is older.

use std::{collections::HashMap, ops::Range};

use iced::{
    Alignment, Element, Event, Length, Rectangle, Renderer, Size, Theme, Vector,
    advanced::{
        Clipboard, Layout, Shell, Widget,
        layout::{Limits, Node},
        mouse, overlay, renderer,
        widget::{Id, Operation, Tree, operation::Focusable},
    },
    keyboard::{self, Key, key::Named},
    widget::{button, container, row, space, text, text_input},
};
use mtrx_core::{Event as TimelineEvent, EventContent, MessageKind, Row};
use unicode_normalization::char::{decompose_canonical, is_combining_mark};

use crate::{
    hint::{Position, hint, shortcut},
    icons, message, style,
};

/// The find field.
pub const FIELD: &str = "find";
/// The id under which the text showing the current match reports where it
/// is, for scrolling to it.
pub const CURRENT: &str = "find-current";

#[derive(Clone, Debug)]
pub enum Message {
    /// ⌘F: open the find bar, or focus it again.
    Open,
    /// Opening, with what the room's texts cut to fit show, for looking
    /// for what is selected.
    Opened(crate::selection::Shown),
    Query(String),
    /// Enter or ⌘G: the next match, an older one.
    Next,
    /// Shift+Enter or ⇧⌘G: the previous match, a newer one.
    Previous,
    Close,
}

/// A match: bytes of a unit of a message's text (see
/// [`crate::selection::Unit`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Match {
    /// [`TimelineEvent::id`] of the message.
    pub event: String,
    pub unit: usize,
    pub range: Range<usize>,
}

/// A match in a message, for drawing it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Found {
    pub unit: usize,
    pub range: Range<usize>,
    pub current: bool,
}

/// The find bar of a room.
#[derive(Debug, Default)]
pub struct Find {
    pub query: String,
    /// Newest first, and from the end of each message.
    matches: Vec<Match>,
    /// Each message's matches, by [`TimelineEvent::id`].
    by_event: HashMap<String, Range<usize>>,
    current: usize,
}

impl Find {
    /// Find `query` in the messages of `rows`.
    pub fn new(query: String, rows: &[Row]) -> Self {
        let mut find = Self {
            query,
            ..Self::default()
        };
        find.search(rows);
        find
    }

    /// Search `rows` again, for a new query or as they changed. The
    /// current match stays, or the nearest one above it does.
    pub fn search(&mut self, rows: &[Row]) {
        let was = self.current().and_then(|current| {
            let index =
                (rows.iter()).position(|row| row.event().is_some_and(|e| e.id == current.event))?;
            Some((index, current.unit, current.range.start))
        });
        let query = fold(&self.query).0;
        // Each match with its place in the timeline, newest first.
        let mut found: Vec<((usize, usize, usize), Match)> = Vec::new();
        if !query.is_empty() {
            for (index, row) in rows.iter().enumerate().rev() {
                let Some(event) = row.event() else {
                    continue;
                };
                let Some(document) = searched(event) else {
                    continue;
                };
                let units = message::text_map(document).units;
                for (unit, shown) in units.iter().enumerate().rev() {
                    for range in find_in(&shown.text, &query).into_iter().rev() {
                        let at = (index, unit, range.start);
                        found.push((
                            at,
                            Match {
                                event: event.id.clone(),
                                unit,
                                range,
                            },
                        ));
                    }
                }
            }
        }
        self.current = was
            .and_then(|was| {
                found
                    .iter()
                    .position(|(at, _)| *at <= was)
                    // All below it: the nearest.
                    .or(found.len().checked_sub(1))
            })
            .unwrap_or(0);
        self.matches = found.into_iter().map(|(_, m)| m).collect();
        self.by_event.clear();
        for (i, m) in self.matches.iter().enumerate() {
            self.by_event
                .entry(m.event.clone())
                .and_modify(|range| range.end = i + 1)
                .or_insert(i..i + 1);
        }
    }

    /// Go to the next (older) match, or the previous one, round at the
    /// ends.
    pub fn step(&mut self, next: bool) {
        let count = self.matches.len();
        if count == 0 {
            return;
        }
        self.current = if next {
            (self.current + 1) % count
        } else {
            (self.current + count - 1) % count
        };
    }

    pub fn current(&self) -> Option<&Match> {
        self.matches.get(self.current)
    }

    /// The matches in message `event`.
    pub fn in_message(&self, event: &str) -> Vec<Found> {
        let Some(range) = self.by_event.get(event) else {
            return Vec::new();
        };
        range
            .clone()
            .map(|i| Found {
                unit: self.matches[i].unit,
                range: self.matches[i].range.clone(),
                current: i == self.current,
            })
            .collect()
    }

    /// "3 of 12", "Not found", or nothing before anything is typed.
    fn count(&self) -> String {
        match self.matches.len() {
            0 if self.query.is_empty() => String::new(),
            0 => "Not found".to_owned(),
            count => format!("{} of {count}", self.current + 1),
        }
    }

    /// The bar above the timeline: field, count, older and newer, Done.
    pub fn view<'a, M: Clone + 'a>(&'a self, on: fn(Message) -> M) -> Element<'a, M> {
        let field = text_input(
            &format!("Find in loaded messages  ({})", shortcut("F")),
            &self.query,
        )
        .id(FIELD)
        .on_input(move |query| on(Message::Query(query)))
        .on_submit(on(Message::Next))
        .style(style::input)
        .padding([5, 8])
        .size(13)
        .width(260);
        let field = shift_enter(field, on(Message::Previous));
        let found = !self.matches.is_empty();
        let arrow = |up, message, tip: String| {
            hint(
                button(icons::arrow(14.0, up, |theme| {
                    theme.extended_palette().background.base.text
                }))
                .padding(5)
                .style(style::list_item(false))
                .on_press_maybe(found.then(|| on(message))),
                tip,
                Position::Bottom,
            )
        };
        let older = arrow(
            true,
            Message::Next,
            format!("Older match (Enter, {})", shortcut("G")),
        );
        let newer = arrow(
            false,
            Message::Previous,
            format!(
                "Newer match (Shift+Enter, {})",
                if cfg!(target_os = "macos") {
                    "⇧⌘G".to_owned()
                } else {
                    shortcut("Shift+G")
                }
            ),
        );
        let done = button(text("Done").size(13))
            .padding([4, 10])
            .style(style::secondary_button)
            .on_press(on(Message::Close));
        container(
            row![
                space().width(Length::Fill),
                text(self.count()).size(12).style(text::secondary),
                older,
                newer,
                field,
                done,
            ]
            .spacing(6)
            .align_y(Alignment::Center),
        )
        .padding([6.0, style::TIMELINE_PADDING + style::MESSAGE_PADDING])
        .width(Length::Fill)
        .into()
    }
}

/// The document of a message searched, one shown as text (or typeset).
fn searched(event: &TimelineEvent) -> Option<&mtrx_format::Document> {
    match &event.content {
        EventContent::Message(m)
            if matches!(
                m.kind,
                MessageKind::Text | MessageKind::Notice | MessageKind::Emote
            ) =>
        {
            Some(&m.document)
        }
        _ => None,
    }
}

/// `text` without case and accents, a character at a time, with the bytes
/// of `text` each comes from.
fn fold(text: &str) -> (Vec<char>, Vec<Range<usize>>) {
    let mut folded = Vec::new();
    let mut from = Vec::new();
    for (start, c) in text.char_indices() {
        let bytes = start..start + c.len_utf8();
        decompose_canonical(c, |base| {
            for lower in base.to_lowercase() {
                if !is_combining_mark(lower) {
                    folded.push(lower);
                    from.push(bytes.clone());
                }
            }
        });
    }
    (folded, from)
}

/// Where `query` (folded) is in `text`, as byte ranges that do not overlap;
/// accents after a match's last letter belong to it.
fn find_in(text: &str, query: &[char]) -> Vec<Range<usize>> {
    if query.is_empty() {
        return Vec::new();
    }
    let (folded, from) = fold(text);
    let mut found = Vec::new();
    let mut i = 0;
    while i + query.len() <= folded.len() {
        if folded[i..i + query.len()] != *query {
            i += 1;
            continue;
        }
        let last = &from[i + query.len() - 1];
        // Up to the next character shown, past any marks.
        let end = from[i + query.len()..]
            .iter()
            .find(|next| next.start >= last.end)
            .map_or(text.len(), |next| next.start);
        found.push(from[i].start..end);
        i += query.len();
    }
    found
}

/// `input`, sending `on_shift_enter` for Shift+Enter while it is focused:
/// iced's text input sends the same for Enter with or without Shift.
fn shift_enter<'a, M: Clone + 'a>(
    input: impl Into<Element<'a, M>>,
    on_shift_enter: M,
) -> Element<'a, M> {
    Element::new(ShiftEnter {
        input: input.into(),
        on_shift_enter,
    })
}

struct ShiftEnter<'a, M> {
    input: Element<'a, M>,
    on_shift_enter: M,
}

/// Whether a focusable widget is focused.
struct Focused(bool);

impl Operation for Focused {
    fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation)) {
        operate(self);
    }

    fn focusable(&mut self, _id: Option<&Id>, _bounds: Rectangle, state: &mut dyn Focusable) {
        self.0 |= state.is_focused();
    }
}

impl<M: Clone> Widget<M, Theme, Renderer> for ShiftEnter<'_, M> {
    fn children(&self) -> Vec<Tree> {
        vec![Tree::new(&self.input)]
    }

    fn diff(&self, tree: &mut Tree) {
        tree.diff_children(&[self.input.as_widget()]);
    }

    fn size(&self) -> Size<Length> {
        self.input.as_widget().size()
    }

    fn layout(&mut self, tree: &mut Tree, renderer: &Renderer, limits: &Limits) -> Node {
        self.input
            .as_widget_mut()
            .layout(&mut tree.children[0], renderer, limits)
    }

    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &Renderer,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, M>,
        viewport: &Rectangle,
    ) {
        if let Event::Keyboard(keyboard::Event::KeyPressed {
            key: Key::Named(Named::Enter),
            modifiers,
            ..
        }) = event
            && modifiers.shift()
        {
            let mut focused = Focused(false);
            self.input.as_widget_mut().operate(
                &mut tree.children[0],
                layout,
                renderer,
                &mut focused,
            );
            if focused.0 {
                shell.publish(self.on_shift_enter.clone());
                shell.capture_event();
                return;
            }
        }
        self.input.as_widget_mut().update(
            &mut tree.children[0],
            event,
            layout,
            cursor,
            renderer,
            clipboard,
            shell,
            viewport,
        );
    }

    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        renderer: &Renderer,
        operation: &mut dyn Operation,
    ) {
        self.input
            .as_widget_mut()
            .operate(&mut tree.children[0], layout, renderer, operation);
    }

    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &Renderer,
    ) -> mouse::Interaction {
        self.input.as_widget().mouse_interaction(
            &tree.children[0],
            layout,
            cursor,
            viewport,
            renderer,
        )
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
        self.input.as_widget().draw(
            &tree.children[0],
            renderer,
            theme,
            style,
            layout,
            cursor,
            viewport,
        );
    }

    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut Tree,
        layout: Layout<'b>,
        renderer: &Renderer,
        viewport: &Rectangle,
        translation: Vector,
    ) -> Option<overlay::Element<'b, M, Theme, Renderer>> {
        self.input.as_widget_mut().overlay(
            &mut tree.children[0],
            layout,
            renderer,
            viewport,
            translation,
        )
    }
}

#[cfg(test)]
mod tests {
    use iced::{event::Status, keyboard::Modifiers};
    use iced_test::simulator::{Simulator, press_key};
    use mtrx_core::{OwnedUserId, TimelineEntry};

    use super::*;
    use crate::fonts;

    fn message(id: &str, body: &str) -> Row {
        let sender = OwnedUserId::try_from("@alice:example.org").unwrap();
        let entry = TimelineEntry::Event(TimelineEvent::test_message(id, sender, body));
        mtrx_core::rows(&[entry], &Default::default()).remove(0)
    }

    fn found<'a>(text: &'a str, query: &str) -> Vec<&'a str> {
        find_in(text, &fold(query).0)
            .into_iter()
            .map(|range| &text[range])
            .collect()
    }

    #[test]
    fn ignores_case_and_accents() {
        assert_eq!(found("Élan and elan", "ELAN"), ["Élan", "elan"]);
        assert_eq!(found("Melt in Zürich", "zur"), ["Zür"]);
        // An accent typed is ignored too.
        assert_eq!(found("cafe", "café"), ["cafe"]);
        // A combining accent after the last letter is part of the match.
        assert_eq!(found("cafe\u{301} au lait", "cafe"), ["cafe\u{301}"]);
        // Not overlapping.
        assert_eq!(found("aaaa", "aa"), ["aa", "aa"]);
        assert!(found("ice", "").is_empty());
    }

    #[test]
    fn counts_from_the_newest_and_goes_round() {
        let mut entries = vec![
            message("$a", "Ice sheets"),
            message("$b", "nothing here"),
            message("$c", "ice shelves and sea ice"),
        ];
        let mut find = Find::new("ice".into(), &entries);
        assert_eq!(find.count(), "1 of 3");
        // The newest: the end of the last message.
        assert_eq!(
            find.current(),
            Some(&Match {
                event: "$c".into(),
                unit: 0,
                range: 20..23,
            })
        );
        find.step(true);
        assert_eq!(find.current().unwrap().range, 0..3);
        find.step(true);
        assert_eq!(find.current().unwrap().event, "$a");
        // Round to the newest, and back.
        find.step(true);
        assert_eq!(find.count(), "1 of 3");
        find.step(false);
        assert_eq!(find.count(), "3 of 3");
        assert_eq!(
            find.in_message("$c"),
            [
                Found {
                    unit: 0,
                    range: 20..23,
                    current: false,
                },
                Found {
                    unit: 0,
                    range: 0..3,
                    current: false,
                },
            ]
        );
        assert!(find.in_message("$b").is_empty());

        // A new message: the current match stays.
        entries.push(message("$d", "more ice"));
        find.search(&entries);
        assert_eq!(find.count(), "4 of 4");
        assert_eq!(find.current().unwrap().event, "$a");
        // Typing on: the current match stays if it still matches, else the
        // nearest above it.
        find.step(false);
        find.step(false);
        assert_eq!(find.current().unwrap().range, 20..23);
        find.query = "ice s".into();
        find.search(&entries);
        assert_eq!(find.count(), "1 of 2");
        assert_eq!(
            find.current(),
            Some(&Match {
                event: "$c".into(),
                unit: 0,
                range: 0..5,
            })
        );
        find.query = "glacier".into();
        find.search(&entries);
        assert_eq!(find.count(), "Not found");
        find.step(true);
        assert_eq!(find.current(), None);
    }

    #[test]
    fn enter_and_shift_enter_in_the_field() {
        let entries = [message("$a", "Ice")];
        let find = Find::new("ice".into(), &entries);
        let mut ui = Simulator::with_size(
            fonts::settings(),
            Size::new(700.0, 60.0),
            find.view(|message| message),
        );
        let enter = |modifiers| match press_key(Key::Named(Named::Enter), None) {
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
        };
        // Not focused: neither is taken.
        assert_eq!(ui.simulate([enter(Modifiers::SHIFT)]), [Status::Ignored]);
        let _ = ui.click(Id::new(FIELD)).unwrap();
        let _ = ui.typewrite("s");
        assert_eq!(
            ui.simulate([enter(Modifiers::empty()), enter(Modifiers::SHIFT)]),
            [Status::Captured, Status::Captured]
        );
        let sent: Vec<_> = ui.into_messages().collect();
        assert!(
            matches!(
                sent.as_slice(),
                [
                    Message::Query(query),
                    Message::Next,
                    Message::Previous
                ] if query == "ices"
            ),
            "{sent:?}"
        );
    }
}
