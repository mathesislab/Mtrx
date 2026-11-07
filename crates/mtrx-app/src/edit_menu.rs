//! The menu of a field one writes in (right-click), the composer or a
//! message being edited: for a misspelled word its spellings, Learn
//! Spelling and Ignore Spelling; then Cut, Copy, Paste, Select All and, on
//! macOS, Emoji & Symbols.

use std::ops::Range;

use iced::{
    Element,
    widget::text_editor::{Content, Cursor, Position},
};

use crate::{
    context_menu::{self, context_menu},
    spelling,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Message {
    Open,
    Close,
    /// Replace the misspelled word by its spelling at this index.
    Correct(usize),
    LearnSpelling,
    IgnoreSpelling,
    Cut,
    Copy,
    Paste,
    SelectAll,
    /// The system's window of emoji and symbols; what is picked there goes
    /// into the field.
    #[cfg(target_os = "macos")]
    EmojiAndSymbols,
}

/// An open menu.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Menu {
    /// The misspelled word right-clicked, if any.
    pub misspelled: Option<Misspelled>,
}

impl Menu {
    /// The menu opened in `content`, right-clicked where its caret now is.
    pub fn at(content: &Content) -> Self {
        Self {
            misspelled: Misspelled::at(content),
        }
    }
}

/// A misspelled word in a field, with its spellings.
#[derive(Debug, Clone, PartialEq)]
pub struct Misspelled {
    line: usize,
    range: Range<usize>,
    pub word: String,
    pub suggestions: Vec<String>,
}

impl Misspelled {
    /// The misspelled word at `content`'s caret, while spell checking is
    /// on.
    fn at(content: &Content) -> Option<Self> {
        spelling::marking()?;
        let Cursor {
            position,
            selection: None,
        } = content.cursor()
        else {
            return None;
        };
        let line = content.line(position.line)?.text;
        let range = spelling::word_at(&line, position.column)?;
        Some(Self {
            line: position.line,
            word: line[range.clone()].to_owned(),
            suggestions: spelling::suggestions(&line, range.clone()),
            range,
        })
    }

    /// Select the word in `content`, for a spelling to replace it.
    pub fn select(&self, content: &mut Content) {
        let at = |column| Position {
            line: self.line,
            column,
        };
        content.move_to(Cursor {
            position: at(self.range.end),
            selection: Some(at(self.range.start)),
        });
    }
}

/// What `menu` offers: the spellings of its misspelled word, then Cut and
/// Copy with text `selected`.
pub fn items(menu: &Menu, selected: bool) -> Vec<(String, Message)> {
    let mut items = Vec::new();
    if let Some(misspelled) = &menu.misspelled {
        items.extend(
            (misspelled.suggestions.iter().enumerate())
                .map(|(i, suggestion)| (suggestion.clone(), Message::Correct(i))),
        );
        items.extend([
            ("Learn Spelling".to_owned(), Message::LearnSpelling),
            ("Ignore Spelling".to_owned(), Message::IgnoreSpelling),
        ]);
    }
    if selected {
        items.extend([("Cut", Message::Cut), ("Copy", Message::Copy)].map(owned));
    }
    items.extend(
        [
            ("Paste", Message::Paste),
            ("Select All", Message::SelectAll),
        ]
        .map(owned),
    );
    #[cfg(target_os = "macos")]
    items.push(owned(("Emoji & Symbols", Message::EmojiAndSymbols)));
    items
}

fn owned((label, message): (&str, Message)) -> (String, Message) {
    (label.to_owned(), message)
}

/// `editor` with its menu on right-click: shown while open (`menu`), Cut
/// and Copy with text `selected`; `to` makes the message of a choice.
pub fn with_menu<'a, M: Clone + 'a>(
    editor: impl Into<Element<'a, M>>,
    menu: Option<&Menu>,
    selected: bool,
    to: impl Fn(Message) -> M,
) -> Element<'a, M> {
    let menu = menu.map(|menu| {
        context_menu::menu(
            items(menu, selected)
                .into_iter()
                .map(|(label, message)| (label, to(message))),
        )
    });
    context_menu(editor, menu, to(Message::Open))
        .on_dismiss(to(Message::Close))
        .into()
}

/// Open the system's Emoji & Symbols window, which types what is picked
/// into the focused field.
#[cfg(target_os = "macos")]
pub fn emoji_and_symbols() {
    use objc2::MainThreadMarker;
    use objc2_app_kit::NSApplication;

    // AppKit only on the main thread, where iced updates the app (not in
    // tests).
    if let Some(main_thread) = MainThreadMarker::new() {
        NSApplication::sharedApplication(main_thread).orderFrontCharacterPalette(None);
    }
}

#[cfg(test)]
mod tests {
    use iced::{
        Event, Size, mouse,
        widget::text_editor::{self, Content},
    };
    use iced_test::simulator::Simulator;

    use super::*;

    /// What an editor with its menu sends.
    #[derive(Debug, Clone)]
    enum Sent {
        Action(text_editor::Action),
        Menu(Message),
    }

    fn labels(menu: &Menu, selected: bool) -> Vec<String> {
        items(menu, selected)
            .into_iter()
            .map(|(label, _)| label)
            .collect()
    }

    #[test]
    fn cut_and_copy_need_a_selection() {
        let mut expected = vec!["Cut", "Copy", "Paste", "Select All"];
        if cfg!(target_os = "macos") {
            expected.push("Emoji & Symbols");
        }
        assert_eq!(labels(&Menu::default(), true), expected);
        assert_eq!(labels(&Menu::default(), false), expected[2..]);
    }

    #[test]
    fn a_misspelled_word_offers_its_spellings_first() {
        let menu = Menu {
            misspelled: Some(Misspelled {
                line: 0,
                range: 4..12,
                word: "tommorow".into(),
                suggestions: vec!["tomorrow".into(), "tomorrows".into()],
            }),
        };
        let labels = labels(&menu, false);
        assert_eq!(
            labels[..5],
            [
                "tomorrow",
                "tomorrows",
                "Learn Spelling",
                "Ignore Spelling",
                "Paste"
            ]
        );
        assert_eq!(items(&menu, false)[1].1, Message::Correct(1));
    }

    #[test]
    fn a_word_is_selected_to_be_replaced() {
        let mut content = Content::<iced::Renderer>::with_text("one\nsee tommorow then");
        let misspelled = Misspelled {
            line: 1,
            range: 4..12,
            word: "tommorow".into(),
            suggestions: Vec::new(),
        };
        misspelled.select(&mut content);
        assert_eq!(content.selection().as_deref(), Some("tommorow"));
    }

    /// What an editor wrapped as a message being edited is, its menu `open`
    /// (at its corner, as nothing was right-clicked yet), sends for
    /// `button` clicked at `at`.
    fn sends(open: bool, button: mouse::Button, at: (f32, f32)) -> Vec<Sent> {
        let content = Content::<iced::Renderer>::with_text("Melt");
        let editor = iced::widget::text_editor(&content)
            .on_action(Sent::Action)
            .key_binding(text_editor::Binding::from_key_press);
        let menu = open.then(Menu::default);
        let field = with_menu(editor, menu.as_ref(), true, Sent::Menu);
        let mut ui = Simulator::with_size(
            crate::fonts::settings(),
            Size::new(400.0, 300.0),
            iced::widget::column![field],
        );
        ui.point_at(at);
        let _ = ui.simulate([
            Event::Mouse(mouse::Event::ButtonPressed(button)),
            Event::Mouse(mouse::Event::ButtonReleased(button)),
        ]);
        ui.into_messages()
            .filter(|message| !matches!(message, Sent::Action(_)))
            .collect()
    }

    #[test]
    fn a_right_click_puts_the_caret_there_then_opens_it() {
        let content = Content::<iced::Renderer>::with_text("Melt");
        let editor = iced::widget::text_editor(&content).on_action(Sent::Action);
        let field = with_menu(editor, None, false, Sent::Menu);
        let mut ui = Simulator::with_size(
            crate::fonts::settings(),
            Size::new(400.0, 300.0),
            iced::widget::column![field],
        );
        ui.point_at((20.0, 10.0));
        let _ = ui.simulate([Event::Mouse(mouse::Event::ButtonPressed(
            mouse::Button::Right,
        ))]);
        let sent: Vec<_> = ui.into_messages().collect();
        assert!(
            matches!(
                sent[..],
                [
                    Sent::Action(text_editor::Action::Click(_)),
                    Sent::Menu(Message::Open)
                ]
            ),
            "{sent:?}"
        );
    }

    #[test]
    fn right_click_opens_it_and_an_item_is_chosen() {
        use mouse::Button::{Left, Right};
        let sent = sends(false, Right, (20.0, 10.0));
        assert!(matches!(sent[..], [Sent::Menu(Message::Open)]), "{sent:?}");
        // The third entry, Paste.
        let sent = sends(true, Left, (60.0, 4.0 + 2.5 * 26.0));
        assert!(matches!(sent[..], [Sent::Menu(Message::Paste)]), "{sent:?}");
        // A click beside it closes it.
        let sent = sends(true, Left, (380.0, 280.0));
        assert!(matches!(sent[..], [Sent::Menu(Message::Close)]), "{sent:?}");
    }

    /// Emoji & Symbols takes the key window while open and types the pick
    /// into the field focused before: the field keeps its input method.
    #[test]
    fn a_focused_field_takes_input_while_its_window_is_not_key() {
        use iced::{
            Point, Theme,
            advanced::{InputMethod, renderer::Headless},
            window,
        };
        use iced_runtime::user_interface::{Cache, State, UserInterface};

        let content = Content::<iced::Renderer>::with_text("Melt");
        let view = |input: bool| -> Element<'_, ()> {
            if input {
                iced::widget::text_input("", "Melt").on_input(|_| ()).into()
            } else {
                iced::widget::text_editor(&content).on_action(|_| ()).into()
            }
        };
        let mut renderer = iced::futures::executor::block_on(iced::Renderer::new(
            crate::fonts::INTER,
            iced::Pixels(crate::style::TEXT_SIZE),
            None,
        ))
        .unwrap();
        for input in [false, true] {
            let mut ui = UserInterface::<_, Theme, _>::build(
                view(input),
                Size::new(300.0, 100.0),
                Cache::default(),
                &mut renderer,
            );
            let mut enabled = |events: &[Event]| {
                let (state, _) = ui.update(
                    events,
                    mouse::Cursor::Available(Point::new(10.0, 10.0)),
                    &mut renderer,
                    &mut iced::advanced::clipboard::Null,
                    &mut Vec::new(),
                );
                match state {
                    State::Updated { input_method, .. } => {
                        matches!(input_method, InputMethod::Enabled { .. })
                    }
                    State::Outdated => panic!("outdated"),
                }
            };
            let redraw =
                || Event::Window(window::Event::RedrawRequested(iced::time::Instant::now()));
            let click = Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left));
            assert!(!enabled(&[redraw()]), "input {input}: not focused");
            let _ = enabled(&[click]);
            assert!(enabled(&[redraw()]), "input {input}: focused");
            assert!(
                enabled(&[Event::Window(window::Event::Unfocused), redraw()]),
                "input {input}: focused, window not key"
            );
        }
    }

    /// The composer's text around the caret, as an input method is told
    /// it: the lines of the caret and the selection, in bytes.
    #[test]
    fn the_composer_tells_the_text_around_the_caret() {
        use text_editor::{Action, Motion};

        let around = |content: &Content<iced::Renderer>| {
            let around = content.surrounding_text().unwrap();
            (around.text, around.cursor, around.anchor)
        };
        let mut content = Content::<iced::Renderer>::with_text("Brûlée\nCrème\nend");
        // The caret after "Crè", on the second line.
        content.perform(Action::Move(Motion::DocumentStart));
        content.perform(Action::Move(Motion::Down));
        for _ in 0..3 {
            content.perform(Action::Move(Motion::Right));
        }
        assert_eq!(around(&content), ("Crème".to_owned(), 4, 4));
        // Selected up to the same place on the first line.
        content.perform(Action::Select(Motion::Up));
        assert_eq!(around(&content), ("Brûlée\nCrème".to_owned(), 4, 13));
    }

    /// Holding "e" for its accents and picking "é": macOS deletes the "e"
    /// before the caret, then commits "é".
    #[test]
    fn the_composer_deletes_around_the_caret_for_an_input_method() {
        use std::sync::Arc;
        use text_editor::{Action, Edit, Motion};

        let edited = |text: &str, setup: &[Action], before, after, commit: &str| {
            let mut content = Content::<iced::Renderer>::with_text(text);
            content.perform(Action::Move(Motion::DocumentEnd));
            for action in setup {
                content.perform(action.clone());
            }
            content.perform(Action::Edit(Edit::DeleteSurrounding { before, after }));
            let kept = content.selection();
            content.perform(Action::Edit(Edit::Paste(Arc::new(commit.to_owned()))));
            (kept, content.text())
        };
        // The "e" held.
        assert_eq!(edited("Crème", &[], 1, 0, "é"), (None, "Crèmé".to_owned()));
        // A count ending inside "è" takes all of it.
        assert_eq!(edited("Crème", &[], 3, 0, "x"), (None, "Crx".to_owned()));
        // Across the line break, and after the caret.
        let after_c = [Action::Move(Motion::Home), Action::Move(Motion::Right)];
        assert_eq!(
            edited("ab\ncd", &after_c, 2, 1, "é"),
            (None, "abé".to_owned())
        );
        // The selection stays, and the commit replaces it.
        let selected = [
            Action::Move(Motion::Left),
            Action::Select(Motion::Left),
            Action::Select(Motion::Left),
        ];
        assert_eq!(
            edited("xyçaz", &selected, 1, 1, "é"),
            (Some("ça".to_owned()), "xé".to_owned())
        );
    }

    /// The search field likewise: its text around the caret, and a delete
    /// before a commit.
    #[test]
    fn a_text_input_deletes_around_the_caret_for_an_input_method() {
        use iced::{
            Point, Theme,
            advanced::{InputMethod, input_method, renderer::Headless},
            keyboard::{self, Key, Modifiers, key::Named},
            window,
        };
        use iced_runtime::user_interface::{Cache, State, UserInterface};

        let mut renderer = iced::futures::executor::block_on(iced::Renderer::new(
            crate::fonts::INTER,
            iced::Pixels(crate::style::TEXT_SIZE),
            None,
        ))
        .unwrap();
        let mut ui = UserInterface::<_, Theme, _>::build(
            iced::widget::text_input("", "Crème").on_input(|text| text),
            Size::new(300.0, 100.0),
            Cache::default(),
            &mut renderer,
        );
        let mut messages = Vec::new();
        let mut update = |events: &[Event], messages: &mut Vec<String>| {
            let (state, _) = ui.update(
                events,
                // Beside the text: the caret at its end.
                mouse::Cursor::Available(Point::new(280.0, 10.0)),
                &mut renderer,
                &mut iced::advanced::clipboard::Null,
                messages,
            );
            match state {
                State::Updated {
                    input_method:
                        InputMethod::Enabled {
                            surrounding_text, ..
                        },
                    ..
                } => surrounding_text.map(|around| (around.text, around.cursor, around.anchor)),
                _ => None,
            }
        };
        let redraw = || Event::Window(window::Event::RedrawRequested(iced::time::Instant::now()));
        let _ = update(
            &[Event::Mouse(mouse::Event::ButtonPressed(
                mouse::Button::Left,
            ))],
            &mut messages,
        );
        assert_eq!(
            update(&[redraw()], &mut messages),
            Some(("Crème".to_owned(), 6, 6))
        );

        // "ème" selected, the caret before it.
        let left = Event::Keyboard(keyboard::Event::KeyPressed {
            key: Key::Named(Named::ArrowLeft),
            modified_key: Key::Named(Named::ArrowLeft),
            physical_key: keyboard::key::Physical::Code(keyboard::key::Code::ArrowLeft),
            location: keyboard::Location::Standard,
            modifiers: Modifiers::SHIFT,
            text: None,
            repeat: false,
        });
        let shift = Event::Keyboard(keyboard::Event::ModifiersChanged(Modifiers::SHIFT));
        let _ = update(&[shift, left.clone(), left.clone(), left], &mut messages);
        assert_eq!(
            update(&[redraw()], &mut messages),
            Some(("Crème".to_owned(), 2, 6))
        );

        // "r" deleted before it (nothing is after it), "ème" replaced.
        let _ = update(
            &[
                Event::InputMethod(input_method::Event::DeleteSurrounding {
                    before_bytes: 1,
                    after_bytes: 1,
                }),
                Event::InputMethod(input_method::Event::Commit("ï".to_owned())),
            ],
            &mut messages,
        );
        assert_eq!(messages, ["Cème", "Cï"]);
    }
}
