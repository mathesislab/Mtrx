//! The frame of dialogs, cards and the settings panel: a header that stays
//! (the title, its actions and a close button, also Esc) over a body that
//! scrolls.

use iced::{
    Alignment, Element, Length, Theme,
    widget::{button, column, container, row, rule, stack, text},
};

use crate::{app, fonts, hint, icons, scroll, style};

/// Most of the window's height; the body scrolls beyond.
const MAX_HEIGHT: f32 = 680.0;
/// Below a body with a status pill: the pill's height.
const STATUS_ROOM: f32 = 28.0;

/// `title` is a heading, or any row (e.g. a picture and a name); `actions`
/// go before the close button.
pub fn frame<'a>(
    title: impl Into<Element<'a, app::Message>>,
    actions: impl IntoIterator<Item = Element<'a, app::Message>>,
    close: app::Message,
    body: impl Into<Element<'a, app::Message>>,
    width: f32,
) -> Element<'a, app::Message> {
    with_bar(title, actions, close, None, body, None, width)
}

/// A [`frame`] with a `bar` below the header that stays too, e.g. tabs,
/// and a `status` pill over the bottom of what scrolls below it.
pub fn with_bar<'a>(
    title: impl Into<Element<'a, app::Message>>,
    actions: impl IntoIterator<Item = Element<'a, app::Message>>,
    close: app::Message,
    bar: Option<Element<'a, app::Message>>,
    body: impl Into<Element<'a, app::Message>>,
    status: Option<Element<'a, app::Message>>,
    width: f32,
) -> Element<'a, app::Message> {
    let mut header = row![container(title).width(Length::Fill)]
        .spacing(8)
        .align_y(Alignment::Center);
    for action in actions {
        header = header.push(action);
    }
    header = header.push(close_button(close));
    let top = container(header).padding(iced::Padding::new(12.0).left(20.0));
    let scrolling = |padding| scroll::vertical(container(body).padding(padding));
    let frame = match bar {
        None => column![
            top,
            rule::horizontal(1).style(style::separator),
            scrolling(iced::Padding::new(20.0).top(16.0)),
        ],
        // Below a bar that stays, a box shows what scrolls.
        Some(bar) => {
            // Room below the content for the status, so it hides nothing.
            let padding = iced::Padding::new(14.0).bottom(if status.is_some() {
                14.0 + STATUS_ROOM
            } else {
                14.0
            });
            let scrolled = container(scrolling(padding))
                .style(style::section)
                .clip(true);
            let scrolled: Element<'a, app::Message> = match status {
                Some(status) => stack![
                    scrolled,
                    // Clear of the scrollbar.
                    container(status)
                        .align_right(Length::Fill)
                        .align_bottom(Length::Fill)
                        .padding(iced::Padding::new(10.0).right(24.0)),
                ]
                .into(),
                None => scrolled.into(),
            };
            column![
                top,
                container(bar).padding([0, 20]),
                container(scrolled).padding(iced::Padding::new(20.0).top(12.0)),
            ]
        }
    };
    container(frame)
        .width(width)
        .max_height(MAX_HEIGHT)
        .style(style::dialog)
        .into()
}

/// A plain title for [`frame`].
pub fn title<'a>(title: impl text::IntoFragment<'a>) -> Element<'a, app::Message> {
    text(title).size(18).font(fonts::SEMIBOLD).into()
}

/// The header's main button: `None` while it cannot be used yet.
pub fn primary<'a>(label: &'a str, on_press: Option<app::Message>) -> Element<'a, app::Message> {
    button(text(label).size(13))
        .padding([4, 12])
        .style(style::primary_button)
        .on_press_maybe(on_press)
        .into()
}

/// What a frame's body shows while its content loads: room for it, so the
/// frame does not jump, and "Loading…" in the middle.
pub fn loading<'a, M: 'a>() -> Element<'a, M> {
    container(text("Loading…").size(15).style(text::secondary))
        .center_x(Length::Fill)
        .center_y(LOADING_HEIGHT)
        .into()
}

const LOADING_HEIGHT: f32 = 260.0;

/// A cross that closes (also Esc), as in every frame's header.
pub fn close_button<'a, M: Clone + 'a>(close: M) -> Element<'a, M> {
    let cross = button(icons::cross(14.0, |theme: &Theme| {
        theme
            .extended_palette()
            .background
            .base
            .text
            .scale_alpha(0.7)
    }))
    .padding(7)
    .style(style::list_item(false))
    .on_press(close);
    hint::hint(cross, "Close (Esc)", hint::Position::Bottom)
}
