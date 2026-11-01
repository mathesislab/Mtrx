//! The menus' standard actions (`docs/native-behaviour.md`, C): macOS's
//! menu takes ⌘C, ⌘V, ⌘F and the rest before the window, so they come as
//! `Event::StandardAction` (see `menu_bar.rs`). They do what their keys do:
//! in a text input (the editors' are in `editing.rs`), and, when no field
//! takes them, in the app's shortcuts.

use iced::{
    Event, Theme,
    advanced::{
        Clipboard, InputMethod, clipboard, renderer::Headless, widget::operation::focusable,
    },
    event::Status,
    keyboard::{self, Key, Modifiers},
};
use iced_core::event::StandardAction::{self, *};
use iced_runtime::user_interface::{Cache, State, UserInterface};

use super::*;

const INPUT: &str = "input";

/// The clipboard's text.
#[derive(Default)]
struct Board(Option<String>);

impl Clipboard for Board {
    fn read(&self, _kind: clipboard::Kind) -> Option<String> {
        self.0.clone()
    }

    fn write(&mut self, _kind: clipboard::Kind, contents: String) {
        self.0 = Some(contents);
    }
}

/// A focused text input, its value as its messages set it.
struct Input {
    value: String,
    renderer: iced::Renderer,
    cache: Cache,
    clipboard: Board,
}

impl Input {
    fn new(value: &str) -> Self {
        let renderer = iced::futures::executor::block_on(iced::Renderer::new(
            fonts::INTER,
            iced::Pixels(style::TEXT_SIZE),
            None,
        ))
        .unwrap();
        let mut input = Self {
            value: value.to_owned(),
            renderer,
            cache: Cache::default(),
            clipboard: Board::default(),
        };
        let value = input.value.clone();
        let mut ui = ui(&value, &mut input.cache, &mut input.renderer);
        ui.operate(&input.renderer, &mut focusable::focus(INPUT.into()));
        input.cache = ui.into_cache();
        input
    }

    /// `events` through the input, its value set from its messages; whether
    /// it took each, and its input method then.
    fn update(&mut self, events: &[Event]) -> (Vec<Status>, InputMethod) {
        let value = self.value.clone();
        let mut ui = ui(&value, &mut self.cache, &mut self.renderer);
        let mut messages = Vec::new();
        let (state, statuses) = ui.update(
            events,
            mouse::Cursor::Unavailable,
            &mut self.renderer,
            &mut self.clipboard,
            &mut messages,
        );
        self.cache = ui.into_cache();
        if let Some(value) = messages.pop() {
            self.value = value;
        }
        let State::Updated { input_method, .. } = state else {
            panic!("outdated");
        };
        (statuses, input_method)
    }

    /// ⌘ with `letter`, the modifiers first: the input reads them from
    /// there.
    fn command(&mut self, letter: &str) -> Status {
        let modifiers = |modifiers| Event::Keyboard(keyboard::Event::ModifiersChanged(modifiers));
        let press = Event::Keyboard(keyboard::Event::KeyPressed {
            key: Key::Character(letter.into()),
            modified_key: Key::Character(letter.into()),
            physical_key: keyboard::key::Physical::Unidentified(
                keyboard::key::NativeCode::Unidentified,
            ),
            location: keyboard::Location::Standard,
            modifiers: Modifiers::COMMAND,
            text: Some(letter.into()),
            repeat: false,
        });
        let events = [
            modifiers(Modifiers::COMMAND),
            press,
            modifiers(Modifiers::empty()),
        ];
        self.update(&events).0[1]
    }

    fn act(&mut self, action: StandardAction) -> Status {
        self.update(&[Event::StandardAction(action)]).0[0]
    }

    /// Whether its input method enables each of `actions`.
    fn enables(&mut self, actions: &[StandardAction]) -> Vec<bool> {
        let redraw = Event::Window(window::Event::RedrawRequested(iced::time::Instant::now()));
        let (_, input_method) = self.update(&[redraw]);
        actions
            .iter()
            .map(|action| input_method.enables(*action))
            .collect()
    }
}

fn ui<'a>(
    value: &'a str,
    cache: &mut Cache,
    renderer: &mut iced::Renderer,
) -> UserInterface<'a, String, Theme, iced::Renderer> {
    let input = iced::widget::text_input("", value)
        .id(INPUT)
        .on_input(|value| value);
    UserInterface::build(
        input,
        Size::new(400.0, 100.0),
        std::mem::take(cache),
        renderer,
    )
}

/// Select All, Copy, Cut and Paste in a text input (the room search, the
/// find bar, a dialog's field) do what ⌘A, ⌘C, ⌘X and ⌘V do; Paste as
/// Plain Text pastes, plain text being all it takes.
#[test]
fn the_menus_actions_in_a_text_input_do_what_their_keys_do() {
    let (mut keys, mut menu) = (Input::new("Basal melt"), Input::new("Basal melt"));
    let steps = [
        ("a", SelectAll),
        ("c", Copy),
        ("x", Cut),
        ("v", Paste),
        ("v", Paste),
    ];
    for (letter, action) in steps {
        let status = keys.command(letter);
        assert_eq!(menu.act(action), status, "{action:?}");
        assert_eq!(menu.value, keys.value, "{action:?}");
        assert_eq!(menu.clipboard.0, keys.clipboard.0, "{action:?}");
    }
    assert_eq!(menu.value, "Basal meltBasal melt");
    assert_eq!(menu.clipboard.0.as_deref(), Some("Basal melt"));
    assert_eq!(menu.act(PasteAsPlainText), Status::Captured);
    assert_eq!(menu.value, "Basal meltBasal meltBasal melt");
    // Undo is not its: it has no history.
    assert_eq!(menu.act(Undo), Status::Ignored);
}

/// A text input enables Cut with a selection, and never Undo or Redo.
#[test]
fn a_text_input_enables_cut_with_a_selection() {
    let actions = [Undo, Redo, Cut, Copy, Paste, SelectAll];
    let mut input = Input::new("Basal melt");
    assert_eq!(
        input.enables(&actions),
        [false, false, false, true, true, true]
    );
    input.act(SelectAll);
    assert_eq!(
        input.enables(&actions),
        [false, false, true, true, true, true]
    );
}

/// With no field focused, the menus keep what the app does itself: Copy and
/// Select All for the timeline, Find, and the text size.
#[test]
fn without_a_field_the_app_keeps_its_actions() {
    let enabled: Vec<_> = StandardAction::ALL
        .into_iter()
        .filter(|action| InputMethod::Disabled.enables(*action))
        .collect();
    assert_eq!(
        enabled,
        [
            Copy,
            SelectAll,
            Find,
            FindNext,
            FindPrevious,
            ZoomIn,
            ZoomOut,
            ZoomReset
        ]
    );
}

/// No field taking them, Copy and Select All act on the timeline's
/// selection, Find and Find Next and Previous on the find bar, and Bigger,
/// Smaller and Actual Size on the text size, as their keys do.
#[test]
fn the_menus_actions_no_field_takes_go_to_the_app() {
    let id = window::Id::unique();
    let shortcut =
        |action, status| format!("{:?}", shortcuts(Event::StandardAction(action), status, id));
    let expected = |message: Message| format!("{:?}", Some(message));
    let cases = [
        (
            Copy,
            Message::Shown(room::Message::Select(selection::Message::Copy)),
        ),
        (
            SelectAll,
            Message::Shown(room::Message::Select(selection::Message::SelectAll)),
        ),
        (
            Find,
            Message::Shown(room::Message::Find(find::Message::Open)),
        ),
        (
            FindNext,
            Message::Shown(room::Message::Find(find::Message::Next)),
        ),
        (
            FindPrevious,
            Message::Shown(room::Message::Find(find::Message::Previous)),
        ),
    ];
    for (action, message) in cases {
        assert_eq!(shortcut(action, Status::Ignored), expected(message));
    }
    // A field took them.
    assert_eq!(shortcut(Copy, Status::Captured), "None");
    assert_eq!(shortcut(SelectAll, Status::Captured), "None");

    for (action, zoom) in [
        (ZoomIn, Zoom::In),
        (ZoomOut, Zoom::Out),
        (ZoomReset, Zoom::Reset),
    ] {
        let zoomed = window_keys(Event::StandardAction(action), Status::Captured, id);
        assert_eq!(format!("{zoomed:?}"), expected(Message::Zoom(zoom)));
    }
}
