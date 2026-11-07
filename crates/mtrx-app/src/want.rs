//! Views ask for what they show: a picture not loaded yet is wanted by the
//! widget that shows it, the first time it is drawn. So whatever is on screen
//! loads, and nothing besides the views needs to know what that is.

use iced::{
    Element, Event, Length, Rectangle, Renderer, Size, Theme, Vector,
    advanced::{
        Clipboard, Layout, Shell, Widget,
        layout::{Limits, Node},
        mouse, overlay, renderer,
        widget::{Operation, Tree},
    },
    window,
};

/// `content`, asking for `want` the first time it is drawn. Once per view: a
/// view built anew asks again for what it still wants, and the loads skip
/// what is already loading.
pub fn wanting<'a, M: 'a>(content: impl Into<Element<'a, M>>, want: Option<M>) -> Element<'a, M> {
    Element::new(Wanting {
        content: content.into(),
        want,
    })
}

struct Wanting<'a, M> {
    content: Element<'a, M>,
    /// Taken when sent.
    want: Option<M>,
}

impl<M> Widget<M, Theme, Renderer> for Wanting<'_, M> {
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
        shell: &mut Shell<'_, M>,
        viewport: &Rectangle,
    ) {
        if let Event::Window(window::Event::RedrawRequested(_)) = event
            && let Some(want) = self.want.take()
        {
            shell.publish(want);
        }
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
    ) -> Option<overlay::Element<'b, M, Theme, Renderer>> {
        self.content.as_widget_mut().overlay(
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
    use std::time::Instant;

    use iced::widget::{column, scrollable, text};
    use iced_test::simulator::Simulator;

    use super::*;
    use crate::fonts;

    fn redraw() -> Event {
        Event::Window(window::Event::RedrawRequested(Instant::now()))
    }

    /// What `view` asks for over a few frames.
    fn asked(view: Element<'_, u32>) -> Vec<u32> {
        let mut ui = Simulator::with_size(fonts::settings(), Size::new(400.0, 300.0), view);
        let _ = ui.simulate([redraw(), redraw(), redraw()]);
        ui.into_messages().collect()
    }

    #[test]
    fn asks_once_when_drawn() {
        assert_eq!(asked(wanting(text("a"), Some(1))), [1]);
        assert!(asked(wanting(text("a"), None)).is_empty());
    }

    /// Also from inside others, and below what fits: the timeline's rows
    /// and the room list's are in scrollables.
    #[test]
    fn asks_from_inside_a_scrollable() {
        let rows = (0..50).map(|i| wanting(text(i.to_string()).height(40), Some(i)));
        let mut wanted = asked(scrollable(column(rows)).into());
        wanted.sort();
        assert_eq!(wanted, (0..50).collect::<Vec<_>>());
    }
}
