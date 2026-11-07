//! The window: its size, place, scale and theme, and closing it.

use iced::{Point, Rectangle, Size, theme::Mode, window::Id};

use crate::text_size::Zoom;

#[derive(Debug, Clone)]
pub enum Message {
    /// The window's size, in layout units.
    Resized(Size),
    /// The window opened at `.2` (its outer position), with these displays
    /// attached (`None` where the platform does not say).
    Opened(Id, Option<Vec<Rectangle>>, Option<Point>),
    /// The window was moved or resized.
    Moved(Id),
    /// Where the window now is, to open there next time.
    Placed {
        position: Option<Point>,
        maximized: bool,
        fullscreen: bool,
    },
    ScaleFactor(f32),
    /// Light or dark, as the system says.
    Theme(Mode),
    /// Dragging the room list's edge.
    Resize(Resize),
    /// Text size, or the PDF viewer's zoom while it is open.
    Zoom(Zoom),
    /// The window gained (`true`) or lost the keyboard focus.
    Focused(bool),
    /// Quit (⌘Q, in the Dock, logging out) on macOS; elsewhere the window's
    /// close button (or Ctrl+W, Alt+F4, ...) was pressed. See
    /// [`super::closing`].
    Close(Id),
}

#[derive(Debug, Clone, Copy)]
pub enum Resize {
    Start,
    /// The cursor's x in the window.
    Move(f32),
    End,
}
