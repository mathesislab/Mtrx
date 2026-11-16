//! Lists and bodies that scroll with the scrollbar beside them rather than
//! over them, so it never covers a field or text at the edge.

use iced::widget::{Scrollable, scrollable};

/// Space between the content and the scrollbar.
const GAP: f32 = 4.0;

/// `content` scrolling vertically, its scrollbar beside it.
pub fn vertical<'a, M: 'a>(content: impl Into<iced::Element<'a, M>>) -> Scrollable<'a, M> {
    scrollable(content).direction(scrollable::Direction::Vertical(
        scrollable::Scrollbar::new().spacing(GAP),
    ))
}
