//! A menu opened by right-clicking a widget, floating at the cursor over
//! everything else, as for a message's menu.

use iced::{
    Element, Event, Length, Point, Rectangle, Renderer, Size, Theme, Vector,
    advanced::{
        Clipboard, Layout, Shell, Widget,
        layout::{Limits, Node},
        overlay, renderer,
        widget::{Operation, Tree, tree},
    },
    mouse,
    widget::space,
};

use crate::popover::{Edge, Side, floating};

pub struct ContextMenu<'a, Message> {
    content: Element<'a, Message>,
    /// The menu, or a placeholder while closed (the widget tree keeps its
    /// shape either way).
    menu: Element<'a, Message>,
    open: bool,
    on_open: Message,
    on_dismiss: Option<Message>,
}

/// `content`, sending `on_open` when right-clicked (unless something in it
/// took the click, as a link does for its own menu); while `menu` is given,
/// it floats where that click was.
pub fn context_menu<'a, Message: Clone + 'a>(
    content: impl Into<Element<'a, Message>>,
    menu: Option<Element<'a, Message>>,
    on_open: Message,
) -> ContextMenu<'a, Message> {
    ContextMenu {
        content: content.into(),
        open: menu.is_some(),
        menu: menu.unwrap_or_else(|| space().into()),
        on_open,
        on_dismiss: None,
    }
}

impl<Message> ContextMenu<'_, Message> {
    /// Sent on a click outside the menu.
    pub fn on_dismiss(mut self, message: Message) -> Self {
        self.on_dismiss = Some(message);
        self
    }
}

/// Where the last right-click was, relative to the content.
#[derive(Default)]
struct State {
    at: Point,
}

impl<'a, Message: Clone + 'a> Widget<Message, Theme, Renderer> for ContextMenu<'a, Message> {
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<State>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(State::default())
    }

    fn children(&self) -> Vec<Tree> {
        vec![Tree::new(&self.content), Tree::new(&self.menu)]
    }

    fn diff(&self, tree: &mut Tree) {
        tree.diff_children(&[self.content.as_widget(), self.menu.as_widget()]);
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
        if let Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Right)) = event
            && let Some(at) = cursor.position_in(layout.bounds())
        {
            // Also when the content took the click for a menu of its own
            // (a link's), given here in place of this one.
            tree.state.downcast_mut::<State>().at = at;
            if !shell.is_event_captured() {
                shell.publish(self.on_open.clone());
                shell.capture_event();
            }
        }
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
        let at = tree.state.downcast_ref::<State>().at;
        let [content_tree, menu_tree] = &mut tree.children[..] else {
            return None;
        };
        let nested = self.content.as_widget_mut().overlay(
            content_tree,
            layout,
            renderer,
            viewport,
            translation,
        );
        let menu = self.open.then(|| {
            // A point, so the menu opens to the right of and below the cursor.
            let anchor = Rectangle::new(
                layout.bounds().position() + Vector::new(at.x, at.y) + translation,
                Size::ZERO,
            );
            floating(
                &mut self.menu,
                menu_tree,
                anchor,
                Side::Below,
                Edge::Left,
                self.on_dismiss.clone(),
                false,
            )
        });
        let overlays: Vec<_> = nested.into_iter().chain(menu).collect();
        (!overlays.is_empty()).then(|| overlay::Group::with_children(overlays).overlay())
    }
}

impl<'a, Message: Clone + 'a> From<ContextMenu<'a, Message>> for Element<'a, Message> {
    fn from(menu: ContextMenu<'a, Message>) -> Self {
        Element::new(menu)
    }
}

#[cfg(test)]
mod tests {
    use iced::{
        Pixels,
        advanced::{
            clipboard,
            renderer::Headless,
            widget::{self, operation::black_box},
        },
        widget::{mouse_area, text},
    };
    use iced_runtime::user_interface::{Cache, UserInterface};

    use super::*;
    use crate::{fonts, style};

    #[derive(Clone, Debug, PartialEq)]
    enum Clicked {
        Inside,
        Menu,
    }

    /// Where the text "Open" is.
    struct FindOpen(Option<Rectangle>);

    impl Operation for FindOpen {
        fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation)) {
            operate(self);
        }

        fn text(&mut self, _id: Option<&widget::Id>, bounds: Rectangle, text: &str) {
            if text == "Open" {
                self.0 = Some(bounds);
            }
        }
    }

    #[test]
    fn a_menu_the_content_opens_floats_where_it_was_clicked() {
        let mut renderer = iced::futures::executor::block_on(iced::Renderer::new(
            fonts::INTER,
            Pixels(style::TEXT_SIZE),
            None,
        ))
        .unwrap();
        let size = Size::new(400.0, 300.0);
        // Content taking right-clicks for a menu of its own.
        let view = |open: bool| -> Element<'static, Clicked> {
            let content = mouse_area(space().width(Length::Fill).height(Length::Fill))
                .on_right_press(Clicked::Inside);
            let menu = open.then(|| text("Open").into());
            context_menu(content, menu, Clicked::Menu).into()
        };
        let mut ui = UserInterface::build(view(false), size, Cache::default(), &mut renderer);
        let mut messages = Vec::new();
        let _ = ui.update(
            &[Event::Mouse(mouse::Event::ButtonPressed(
                mouse::Button::Right,
            ))],
            mouse::Cursor::Available(Point::new(120.0, 80.0)),
            &mut renderer,
            &mut clipboard::Null,
            &mut messages,
        );
        assert_eq!(messages, [Clicked::Inside], "not this menu");
        let cache = ui.into_cache();
        let mut ui = UserInterface::build(view(true), size, cache, &mut renderer);
        let mut find = FindOpen(None);
        ui.operate(&renderer, &mut black_box(&mut find));
        let open = find.0.expect("the menu is shown");
        assert_eq!(open.x, 120.0);
        assert!((80.0..90.0).contains(&open.y), "{open:?}");
    }
}
