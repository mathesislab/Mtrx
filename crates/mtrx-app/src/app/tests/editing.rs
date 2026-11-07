//! Editing in the composer and the message editor (`docs/native-behaviour.md`,
//! B2), done in the vendored `text_editor`: keys as the window gets them,
//! through an editor with the default bindings.

use iced::event::Status;
use iced::{
    Event, Theme,
    advanced::{clipboard, input_method, renderer::Headless, widget::operation::focusable},
    keyboard::{self, Key, Modifiers, key::Named},
    widget::text_editor::{Action, Binding, Content, Edit, Motion},
};
use iced_core::event::StandardAction::{self, Cut, Paste, Redo, SelectAll, Undo};
use iced_runtime::user_interface::{Cache, State, UserInterface};

use super::*;

const FIELD: &str = "field";

/// A focused editor with the default key bindings, its actions performed
/// on its content as the app does.
struct Field {
    content: Content,
    renderer: iced::Renderer,
    cache: Cache,
    cursor: iced::Point,
}

impl Field {
    /// `text` in the editor, the caret at its end.
    fn new(text: &str) -> Self {
        let renderer = iced::futures::executor::block_on(iced::Renderer::new(
            fonts::INTER,
            iced::Pixels(style::TEXT_SIZE),
            None,
        ))
        .unwrap();
        let mut field = Self {
            content: typed(text),
            renderer,
            cache: Cache::default(),
            cursor: iced::Point::ORIGIN,
        };
        let mut ui = ui(&field.content, &mut field.cache, &mut field.renderer);
        ui.operate(&field.renderer, &mut focusable::focus(FIELD.into()));
        field.cache = ui.into_cache();
        field
    }

    /// `events` through the editor, its actions performed; whether it took
    /// each.
    fn send(&mut self, events: &[Event]) -> Vec<Status> {
        let cursor = mouse::Cursor::Available(self.cursor);
        let mut ui = ui(&self.content, &mut self.cache, &mut self.renderer);
        let mut actions = Vec::new();
        let (_, statuses) = ui.update(
            events,
            cursor,
            &mut self.renderer,
            &mut clipboard::Null,
            &mut actions,
        );
        self.cache = ui.into_cache();
        for action in actions {
            self.content.perform(action);
        }
        statuses
    }

    /// `key` pressed with `modifiers`, giving `text` (macOS gives a
    /// letter's also with ⌘, and a control character with ⌃).
    fn key(&mut self, key: impl Into<Key>, modifiers: Modifiers, text: Option<&str>) -> Status {
        let key = key.into();
        self.send(&[Event::Keyboard(keyboard::Event::KeyPressed {
            modified_key: key.clone(),
            key,
            physical_key: keyboard::key::Physical::Unidentified(
                keyboard::key::NativeCode::Unidentified,
            ),
            location: keyboard::Location::Standard,
            modifiers,
            text: text.map(Into::into),
            repeat: false,
        })])[0]
    }

    /// A shortcut with a letter: ⌘ with it on macOS, Ctrl elsewhere.
    fn command(&mut self, letter: &str, shift: bool) {
        let modifiers = if shift {
            Modifiers::COMMAND | Modifiers::SHIFT
        } else {
            Modifiers::COMMAND
        };
        let text = shift.then(|| letter.to_uppercase());
        self.key(
            Key::Character(letter.into()),
            modifiers,
            Some(text.as_deref().unwrap_or(letter)),
        );
    }

    /// `text` typed, a line break with Shift+Enter (as in the composer).
    fn type_text(&mut self, text: &str) {
        for c in text.chars() {
            if c == '\n' {
                self.named(Named::Enter, Modifiers::SHIFT);
                continue;
            }
            let c = c.to_string();
            self.key(
                Key::Character(c.as_str().into()),
                Modifiers::empty(),
                Some(&c),
            );
        }
    }

    fn named(&mut self, key: Named, modifiers: Modifiers) -> Status {
        self.key(key, modifiers, None)
    }

    fn undo(&mut self) -> String {
        self.command("z", false);
        self.content.text()
    }

    fn redo(&mut self) -> String {
        self.command("z", true);
        self.content.text()
    }

    fn input_method(&mut self, event: input_method::Event) {
        let _ = self.send(&[Event::InputMethod(event)]);
    }

    /// A standard action of the menus, which comes instead of its key on
    /// macOS.
    fn act(&mut self, action: StandardAction) -> Status {
        self.send(&[Event::StandardAction(action)])[0]
    }

    /// Whether the editor's input method enables each of `actions`, for
    /// their menu items.
    fn enables(&mut self, actions: &[StandardAction]) -> Vec<bool> {
        let mut ui = ui(&self.content, &mut self.cache, &mut self.renderer);
        let (state, _) = ui.update(
            &[Event::Window(iced::window::Event::RedrawRequested(
                iced::time::Instant::now(),
            ))],
            mouse::Cursor::Available(self.cursor),
            &mut self.renderer,
            &mut clipboard::Null,
            &mut Vec::new(),
        );
        self.cache = ui.into_cache();
        let State::Updated { input_method, .. } = state else {
            panic!("outdated");
        };
        actions
            .iter()
            .map(|action| input_method.enables(*action))
            .collect()
    }
}

/// The editor, laid out.
fn ui<'a>(
    content: &'a Content,
    cache: &mut Cache,
    renderer: &mut iced::Renderer,
) -> UserInterface<'a, Action, Theme, iced::Renderer> {
    let editor = iced::widget::text_editor(content)
        .id(FIELD)
        .on_action(|action| action)
        .key_binding(Binding::from_key_press);
    UserInterface::build(
        editor,
        Size::new(400.0, 300.0),
        std::mem::take(cache),
        renderer,
    )
}

#[test]
fn typing_undoes_by_words() {
    let mut field = Field::new("");
    field.type_text("Crème brûlée au café");
    assert_eq!(field.undo(), "Crème brûlée au ");
    assert_eq!(field.undo(), "Crème brûlée ");
    assert_eq!(field.undo(), "Crème ");
    assert_eq!(field.undo(), "");
    // Nothing more to undo.
    assert_eq!(field.undo(), "");
    assert_eq!(field.redo(), "Crème ");
    assert_eq!(field.redo(), "Crème brûlée ");
    // A line break is a space too.
    let mut field = Field::new("");
    field.type_text("Melt  \nrates");
    assert_eq!(field.undo(), "Melt  \n");
    assert_eq!(field.undo(), "");
}

#[test]
fn backspace_and_delete_undo_by_words() {
    let mut field = Field::new("Calving fronts");
    for _ in 0.."fronts ".len() + 1 {
        field.named(Named::Backspace, Modifiers::empty());
    }
    assert_eq!(field.content.text(), "Calvin");
    // "g" deleted after the space that followed "fronts".
    assert_eq!(field.undo(), "Calving");
    assert_eq!(field.undo(), "Calving fronts");
    // Forward, from the start.
    let mut field = Field::new("Sea ice");
    field.named(Named::Home, Modifiers::empty());
    for _ in 0.."Sea i".len() {
        field.named(Named::Delete, Modifiers::empty());
    }
    assert_eq!(field.content.text(), "ce");
    assert_eq!(field.undo(), "ice");
    assert_eq!(field.undo(), "Sea ice");
}

#[test]
fn a_caret_move_or_switching_between_typing_and_deleting_starts_a_step() {
    let mut field = Field::new("");
    field.type_text("ab");
    field.named(Named::ArrowLeft, Modifiers::empty());
    field.type_text("c");
    assert_eq!(field.content.text(), "acb");
    assert_eq!(field.undo(), "ab");
    assert_eq!(field.undo(), "");

    let mut field = Field::new("");
    field.type_text("abc");
    field.named(Named::Backspace, Modifiers::empty());
    field.type_text("d");
    assert_eq!(field.undo(), "ab");
    assert_eq!(field.undo(), "abc");
    assert_eq!(field.undo(), "");
    // A selection deleted is a step of its own.
    let mut field = Field::new("Ice shelf");
    field.named(Named::Home, Modifiers::empty());
    for _ in 0.."Ice ".len() {
        field.named(Named::ArrowRight, Modifiers::SHIFT);
    }
    field.named(Named::Delete, Modifiers::empty());
    field.named(Named::Delete, Modifiers::empty());
    assert_eq!(field.content.text(), "helf");
    assert_eq!(field.undo(), "shelf");
    assert_eq!(field.undo(), "Ice shelf");
}

#[test]
fn a_paste_and_an_input_methods_commit_are_steps_of_their_own() {
    let mut field = Field::new("");
    field.type_text("ab");
    field
        .content
        .perform(Action::Edit(Edit::Paste(Arc::new("ΔT".into()))));
    field.type_text("cd");
    assert_eq!(field.undo(), "abΔT");
    assert_eq!(field.undo(), "ab");
    assert_eq!(field.undo(), "");

    // Holding "e" and picking "é": macOS deletes the "e", then commits;
    // one step, after the typing.
    let mut field = Field::new("");
    field.type_text("Crème");
    field.input_method(input_method::Event::DeleteSurrounding {
        before_bytes: 1,
        after_bytes: 0,
    });
    field.input_method(input_method::Event::Commit("é".into()));
    assert_eq!(field.content.text(), "Crèmé");
    assert_eq!(field.undo(), "Crème");
    assert_eq!(field.undo(), "");
    // And a commit alone (a dead key: ⌥E, E).
    field.type_text("caf");
    field.input_method(input_method::Event::Commit("é".into()));
    field.type_text("s");
    assert_eq!(field.undo(), "café");
    assert_eq!(field.undo(), "caf");
}

#[test]
fn undo_brings_back_the_selection_and_an_edit_ends_redo() {
    let mut field = Field::new("Basal melt");
    field.command("a", false);
    field.type_text("x");
    assert_eq!(field.undo(), "Basal melt");
    assert_eq!(field.content.selection().as_deref(), Some("Basal melt"));
    assert_eq!(field.redo(), "x");
    assert_eq!(field.content.selection(), None);

    let mut field = Field::new("");
    field.type_text("ab");
    assert_eq!(field.undo(), "");
    field.type_text("c");
    assert_eq!(field.redo(), "c");
}

#[test]
fn a_new_content_has_no_history_and_the_history_is_bounded() {
    let mut field = Field::new("");
    field.type_text("Draft");
    field.content = typed("Sent");
    assert_eq!(field.undo(), "Sent");

    let mut content = typed("");
    for _ in 0..1000 {
        content.perform(Action::Edit(Edit::Paste(Arc::new("x".into()))));
    }
    for _ in 0..1000 {
        content.perform(Action::Edit(Edit::Undo));
    }
    let kept = content.text().len();
    assert!(0 < kept && kept < 1000, "{kept}");
}

/// The app completes a mention by pasting over the partial one: one step.
#[test]
fn a_completed_mention_undoes_in_one_step() {
    let mut content = typed("Ask @ali");
    let query = mention::completion(&content).unwrap();
    mention::insert(&mut content, &query, "@alice:example.org");
    assert_eq!(content.text(), "Ask @alice:example.org ");
    content.perform(Action::Edit(Edit::Undo));
    assert_eq!(content.text(), "Ask @ali");
    content.perform(Action::Move(Motion::DocumentEnd));
    content.perform(Action::Edit(Edit::Undo));
    assert_eq!(content.text(), "Ask @ali");
}

/// The key that widens a motion: ⌥ on macOS, Ctrl elsewhere.
fn jump() -> Modifiers {
    if cfg!(target_os = "macos") {
        Modifiers::ALT
    } else {
        Modifiers::CTRL
    }
}

#[test]
fn option_backspace_and_option_delete_delete_a_word() {
    let mut field = Field::new("Crème brûlée au café");
    field.named(Named::Backspace, jump());
    assert_eq!(field.content.text(), "Crème brûlée au ");
    field.named(Named::Backspace, jump());
    assert_eq!(field.content.text(), "Crème brûlée ");
    // A step each.
    assert_eq!(field.undo(), "Crème brûlée au ");

    let mut field = Field::new("Sea ice");
    field.named(Named::Home, Modifiers::empty());
    field.named(Named::Delete, jump());
    assert_eq!(field.content.text(), " ice");
    field.named(Named::Delete, jump());
    assert_eq!(field.content.text(), "");

    // At the start of a line, the line break.
    let mut field = Field::new("Melt\nrates");
    field.named(Named::Home, Modifiers::empty());
    field.named(Named::Backspace, jump());
    assert_eq!(field.content.text(), "Meltrates");

    // A selection, as Backspace.
    let mut field = Field::new("Ice shelf");
    field.named(Named::ArrowLeft, Modifiers::SHIFT);
    field.named(Named::Backspace, jump());
    assert_eq!(field.content.text(), "Ice shel");
}

#[cfg(target_os = "macos")]
#[test]
fn command_backspace_deletes_to_the_start_of_the_line() {
    let mut field = Field::new("Calving fronts");
    for _ in 0.." fronts".len() {
        field.named(Named::ArrowLeft, Modifiers::empty());
    }
    field.named(Named::Backspace, Modifiers::COMMAND);
    assert_eq!(field.content.text(), " fronts");
    // At its start, it joins the line to the one before.
    let mut field = Field::new("Melt\nrates");
    field.named(Named::Home, Modifiers::empty());
    field.named(Named::Backspace, Modifiers::COMMAND);
    assert_eq!(field.content.text(), "Meltrates");
    // The line as shown: a long paragraph wraps, and only its last line
    // goes.
    let long = "Ice shelves buttress the glaciers that feed them. ".repeat(4);
    let mut field = Field::new(long.trim_end());
    field.named(Named::Backspace, Modifiers::COMMAND);
    let left = field.content.text();
    assert!(
        left.starts_with("Ice shelves buttress") && left.len() < long.len() - 10,
        "{left:?}"
    );
    // ⌘⌦ to its end.
    let mut field = Field::new("Calving fronts");
    field.named(Named::Home, Modifiers::empty());
    field.named(Named::Delete, Modifiers::COMMAND);
    assert_eq!(field.content.text(), "");
}

/// ⌘↑ and ⌘↓ go to the start and end of the draft, with ⇧ selecting; the
/// editor takes them only while focused, and the timeline scrolls with
/// them otherwise (A2).
#[cfg(target_os = "macos")]
#[test]
fn command_up_and_down_go_to_the_start_and_end_of_the_draft() {
    let mut field = Field::new("Calving\nfronts\nretreat");
    field.named(Named::ArrowUp, Modifiers::COMMAND);
    field.type_text("x");
    assert_eq!(field.content.text(), "xCalving\nfronts\nretreat");
    field.named(Named::ArrowDown, Modifiers::COMMAND);
    field.type_text("y");
    let all = "xCalving\nfronts\nretreaty";
    assert_eq!(field.content.text(), all);
    field.named(Named::ArrowUp, Modifiers::COMMAND | Modifiers::SHIFT);
    assert_eq!(field.content.selection().as_deref(), Some(all));
    field.named(Named::ArrowLeft, Modifiers::empty());
    field.named(Named::ArrowDown, Modifiers::COMMAND | Modifiers::SHIFT);
    assert_eq!(field.content.selection().as_deref(), Some(all));

    assert_eq!(
        field.named(Named::ArrowUp, Modifiers::COMMAND),
        Status::Captured
    );
    assert_eq!(
        field.named(Named::Escape, Modifiers::empty()),
        Status::Ignored
    );
    assert_eq!(
        field.named(Named::ArrowUp, Modifiers::COMMAND),
        Status::Ignored
    );
}

/// ⌥↑ and ⌥↓ (Ctrl with them elsewhere) go to the start and end of the
/// paragraph, and on to the one before or after when already there.
#[test]
fn option_up_and_down_go_by_paragraph() {
    let long = "Ice shelves buttress the glaciers that feed them. ".repeat(4);
    let text = format!("Calving fronts\n{}\nSea ice", long.trim_end());
    let mut field = Field::new(&text);
    let at = |field: &mut Field, key, modifiers| {
        field.named(key, modifiers);
        let position = field.content.cursor().position;
        (position.line, position.column)
    };
    assert_eq!(at(&mut field, Named::ArrowUp, jump()), (2, 0));
    // The start of the paragraph, not of the line as shown.
    assert_eq!(at(&mut field, Named::ArrowUp, jump()), (1, 0));
    assert_eq!(at(&mut field, Named::ArrowUp, jump()), (0, 0));
    assert_eq!(at(&mut field, Named::ArrowUp, jump()), (0, 0));
    assert_eq!(at(&mut field, Named::ArrowDown, jump()), (0, 14));
    let end = long.trim_end().len();
    assert_eq!(at(&mut field, Named::ArrowDown, jump()), (1, end));
    // From inside one, its end.
    let _ = at(&mut field, Named::ArrowUp, Modifiers::empty());
    assert_eq!(at(&mut field, Named::ArrowDown, jump()), (1, end));
    // With ⇧, selecting.
    let _ = at(&mut field, Named::ArrowUp, jump() | Modifiers::SHIFT);
    assert_eq!(field.content.selection().as_deref(), Some(long.trim_end()));
    // A selection goes to the edge of its paragraph.
    assert_eq!(at(&mut field, Named::ArrowDown, jump()), (1, end));
    assert_eq!(field.content.selection(), None);
}

#[cfg(target_os = "macos")]
impl Field {
    /// ⌃ with `letter`, which gives a control character as its text.
    fn control(&mut self, letter: char) {
        let text = char::from(letter as u8 - b'a' + 1).to_string();
        self.key(
            Key::Character(letter.to_string().into()),
            Modifiers::CTRL,
            Some(&text),
        );
    }

    /// The caret moved to `column` of `line`.
    fn caret_at(&mut self, line: usize, column: usize) {
        use iced::widget::text_editor::{Cursor, Position};
        self.content.move_to(Cursor {
            position: Position { line, column },
            selection: None,
        });
    }

    fn caret(&self) -> (usize, usize) {
        let position = self.content.cursor().position;
        (position.line, position.column)
    }
}

/// iced's ⌃A, ⌃E, ⌃B, ⌃F and ⌃D bound nothing (their text is a control
/// character); ⌃H did.
#[cfg(target_os = "macos")]
#[test]
fn control_keys_move_and_delete() {
    let long = "Ice shelves buttress the glaciers that feed them. ".repeat(4);
    let text = format!("Calving fronts\n{}", long.trim_end());
    let mut field = Field::new(&text);
    // The paragraph's edges, not the line's as shown.
    field.control('a');
    assert_eq!(field.caret(), (1, 0));
    field.control('e');
    assert_eq!(field.caret(), (1, long.trim_end().len()));
    field.caret_at(0, 7);
    field.control('f');
    assert_eq!(field.caret(), (0, 8));
    field.control('b');
    field.control('b');
    assert_eq!(field.caret(), (0, 6));
    field.control('n');
    assert_eq!(field.caret().0, 1);
    field.control('p');
    assert_eq!(field.caret().0, 0);

    let mut field = Field::new("Calving fronts");
    field.caret_at(0, 7);
    field.control('h');
    field.control('d');
    assert_eq!(field.content.text(), "Calvinfronts");
}

/// ⌃K kills to the end of the paragraph, ⌃Y yanks it back, in any editor.
#[cfg(target_os = "macos")]
#[test]
fn control_k_kills_and_control_y_yanks() {
    let mut field = Field::new("Calving fronts\nBasal melt");
    field.caret_at(0, 7);
    field.control('k');
    assert_eq!(field.content.text(), "Calving\nBasal melt");
    let mut other = Field::new("Retreating ");
    other.control('y');
    assert_eq!(other.content.text(), "Retreating  fronts");
    // At the end of a line, its line ending; a step to undo.
    field.control('k');
    assert_eq!(field.content.text(), "CalvingBasal melt");
    assert_eq!(field.undo(), "Calving\nBasal melt");
    field.named(Named::ArrowDown, Modifiers::COMMAND);
    field.control('y');
    assert_eq!(field.content.text(), "Calving\nBasal melt\n");
    // A selection.
    let mut field = Field::new("Crème brûlée");
    field.named(Named::ArrowLeft, Modifiers::SHIFT | Modifiers::ALT);
    field.control('k');
    assert_eq!(field.content.text(), "Crème ");
    field.named(Named::Home, Modifiers::empty());
    field.control('y');
    assert_eq!(field.content.text(), "brûléeCrème ");
}

/// ⌃T swaps the characters around the caret (the two before it at a
/// line's end), ⌃O breaks the line after it.
#[cfg(target_os = "macos")]
#[test]
fn control_t_transposes_and_control_o_opens_a_line() {
    let mut field = Field::new("brûlé");
    field.control('t');
    assert_eq!(field.content.text(), "brûél");
    field.caret_at(0, "br".len());
    field.control('t');
    assert_eq!(field.content.text(), "bûrél");
    assert_eq!(field.caret(), (0, "bûr".len()));
    // A letter and its combining accent move together.
    let mut field = Field::new("xe\u{301}");
    field.control('t');
    assert_eq!(field.content.text(), "e\u{301}x");
    // Nothing before the caret.
    field.caret_at(0, 0);
    field.control('t');
    assert_eq!(field.content.text(), "e\u{301}x");

    let mut field = Field::new("Calving fronts");
    field.caret_at(0, 7);
    field.control('o');
    assert_eq!(field.content.text(), "Calving\n fronts");
    assert_eq!(field.caret(), (0, 7));
}

impl Field {
    /// The middle of the `line`th line, `x` into it (the editor's padding
    /// is 5, its lines the text size × 1.3).
    fn on_line(line: usize, x: f32) -> iced::Point {
        let height = style::TEXT_SIZE * 1.3;
        iced::Point::new(5.0 + x, 5.0 + height * (line as f32 + 0.5))
    }

    fn mouse(&mut self, at: iced::Point, event: mouse::Event) {
        self.cursor = at;
        let _ = self.send(&[Event::Mouse(event)]);
    }

    /// The pointer pressed at `from`, moved to `to` with `modifiers`, and
    /// released there.
    fn drag(&mut self, from: iced::Point, to: iced::Point, modifiers: Modifiers) {
        let left = mouse::Button::Left;
        self.mouse(from, mouse::Event::ButtonPressed(left));
        self.mouse(to, mouse::Event::CursorMoved { position: to });
        let _ = self.send(&[Event::Keyboard(keyboard::Event::ModifiersChanged(
            modifiers,
        ))]);
        self.mouse(to, mouse::Event::ButtonReleased(left));
    }

    /// The first line selected.
    fn select_first_line(&mut self) {
        // To the start of the text: ⌘↑ on macOS, Ctrl+Home elsewhere (where
        // Ctrl+↑ goes by paragraph).
        let start = if cfg!(target_os = "macos") {
            Named::ArrowUp
        } else {
            Named::Home
        };
        self.named(start, Modifiers::COMMAND);
        self.named(Named::Home, Modifiers::empty());
        self.named(Named::End, Modifiers::SHIFT);
    }
}

/// Selected text dragged moves (with ⌥, copies) to where it is dropped.
#[test]
fn dragging_the_selection_moves_it() {
    let text = "Crème brûlée\nau café";
    let mut field = Field::new(text);
    field.select_first_line();
    assert_eq!(field.content.selection().as_deref(), Some("Crème brûlée"));
    // To the end of the second line.
    let (from, to) = (Field::on_line(0, 10.0), Field::on_line(1, 380.0));
    field.drag(from, to, Modifiers::empty());
    assert_eq!(field.content.text(), "\nau caféCrème brûlée");
    assert_eq!(field.content.selection().as_deref(), Some("Crème brûlée"));
    // A step to undo, which selects it where it was.
    assert_eq!(field.undo(), text);
    assert_eq!(field.content.selection().as_deref(), Some("Crème brûlée"));

    // With ⌥ (Ctrl elsewhere), a copy.
    let copy = if cfg!(target_os = "macos") {
        Modifiers::ALT
    } else {
        Modifiers::CTRL
    };
    // (Each press 6 apart from the last, or it is a double click.)
    field.drag(Field::on_line(0, 25.0), to, copy);
    assert_eq!(field.content.text(), "Crème brûlée\nau caféCrème brûlée");

    // Dropped onto itself, nothing moves.
    let mut field = Field::new(text);
    field.select_first_line();
    field.drag(
        Field::on_line(0, 40.0),
        Field::on_line(0, 60.0),
        Modifiers::empty(),
    );
    assert_eq!(field.content.text(), text);
    assert_eq!(field.content.selection().as_deref(), Some("Crème brûlée"));
    // Pressed and released in it: a click, which places the caret.
    field.drag(from, from, Modifiers::empty());
    assert_eq!(field.content.text(), text);
    assert_eq!(field.content.selection(), None);
    // Pressed beside it: a new selection.
    field.select_first_line();
    field.drag(Field::on_line(1, 0.0), to, Modifiers::empty());
    assert_eq!(field.content.text(), text);
    assert_eq!(field.content.selection().as_deref(), Some("au café"));
    // Not focused, the selection is not drawn: a press in it is a click.
    field.select_first_line();
    field.named(Named::Escape, Modifiers::empty());
    field.drag(from, to, Modifiers::empty());
    assert_eq!(field.content.text(), text);
}

/// The Edit menu's Undo, Redo, Select All and Cut (macOS's menu takes ⌘Z,
/// ⇧⌘Z, ⌘A and ⌘X before the window) do what the keys do.
#[test]
fn the_menus_actions_edit_as_their_keys() {
    let (mut keys, mut menu) = (Field::new(""), Field::new(""));
    keys.type_text("Crème brûlée");
    menu.type_text("Crème brûlée");
    let steps = [
        ("z", false, Undo, "Crème "),
        ("z", false, Undo, ""),
        ("z", true, Redo, "Crème "),
        ("a", false, SelectAll, "Crème "),
        ("x", false, Cut, ""),
        ("z", false, Undo, "Crème "),
    ];
    for (letter, shift, action, text) in steps {
        keys.command(letter, shift);
        assert_eq!(menu.act(action), Status::Captured, "{action:?}");
        assert_eq!(menu.content.text(), text, "{action:?}");
        assert_eq!(menu.content.text(), keys.content.text(), "{action:?}");
        assert_eq!(
            menu.content.selection(),
            keys.content.selection(),
            "{action:?}"
        );
    }
}

/// The editor enables Undo and Redo with something to undo and redo, Cut
/// with a selection; Copy and Paste always (Copy with no selection copies
/// the timeline's).
#[test]
fn the_editor_enables_undo_redo_and_cut_as_it_can() {
    use iced_core::event::StandardAction::Copy;

    let actions = [Undo, Redo, Cut, Copy, Paste];
    let mut field = Field::new("");
    assert_eq!(field.enables(&actions), [false, false, false, true, true]);
    field.type_text("Melt");
    assert_eq!(field.enables(&actions), [true, false, false, true, true]);
    field.act(Undo);
    assert_eq!(field.enables(&actions), [false, true, false, true, true]);
    field.act(Redo);
    field.act(SelectAll);
    assert_eq!(field.enables(&actions), [true, false, true, true, true]);
}
