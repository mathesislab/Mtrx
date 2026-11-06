//! Scrolling while a selection is dragged past the top or bottom of the
//! timeline, or scrolled with the wheel: wraps the timeline's scrollable,
//! asks for it to be scrolled each frame past an edge, and tells the text
//! under the cursor (or the edge) where it is, as the content moves under a
//! cursor that does not.

use iced::{
    Element, Event, Length, Point, Rectangle, Renderer, Size, Theme, Vector,
    advanced::{
        Clipboard, Layout, Shell, Widget,
        layout::{Limits, Node},
        mouse, overlay, renderer,
        widget::{Operation, Tree, tree},
    },
    window,
};

/// Scrolled per frame: a share of how far past the edge the cursor is,
/// within these limits.
const MIN_STEP: f32 = 2.0;
const MAX_STEP: f32 = 40.0;
const SHARE: f32 = 0.4;

pub struct DragScroll<'a, Message> {
    content: Element<'a, Message>,
    active: bool,
    on_scroll: Box<dyn Fn(f32) -> Message + 'a>,
}

/// `content`, sending `on_scroll` with the distance to scroll (negative is
/// up) while `active` and the cursor is above or below it.
pub fn drag_scroll<'a, Message>(
    content: impl Into<Element<'a, Message>>,
    active: bool,
    on_scroll: impl Fn(f32) -> Message + 'a,
) -> DragScroll<'a, Message> {
    DragScroll {
        content: content.into(),
        active,
        on_scroll: Box::new(on_scroll),
    }
}

#[derive(Default)]
struct State {
    cursor: Option<Point>,
}

/// How far to scroll for a cursor at `y`, given the content's top and
/// bottom; zero inside.
fn step(y: f32, top: f32, bottom: f32) -> f32 {
    let past = if y < top {
        y - top
    } else if y > bottom {
        y - bottom
    } else {
        return 0.0;
    };
    (past.abs() * SHARE)
        .clamp(MIN_STEP, MAX_STEP)
        .copysign(past)
}

impl<'a, Message> Widget<Message, Theme, Renderer> for DragScroll<'a, Message> {
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<State>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(State::default())
    }

    fn children(&self) -> Vec<Tree> {
        vec![Tree::new(&self.content)]
    }

    fn diff(&self, tree: &mut Tree) {
        tree.diff_children(&[self.content.as_widget()]);
    }

    fn size(&self) -> Size<Length> {
        self.content.as_widget().size()
    }

    fn size_hint(&self) -> Size<Length> {
        self.content.as_widget().size_hint()
    }

    fn layout(&mut self, tree: &mut Tree, renderer: &Renderer, limits: &Limits) -> Node {
        self.content
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
        shell: &mut Shell<'_, Message>,
        viewport: &Rectangle,
    ) {
        self.content.as_widget_mut().update(
            &mut tree.children[0],
            event,
            layout,
            cursor,
            renderer,
            clipboard,
            shell,
            viewport,
        );
        let bounds = layout.bounds();
        let (top, bottom) = (bounds.y, bounds.y + bounds.height);
        let state = tree.state.downcast_mut::<State>();
        let at = match event {
            Event::Mouse(mouse::Event::CursorMoved { position }) => {
                state.cursor = Some(*position);
                if self.active && step(position.y, top, bottom) != 0.0 {
                    shell.request_redraw();
                }
                return;
            }
            // The content was scrolled under the cursor.
            Event::Mouse(mouse::Event::WheelScrolled { .. }) if self.active => {
                let Some(at) = state.cursor else {
                    return;
                };
                at
            }
            Event::Window(window::Event::RedrawRequested(_)) if self.active => {
                let Some(at) = state.cursor else {
                    return;
                };
                let step = step(at.y, top, bottom);
                if step == 0.0 {
                    return;
                }
                shell.publish((self.on_scroll)(step));
                shell.request_redraw();
                at
            }
            _ => return,
        };
        // The text now under the cursor, or under the edge it is past,
        // extends the selection.
        let inside = Point::new(at.x, at.y.clamp(top + 1.0, bottom - 1.0));
        self.content.as_widget_mut().update(
            &mut tree.children[0],
            &Event::Mouse(mouse::Event::CursorMoved { position: inside }),
            layout,
            mouse::Cursor::Available(inside),
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
        self.content
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
        self.content.as_widget().mouse_interaction(
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
        self.content.as_widget().draw(
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
    ) -> Option<overlay::Element<'b, Message, Theme, Renderer>> {
        self.content.as_widget_mut().overlay(
            &mut tree.children[0],
            layout,
            renderer,
            viewport,
            translation,
        )
    }
}

impl<'a, Message: 'a> From<DragScroll<'a, Message>> for Element<'a, Message> {
    fn from(widget: DragScroll<'a, Message>) -> Self {
        Element::new(widget)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scrolls_faster_further_past_the_edge() {
        assert_eq!(step(50.0, 10.0, 500.0), 0.0);
        assert_eq!(step(8.0, 10.0, 500.0), -MIN_STEP);
        assert_eq!(step(600.0, 10.0, 500.0), MAX_STEP);
        assert_eq!(step(520.0, 10.0, 500.0), 20.0 * SHARE);
    }

    #[test]
    fn wheel_scrolling_while_dragging_moves_the_selection() {
        use std::rc::Rc;

        use iced::{
            advanced::text::Span,
            widget::{column, scrollable},
        };
        use iced_test::simulator::Simulator;

        use crate::{selectable::text::selectable, selection::Gesture};

        for active in [false, true] {
            let lines = column((0..100).map(|i| {
                selectable(vec![Span::new(format!("Line {i}"))], i)
                    .dragging(true)
                    .on_gesture(Rc::new(|gesture| gesture))
                    .into()
            }));
            let view = drag_scroll(scrollable(lines), active, |_| Gesture::Drag {
                unit: usize::MAX,
                offset: 0,
            });
            let mut ui =
                Simulator::with_size(crate::fonts::settings(), Size::new(200.0, 200.0), view);
            ui.point_at((50.0, 100.0));
            let _ = ui.simulate([
                Event::Mouse(mouse::Event::CursorMoved {
                    position: Point::new(50.0, 100.0),
                }),
                Event::Mouse(mouse::Event::WheelScrolled {
                    delta: mouse::ScrollDelta::Lines { x: 0.0, y: -2.0 },
                }),
            ]);
            let units: Vec<usize> = ui
                .into_messages()
                .filter_map(|gesture| match gesture {
                    Gesture::Drag { unit, .. } => Some(unit),
                    Gesture::Press { .. } => None,
                })
                .collect();
            // Where the cursor moved to, and while dragging, the line
            // scrolled under it.
            if active {
                assert_eq!(units.len(), 2, "{units:?}");
                assert!(units[1] > units[0], "{units:?}");
            } else {
                assert_eq!(units.len(), 1, "{units:?}");
            }
        }
    }
}
