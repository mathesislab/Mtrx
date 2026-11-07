//! The window: its size, place, scale and theme, and closing it.

use std::path::PathBuf;

use iced::{
    Point, Rectangle, Size, Subscription, Task, Theme, mouse,
    theme::Mode,
    window::{self, Id},
};

use super::{App, Screen, as_key};
use crate::{
    app, math,
    placement::{self, Frame, Placement},
    presence,
    room::keep_now,
    settings::Settings,
    style::{self, MESSAGE_INSET, SPACE_BAR_WIDTH},
    text_size::Zoom,
};

/// The room list's width limits, and its draggable edge's.
const MIN_ROOM_LIST_WIDTH: f32 = 200.0;
const MAX_ROOM_LIST_WIDTH: f32 = 520.0;
pub const RESIZE_HANDLE: f32 = 5.0;

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
    /// close button (or Ctrl+W, Alt+F4, ...) was pressed. See [`closing`].
    Close(Id),
}

#[derive(Debug, Clone, Copy)]
pub enum Resize {
    Start,
    /// The cursor's x in the window.
    Move(f32),
    End,
}

/// The window's size, place and scale, and the system's theme.
pub struct Window {
    /// The window's size and position, to open it there again.
    pub placement: Placement,
    /// Where the placement is saved; `None` in tests.
    pub placement_file: Option<PathBuf>,
    /// The window's size in logical pixels, whatever the text size.
    pub size: Size,
    /// The display's scale: physical pixels per logical pixel.
    pub scale: f32,
    /// Light or dark as iced reports the system (on macOS, the app's
    /// appearance, see [`crate::appearance`]).
    pub system_mode: Mode,
    /// The room list's edge is being dragged.
    pub resizing: bool,
    /// The window has the keyboard focus (notifications only when not
    /// looking at the room).
    pub focused: bool,
}

/// What a window message changed besides the window.
pub enum Changed {
    Nothing,
    /// Its size in layout units or its pixels: the PDF viewer follows.
    Viewport,
    /// The settings, to save.
    Settings,
}

impl Window {
    /// The window as `placement` left it, saved to `placement_file`.
    pub fn new(placement: Placement, placement_file: Option<PathBuf>) -> Self {
        Self {
            size: placement
                .frame
                .map_or(placement::DEFAULT_SIZE, |frame| frame.size()),
            placement,
            placement_file,
            scale: 1.0,
            system_mode: Mode::Light,
            resizing: false,
            focused: true,
        }
    }

    /// The display's scale and the system's theme, and whether the window
    /// opened on a display: asked for once it opened.
    pub fn boot() -> Task<app::Message> {
        let scale = window::oldest().then(|id| match id {
            Some(id) => window::scale_factor(id).map(|scale| to(Message::ScaleFactor(scale))),
            None => Task::none(),
        });
        let mode = iced::system::theme().map(|mode| to(Message::Theme(mode)));
        // See `Message::Opened`.
        let opened = window::oldest().then(|id| match id {
            Some(id) => window::run(id, placement::displays).then(move |displays| {
                window::position(id)
                    .map(move |position| to(Message::Opened(id, displays.clone(), position)))
            }),
            None => Task::none(),
        });
        Task::batch([scale, mode, opened])
    }

    /// `message`, but closing, the focus and the PDF viewer's zoom (see
    /// [`App::window_update`]); `settings` change with the text size and
    /// the room list's width.
    pub fn update(
        &mut self,
        message: Message,
        settings: &mut Settings,
    ) -> (Task<app::Message>, Changed) {
        let task = match message {
            Message::Resized(size) => {
                // Reported in layout units.
                self.size = size * settings.text_size.scale();
                return (Task::none(), Changed::Viewport);
            }
            Message::ScaleFactor(scale) => {
                self.scale = scale;
                return (Task::none(), Changed::Viewport);
            }
            Message::Zoom(zoom) => {
                settings.text_size = settings.text_size.zoom(zoom);
                return (Task::none(), Changed::Settings);
            }
            Message::Resize(Resize::Start) => {
                self.resizing = true;
                Task::none()
            }
            Message::Resize(Resize::Move(x)) => {
                settings.room_list_width =
                    (x - SPACE_BAR_WIDTH).clamp(MIN_ROOM_LIST_WIDTH, MAX_ROOM_LIST_WIDTH);
                Task::none()
            }
            Message::Resize(Resize::End) => {
                self.resizing = false;
                return (Task::none(), Changed::Settings);
            }
            Message::Opened(id, Some(displays), Some(position)) => {
                // Left on a display since unplugged, the window opens on the
                // main one instead, as the first time.
                let Some(left) = self.placement.frame else {
                    return (Task::none(), Changed::Nothing);
                };
                if Frame::new(position, left.size()).reachable(&displays) {
                    return (Task::none(), Changed::Nothing);
                }
                let Some(frame) = Frame::fallback(&displays) else {
                    return (Task::none(), Changed::Nothing);
                };
                Task::batch([
                    window::maximize(id, false),
                    window::resize(id, frame.size() / settings.text_size.scale()),
                    window::move_to(id, frame.position()),
                ])
            }
            Message::Opened(..) => Task::none(),
            // Not while closing hid it (macOS): where it is not shown.
            Message::Moved(id) => window::mode(id).then(move |mode| {
                if mode == window::Mode::Hidden {
                    return Task::none();
                }
                window::is_maximized(id).then(move |maximized| {
                    window::position(id).map(move |position| {
                        to(Message::Placed {
                            position,
                            maximized,
                            fullscreen: mode == window::Mode::Fullscreen,
                        })
                    })
                })
            }),
            Message::Placed {
                position: Some(position),
                maximized,
                fullscreen,
            } => {
                let frame = Frame::new(position, self.size);
                if self.placement.follow(frame, maximized, fullscreen)
                    && let Some(file) = &self.placement_file
                {
                    self.placement.save(file);
                }
                Task::none()
            }
            Message::Placed { position: None, .. } => Task::none(),
            Message::Theme(mode) => {
                self.system_mode = mode;
                Task::none()
            }
            Message::Focused(focused) => {
                self.focused = focused;
                Task::none()
            }
            // The app's.
            Message::Close(_) => Task::none(),
        };
        (task, Changed::Nothing)
    }

    /// The theme the app is drawn in.
    pub fn theme(&self, settings: &Settings) -> Theme {
        settings.appearance.theme(self.system_mode)
    }

    /// The window's size in layout units.
    pub fn viewport(&self, settings: &Settings) -> Size {
        self.size / settings.text_size.scale()
    }

    /// Physical pixels per layout unit, for drawing images sharp.
    pub fn pixel_scale(&self, settings: &Settings) -> f32 {
        self.scale * settings.text_size.scale()
    }

    /// The width of the open room, right of the room list.
    pub fn room_width(&self, settings: &Settings) -> f32 {
        self.viewport(settings).width - SPACE_BAR_WIDTH - settings.room_list_width - RESIZE_HANDLE
    }

    /// How math is typeset for the room shown.
    pub fn math_params(&self, settings: &Settings) -> math::Params {
        let theme = self.theme(settings);
        let palette = theme.extended_palette();
        let max_width = self.room_width(settings) - MESSAGE_INSET;
        math::Params {
            max_width: max_width.max(200.0),
            scale: self.pixel_scale(settings),
            text: palette.background.base.text,
            link: palette.primary.base.color,
            muted: style::muted(&theme),
        }
    }

    /// Its size, place and keys, the system's theme, and dragging the room
    /// list's edge.
    pub fn subscription(&self) -> Subscription<app::Message> {
        let mut subscriptions = vec![
            window::resize_events().map(|(_, size)| to(Message::Resized(size))),
            window::events().filter_map(|(id, event)| match event {
                window::Event::Moved(_) | window::Event::Resized(_) => Some(to(Message::Moved(id))),
                _ => None,
            }),
            window::close_requests().map(|id| to(Message::Close(id))),
            iced::system::theme_changes().map(|mode| to(Message::Theme(mode))),
            // Also on the sign-in screen.
            iced::event::listen_with(keys),
        ];
        if self.resizing {
            subscriptions.push(iced::event::listen_with(|event, _, _| match event {
                iced::Event::Mouse(mouse::Event::CursorMoved { position }) => {
                    Some(to(Message::Resize(Resize::Move(position.x))))
                }
                iced::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
                    Some(to(Message::Resize(Resize::End)))
                }
                _ => None,
            }));
        }
        Subscription::batch(subscriptions)
    }
}

impl App {
    /// A message to the window, and what it changes elsewhere: closing it
    /// ends the session, the focus is input, and zooming zooms the PDF
    /// viewer while it shows.
    pub(super) fn window_update(&mut self, message: Message) -> Task<app::Message> {
        let input = match message {
            Message::Close(id) => return self.close(id),
            Message::Zoom(zoom) => {
                if let Screen::Main(main) = &mut self.screen
                    && let Some(pdf) = main.modal.pdf()
                {
                    return pdf.update(crate::pdf_viewer::Message::Zoom(match zoom {
                        Zoom::In => crate::pdf_viewer::Zoom::In,
                        Zoom::Out => crate::pdf_viewer::Zoom::Out,
                        Zoom::Reset => crate::pdf_viewer::Zoom::Fit,
                    }));
                }
                Task::none()
            }
            // The window was in use until now.
            Message::Focused(_) => match &self.screen {
                Screen::Main(main) => main.activity(mtrx_core::Activity::Input),
                _ => Task::none(),
            },
            _ => Task::none(),
        };
        let (task, changed) = self.window.update(message, &mut self.settings);
        let changed = match changed {
            Changed::Nothing => Task::none(),
            Changed::Viewport => self.pdf_resized(),
            Changed::Settings => {
                self.save_settings();
                Task::none()
            }
        };
        Task::batch([input, task, changed])
    }

    /// Quit, or close the window where that quits: the session ends first.
    fn close(&mut self, id: Id) -> Task<app::Message> {
        // What is being written is kept before the session ends.
        let draft = match &self.screen {
            Screen::Main(main) => {
                (main.pane.room()).map(|room| keep_now(room.drafts.clone(), room.draft()))
            }
            _ => None,
        }
        .unwrap_or_else(Task::none);
        // Others see us offline now, not once the server forgets us.
        let offline = match &self.screen {
            Screen::Main(main) if !main.presences.unavailable() => {
                let session = main.session.clone();
                Task::future(async move {
                    presence::go_offline(&session, mtrx_core::Activity::Background).await
                })
                .discard()
            }
            _ => Task::none(),
        };
        // iced drops the app only after its runtime has stopped, when
        // the SDK's stores can no longer close: end the session (once
        // it is offline), or a browser sign-in, now.
        self.screen = Screen::Closing;
        Task::batch([offline, draft]).chain(window::close(id))
    }
}

/// The window's settings for closing it (`docs/native-behaviour.md`, A1). On
/// macOS closing it hides it, as a Mac app's main window, and the app goes
/// on (synced, notifying); clicking its Dock icon shows it again (our
/// winit). Quitting, and elsewhere closing, requests to close it: the app
/// goes offline and ends its session first, see [`Message::Close`].
pub fn closing(mut window: window::Settings) -> window::Settings {
    window.exit_on_close_request = false;
    #[cfg(target_os = "macos")]
    {
        window.platform_specific.hides_on_close = true;
    }
    window
}

/// Keys on every screen, whatever is focused: ⌘+ ⌘− ⌘0 zoom (Ctrl on Windows
/// and Linux), and Ctrl+W closes the window as its close button does. On
/// macOS ⌘W is the Window menu's Close, which hides it (see [`closing`]).
#[cfg_attr(target_os = "macos", allow(unused_variables))]
pub(super) fn keys(
    event: iced::Event,
    _status: iced::event::Status,
    window: Id,
) -> Option<app::Message> {
    use iced::keyboard;
    match as_key(event) {
        iced::Event::Keyboard(keyboard::Event::KeyPressed { key, modifiers, .. }) => {
            #[cfg(not(target_os = "macos"))]
            if key.as_ref() == keyboard::Key::Character("w") && modifiers.command() {
                return Some(to(Message::Close(window)));
            }
            Zoom::from_key(&key, modifiers).map(|zoom| to(Message::Zoom(zoom)))
        }
        _ => None,
    }
}

/// `message` for the window, as the app gets it.
pub fn to(message: Message) -> app::Message {
    app::Message::Window(message)
}
