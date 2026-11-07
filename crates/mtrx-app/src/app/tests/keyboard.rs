//! Keys as the window gets them: through the widgets first, then, unless
//! one took them, to the app's shortcuts; the tasks they start run on the
//! window's widgets.

use iced::{
    Event,
    advanced::{clipboard, renderer::Headless, widget::operation::Outcome},
    event::Status,
    keyboard::{self, Key, Modifiers, key::Named},
};
use iced_runtime::{
    Action,
    user_interface::{Cache, UserInterface},
};

use super::*;

const SIZE: Size = Size::new(1200.0, 800.0);

/// The app in its window.
struct Ui {
    app: App,
    renderer: iced::Renderer,
    cache: Cache,
    cursor: iced::Point,
}

impl Ui {
    fn new(app: App) -> Self {
        let renderer = iced::futures::executor::block_on(iced::Renderer::new(
            fonts::INTER,
            iced::Pixels(style::TEXT_SIZE),
            None,
        ))
        .unwrap();
        let mut ui = Self {
            app,
            renderer,
            cache: Cache::default(),
            cursor: iced::Point::ORIGIN,
        };
        // Laid out once, as when shown.
        ui.send(&[]);
        ui
    }

    fn main(&mut self) -> &mut Main {
        let Screen::Main(main) = &mut self.app.screen else {
            panic!("not the main screen")
        };
        main
    }

    fn room(&mut self) -> &mut Room {
        self.main().pane.room_mut().expect("a room is open")
    }

    /// `events` through the widgets, then to the shortcuts with whether a
    /// widget took them; returns that.
    fn send(&mut self, events: &[Event]) -> Vec<Status> {
        let (statuses, messages) = self.messages(events);
        for message in messages {
            self.update(message);
        }
        statuses
    }

    /// What [`Ui::send`] does without updating the app: whether a widget
    /// took each event, and the messages.
    fn messages(&mut self, events: &[Event]) -> (Vec<Status>, Vec<Message>) {
        let mut ui = UserInterface::build(
            self.app.view(),
            SIZE,
            std::mem::take(&mut self.cache),
            &mut self.renderer,
        );
        let mut messages = Vec::new();
        let (_, statuses) = ui.update(
            events,
            mouse::Cursor::Available(self.cursor),
            &mut self.renderer,
            &mut clipboard::Null,
            &mut messages,
        );
        self.cache = ui.into_cache();
        for (event, status) in events.iter().zip(&statuses) {
            messages.extend(shortcuts(event.clone(), *status, window::Id::unique()));
        }
        (statuses, messages)
    }

    /// A key pressed, as [`Ui::send`].
    fn press(&mut self, key: impl Into<Key>, modifiers: Modifiers) -> Status {
        self.send(&[press(key.into(), modifiers)])[0]
    }

    fn update(&mut self, message: Message) {
        let task = self.app.update(message);
        self.run(task);
    }

    /// Run `task` to its end: its messages to the app, its operations on
    /// the widgets.
    fn run(&mut self, task: Task<Message>) {
        let Some(mut actions) = iced_runtime::task::into_stream(task) else {
            return;
        };
        while let Some(action) = iced::futures::executor::block_on(actions.next()) {
            match action {
                Action::Output(message) => self.update(message),
                Action::Widget(operation) => self.operate(operation),
                _ => {}
            }
        }
    }

    fn operate(&mut self, operation: Box<dyn iced::advanced::widget::Operation>) {
        let mut ui = UserInterface::build(
            self.app.view(),
            SIZE,
            std::mem::take(&mut self.cache),
            &mut self.renderer,
        );
        let mut next = Some(operation);
        while let Some(mut operation) = next.take() {
            ui.operate(&self.renderer, operation.as_mut());
            if let Outcome::Chain(chained) = operation.finish() {
                next = Some(chained);
            }
        }
        self.cache = ui.into_cache();
    }
}

/// `key` pressed with `modifiers`, with its character as text if it has one
/// (macOS gives it also with ⌘).
fn press(key: Key, modifiers: Modifiers) -> Event {
    let text = match key.as_ref() {
        Key::Character(c) => Some(c.into()),
        _ => None,
    };
    Event::Keyboard(keyboard::Event::KeyPressed {
        modified_key: key.clone(),
        key,
        physical_key: keyboard::key::Physical::Unidentified(
            keyboard::key::NativeCode::Unidentified,
        ),
        location: keyboard::Location::Standard,
        modifiers,
        text,
        repeat: false,
    })
}

/// A room of ours with `messages`, open in the app.
async fn room_with(prefix: &str, messages: &[&str]) -> (Ui, TempDir) {
    let dir = TempDir::new().unwrap();
    let (_, alice) = user(prefix, &dir).await;
    let room = alice
        .create_room(NewRoom {
            name: Some(prefix.into()),
            ..Default::default()
        })
        .await
        .unwrap();
    let timeline = Arc::new(alice.timeline(&room).await.unwrap());
    for text in messages {
        send(&timeline, text).await;
    }
    while !timeline.paginate_back(50).await.unwrap() {}
    let mut main = Main::new(Arc::new(alice));
    main.verified = Some(true);
    let drafts = main.session.draft_keeper(&room);
    let mut open = Room::new(main.session.room(&room).unwrap(), timeline.clone(), drafts);
    show(&mut open, timeline.current());
    main.pane = Pane::Open(open);
    (Ui::new(app(Screen::Main(Box::new(main)), Mode::Light)), dir)
}

/// [`Event::id`] of the message with `body`.
fn row_of(room: &Room, body: &str) -> String {
    (room.rows.iter().filter_map(Row::event))
        .find(|event| matches!(&event.content, EventContent::Message(m) if m.body == body))
        .map(|event| event.id.clone())
        .unwrap_or_else(|| panic!("no message {body:?}"))
}

/// The shortcuts give `expected` for each key with its modifiers when no
/// widget took it, and nothing when one did.
fn assert_shortcuts(cases: Vec<(Key, Modifiers, Message)>) {
    for (key, modifiers, expected) in cases {
        let message = shortcuts(
            press(key.clone(), modifiers),
            Status::Ignored,
            window::Id::unique(),
        );
        assert_eq!(
            format!("{message:?}"),
            format!("{:?}", Some(expected)),
            "{key:?} with {modifiers:?}"
        );
        let taken = shortcuts(
            press(key.clone(), modifiers),
            Status::Captured,
            window::Id::unique(),
        );
        assert!(taken.is_none(), "{key:?} with {modifiers:?}");
    }
}

#[test]
fn shift_and_arrows_move_the_end_of_the_selection() {
    use selection::{Message::*, Step};

    let (shift, command) = (Modifiers::SHIFT, Modifiers::SHIFT | Modifiers::COMMAND);
    let arrow = Key::Named;
    let cases = [
        (arrow(Named::ArrowLeft), shift, Extend(Step::Back)),
        (
            arrow(Named::ArrowRight),
            shift | Modifiers::ALT,
            Extend(Step::WordForward),
        ),
        (arrow(Named::ArrowUp), shift, ExtendLine(true)),
        (arrow(Named::ArrowLeft), command, ExtendToLineEdge(false)),
        (arrow(Named::ArrowRight), command, ExtendToLineEdge(true)),
        (arrow(Named::ArrowUp), command, ExtendToScreenEdge(true)),
        (arrow(Named::ArrowDown), command, ExtendToScreenEdge(false)),
    ];
    assert_shortcuts(
        cases
            .into_iter()
            .map(|(key, modifiers, message)| {
                (
                    key,
                    modifiers,
                    Message::Shown(room::Message::Select(message)),
                )
            })
            .collect(),
    );
}

/// ⇧⌘← and ⇧⌘→ move the end of the selection to the edges of its line,
/// ⇧⌘↑ and ⇧⌘↓ to the first and last text on screen: here the room's
/// start and the last message.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs local Synapse (testserver/run.sh)"]
async fn shift_command_arrows_extend_to_the_edges() {
    let (mut ui, _dir) = room_with("edges", &["Calving fronts", "Basal melt", "Sea ice"]).await;
    let room = ui.room();
    let (middle, last) = (row_of(room, "Basal melt"), row_of(room, "Sea ice"));
    let at = |event: &str, offset| Point {
        event: event.to_owned(),
        pos: Pos { unit: 0, offset },
    };
    room.selection = Some(Selection {
        anchor: at(&middle, 2),
        focus: at(&middle, 4),
        dragging: false,
    });
    ui.send(&[]);
    let focus = |ui: &mut Ui| ui.room().selection.as_ref().unwrap().focus.clone();

    ui.press(Named::ArrowRight, Modifiers::SHIFT | Modifiers::COMMAND);
    assert_eq!(focus(&mut ui), at(&middle, "Basal melt".len()));
    ui.press(Named::ArrowLeft, Modifiers::SHIFT | Modifiers::COMMAND);
    assert_eq!(focus(&mut ui), at(&middle, 0));
    ui.press(Named::ArrowUp, Modifiers::SHIFT | Modifiers::COMMAND);
    assert_eq!(focus(&mut ui), at("start", 0));
    ui.press(Named::ArrowDown, Modifiers::SHIFT | Modifiers::COMMAND);
    assert_eq!(focus(&mut ui), at(&last, "Sea ice".len()));
    // The anchor stays.
    assert_eq!(ui.room().selection.as_ref().unwrap().anchor, at(&middle, 2));
}

#[test]
fn page_keys_and_arrows_scroll_the_timeline() {
    use room::Scroll;

    let key = Key::Named;
    let cases = [
        (key(Named::PageUp), Modifiers::empty(), Scroll::PageUp),
        (key(Named::PageDown), Modifiers::empty(), Scroll::PageDown),
        (key(Named::ArrowUp), Modifiers::ALT, Scroll::PageUp),
        (key(Named::ArrowDown), Modifiers::ALT, Scroll::PageDown),
        (key(Named::ArrowUp), Modifiers::COMMAND, Scroll::Oldest),
        (key(Named::ArrowDown), Modifiers::COMMAND, Scroll::Newest),
    ];
    assert_shortcuts(
        cases
            .into_iter()
            .map(|(key, modifiers, scroll)| {
                (
                    key,
                    modifiers,
                    Message::Shown(room::Message::ScrollKey(scroll)),
                )
            })
            .collect(),
    );
}

/// The timeline's view height, content height and how far it is scrolled
/// from the top, once operated.
#[derive(Default)]
struct Timeline(Option<(f32, f32, f32)>);

impl iced::advanced::widget::Operation for Timeline {
    fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn iced::advanced::widget::Operation)) {
        operate(self);
    }

    fn scrollable(
        &mut self,
        id: Option<&iced::advanced::widget::Id>,
        bounds: iced::Rectangle,
        content_bounds: iced::Rectangle,
        translation: iced::Vector,
        _state: &mut dyn iced::advanced::widget::operation::Scrollable,
    ) {
        if id == Some(&iced::advanced::widget::Id::new(room::TIMELINE)) {
            self.0 = Some((bounds.height, content_bounds.height, translation.y));
        }
    }
}

impl Ui {
    /// The timeline's view height, content height and how far it is
    /// scrolled from the top.
    fn timeline(&mut self) -> (f32, f32, f32) {
        let mut timeline = Timeline::default();
        let mut ui = UserInterface::build(
            self.app.view(),
            SIZE,
            std::mem::take(&mut self.cache),
            &mut self.renderer,
        );
        ui.operate(&self.renderer, &mut timeline);
        self.cache = ui.into_cache();
        timeline.0.expect("a timeline")
    }

    fn scrolled(&mut self) -> f32 {
        self.timeline().2
    }
}

/// Page Up and ⌥↑ scroll the timeline up a page, Page Down and ⌥↓ down;
/// ⌘↑ goes to the oldest loaded message, ⌘↓ to the newest. Not while the
/// composer has the keys.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs local Synapse (testserver/run.sh)"]
async fn keys_scroll_the_timeline() {
    let messages: Vec<String> = (0..90).map(|i| format!("Message {i}")).collect();
    let messages: Vec<&str> = messages.iter().map(String::as_str).collect();
    let (mut ui, _dir) = room_with("pages", &messages).await;
    let (view, height, bottom) = ui.timeline();
    assert_near(bottom, height - view);
    let page = view * 0.9;

    ui.press(Named::PageUp, Modifiers::empty());
    assert_near(ui.scrolled(), bottom - page);
    ui.press(Named::ArrowUp, Modifiers::ALT);
    assert_near(ui.scrolled(), bottom - 2.0 * page);
    ui.press(Named::ArrowDown, Modifiers::ALT);
    assert_near(ui.scrolled(), bottom - page);
    ui.press(Named::PageDown, Modifiers::empty());
    assert_near(ui.scrolled(), bottom);

    // All loaded messages are shown, from the top.
    ui.press(Named::ArrowUp, Modifiers::COMMAND);
    let (_, all, top) = ui.timeline();
    assert_near(top, 0.0);
    assert!(all > height, "{all} {height}");
    ui.press(Named::ArrowDown, Modifiers::COMMAND);
    assert_near(ui.scrolled(), all - view);

    // The composer takes them.
    ui.run(iced::widget::operation::focus(message_field::COMPOSER));
    assert_eq!(
        ui.press(Named::PageUp, Modifiers::empty()),
        Status::Captured
    );
    assert_near(ui.scrolled(), all - view);
}

/// Offsets, to within rounding.
#[track_caller]
/// Within a pixel: the timeline rounds its scroll offset to whole pixels,
/// the one an expectation starts from too.
fn assert_near(offset: f32, expected: f32) {
    assert!((offset - expected).abs() <= 1.0, "{offset}, not {expected}");
}

/// The keys going to another room do so also while a field has them.
#[test]
fn keys_go_to_rooms_from_fields_too() {
    use sidebar::Go;

    let cases = [
        (
            Key::Named(Named::ArrowUp),
            Modifiers::COMMAND | Modifiers::ALT,
            Go::Above,
        ),
        (Key::Named(Named::Tab), Modifiers::CTRL, Go::Below),
        (Key::Character("3".into()), Modifiers::COMMAND, Go::Row(2)),
    ];
    for (key, modifiers, go) in cases {
        for status in [Status::Ignored, Status::Captured] {
            let message = shortcuts(press(key.clone(), modifiers), status, window::Id::unique());
            assert_eq!(
                format!("{message:?}"),
                format!("{:?}", Some(Message::GoToRoom(go))),
                "{key:?} with {modifiers:?}"
            );
        }
    }
}

/// ⌥⌘↓ and ⌘1 open the room below and the first, also from the composer,
/// which leaves them: ⌘1 types no 1.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs local Synapse (testserver/run.sh)"]
async fn keys_go_to_other_rooms_from_the_composer() {
    let (mut ui, _dir) = room_with("goto", &[]).await;
    let open = ui.room().id.clone();
    let timeline = ui.room().timeline.clone();
    let other: OwnedRoomId = "!other:localhost".try_into().unwrap();
    ui.main().show_rooms(RoomListState {
        rooms: vec![summary(&other, "other"), summary(&open, "open")],
        loaded: false,
        ..Default::default()
    });
    ui.run(iced::widget::operation::focus(message_field::COMPOSER));

    let keys = [
        (
            Key::Named(Named::ArrowUp),
            Modifiers::COMMAND | Modifiers::ALT,
        ),
        (Key::Character("1".into()), Modifiers::COMMAND),
    ];
    for (key, modifiers) in keys {
        let (statuses, messages) = ui.messages(&[press(key.clone(), modifiers)]);
        assert_eq!(statuses, [Status::Ignored], "{key:?}");
        let typing = |m: &Message| {
            matches!(
                m,
                Message::Room(
                    _,
                    room::Message::Field(
                        Field::Composer,
                        message_field::Message::Action(text_editor::Action::Edit(_))
                    )
                )
            )
        };
        assert!(!messages.iter().any(typing), "{key:?}");
        // Opening it (its tasks are not run: rooms load their timeline by
        // a subscription, which this window has not).
        for message in messages {
            let _ = ui.app.update(message);
        }
        assert!(
            matches!(&ui.main().pane, Pane::Opening(id) if *id == other),
            "{key:?}"
        );
        // Open again for the next key.
        let drafts = ui.main().session.draft_keeper(&open);
        let handle = ui.main().session.room(&open).unwrap();
        ui.main().pane = Pane::Open(Room::new(handle, timeline.clone(), drafts));
    }
}

#[test]
fn characters_typed_with_nothing_focused_go_to_the_composer() {
    let typed = |key: &str, modifiers, status| {
        let key = Key::Character(key.into());
        format!(
            "{:?}",
            shortcuts(press(key, modifiers), status, window::Id::unique())
        )
    };
    let to_composer = |text: &str| {
        format!(
            "{:?}",
            Some(Message::Shown(room::Message::TypeInComposer(text.into())))
        )
    };
    assert_eq!(
        typed("h", Modifiers::empty(), Status::Ignored),
        to_composer("h")
    );
    assert_eq!(
        typed("H", Modifiers::SHIFT, Status::Ignored),
        to_composer("H")
    );
    // ⌥E, E: é.
    assert_eq!(
        typed("é", Modifiers::empty(), Status::Ignored),
        to_composer("é")
    );
    // A field took it.
    assert_eq!(typed("h", Modifiers::empty(), Status::Captured), "None");
    // Shortcuts.
    assert_eq!(typed("j", Modifiers::COMMAND, Status::Ignored), "None");
    assert_eq!(typed("j", Modifiers::CTRL, Status::Ignored), "None");
    // Not a character.
    let enter = press(Key::Named(Named::Enter), Modifiers::empty());
    assert!(shortcuts(enter, Status::Ignored, window::Id::unique()).is_none());
}

impl Ui {
    /// Whether the composer has the focus.
    fn composer_focused(&mut self) -> bool {
        let mut focused = super::ComposerFocused(None);
        let mut ui = UserInterface::build(
            self.app.view(),
            SIZE,
            std::mem::take(&mut self.cache),
            &mut self.renderer,
        );
        ui.operate(&self.renderer, &mut focused);
        self.cache = ui.into_cache();
        focused.0 == Some(true)
    }
}

/// Typing with nothing focused goes to the composer, from the first
/// character on; not while something shows over the room.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs local Synapse (testserver/run.sh)"]
async fn typing_with_nothing_focused_goes_to_the_composer() {
    let (mut ui, _dir) = room_with("typing", &["Hello"]).await;
    assert!(!ui.composer_focused());
    assert_eq!(
        ui.press(Key::Character("O".into()), Modifiers::SHIFT),
        Status::Ignored
    );
    assert!(ui.composer_focused());
    // From here the composer has the keys.
    assert_eq!(
        ui.press(Key::Character("k".into()), Modifiers::empty()),
        Status::Captured
    );
    assert_eq!(ui.room().composer.text(), "Ok");

    // Nothing focused, under the settings: nothing.
    ui.main().modal = Some(Modal::Settings(settings::Panel::default()));
    ui.run(iced::widget::operation::focus("nothing"));
    assert_eq!(
        ui.press(Key::Character("x".into()), Modifiers::empty()),
        Status::Ignored
    );
    assert_eq!(ui.room().composer.text(), "Ok");
}

/// ↑ in the empty composer edits our newest message; not with a draft, nor
/// with nothing focused.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs local Synapse (testserver/run.sh)"]
async fn up_in_the_empty_composer_edits_our_last_message() {
    let (mut ui, _dir) = room_with("editlast", &["Calving", "Melt"]).await;
    let up = Key::Named(Named::ArrowUp);
    // Nothing focused.
    assert_eq!(ui.press(up.clone(), Modifiers::empty()), Status::Ignored);
    assert!(ui.room().editing.is_none());
    // A draft: the cursor moves.
    ui.room().composer.set_text("Draft");
    ui.run(iced::widget::operation::focus(message_field::COMPOSER));
    assert_eq!(ui.press(up.clone(), Modifiers::empty()), Status::Captured);
    assert!(ui.room().editing.is_none());

    ui.room().composer.set_text("");
    ui.send(&[]);
    assert_eq!(ui.press(up, Modifiers::empty()), Status::Captured);
    let edit = ui.room().editing.as_ref().expect("editing");
    assert!(matches!(edit.target, Editing::Sent(_)));
    assert_eq!(edit.text(), "Melt");
}

/// A message edited in place completes mentions as the composer does:
/// Enter takes the person offered and goes on editing. ⌥⌘↑ is left to
/// the window, to go to another room.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs local Synapse (testserver/run.sh)"]
async fn an_edit_completes_mentions() {
    let (mut ui, _dir) = room_with("editmention", &["Thanks"]).await;
    let room = ui.room();
    room.members_loaded(Ok(vec![person(
        "@ana:example.org",
        "Ana",
        mtrx_core::Role::Member,
    )]));
    let row = row_of(room, "Thanks");
    let key = (room.rows.iter().filter_map(Row::event))
        .find(|event| event.id == row)
        .map(|event| event.key.clone())
        .unwrap();
    ui.update(Message::Shown(room::Message::Act(actions::Message::Edit(
        key,
        "Thanks".into(),
    ))));
    for c in [" ", "@", "a", "n"] {
        assert_eq!(
            ui.press(Key::Character(c.into()), Modifiers::empty()),
            Status::Captured,
            "{c:?}"
        );
    }
    let edit = ui.room().editing.as_ref().expect("editing");
    assert_eq!(edit.text(), "Thanks @an");
    assert!(edit.field.completion().is_some());

    let (statuses, _) = ui.messages(&[press(
        Key::Named(Named::ArrowUp),
        Modifiers::COMMAND | Modifiers::ALT,
    )]);
    assert_eq!(statuses, [Status::Ignored]);

    assert_eq!(
        ui.press(Key::Named(Named::Enter), Modifiers::empty()),
        Status::Captured
    );
    let edit = ui.room().editing.as_ref().expect("still editing");
    assert_eq!(edit.text(), "Thanks @Ana");
    assert!(edit.field.completion().is_none());
}

/// Whether the widget with `id` is focused, once operated.
struct Focused(iced::advanced::widget::Id, Option<bool>);

impl iced::advanced::widget::Operation for Focused {
    fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn iced::advanced::widget::Operation)) {
        operate(self);
    }

    fn focusable(
        &mut self,
        id: Option<&iced::advanced::widget::Id>,
        _bounds: iced::Rectangle,
        state: &mut dyn iced::advanced::widget::operation::Focusable,
    ) {
        if id == Some(&self.0) {
            self.1 = Some(state.is_focused());
        }
    }
}

/// A press captured beside a stack still reaches its layers: a text editor
/// in one loses the focus, as it does outside a stack (our patch of
/// `Stack::update`, `vendor/iced_widget`).
#[test]
fn a_press_captured_beside_a_stack_reaches_its_layers() {
    use iced::advanced::widget::operation::focusable;

    let content = text_editor::Content::<iced::Renderer>::with_text("Draft");
    let mut renderer = iced::futures::executor::block_on(iced::Renderer::new(
        fonts::INTER,
        iced::Pixels(style::TEXT_SIZE),
        None,
    ))
    .unwrap();
    for stacked in [false, true] {
        let editor = || text_editor(&content).id("field").on_action(|_| ());
        let field: iced::Element<'_, ()> = if stacked {
            iced::widget::stack![editor(), iced::widget::space()].into()
        } else {
            editor().into()
        };
        // Above it, something that captures the press.
        let above = iced::widget::mouse_area(iced::widget::container("Timeline").height(100));
        let view: iced::Element<'_, ()> = iced::widget::column![above.on_press(()), field].into();
        let size = Size::new(300.0, 300.0);
        let mut ui = UserInterface::build(view, size, Cache::default(), &mut renderer);
        ui.operate(&renderer, &mut focusable::focus("field".into()));
        let (_, statuses) = ui.update(
            &[Event::Mouse(mouse::Event::ButtonPressed(
                mouse::Button::Left,
            ))],
            mouse::Cursor::Available(iced::Point::new(10.0, 10.0)),
            &mut renderer,
            &mut clipboard::Null,
            &mut Vec::new(),
        );
        assert_eq!(statuses, [Status::Captured]);
        let mut focused = Focused("field".into(), None);
        ui.operate(&renderer, &mut focused);
        assert_eq!(focused.1, Some(false), "stacked: {stacked}");
    }
}

/// The bounds of a container, once operated.
struct Bounds(iced::advanced::widget::Id, Option<iced::Rectangle>);

impl iced::advanced::widget::Operation for Bounds {
    fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn iced::advanced::widget::Operation)) {
        operate(self);
    }

    fn container(&mut self, id: Option<&iced::advanced::widget::Id>, bounds: iced::Rectangle) {
        if id == Some(&self.0) {
            self.1 = Some(bounds);
        }
    }
}

impl Ui {
    /// The bounds of the container with `id`.
    fn bounds(&mut self, id: iced::advanced::widget::Id) -> iced::Rectangle {
        let mut bounds = Bounds(id, None);
        let mut ui = UserInterface::build(
            self.app.view(),
            SIZE,
            std::mem::take(&mut self.cache),
            &mut self.renderer,
        );
        ui.operate(&self.renderer, &mut bounds);
        self.cache = ui.into_cache();
        bounds.1.expect("shown")
    }

    /// Press at `from`, move to `to` and release, as the app's listeners
    /// see it.
    fn drag(&mut self, from: iced::Point, to: iced::Point) {
        self.cursor = from;
        self.send(&[
            Event::Mouse(mouse::Event::CursorMoved { position: from }),
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)),
        ]);
        self.cursor = to;
        self.send(&[Event::Mouse(mouse::Event::CursorMoved { position: to })]);
        let (_, mut messages) = self.messages(&[Event::Mouse(mouse::Event::ButtonReleased(
            mouse::Button::Left,
        ))]);
        // The app listens for the release while a selection is dragged.
        messages.push(Message::Shown(room::Message::Select(
            selection::Message::End,
        )));
        for message in messages {
            self.update(message);
        }
    }
}

/// Shift with an arrow extends a selection made with the mouse also when
/// the composer had the focus before (reported with 0.2.0): the press in
/// the timeline takes it from the composer.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs local Synapse (testserver/run.sh)"]
async fn shift_arrow_extends_a_selection_made_after_typing() {
    let (mut ui, _dir) = room_with("shiftsel", &["Calving fronts retreat"]).await;
    let event = row_of(ui.room(), "Calving fronts retreat");
    ui.run(iced::widget::operation::focus(message_field::COMPOSER));
    assert!(ui.composer_focused());
    // Across the start of the message's text.
    let row = ui.bounds(iced::advanced::widget::Id::from(format!("row:{event}")));
    let (x, y) = (row.x + 60.0, row.y + row.height - 14.0);
    ui.drag(iced::Point::new(x, y), iced::Point::new(x + 40.0, y));
    assert!(!ui.composer_focused());
    let before = ui.room().selection.clone().expect("selected").focus;
    assert_eq!(
        ui.press(Named::ArrowRight, Modifiers::SHIFT),
        Status::Ignored
    );
    let after = ui.room().selection.clone().expect("selected").focus;
    assert_eq!(after.event, before.event);
    assert_eq!(after.pos.offset, before.pos.offset + 1);
}

/// The composer's keys are its own only while it has the focus: Enter
/// elsewhere does not send the draft, Esc elsewhere still cancels a reply
/// (the window's Esc).
#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs local Synapse (testserver/run.sh)"]
async fn enter_sends_only_from_the_composer() {
    let (mut ui, _dir) = room_with("enter", &["Hello"]).await;
    let enter = || Key::Named(Named::Enter);
    ui.room().composer.set_text("Draft");
    ui.send(&[]);
    assert!(!ui.composer_focused());
    assert_eq!(ui.press(enter(), Modifiers::empty()), Status::Ignored);
    assert_eq!(ui.room().composer.text(), "Draft");

    // Replying, Esc with nothing focused cancels the reply.
    let hello = (ui.room().rows.iter().filter_map(Row::event))
        .find(|event| event.event_id.is_some())
        .and_then(|event| crate::actions::Target::of(event, "alice".into(), "Hello".into()));
    ui.room().compose = Compose::Reply(hello.expect("a message"));
    ui.press(Key::Named(Named::Escape), Modifiers::empty());
    assert!(!ui.room().compose.cancellable());

    ui.run(iced::widget::operation::focus(message_field::COMPOSER));
    assert_eq!(ui.press(enter(), Modifiers::empty()), Status::Captured);
    assert_eq!(ui.room().composer.text(), "");
}

/// Shortcuts type nothing in a field (macOS gives the key's text with ⌘
/// too): ⌘1 in the room search goes to the first room, ⌘K in the composer
/// to the search.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs local Synapse (testserver/run.sh)"]
async fn shortcuts_type_nothing_in_fields() {
    let (mut ui, _dir) = room_with("search", &[]).await;
    let open = ui.room().id.clone();
    let other: OwnedRoomId = "!other:localhost".try_into().unwrap();
    ui.main().show_rooms(RoomListState {
        rooms: vec![summary(&other, "other"), summary(&open, "open")],
        loaded: false,
        ..Default::default()
    });
    let typed = |ui: &mut Ui, key: &str, modifiers| {
        let (_, messages) = ui.messages(&[press(Key::Character(key.into()), modifiers)]);
        format!("{messages:?}")
    };

    ui.run(iced::widget::operation::focus(sidebar::SEARCH));
    assert_eq!(
        typed(&mut ui, "o", Modifiers::empty()),
        r#"[RoomFilter("o")]"#
    );
    assert_eq!(
        typed(&mut ui, "1", Modifiers::COMMAND),
        "[GoToRoom(Row(0))]"
    );

    ui.run(iced::widget::operation::focus(message_field::COMPOSER));
    assert_eq!(typed(&mut ui, "k", Modifiers::COMMAND), "[FocusSearch]");
    // Typing still types, also with ⌥.
    assert_eq!(
        typed(&mut ui, "é", Modifiers::ALT),
        format!("[Room({open:?}, Field(Composer, Action(Edit(Insert('é')))))]")
    );
}

/// One panel over the window at a time, above the card: a dialog opened
/// from the settings replaces them; Esc closes it, then the card.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs local Synapse (testserver/run.sh)"]
async fn escape_closes_the_modal_then_the_card() {
    let (mut ui, _dir) = room_with("modal", &[]).await;
    let room = ui.room().id.clone();
    ui.main().card = Some(info::Card::new(
        info::Target::Room(room.clone()),
        &[],
        overview::Tab::Rooms,
    ));
    ui.main().modal = Some(Modal::Settings(settings::Panel::default()));
    ui.update(Message::OpenDialog(dialog::Kind::Leave {
        room,
        name: "modal".into(),
    }));
    assert!(matches!(ui.main().modal, Some(Modal::Dialog(_))));

    ui.press(Key::Named(Named::Escape), Modifiers::empty());
    assert!(ui.main().modal.is_none());
    assert!(ui.main().card.is_some());
    ui.press(Key::Named(Named::Escape), Modifiers::empty());
    assert!(ui.main().card.is_none());
}

/// One popup at a time in the whole window: one opened replaces another,
/// Esc closes it before the card; the open room's closes as its timeline
/// is reloaded or another pane shows, the room list's stays.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs local Synapse (testserver/run.sh)"]
async fn one_popup_in_the_window() {
    let (mut ui, _dir) = room_with("popup", &["Calving"]).await;
    let room = ui.room().id.clone();
    let row = row_of(ui.room(), "Calving");
    let menu = actions::Message::OpenMenu(row.clone(), MenuAt::Toolbar);
    ui.update(actions::act(&room, menu));
    assert!(matches!(ui.main().popup, Some(Popup::MessageMenu { .. })));
    ui.update(Message::AddMenu(true));
    assert!(matches!(ui.main().popup, Some(Popup::AddMenu)));

    ui.main().card = Some(info::Card::new(
        info::Target::Room(room.clone()),
        &[],
        overview::Tab::Rooms,
    ));
    ui.press(Key::Named(Named::Escape), Modifiers::empty());
    assert!(ui.main().popup.is_none());
    assert!(ui.main().card.is_some());
    ui.main().card = None;

    let reload = |ui: &mut Ui| {
        let (id, timeline) = (ui.room().id.clone(), ui.room().timeline.clone());
        let reloaded = room::Message::Reloaded(Ok(Shared(timeline)));
        let _ = ui.app.update(reloaded.to(&id));
    };
    ui.update(actions::act(&room, actions::Message::OpenReaders(row)));
    reload(&mut ui);
    assert!(ui.main().popup.is_none());
    ui.main().popup = Some(Popup::RoomMenu(room.clone()));
    reload(&mut ui);
    assert!(matches!(ui.main().popup, Some(Popup::RoomMenu(_))));

    ui.update(room::Message::Calls(calls::Message::OpenMenu).to(&room));
    assert!(matches!(ui.main().popup, Some(Popup::Calls(_))));
    ui.main().set_pane(Pane::Empty);
    assert!(ui.main().popup.is_none());
}
