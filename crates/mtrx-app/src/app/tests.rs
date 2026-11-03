//! UI snapshots against the local Synapse (`testserver/run.sh`):
//! `cargo test -p mtrx-app -- --ignored`, images land in `target/ui-snapshots/`.

use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::Arc,
    time::{Duration, SystemTime},
};

use futures_util::StreamExt;
use iced::Size;
use iced_test::simulator::Simulator;
use mtrx_core::{
    Attachment, Connection, EventContent, NewRoom, OwnedEventId, Permissions, Profile, RoomKind,
    RoomOrAliasId, RoomTag, Row, RowContext, ScheduledFile, SecretStore, Section, SendState,
    Session, TimelineEntry,
    test_support::{PASSWORD, homeserver, register, unique_name},
};
use tempfile::TempDir;

use super::*;
use crate::{
    fonts,
    message_menu::MenuAt,
    selection::{Point, Pos, Selection},
};

mod editing;
mod keyboard;
mod menu;

const TIMEOUT: Duration = Duration::from_secs(30);
/// The snapshot room's topic, with its calls and links.
const TOPIC: &str = "Ice sheet runs and the weekly call.\n\
    [call] https://meet.jit.si/ice-sheets-weekly\n\
    [call Lab passcode 4711] https://zoom.us/j/123456789\n\
    [link Paper draft] https://www.overleaf.com/project/ice";

fn snapshot_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/ui-snapshots")
}

async fn user(prefix: &str, dir: &TempDir) -> (String, Session) {
    let name = unique_name(prefix);
    register(&name).await.unwrap();
    let profile = Profile::new(prefix, dir.path(), SecretStore::File).unwrap();
    let session = Session::login(profile, &homeserver(), &name, PASSWORD)
        .await
        .unwrap();
    session.start_sync().await;
    (name, session)
}

/// A member of a room, for mentions.
fn person(id: &str, name: &str, role: mtrx_core::Role) -> mtrx_core::Member {
    mtrx_core::Member {
        user: mtrx_core::User {
            id: OwnedUserId::try_from(id).unwrap(),
            name: Some(name.into()),
            avatar: None,
        },
        role,
        level: Some(role.level(0)),
        invited: false,
        own: false,
        manage: Default::default(),
    }
}

/// `text` in an editor, the cursor at its end.
fn typed(text: &str) -> text_editor::Content {
    let mut content = text_editor::Content::with_text(text);
    content.perform(text_editor::Action::Move(text_editor::Motion::DocumentEnd));
    content
}

/// Show `entries` in `room`, from the room's start, as once
/// back-pagination reached it.
fn show(room: &mut Room, mut entries: Vec<TimelineEntry>) {
    if !matches!(entries.first(), Some(TimelineEntry::Start)) {
        entries.insert(0, TimelineEntry::Start);
    }
    room.rows = Arc::new(mtrx_core::rows(&entries, &RowContext::default()));
}

/// The events of the message rows of `room`, to change.
fn events_mut(room: &mut Room) -> impl Iterator<Item = &mut mtrx_core::Event> {
    Arc::make_mut(&mut room.rows)
        .iter_mut()
        .filter_map(|row| match row {
            Row::Message(message) => Some(&mut message.event),
            _ => None,
        })
}

impl Main {
    /// Show `state` as the core arranges it.
    fn show_rooms(&mut self, state: RoomListState) {
        self.lists = Arc::new(RoomLists::new(
            state,
            self.arrange.categories.clone(),
            HashMap::new(),
            &self.own_user,
            BTreeSet::new(),
        ));
    }
}

fn snapshot(app: &App, name: &str) {
    snapshot_at(app, name, None);
}

/// A snapshot with the cursor at `cursor`, e.g. for a hint.
fn snapshot_at(app: &App, name: &str, cursor: Option<iced::Point>) {
    // One at a time: renderers drawing at once in parallel tests crashed
    // on Windows CI (software Direct3D).
    static RENDERING: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _rendering = RENDERING.lock().unwrap_or_else(|e| e.into_inner());
    // Snapshots are for looking at, not regression baselines: always rewrite.
    // iced_test appends the renderer name, e.g. `login-wgpu.png`.
    let path = snapshot_dir().join(name);
    for entry in std::fs::read_dir(snapshot_dir())
        .into_iter()
        .flatten()
        .flatten()
    {
        if entry
            .file_name()
            .to_string_lossy()
            .starts_with(&format!("{name}-"))
        {
            let _ = std::fs::remove_file(entry.path());
        }
    }
    let mut ui = Simulator::with_size(fonts::settings(), Size::new(1200.0, 800.0), app.view());
    if let Some(cursor) = cursor {
        ui.point_at(cursor);
    }
    // Images too large for the renderer's texture atlas only show after a
    // few frames, as the atlas grows (the app draws many).
    for _ in 0..2 {
        let _ = ui.snapshot(&app.theme()).unwrap();
    }
    let snapshot = ui.snapshot(&app.theme()).unwrap();
    assert!(snapshot.matches_image(&path).unwrap());
}

fn app(screen: Screen, mode: Mode) -> App {
    App {
        profile: Profile::new(
            "ui",
            &Profile::default_data_root().unwrap(),
            SecretStore::File,
        )
        .unwrap(),
        focused: true,
        google: Arc::new(Google::new(&snapshot_dir(), SecretStore::File)),
        settings: Settings::default(),
        settings_file: None,
        view_file: None,
        placement: Placement::default(),
        placement_file: None,
        resizing: false,
        screen,
        system_mode: mode,
        window: Size::new(1200.0, 800.0),
        scale: 2.0,
        math: Math::new(),
        badge: Badge::default(),
        updater: Updater::default(),
    }
}

#[test]
fn login_screen() {
    snapshot(&app(Screen::Login(Login::default()), Mode::Light), "login");
}

/// Without a keyring (Linux: no Secret Service) the sign-in screen says so.
#[test]
fn login_without_keyring() {
    let mut app = app(Screen::Starting, Mode::Light);
    let error = mtrx_core::Error::NoSecretStore("no secrets service".into());
    let _ = app.update(Message::Restored(Err(Shared(Arc::new(error)))));
    let Screen::Login(login) = &app.screen else {
        panic!("not the sign-in screen");
    };
    assert!(login.error.is_some());
    snapshot(&app, "login-no-keyring");
}

/// A click on a mention shows the person's card, not the direct chat with
/// them.
#[test]
fn clicking_a_mention_shows_the_persons_card() {
    use iced::{
        Event, Theme,
        mouse::{self, Button::Left},
    };
    use std::rc::Rc;

    let document = mtrx_format::Document::from_html(
        r#"Ask <a href="https://matrix.to/#/@nils:server.org">Nils</a>"#,
    );
    let own: OwnedUserId = "@ana:server.org".try_into().unwrap();
    // Links are messages, gestures nothing.
    let view = message::view(
        &document,
        &Theme::Light,
        &own,
        message::Links {
            open: |url| Some(Message::LinkClicked(url)),
            menu: Rc::new(|_| None),
        },
        message::Select {
            range: None,
            focus: None,
            dragging: false,
            on_gesture: Rc::new(|_| None),
            found: Vec::new(),
        },
        false,
    );
    let mut ui = Simulator::with_size(fonts::settings(), Size::new(400.0, 100.0), view);
    // Pressed and released on "Nils".
    ui.point_at((45.0, 8.0));
    let _ = ui.simulate([
        Event::Mouse(mouse::Event::ButtonPressed(Left)),
        Event::Mouse(mouse::Event::ButtonReleased(Left)),
    ]);
    let clicked: Vec<Message> = ui.into_messages().flatten().collect();
    assert!(
        matches!(&clicked[..], [Message::LinkClicked(_)]),
        "{clicked:?}"
    );

    let mut app = app(Screen::Login(Login::default()), Mode::Light);
    let task = app.update(clicked.into_iter().next().unwrap());
    let sent: Vec<Message> = iced::futures::executor::block_on(
        iced_runtime::task::into_stream(task)
            .expect("a message")
            .filter_map(|action| async move {
                match action {
                    iced_runtime::Action::Output(message) => Some(message),
                    _ => None,
                }
            })
            .collect(),
    );
    assert!(
        matches!(
            &sent[..],
            [Message::ShowInfo(info::Target::User(user))] if user.id == "@nils:server.org"
        ),
        "{sent:?}"
    );
}

/// The zoom keys step the text size, also on the sign-in screen; the
/// window keeps its size in logical pixels, and images are drawn for the
/// display's scale times the text size.
#[test]
fn zoom_steps_the_text_size() {
    use crate::text_size::TextSize;

    let mut app = app(Screen::Login(Login::default()), Mode::Light);
    let _ = app.update(Message::Zoom(Zoom::In));
    assert_eq!(app.settings.text_size, TextSize::Larger);
    assert_eq!(app.scale_factor(), 1.25);
    for _ in 0..5 {
        let _ = app.update(Message::Zoom(Zoom::In));
    }
    assert_eq!(app.settings.text_size, TextSize::Largest);
    assert_eq!(app.viewport(), Size::new(800.0, 800.0 / 1.5));
    assert_eq!(app.pixel_scale(), 3.0);
    // Resizes come in layout units.
    let _ = app.update(Message::WindowResized(Size::new(600.0, 400.0)));
    assert_eq!(app.window, Size::new(900.0, 600.0));
    let _ = app.update(Message::Zoom(Zoom::Out));
    assert_eq!(app.settings.text_size, TextSize::Larger);
    let _ = app.update(Message::Zoom(Zoom::Reset));
    assert_eq!(app.settings.text_size, TextSize::Default);
    assert_eq!(app.viewport(), Size::new(900.0, 600.0) / 1.1);
    assert_eq!(app.pixel_scale(), 2.0 * 1.1);
}

/// ⌘F opens the find bar, ⌘G and ⇧⌘G go to the next and previous match,
/// wherever the focus is.
#[test]
fn find_keys() {
    use iced::{event::Status, keyboard};

    let key = |c: &str, modifiers| {
        let event = iced::Event::Keyboard(keyboard::Event::KeyPressed {
            key: keyboard::Key::Character(c.into()),
            modified_key: keyboard::Key::Character(c.into()),
            physical_key: keyboard::key::Physical::Unidentified(
                keyboard::key::NativeCode::Unidentified,
            ),
            location: keyboard::Location::Standard,
            modifiers,
            text: None,
            repeat: false,
        });
        shortcuts(event, Status::Captured, window::Id::unique())
    };
    let command = keyboard::Modifiers::COMMAND;
    assert!(matches!(
        key("f", command),
        Some(Message::Shown(room::Message::Find(find::Message::Open)))
    ));
    assert!(matches!(
        key("g", command),
        Some(Message::Shown(room::Message::Find(find::Message::Next)))
    ));
    assert!(matches!(
        key("g", command | keyboard::Modifiers::SHIFT),
        Some(Message::Shown(room::Message::Find(find::Message::Previous)))
    ));
    assert!(key("f", keyboard::Modifiers::empty()).is_none());
}

/// The window opens at the same size in logical pixels at any text size,
/// the first time and as it was left.
#[test]
fn window_opens_at_one_size() {
    use crate::text_size::TextSize;

    let profile = app(Screen::Starting, Mode::Light).profile;
    let left = Frame::new(iced::Point::new(40.0, 60.0), Size::new(900.0, 700.0));
    for (placement, size) in [
        (Placement::default(), placement::DEFAULT_SIZE),
        (
            Placement {
                frame: Some(left),
                maximized: false,
            },
            left.size(),
        ),
    ] {
        for text_size in TextSize::ALL {
            let environment = Environment {
                profile: profile.clone(),
                data_root: snapshot_dir(),
                secrets: SecretStore::File,
                settings: Settings {
                    text_size,
                    ..Settings::default()
                },
                placement,
            };
            let opened = environment.window().size * text_size.scale();
            assert!((opened.width - size.width).abs() < 1e-3, "{text_size}");
            assert!((opened.height - size.height).abs() < 1e-3, "{text_size}");
        }
    }
}

/// Moved and resized, the window is saved where it is; maximised, its frame
/// from before is kept to go back to.
#[test]
fn window_placement_is_saved() {
    let dir = TempDir::new().unwrap();
    let file = dir.path().join("window.json");
    let mut app = app(Screen::Login(Login::default()), Mode::Light);
    app.placement_file = Some(file.clone());
    app.scale = 2.0;
    app.settings.text_size = crate::text_size::TextSize::Default;
    let placed = |maximized| Message::WindowPlaced {
        position: Some(iced::Point::new(1800.0, 200.0)),
        maximized,
        fullscreen: false,
    };

    // Reported in layout units.
    let _ = app.update(Message::WindowResized(
        Size::new(1000.0, 700.0) / app.scale_factor(),
    ));
    let _ = app.update(placed(false));
    let left = Frame::new(iced::Point::new(1800.0, 200.0), Size::new(1000.0, 700.0));
    let saved = Placement::load(&file);
    assert!(!saved.maximized);
    let frame = saved.frame.unwrap();
    assert_eq!(frame.position(), left.position());
    assert!((frame.width - left.width).abs() < 1e-3 && (frame.height - left.height).abs() < 1e-3);

    let _ = app.update(Message::WindowResized(
        Size::new(1512.0, 920.0) / app.scale_factor(),
    ));
    let _ = app.update(placed(true));
    let saved = Placement::load(&file);
    assert!(saved.maximized);
    assert_eq!(saved.frame, Some(frame));
}

/// iced drops the app only after its runtime has stopped, when the SDK's
/// stores can no longer close (dropping a session then panics): quitting
/// (on Windows and Linux, closing the window) ends the session first, and
/// one restored meanwhile.
#[test]
fn quitting_ends_the_session() {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let dir = TempDir::new().unwrap();
    let session = |name: &str| {
        let profile = Profile::new(name, dir.path(), SecretStore::File).unwrap();
        let session = runtime
            .block_on(mtrx_core::test_support::offline_session(profile))
            .unwrap();
        Shared(Arc::new(session))
    };
    let (open, restored) = (session("open"), session("restored"));
    let app = {
        // As iced runs the app.
        let _runtime = runtime.enter();
        let mut app = app(Screen::Main(Box::new(Main::new(open.0))), Mode::Light);
        let close = app.update(Message::CloseWindow(window::Id::unique()));
        assert!(matches!(app.screen, Screen::Closing));
        let restored = app.update(Message::Restored(Ok(restored)));
        drop((close, restored));
        app
    };
    assert!(matches!(app.screen, Screen::Closing));
    drop(runtime);
    drop(app);
}

/// Closing the window cancels a browser sign-in.
#[test]
fn closing_the_window_cancels_a_browser_sign_in() {
    let mut app = app(Screen::Login(Login::default()), Mode::Light);
    let (_sign_in, handle) = Task::<Message>::none().abortable();
    let Screen::Login(login) = &mut app.screen else {
        unreachable!()
    };
    login.browser_started(handle.clone());
    let _ = app.update(Message::CloseWindow(window::Id::unique()));
    assert!(handle.is_aborted());
}

/// A window left on a display since unplugged opens on the main display,
/// as the first time; one on a display still attached stays.
#[test]
fn window_left_off_screen_opens_on_the_main_display() {
    use iced_runtime::{Action, task, window::Action as Window};

    let moves = |task: Task<Message>| -> Vec<iced::Point> {
        let Some(stream) = task::into_stream(task) else {
            return Vec::new();
        };
        iced::futures::executor::block_on(stream.collect::<Vec<_>>())
            .into_iter()
            .filter_map(|action| match action {
                Action::Window(Window::Move(_, position)) => Some(position),
                _ => None,
            })
            .collect()
    };
    let laptop = Rectangle::new(iced::Point::new(0.0, 25.0), Size::new(1512.0, 920.0));
    let monitor = Rectangle::new(iced::Point::new(1512.0, 0.0), Size::new(2560.0, 1415.0));
    let left = iced::Point::new(1800.0, 200.0);
    let mut app = app(Screen::Login(Login::default()), Mode::Light);
    app.placement.frame = Some(Frame::new(left, Size::new(1000.0, 700.0)));
    let id = window::Id::unique();

    let both = app.update(Message::WindowOpened(
        id,
        Some(vec![laptop, monitor]),
        Some(left),
    ));
    assert_eq!(moves(both), []);
    let unknown = app.update(Message::WindowOpened(id, None, Some(left)));
    assert_eq!(moves(unknown), []);
    let unplugged = app.update(Message::WindowOpened(id, Some(vec![laptop]), Some(left)));
    assert_eq!(moves(unplugged), [iced::Point::new(156.0, 85.0)]);
}

/// Closing the window hides it on macOS, where the app goes on; elsewhere
/// the app gets the close request, and quits through `CloseWindow`.
#[test]
fn closing_the_window_hides_it_on_macos_and_quits_elsewhere() {
    let settings = closing(window::Settings::default());
    assert!(!settings.exit_on_close_request);
    #[cfg(target_os = "macos")]
    assert!(settings.platform_specific.hides_on_close);
}

/// Ctrl+W closes the window as its close button does, also with the
/// composer or another field focused, and on the sign-in screen. On macOS
/// ⌘W is the Window menu's Close, which hides the window.
#[test]
fn command_w_closes_the_window() {
    use iced::{event::Status, keyboard};

    let id = window::Id::unique();
    let press = |with: keyboard::Modifiers| {
        let mut event =
            iced_test::simulator::press_key(keyboard::Key::Character("w".into()), Some("w".into()));
        if let iced::Event::Keyboard(keyboard::Event::KeyPressed { modifiers, .. }) = &mut event {
            *modifiers = with;
        }
        event
    };
    assert!(window_keys(press(keyboard::Modifiers::empty()), Status::Ignored, id).is_none());
    let close = window_keys(press(keyboard::Modifiers::COMMAND), Status::Captured, id);
    if cfg!(target_os = "macos") {
        assert!(close.is_none());
        return;
    }
    assert!(matches!(close, Some(Message::CloseWindow(closed)) if closed == id));

    let mut app = app(Screen::Login(Login::default()), Mode::Light);
    let _ = app.update(close.unwrap());
    assert!(matches!(app.screen, Screen::Closing));
}

/// [`Event::id`](mtrx_core::Event::id) of message `event` in `rows`.
fn room_row(rows: &[Row], event: &mtrx_core::OwnedEventId) -> Option<String> {
    (rows.iter().filter_map(Row::event))
        .find(|ev| ev.event_id.as_ref() == Some(event))
        .map(|ev| ev.id.clone())
}

/// A diagonal gradient as PNG, for avatars.
fn picture(from: [u8; 3], to: [u8; 3]) -> Vec<u8> {
    gradient(96, 96, from, to)
}

/// A `width`×`height` diagonal gradient as PNG.
fn gradient(width: u32, height: u32, from: [u8; 3], to: [u8; 3]) -> Vec<u8> {
    let img = ::image::RgbImage::from_fn(width, height, |x, y| {
        let t = (x + y) as f32 / (width + height) as f32;
        ::image::Rgb(std::array::from_fn(|i| {
            (from[i] as f32 * (1.0 - t) + to[i] as f32 * t) as u8
        }))
    });
    let mut png = Vec::new();
    img.write_to(
        &mut std::io::Cursor::new(&mut png),
        ::image::ImageFormat::Png,
    )
    .unwrap();
    png
}

/// Wait for the invite to `room`, then join.
async fn accept_invite(session: &Session, room: &OwnedRoomId) {
    let list = session.room_list().await.unwrap();
    let mut updates = std::pin::pin!(list.updates());
    tokio::time::timeout(TIMEOUT, async {
        while let Some(state) = updates.next().await {
            if state.rooms.iter().any(|r| &r.id == room) {
                break;
            }
        }
    })
    .await
    .unwrap();
    session
        .join(<&RoomOrAliasId>::from(&**room), &[])
        .await
        .unwrap();
}

/// Send and wait until the server has it.
async fn send(timeline: &RoomTimeline, text: &str) -> OwnedEventId {
    send_reply(timeline, text, None).await
}

async fn send_reply(
    timeline: &RoomTimeline,
    text: &str,
    to: Option<&OwnedEventId>,
) -> OwnedEventId {
    tokio::time::timeout(
        TIMEOUT,
        timeline
            .send_markdown(text, to.map(|id| &**id))
            .await
            .unwrap()
            .sent(),
    )
    .await
    .unwrap()
    .unwrap()
}

/// The key of the message with `body`, once the timeline shows it sent: a
/// message [`send`] waited for can still show as being sent, and an edit
/// then goes to the send queue, which sent it already.
async fn key_of(timeline: &RoomTimeline, body: &str) -> mtrx_core::EventKey {
    tokio::time::timeout(TIMEOUT, async {
        let mut updates = std::pin::pin!(timeline.updates());
        while let Some(entries) = updates.next().await {
            let found = entries.iter().find_map(|e| match e {
                TimelineEntry::Event(ev)
                    if ev.send_state == SendState::Sent
                        && matches!(&ev.content, EventContent::Message(m) if m.body == body) =>
                {
                    Some(ev.key.clone())
                }
                _ => None,
            });
            if let Some(key) = found {
                return key;
            }
        }
        unreachable!()
    })
    .await
    .unwrap()
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs local Synapse (testserver/run.sh)"]
async fn main_screen_with_math() {
    let dir = TempDir::new().unwrap();
    let (_, alice) = user("alice", &dir).await;
    let (_, bob) = user("bob", &dir).await;
    let (_, carol) = user("carol", &dir).await;
    // Alice and the rooms keep the initial-letter fallback.
    bob.set_avatar("image/png", picture([40, 90, 200], [120, 220, 255]))
        .await
        .unwrap();
    carol
        .set_avatar("image/png", picture([230, 120, 40], [250, 210, 90]))
        .await
        .unwrap();

    let space = alice
        .create_room(NewRoom {
            name: Some("Climate".into()),
            space: true,
            ..Default::default()
        })
        .await
        .unwrap();
    let room = alice
        .create_room(NewRoom {
            name: Some("ice-sheets".into()),
            topic: Some("Basal melt and ice flux: drafts, figures and model runs".into()),
            encrypted: true,
            invite: vec![bob.user_id(), carol.user_id()],
            ..Default::default()
        })
        .await
        .unwrap();
    let general = alice
        .create_room(NewRoom {
            name: Some("general".into()),
            encrypted: true,
            ..Default::default()
        })
        .await
        .unwrap();
    let dm = alice
        .create_room(NewRoom {
            invite: vec![carol.user_id()],
            direct: true,
            encrypted: true,
            ..Default::default()
        })
        .await
        .unwrap();
    let alice = Arc::new(alice);
    let spaced = alice.room(&space).unwrap();
    spaced.add_child(&room).await.unwrap();
    spaced.add_child(&general).await.unwrap();

    accept_invite(&bob, &room).await;
    accept_invite(&carol, &room).await;
    accept_invite(&carol, &dm).await;

    let alice_timeline = alice.timeline(&room).await.unwrap();
    // Wait for the joins so bob and carol receive the room keys.
    let mut entries = std::pin::pin!(alice_timeline.updates());
    tokio::time::timeout(TIMEOUT, async {
        while let Some(entries) = entries.next().await {
            let joined = |user: &Session| {
                entries.iter().any(
                    |e| matches!(e, TimelineEntry::Event(ev) if ev.sender.id == user.user_id()),
                )
            };
            if joined(&bob) && joined(&carol) {
                break;
            }
        }
    })
    .await
    .unwrap();
    let mut sent = Vec::new();
    for text in [
        "Morning! Draft of the **ice flux** section is up.",
        r"The basal melt rate follows $\dot m = \frac{G + \tau_b u_b}{\rho_i L}$ where $G$ is the geothermal flux.",
        "Steady state requires\n$$\\frac{\\partial H}{\\partial t} = -\\nabla \\cdot (H \\bar{\\mathbf{u}}) + \\dot a - \\dot m = 0$$",
    ] {
        sent.push(send(&alice_timeline, text).await);
    }
    let bob_timeline = bob.timeline(&room).await.unwrap();
    send(
        &bob_timeline,
        "Looks good, but check the sign of $\\dot m$ in eq. 2 🙂",
    )
    .await;
    send_reply(
        &bob_timeline,
        "Which geothermal flux dataset is it?",
        Some(&sent[0]),
    )
    .await;
    let carol_timeline = carol.timeline(&room).await.unwrap();
    // Sent with a typo, then edited.
    let typo = "I can run the sensitivity tests tonite. Quick call first? https://meet.google.com/abc-defg-hij";
    send(&carol_timeline, typo).await;
    carol_timeline
        .edit_markdown(
            &key_of(&carol_timeline, typo).await,
            "I can run the sensitivity tests tonight. Quick call first? https://meet.google.com/abc-defg-hij",
        )
        .await
        .unwrap();
    send(
        &carol.timeline(&dm).await.unwrap(),
        "Coffee before the meeting?",
    )
    .await;
    let draft = "Morning! Draft of the **ice flux** section is up.";
    carol_timeline
        .toggle_reaction(&key_of(&carol_timeline, draft).await, "🎉")
        .await
        .unwrap();
    bob_timeline
        .toggle_reaction(&key_of(&bob_timeline, draft).await, "🎉")
        .await
        .unwrap();
    alice_timeline
        .toggle_reaction(
            &key_of(&alice_timeline, "Which geothermal flux dataset is it?").await,
            "👍",
        )
        .await
        .unwrap();
    bob_timeline.mark_read().await.unwrap();

    // Alice's view once the room list knows the space and the DM's picture.
    let list = Arc::new(alice.room_list().await.unwrap());
    let rooms = tokio::time::timeout(TIMEOUT, async {
        let mut updates = std::pin::pin!(list.updates());
        while let Some(state) = updates.next().await {
            let in_space = state.rooms.iter().any(|r| r.id == room)
                && state.graph.parents(&room).contains(&space);
            let dm_avatar = state.rooms.iter().any(|r| r.id == dm && r.avatar.is_some());
            if state.loaded && in_space && dm_avatar {
                return state;
            }
        }
        unreachable!()
    })
    .await
    .unwrap();
    let timeline = Arc::new(alice.timeline(&room).await.unwrap());
    while !timeline.paginate_back(50).await.unwrap() {}
    let entries = tokio::time::timeout(TIMEOUT, async {
        let mut updates = std::pin::pin!(timeline.updates());
        while let Some(entries) = updates.next().await {
            let events = || {
                entries.iter().filter_map(|e| match e {
                    TimelineEntry::Event(ev) => Some(ev),
                    _ => None,
                })
            };
            let messages = || {
                events().filter_map(|ev| match &ev.content {
                    EventContent::Message(m) => Some(m),
                    _ => None,
                })
            };
            let from_carol = events().any(|ev| {
                ev.sender.id == carol.user_id() && matches!(ev.content, EventContent::Message(_))
            });
            let reply_loaded =
                messages().any(|m| m.in_reply_to.as_ref().is_some_and(|r| r.preview.is_some()));
            let reactions = messages().filter(|m| !m.reactions.is_empty()).count() == 2
                && messages().any(|m| m.reactions.iter().any(|r| r.count == 2));
            let read = events().any(|ev| ev.read_by.iter().any(|r| r.id == bob.user_id()));
            let edited = messages().any(|m| m.edited);
            if from_carol && reply_loaded && reactions && read && edited {
                return entries;
            }
        }
        unreachable!()
    })
    .await
    .unwrap();
    // Shown hovered, with its time in the gutter.
    let hovered = entries
        .iter()
        .find_map(|e| match e {
            TimelineEntry::Event(ev) => match &ev.content {
                EventContent::Message(m) if m.body.starts_with("The basal melt") => {
                    Some(ev.id.clone())
                }
                _ => None,
            },
            _ => None,
        })
        .unwrap();

    for mode in [Mode::Light, Mode::Dark] {
        let session = Arc::new(Session::restore(alice.profile().clone()).await.unwrap());
        let mut main = Main::new(session);
        let mut state = rooms.clone();
        if mode == Mode::Dark {
            // Notes to self, started and unread; pinned above the sections.
            let mut notes = (rooms.rooms.iter())
                .find(|r| r.direct_user.is_some())
                .unwrap()
                .clone();
            notes.id = "!notes:localhost".try_into().unwrap();
            notes.direct_user = Some(mtrx_core::User::from(alice.user_id()));
            notes.unread_messages = 2;
            state.rooms.push(notes);
        }
        main.verified = Some(true);
        let mut open = Room::new(
            alice.room(&room).unwrap(),
            timeline.clone(),
            main.session.draft_keeper(&room),
        );
        show(&mut open, entries.clone());
        open.hover(hovered.clone(), true);
        // The room's calls and links, in its topic; its first message
        // pinned.
        let first = (entries.iter())
            .find_map(|e| match e {
                TimelineEntry::Event(ev) if matches!(ev.content, EventContent::Message(_)) => {
                    ev.event_id.clone()
                }
                _ => None,
            })
            .unwrap();
        if let Some(summary) = state.rooms.iter_mut().find(|r| r.id == room) {
            summary.topic = Some(TOPIC.into());
            summary.pinned = vec![first];
        }
        main.show_rooms(state);
        open.pins.update(pins::Message::CanPin(Some(true)));
        open.pins.update(pins::Message::Updated(entries.clone()));
        open.calls.space_service =
            Some(CallService::jitsi("Lab Jitsi", "https://video.lab.example").unwrap());
        // The call menu in one shot (the reaction picker after it), the
        // topic's editor and a reply in the other.
        let mut reaction_picker = None;
        if mode == Mode::Dark {
            main.popup = Some(Popup::Calls(Default::default()));
        }
        if mode == Mode::Light {
            open.topic.open = true;
            open.topic.can_edit = Some(true);
            open.topic.editor = Some(topic::Editor::new(TOPIC));
            // The hovered message's menu, from its toolbar.
            main.popup = Some(Popup::MessageMenu {
                row: hovered.clone(),
                at: MenuAt::Toolbar,
            });
            // Text selected from the first message into bob's reply.
            let id_of = |start: &str| {
                entries
                    .iter()
                    .find_map(|e| match e {
                        TimelineEntry::Event(ev) => match &ev.content {
                            EventContent::Message(m) if m.body.starts_with(start) => {
                                Some(ev.id.clone())
                            }
                            _ => None,
                        },
                        _ => None,
                    })
                    .unwrap()
            };
            open.selection = Some(Selection {
                anchor: Point {
                    event: id_of("The basal"),
                    pos: Pos { unit: 0, offset: 4 },
                },
                focus: Point {
                    event: id_of("I can run"),
                    pos: Pos { unit: 0, offset: 9 },
                },
                dragging: false,
            });
            // From inside typeset math, formulas as LaTeX; each sender's
            // name and time first, the first not selected but put there; a
            // reply's quote before it.
            let copied = open.selected(&Default::default()).unwrap().text;
            let name = |user: &mtrx_core::OwnedUserId| user.localpart().to_owned();
            assert!(copied.starts_with(&format!("{}, ", name(&alice.user_id()))));
            assert!(copied.contains(&format!("\n\n> {}: Morning! Draft", name(&alice.user_id()))));
            assert!(copied.contains(
                "\nbasal melt rate follows $\\dot m = \\frac{G + \\tau_b u_b}{\\rho_i L}$ where"
            ));
            assert!(copied.contains(&format!("\n\n{}, ", name(&bob.user_id()))));
            assert!(copied.contains("\nWhich geothermal flux dataset is it?\n\n"));
            assert!(copied.ends_with("\nI can run"));
            assert!(copied.contains(&format!("\n\n{}, ", name(&carol.user_id()))));
        }
        open.typing = vec!["bob".into()];
        if mode == Mode::Dark {
            // From the smiley after alice's reactions.
            let reacted = entries
                .iter()
                .find_map(|e| match e {
                    TimelineEntry::Event(ev) => match &ev.content {
                        EventContent::Message(m) if !m.reactions.is_empty() => Some(ev.id.clone()),
                        _ => None,
                    },
                    _ => None,
                })
                .unwrap();
            reaction_picker = Some(Popup::Reactions {
                row: reacted,
                at: actions::PickerAt::Reactions,
                all: None,
            });
        } else {
            let carol_message = entries.iter().rev().find_map(|e| match e {
                TimelineEntry::Event(ev) if ev.sender.id == carol.user_id() => Some(ev),
                _ => None,
            });
            open.compose = Compose::Reply(
                actions::Target::of(
                    carol_message.unwrap(),
                    "carol".into(),
                    "I can run the sensitivity tests tonight.".into(),
                )
                .unwrap(),
            );
        }
        main.pane = Pane::Open(open);
        for (mxc, size) in main.avatars_on_screen(&Default::default()) {
            main.avatars.load_now(&alice, &mxc, size).await;
        }

        let mut app = app(Screen::Main(Box::new(main)), mode);
        // Both name styles: plain (the default) in light, coloured in dark.
        app.settings.colored_names = mode == Mode::Dark;
        let params = app.math_params();
        if let Screen::Main(main) = &app.screen {
            for document in main.pane.room().unwrap().math_documents() {
                app.math.render_now(document, &params);
            }
        }
        if mode == Mode::Dark
            && let Screen::Main(main) = &mut app.screen
        {
            // A quote of a long message wraps onto a few lines; the message
            // it quotes is marked, as just gone to.
            let room = main.pane.room_mut().unwrap();
            let quoted = events_mut(room).find_map(|ev| match &mut ev.content {
                EventContent::Message(m) => m.in_reply_to.as_mut(),
                _ => None,
            });
            let reply = quoted.unwrap();
            reply.preview = Some("Morning! Draft of the ice flux section is up. ".repeat(12));
            let original = reply.event_id.clone();
            let row = room_row(&room.rows, &original);
            let _ = app.update(Message::Shown(room::Message::GoTo(original)));
            let Screen::Main(main) = &app.screen else {
                unreachable!()
            };
            assert_eq!(main.pane.room().unwrap().marked, row);
        }
        let name = if mode == Mode::Dark {
            "main-dark"
        } else {
            "main-light"
        };
        snapshot(&app, name);
        if mode == Mode::Light {
            // Everyone who read up to a message, listed from its markers,
            // instead of the hovered message's menu.
            let Screen::Main(main) = &mut app.screen else {
                unreachable!()
            };
            let open = main.pane.room_mut().unwrap();
            let read = (open.rows.iter().filter_map(Row::event))
                .find(|ev| ev.read_by.iter().any(|r| r.id == bob.user_id()))
                .map(|ev| ev.id.clone())
                .unwrap();
            main.popup = Some(Popup::Readers(read));
            for (mxc, size) in main.avatars_on_screen(&Default::default()) {
                main.avatars.load_now(&alice, &mxc, size).await;
            }
            snapshot(&app, "readers-light");
            // Everyone who reacted with the reaction of two, listed from its
            // count, instead of the readers.
            let Screen::Main(main) = &mut app.screen else {
                unreachable!()
            };
            let open = main.pane.room_mut().unwrap();
            let reacted = (open.rows.iter().filter_map(Row::event))
                .find_map(|ev| match &ev.content {
                    EventContent::Message(m) => m
                        .reactions
                        .iter()
                        .find(|r| r.count == 2)
                        .map(|r| (ev.id.clone(), r.key.clone())),
                    _ => None,
                })
                .unwrap();
            let readers = main.popup.take();
            main.popup = Some(Popup::Reactors {
                row: reacted.0,
                key: reacted.1,
            });
            for (mxc, size) in main.avatars_on_screen(&Default::default()) {
                main.avatars.load_now(&alice, &mxc, size).await;
            }
            snapshot(&app, "reactors-light");
            let Screen::Main(main) = &mut app.screen else {
                unreachable!()
            };
            main.popup = readers;
            // Alice's first message not sent, offered to retry or delete,
            // instead of the list of readers.
            let Screen::Main(main) = &mut app.screen else {
                unreachable!()
            };
            let readers = main.popup.take();
            let open = main.pane.room_mut().unwrap();
            let unsent = events_mut(open)
                .find(|ev| ev.own && matches!(ev.content, EventContent::Message(_)))
                .unwrap();
            let sent = std::mem::replace(
                &mut unsent.send_state,
                mtrx_core::SendState::Failed("event too large".into()),
            );
            main.popup = Some(Popup::Unsent(unsent.id.clone()));
            snapshot(&app, "unsent-light");
            let Screen::Main(main) = &mut app.screen else {
                unreachable!()
            };
            main.popup = readers;
            let open = main.pane.room_mut().unwrap();
            for ev in events_mut(open) {
                if matches!(ev.send_state, mtrx_core::SendState::Failed(_)) {
                    ev.send_state = sent.clone();
                }
            }
        }
        // A click elsewhere closes the call menu (dark) or the list of
        // readers and the topic panel (light), and does nothing else:
        // what is behind neither lights up under the cursor nor takes the
        // click.
        let mut ui = Simulator::with_size(fonts::settings(), Size::new(1200.0, 800.0), app.view());
        // Below the topic panel, on the timeline.
        ui.point_at(iced::Point::new(400.0, 450.0));
        ui.simulate(iced_test::simulator::click());
        let messages: Vec<_> = ui.into_messages().collect();
        assert!(
            mode == Mode::Light
                || messages.iter().any(|m| matches!(
                    m,
                    Message::Room(_, room::Message::Calls(calls::Message::Close))
                )),
            "{messages:?}"
        );
        assert!(
            messages.iter().all(|m| matches!(
                m,
                Message::Room(
                    _,
                    room::Message::Calls(calls::Message::Close)
                        | room::Message::Topic(topic::Message::Dismiss)
                        | room::Message::Act(
                            actions::Message::CloseReaders | actions::Message::ClosePicker
                        )
                )
            )),
            "{messages:?}"
        );
        if mode == Mode::Light {
            assert!(
                messages.iter().any(|m| {
                    matches!(
                        m,
                        Message::Room(_, room::Message::Act(actions::Message::CloseReaders))
                    )
                }),
                "{messages:?}"
            );
            if let Screen::Main(main) = &mut app.screen {
                main.popup = None;
            }
            composer_keeps_focus(&mut app);
            // The topic panel, not editing: Edit in its corner.
            snapshot(&app, "topic-panel");
        } else {
            // A service in the call menu says what starting a call does.
            snapshot_at(&app, "call-menu-hint", Some(iced::Point::new(864.0, 110.0)));
            // Esc closes the call menu too.
            let _ = app.update(Message::Escape);
            let Screen::Main(main) = &app.screen else {
                unreachable!()
            };
            assert!(main.popup.is_none());
            // The reaction window, from the smiley after alice's reactions.
            if let Screen::Main(main) = &mut app.screen {
                main.popup = reaction_picker.take();
            }
            snapshot(&app, "reactions-dark");
            if let Screen::Main(main) = &mut app.screen {
                main.popup = None;
            }
        }
        // ⌘F: every match highlighted, in text and typeset math, the
        // newest one current; Esc closes the bar.
        if let Screen::Main(main) = &mut app.screen {
            main.pane.room_mut().unwrap().selection = None;
        }
        let _ = app.update(Message::Shown(room::Message::Find(find::Message::Open)));
        let _ = app.update(Message::Shown(room::Message::Find(find::Message::Query(
            "FLUX".into(),
        ))));
        let found = |app: &App| {
            let Screen::Main(main) = &app.screen else {
                unreachable!()
            };
            main.pane
                .room()
                .unwrap()
                .find
                .as_ref()
                .map(|f| f.current().cloned())
        };
        assert!(found(&app).flatten().is_some());
        let name = if mode == Mode::Dark {
            "find-dark"
        } else {
            "find-light"
        };
        snapshot(&app, name);
        let _ = app.update(Message::Escape);
        assert_eq!(found(&app), None);
    }
}

/// The composer keeps its focus, so typing goes on, while what is around
/// it comes and goes: someone typing, bars, banners, the member list,
/// panels over the window.
fn composer_keeps_focus(app: &mut App) {
    use iced::advanced::{renderer::Headless, widget::operation::focusable};
    use iced_runtime::user_interface::{Cache, UserInterface};

    fn main(app: &mut App) -> &mut Main {
        let Screen::Main(main) = &mut app.screen else {
            unreachable!()
        };
        main
    }
    fn room(app: &mut App) -> &mut Room {
        main(app).pane.room_mut().unwrap()
    }
    // Nothing else open that could take the focus.
    main(app).popup = None;
    room(app).topic.editor = None;
    room(app).typing.clear();

    let mut renderer = iced::futures::executor::block_on(iced::Renderer::new(
        fonts::INTER,
        iced::Pixels(style::TEXT_SIZE),
        None,
    ))
    .unwrap();
    let size = Size::new(1200.0, 800.0);
    let mut ui = UserInterface::build(app.view(), size, Cache::default(), &mut renderer);
    ui.operate(&renderer, &mut focusable::focus(room::COMPOSER.into()));
    let mut cache = ui.into_cache();

    type Change = (&'static str, fn(&mut App));
    let changes: [Change; 18] = [
        ("the find bar opens", |app| {
            room(app).find = Some(Find::default())
        }),
        ("and closes", |app| room(app).find = None),
        ("someone types", |app| room(app).typing = vec!["bob".into()]),
        ("they stop", |app| room(app).typing.clear()),
        ("an error below", |app| {
            room(app).error = Some("Not sent".into())
        }),
        ("it goes", |app| room(app).error = None),
        ("members open", |app| main(app).show_members = true),
        ("members close", |app| main(app).show_members = false),
        ("mentions offered", |app| {
            let member = person("@ana:example.org", "Ana", mtrx_core::Role::Member);
            room(app).members_loaded(Ok((vec![member], false)));
            room(app).set_composer(typed("Thanks @an"));
        }),
        ("one picked", |app| room(app).complete("@Ana")),
        ("offline", |app| main(app).connection = Connection::Offline),
        ("online", |app| main(app).connection = Connection::Online),
        ("a banner", |app| {
            main(app).report("Something failed".into(), None)
        }),
        ("it goes", |app| main(app).notice = None),
        ("settings open", |app| {
            main(app).modal = Some(Modal::Settings(settings::Panel::default()));
        }),
        ("settings close", |app| main(app).modal = None),
        ("a message is queued", |app| {
            let room_id = room(app).id.clone();
            main(app).scheduled = vec![ScheduledMessage {
                id: "q".into(),
                room_id,
                text: "Later".into(),
                send_at: SystemTime::now() + Duration::from_secs(3600),
                in_reply_to: None,
                overdue: false,
                files: Vec::new(),
            }];
        }),
        ("and sent", |app| main(app).scheduled.clear()),
    ];
    for (what, change) in changes {
        change(app);
        let mut ui = UserInterface::build(app.view(), size, cache, &mut renderer);
        let mut focused = ComposerFocused(None);
        ui.operate(&renderer, &mut focused);
        cache = ui.into_cache();
        // Panels over the window hide it; it is focused again once they close.
        if what == "settings open" {
            continue;
        }
        assert_eq!(
            focused.0,
            Some(true),
            "the composer lost its focus when {what}"
        );
    }
}

/// Whether the composer is focused, once operated.
struct ComposerFocused(Option<bool>);

impl iced::advanced::widget::Operation for ComposerFocused {
    fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn iced::advanced::widget::Operation)) {
        operate(self);
    }

    fn focusable(
        &mut self,
        id: Option<&iced::advanced::widget::Id>,
        _bounds: iced::Rectangle,
        state: &mut dyn iced::advanced::widget::operation::Focusable,
    ) {
        if id == Some(&iced::advanced::widget::Id::new(room::COMPOSER)) {
            self.0 = Some(state.is_focused());
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs local Synapse (testserver/run.sh)"]
async fn send_later_bar() {
    let dir = TempDir::new().unwrap();
    let (_, alice) = user("later", &dir).await;
    let room = alice
        .create_room(NewRoom {
            name: Some("ice-sheets".into()),
            encrypted: true,
            ..Default::default()
        })
        .await
        .unwrap();
    let long_name = "Working group on ice-sheet model intercomparison and data";
    let other = alice
        .create_room(NewRoom {
            name: Some(long_name.into()),
            ..Default::default()
        })
        .await
        .unwrap();
    let list = alice.room_list().await.unwrap();
    let rooms = tokio::time::timeout(TIMEOUT, async {
        let mut updates = std::pin::pin!(list.updates());
        while let Some(state) = updates.next().await {
            let named = |id: &OwnedRoomId, name: &str| {
                state.rooms.iter().any(|r| &r.id == id && r.name == name)
            };
            if state.loaded && named(&room, "ice-sheets") && named(&other, long_name) {
                return state;
            }
        }
        unreachable!()
    })
    .await
    .unwrap();
    let timeline = Arc::new(alice.timeline(&room).await.unwrap());
    tokio::time::timeout(
        TIMEOUT,
        timeline
            .send_markdown("Reviews are due **Friday** ❤ 👍🏽 ☀️", None)
            .await
            .unwrap()
            .sent(),
    )
    .await
    .unwrap()
    .unwrap();
    while !timeline.paginate_back(50).await.unwrap() {}

    let now = SystemTime::now();
    let scheduled = |id: &str, text: &str, send_at, overdue| ScheduledMessage {
        id: id.into(),
        room_id: room.clone(),
        text: text.into(),
        send_at,
        in_reply_to: None,
        overdue,
        files: Vec::new(),
    };
    let mut main = Main::new(Arc::new(alice));
    main.show_rooms(rooms);
    main.verified = Some(true);
    main.now = now;
    main.scheduled = vec![
        scheduled(
            "a",
            "Reminder: send me your figures before the meeting.",
            now - Duration::from_secs(3 * 3600),
            true,
        ),
        ScheduledMessage {
            files: vec![ScheduledFile {
                filename: "draft-v3.pdf".into(),
                mimetype: "application/pdf".into(),
                size: 182_000,
            }],
            ..scheduled(
                "b",
                "Good morning! The draft is ready for review: $\\dot m$ is fixed.",
                now + Duration::from_secs(2 * 3600 + 600),
                false,
            )
        },
        ScheduledMessage {
            room_id: other.clone(),
            ..scheduled(
                "c",
                "Agenda for Thursday",
                now + Duration::from_secs(86400),
                false,
            )
        },
    ];
    let mut open = Room::new(
        main.session.room(&room).unwrap(),
        timeline.clone(),
        main.session.draft_keeper(&room),
    );
    show(&mut open, timeline.current());
    open.composer = text_editor::Content::with_text("Who can take the Monday slot? ❤");
    main.popup = Some(Popup::SendLater(send_later::SendLater {
        picker: send_later::Picker::new(now, mtrx_core::parse_send_time("tomorrow 8:00", now)),
        rescheduling: None,
    }));
    main.pane = Pane::Open(open);
    let mut app = app(Screen::Main(Box::new(main)), Mode::Light);
    snapshot(&app, "send-later");

    // Editing in place: a sent message, then a scheduled one, while the
    // composer keeps its draft.
    if let Screen::Main(main) = &mut app.screen
        && let Some(open) = main.pane.room_mut()
    {
        main.popup = None;
        let sent = (open.rows.iter().filter_map(Row::event))
            .find(|ev| ev.editable && ev.event_id.is_some())
            .map(|ev| ev.key.clone())
            .unwrap();
        open.editing = Some(actions::InlineEdit::new(
            actions::Editing::Sent(sent),
            "Reviews are due **Monday**.",
        ));
    }
    snapshot(&app, "edit-sent");
    if let Screen::Main(main) = &mut app.screen
        && let Some(open) = main.pane.room_mut()
    {
        open.editing = Some(actions::InlineEdit::new(
            actions::Editing::Scheduled("b".into()),
            "Good morning! The draft is ready.",
        ));
    }
    snapshot(&app, "edit-scheduled");
    if let Screen::Main(main) = &mut app.screen
        && let Some(open) = main.pane.room_mut()
    {
        open.editing = None;
        let message = (open.rows.iter().filter_map(Row::event))
            .find(|ev| ev.editable)
            .map(|ev| ev.id.clone())
            .unwrap();
        main.popup = Some(Popup::Reactions {
            row: message,
            at: actions::PickerAt::Toolbar,
            all: Some(emoji::Panel::default()),
        });
        main.frequent_emoji = vec!["🦀".into(), "🧊".into()];
    }
    snapshot(&app, "emoji");
    if let Screen::Main(main) = &mut app.screen {
        main.popup = Some(Popup::FieldEmoji(Field::Composer, emoji::Panel::default()));
    }
    snapshot(&app, "composer-emoji");
    // A click outside the panel closes it; one on an emoji inserts it.
    let messages = |at: iced::Point| {
        let mut ui = Simulator::with_size(fonts::settings(), Size::new(1200.0, 800.0), app.view());
        ui.point_at(at);
        ui.simulate(iced_test::simulator::click());
        ui.into_messages().collect::<Vec<_>>()
    };
    let outside = messages(iced::Point::new(700.0, 150.0));
    assert!(
        outside.iter().any(|m| {
            matches!(
                m,
                Message::Room(_, room::Message::ComposerEmoji(emoji::Message::Close))
            )
        }),
        "{outside:?}"
    );
    let on_crab = messages(iced::Point::new(782.0, 447.0));
    assert!(
        on_crab
            .iter()
            .any(|m| matches!(m, Message::Room(_, room::Message::InsertEmoji(e)) if e == "🦀")),
        "{on_crab:?}"
    );
    if let Screen::Main(main) = &mut app.screen
        && let Some(open) = main.pane.room_mut()
    {
        main.popup = None;
        let sent = (open.rows.iter().filter_map(Row::event))
            .find(|ev| ev.editable)
            .map(|ev| ev.key.clone())
            .unwrap();
        let edit =
            actions::InlineEdit::new(actions::Editing::Sent(sent), "Reviews are due Monday 🎉");
        let mut panel = emoji::Panel::default();
        panel.update(emoji::Message::Search("ice".into()));
        main.popup = Some(Popup::FieldEmoji(Field::Edit, panel));
        open.editing = Some(edit);
    }
    snapshot(&app, "edit-emoji");
    if let Screen::Main(main) = &mut app.screen
        && let Some(open) = main.pane.room_mut()
    {
        open.editing = None;
    }
    if let Screen::Main(main) = &mut app.screen
        && let Some(open) = main.pane.room_mut()
    {
        open.editing = None;
    }
    // Completing a mention: names starting with "an" first, then those
    // with it inside; the two people named Ana complete as their IDs.
    if let Screen::Main(main) = &mut app.screen
        && let Some(open) = main.pane.room_mut()
    {
        use mtrx_core::Role::{Admin, Member};
        open.members_loaded(Ok((
            vec![
                person("@anna:example.org", "Anna Lindqvist", Admin),
                person("@jan:example.org", "Jan Hansen", Member),
                person("@andreas:example.org", "Andreas Born", Member),
                person("@ana:example.org", "Ana", Member),
                person("@ana:other.org", "Ana", Member),
            ],
            true,
        )));
        open.set_composer(typed(
            "Thanks @Anna Lindqvist! Who can take the Monday slot? @an",
        ));
    }
    snapshot(&app, "composer-mention");
    let _ = app.update(Message::Shown(room::Message::Mention(
        mention::Message::Down,
    )));
    let _ = app.update(Message::Shown(room::Message::Mention(
        mention::Message::Choose,
    )));
    if let Screen::Main(main) = &mut app.screen
        && let Some(open) = main.pane.room_mut()
    {
        assert_eq!(
            open.composer.text(),
            "Thanks @Anna Lindqvist! Who can take the Monday slot? @Andreas Born "
        );
        assert_eq!(open.completion, None);
        open.set_composer(typed("Who can take the Monday slot? ❤"));
    }
    let mut panel = settings::Panel::default();
    panel.room = Some(settings::Access::new(
        room.clone(),
        "ice-sheets".into(),
        false,
    ));
    panel.update(
        &mut app.settings,
        settings::Message::AccessLoaded(room.clone(), Some(Permissions::Moderators), Some(true)),
    );
    if let Screen::Main(main) = &mut app.screen {
        main.modal = Some(Modal::Settings(panel));
    }
    // A newer version, also the dot on the gear behind the panel.
    app.updater.available = Some(update::Update {
        version: semver::Version::new(0, 4, 0),
        page: "https://github.com/mathesislab/Mtrx/releases/tag/v0.4.0".into(),
        file: None,
        install: update::Install::Source,
    });
    snapshot(&app, "settings");
    // Downloading it: how much has come, below.
    app.updater.installing = Some(update::Step::Downloading {
        done: 34,
        total: Some(100),
    });
    snapshot(&app, "settings-downloading");
}

/// Timings for a big encrypted room opened on a new, recovered device:
/// `cargo test --release -p mtrx-app big_room -- --ignored --nocapture`
#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs local Synapse (testserver/run.sh); slow"]
async fn big_room_benchmark() {
    const MESSAGES: usize = 1500;
    let dir = TempDir::new().unwrap();
    let (name, first) = user("bench", &dir).await;
    let room = first
        .create_room(NewRoom {
            name: Some("big".into()),
            encrypted: true,
            ..Default::default()
        })
        .await
        .unwrap();
    let timeline = first.timeline(&room).await.unwrap();
    let start = std::time::Instant::now();
    let mut last = None;
    for i in 0..MESSAGES {
        let text = if i % 10 == 0 {
            format!("message {i} with math $x_{{{i}}} = \\frac{{a}}{{b}}$")
        } else {
            format!("message {i}: some **text** to read")
        };
        last = Some(timeline.send_markdown(&text, None).await.unwrap());
    }
    tokio::time::timeout(Duration::from_secs(300), last.unwrap().sent())
        .await
        .unwrap()
        .unwrap();
    println!("sent {MESSAGES} messages in {:.1?}", start.elapsed());
    let key = first.enable_recovery().await.unwrap();

    // New device, recovered: like a fresh install.
    let dir2 = TempDir::new().unwrap();
    let profile = Profile::new("second", dir2.path(), SecretStore::File).unwrap();
    let second = Session::login(profile, &homeserver(), &name, PASSWORD)
        .await
        .unwrap();
    second.start_sync().await;
    (second.room_list().await.unwrap())
        .wait_for(|s| s.loaded)
        .await;
    second.recover(&key, |_| {}).await.unwrap();

    let start = std::time::Instant::now();
    let timeline = Arc::new(second.timeline(&room).await.unwrap());
    let mut updates = std::pin::pin!(timeline.updates());
    let paginate = {
        let timeline = timeline.clone();
        tokio::spawn(async move { while !timeline.paginate_back(50).await.unwrap() {} })
    };
    let mut count = 0;
    let decrypted = |entries: &[TimelineEntry]| {
        let messages = entries
            .iter()
            .filter(|e| matches!(e, TimelineEntry::Event(ev) if matches!(ev.content, EventContent::Message(_))))
            .count();
        let utd = entries
            .iter()
            .filter(|e| matches!(e, TimelineEntry::Event(ev) if ev.content == EventContent::UnableToDecrypt))
            .count();
        (messages, utd)
    };
    let mut paginated_at = None;
    while let Some(entries) = tokio::time::timeout(Duration::from_secs(120), updates.next())
        .await
        .unwrap()
    {
        count += 1;
        if paginate.is_finished() && paginated_at.is_none() {
            paginated_at = Some(start.elapsed());
        }
        let (messages, utd) = decrypted(&entries);
        if messages == MESSAGES && utd == 0 {
            break;
        }
    }
    println!(
        "new device: history loaded after {:.1?}, all decrypted after {:.1?} ({count} timeline updates)",
        paginated_at.unwrap_or_default(),
        start.elapsed()
    );

    let start = std::time::Instant::now();
    for _ in 0..100 {
        std::hint::black_box(timeline.current());
    }
    println!(
        "timeline snapshot ({} entries): {:.2?} each",
        timeline.current().len(),
        start.elapsed() / 100
    );

    // View cost with every entry in the room view.
    let session = Arc::new(second);
    let mut main = Main::new(session);
    let mut open = Room::new(
        main.session.room(&room).unwrap(),
        timeline.clone(),
        main.session.draft_keeper(&room),
    );
    show(&mut open, timeline.current());
    main.pane = Pane::Open(open);
    let app = app(Screen::Main(Box::new(main)), Mode::Light);
    let start = std::time::Instant::now();
    for _ in 0..5 {
        let mut ui = Simulator::with_size(fonts::settings(), Size::new(1200.0, 800.0), app.view());
        std::hint::black_box(ui.snapshot(&app.theme()).unwrap());
    }
    println!(
        "view + layout + draw: {:.1?} per frame",
        start.elapsed() / 5
    );
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs local Synapse (testserver/run.sh)"]
async fn room_management_screens() {
    let dir = TempDir::new().unwrap();
    let (_, alice) = user("manage", &dir).await;
    let (_, bob) = user("member", &dir).await;
    let space = alice
        .create_room(NewRoom {
            name: Some("Climate".into()),
            space: true,
            ..Default::default()
        })
        .await
        .unwrap();
    let room = alice
        .create_room(NewRoom {
            name: Some("ice-sheets".into()),
            parent: Some(space.clone()),
            invite: vec![bob.user_id()],
            ..Default::default()
        })
        .await
        .unwrap();
    // Switched to with the member list open.
    let other = alice
        .create_room(NewRoom {
            name: Some("firn".into()),
            parent: Some(space.clone()),
            ..Default::default()
        })
        .await
        .unwrap();
    let list = alice.room_list().await.unwrap();
    let rooms = tokio::time::timeout(TIMEOUT, async {
        let mut updates = std::pin::pin!(list.updates());
        while let Some(state) = updates.next().await {
            if state.loaded
                && state
                    .rooms
                    .iter()
                    .any(|r| r.id == room && r.name == "ice-sheets")
                && state.graph.parents(&room).contains(&space)
                && state
                    .rooms
                    .iter()
                    .any(|r| r.id == other && r.name == "firn")
            {
                return state;
            }
        }
        unreachable!()
    })
    .await
    .unwrap();
    let timeline = Arc::new(alice.timeline(&room).await.unwrap());
    let session = Arc::new(alice);
    let members = session.room(&room).unwrap().members().await.unwrap();

    let main_with = |configure: &dyn Fn(&mut Main)| {
        let mut main = Main::new(session.clone());
        main.show_rooms(rooms.clone());
        main.space = Some(space.clone());
        main.verified = Some(true);
        let mut open = Room::new(
            main.session.room(&room).unwrap(),
            timeline.clone(),
            main.session.draft_keeper(&room),
        );
        show(&mut open, timeline.current());
        main.pane = Pane::Open(open);
        configure(&mut main);
        app(Screen::Main(Box::new(main)), Mode::Light)
    };

    snapshot(
        &main_with(&|main| {
            main.popup = Some(Popup::AddMenu);
        }),
        "menus",
    );
    // The status pill: looking for a message gone to, and a problem.
    snapshot(
        &main_with(&|main| {
            let room = main.pane.room_mut().unwrap();
            room.seek = Some(Task::<Message>::none().abortable().1);
        }),
        "status-seeking",
    );
    snapshot(
        &main_with(&|main| {
            main.pane.room_mut().unwrap().error =
                Some("The original message was not found in the room's history".into());
        }),
        "status-problem",
    );
    snapshot(
        &main_with(&|main| main.connection = Connection::Offline),
        "offline",
    );
    snapshot(
        &main_with(&|main| main.connection = Connection::SignedOut),
        "signed-out",
    );
    snapshot(
        &main_with(&|main| {
            main.out_of_view = sidebar::OutOfView {
                above: Some((1, 0.0)),
                below: Some((3, 400.0)),
            }
        }),
        "unread-arrows",
    );
    snapshot(
        &main_with(&|main| {
            let mut dialog = Dialog::new(dialog::Kind::NewRoom { space: None });
            dialog.update(dialog::Message::Name("model runs".into()));
            dialog.update(dialog::Message::OwnersOffered(true));
            let nils = OwnedUserId::try_from("@nils:example.org").unwrap();
            dialog.update(dialog::Message::Pick(mtrx_core::User {
                id: nils.clone(),
                name: Some("Nils".into()),
                avatar: None,
            }));
            dialog.update(dialog::Message::CoOwner(nils, true));
            main.modal = Some(Modal::Dialog(dialog));
        }),
        "new-room-owners",
    );
    snapshot(
        &main_with(&|main| {
            let mut dialog = Dialog::new(dialog::Kind::NewRoom {
                space: Some((space.clone(), "Climate".into())),
            });
            dialog.update(dialog::Message::Name("model runs".into()));
            dialog.update(dialog::Message::Submit);
            dialog.tick(Instant::now() + Duration::from_secs(20));
            main.modal = Some(Modal::Dialog(dialog));
        }),
        "dialog-slow",
    );
    snapshot(
        &main_with(&|main| {
            main.pane = Pane::Invite {
                id: room.clone(),
                answering: false,
            };
            main.notice = Some(Notice {
                text: "The room “model runs” is ready.".into(),
                room: Some(room.clone()),
                error: false,
            });
        }),
        "invite",
    );
    snapshot(
        &main_with(&|main| {
            main.filter = "ice".into();
            main.show_members = true;
            let room = main.pane.room_mut().unwrap();
            room.members_loaded(Ok((members.clone(), true)));
        }),
        "members-search",
    );
    // The member list stays open, at its width, while another room opens
    // and its members load.
    let mut app = main_with(&|main| {
        main.show_members = true;
        let room = main.pane.room_mut().unwrap();
        room.members_loaded(Ok((members.clone(), true)));
    });
    let _ = app.update(Message::OpenRoom(other.clone()));
    snapshot(&app, "members-opening");
    let opened = Shared(Arc::new(session.timeline(&other).await.unwrap()));
    let _ = app.update(Message::RoomOpened(other.clone(), Ok(opened)));
    let Screen::Main(main) = &app.screen else {
        unreachable!()
    };
    let open = main.pane.room().unwrap();
    assert_eq!(open.id, other);
    assert!(main.show_members);
    assert!(open.loading_members && open.member_list.is_none());
    snapshot(&app, "members-switched");

    let bob_info = session.user_info(&bob.user_id()).await.unwrap();
    let contacts = session.contacts().await.unwrap();
    snapshot(
        &main_with(&|main| {
            let mut card = info::Card::new(
                info::Target::User(bob_info.user.clone()),
                &[],
                overview::Tab::Rooms,
            );
            card.update(info::Message::UserLoaded(
                bob.user_id(),
                Ok(bob_info.clone()),
            ));
            card.update(info::Message::Copy(bob.user_id().to_string()));
            main.card = Some(card);
        }),
        "info-user",
    );
    let settings = session.room(&room).unwrap().settings().await.unwrap();
    snapshot(
        &main_with(&|main| {
            let mut card = info::Card::new(
                info::Target::Room(room.clone()),
                &main.lists.rooms,
                overview::Tab::Rooms,
            );
            card.update(info::Message::SettingsLoaded(
                room.clone(),
                Ok(settings.clone()),
            ));
            // A link being added.
            card.update(info::Message::Topic(topic::Edit::Add(false)));
            main.card = Some(card);
        }),
        "room-settings",
    );
    snapshot(
        &main_with(&|main| {
            let mut dialog = Dialog::new(dialog::Kind::NewRoom {
                space: Some((space.clone(), "Climate".into())),
            });
            dialog.prefer_spaces(vec![space.clone()]);
            dialog.update(dialog::Message::Contacts(contacts.clone()));
            main.modal = Some(Modal::Dialog(dialog));
        }),
        "new-room-people",
    );
    snapshot(
        &main_with(&|main| {
            let mut dialog = Dialog::new(dialog::Kind::DirectChat);
            dialog.offer_self(main.own_user.clone());
            dialog.update(dialog::Message::Contacts(contacts.clone()));
            main.modal = Some(Modal::Dialog(dialog));
        }),
        "direct-chat-people",
    );

    // A space's overview and the join dialog, with a room not in the app's
    // room list (made since), open to everyone.
    let glaciers = session
        .create_room(NewRoom {
            name: Some("glaciers".into()),
            topic: Some("Mass balance of mountain glaciers".into()),
            parent: Some(space.clone()),
            ..Default::default()
        })
        .await
        .unwrap();
    let handle = session.room(&glaciers).unwrap();
    tokio::time::timeout(TIMEOUT, async {
        while handle
            .set_join_rule(mtrx_core::Access::Public)
            .await
            .is_err()
        {
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .unwrap();
    let children = session.space_children(&space).await.unwrap();
    let space_settings = session.room(&space).unwrap().settings().await.unwrap();
    snapshot(
        &main_with(&|main| {
            let mut card = info::Card::new(
                info::Target::Room(space.clone()),
                &main.lists.rooms,
                overview::Tab::Rooms,
            );
            card.update(info::Message::SettingsLoaded(
                space.clone(),
                Ok(space_settings.clone()),
            ));
            main.space_rooms.loaded(space.clone(), Ok(children.clone()));
            main.card = Some(card);
        }),
        "space-overview",
    );
    // The space's topic below its name, open over the room list.
    for (open, name) in [(false, "space-topic"), (true, "space-topic-open")] {
        snapshot(
            &main_with(&|main| {
                let mut state = rooms.clone();
                if let Some(summary) = state.rooms.iter_mut().find(|r| r.id == space) {
                    summary.topic = Some(
                        "Paleoclimate modelling and analysis, ice group\n\
                         [call Weekly] https://meet.google.com/abc-defg-hij\n\
                         [link Shared drive] https://drive.google.com/drive/folders/xyz"
                            .into(),
                    );
                }
                main.show_rooms(state);
                main.space_topic.can_edit = Some(true);
                main.space_topic.open = open;
            }),
            name,
        );
    }
    // Listed before, asked again: the list shows while it updates.
    snapshot(
        &main_with(&|main| {
            let card = info::Card::new(
                info::Target::Room(space.clone()),
                &main.lists.rooms,
                overview::Tab::Rooms,
            );
            main.space_rooms.loaded(space.clone(), Ok(children.clone()));
            main.space_rooms.start(&space);
            main.card = Some(card);
        }),
        "space-overview-updating",
    );
    snapshot(
        &main_with(&|main| {
            let (mut join, action) = join::Join::new(
                main.session.user_id().server_name().to_owned(),
                &["matrix.org".into()],
            );
            let join::Action::Search(search) = action else {
                panic!("no search");
            };
            join.update(join::Message::Found(
                search,
                Ok(mtrx_core::DirectoryPage {
                    rooms: children.clone(),
                    next: None,
                }),
            ));
            main.modal = Some(Modal::Join(join));
        }),
        "join",
    );
}

/// Mentions of the viewer and of someone else in a received message.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs local Synapse (testserver/run.sh)"]
async fn mention_colours() {
    let dir = TempDir::new().unwrap();
    let (_, alice) = user("pill-alice", &dir).await;
    let (_, bob) = user("pill-bob", &dir).await;
    let (_, carol) = user("pill-carol", &dir).await;
    for (session, name) in [
        (&alice, "Alice Nordin"),
        (&bob, "Bob Weitzel"),
        (&carol, "Carol Berg"),
    ] {
        session.set_display_name(name).await.unwrap();
    }
    let room = alice
        .create_room(NewRoom {
            name: Some("ice-sheets".into()),
            invite: vec![bob.user_id(), carol.user_id()],
            ..Default::default()
        })
        .await
        .unwrap();
    accept_invite(&bob, &room).await;
    let bob_timeline = bob.timeline(&room).await.unwrap();
    let text = "Thanks @Alice Nordin! @Carol Berg, can you check the sign in eq. 2?";
    send(&bob_timeline, text).await;

    let list = alice.room_list().await.unwrap();
    let rooms = tokio::time::timeout(TIMEOUT, async {
        let mut updates = std::pin::pin!(list.updates());
        while let Some(state) = updates.next().await {
            if state.loaded
                && state
                    .rooms
                    .iter()
                    .any(|r| r.id == room && r.name == "ice-sheets")
            {
                return state;
            }
        }
        unreachable!()
    })
    .await
    .unwrap();
    let timeline = Arc::new(alice.timeline(&room).await.unwrap());
    while !timeline.paginate_back(50).await.unwrap() {}
    let entries = tokio::time::timeout(TIMEOUT, async {
        let mut updates = std::pin::pin!(timeline.updates());
        while let Some(entries) = updates.next().await {
            let mentioned = entries.iter().any(|e| {
                matches!(e, TimelineEntry::Event(ev)
                    if matches!(&ev.content, EventContent::Message(m) if m.body == text))
            });
            if mentioned {
                return entries;
            }
        }
        unreachable!()
    })
    .await
    .unwrap();
    let message = entries
        .iter()
        .find_map(|e| match e {
            TimelineEntry::Event(ev) => match &ev.content {
                EventContent::Message(m) if m.body == text => Some(m),
                _ => None,
            },
            _ => None,
        })
        .unwrap();
    assert_eq!(
        message.document.to_plain_text(),
        "Thanks Alice Nordin! Carol Berg, can you check the sign in eq. 2?"
    );

    let mut main = Main::new(Arc::new(alice));
    main.show_rooms(rooms);
    main.verified = Some(true);
    let mut open = Room::new(
        main.session.room(&room).unwrap(),
        timeline.clone(),
        main.session.draft_keeper(&room),
    );
    show(&mut open, entries);
    main.pane = Pane::Open(open);
    let app = app(Screen::Main(Box::new(main)), Mode::Light);
    snapshot(&app, "mention-colours");
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs local Synapse (testserver/run.sh)"]
async fn media_messages() {
    let dir = TempDir::new().unwrap();
    let (_, alice) = user("media", &dir).await;
    let room = alice
        .create_room(NewRoom {
            name: Some("ice-sheets".into()),
            encrypted: true,
            ..Default::default()
        })
        .await
        .unwrap();
    let list = alice.room_list().await.unwrap();
    let rooms = tokio::time::timeout(TIMEOUT, async {
        let mut updates = std::pin::pin!(list.updates());
        while let Some(state) = updates.next().await {
            if state.loaded
                && state
                    .rooms
                    .iter()
                    .any(|r| r.id == room && r.name == "ice-sheets")
            {
                return state;
            }
        }
        unreachable!()
    })
    .await
    .unwrap();
    let timeline = Arc::new(alice.timeline(&room).await.unwrap());
    let map = gradient(640, 400, [16, 52, 120], [110, 205, 225]);
    let files = vec![
        Attachment {
            filename: "melt-rate-map.png".into(),
            mimetype: "image/png".into(),
            bytes: map,
        },
        Attachment {
            filename: "ice-flux-draft.pdf".into(),
            mimetype: "application/pdf".into(),
            bytes: crate::pdf::sample(),
        },
    ];
    timeline
        .send(
            "Melt-rate map from last night's run, and the draft:",
            files,
            None,
        )
        .await
        .unwrap();
    while !timeline.paginate_back(50).await.unwrap() {}
    // Each file as the server has it, and no local echo pointing into the
    // send queue's store: once uploaded, those files are no longer there, and
    // such an echo can still show beside the server's copy.
    let mut seen = Vec::new();
    let entries = tokio::time::timeout(TIMEOUT, async {
        let mut updates = std::pin::pin!(timeline.updates());
        while let Some(entries) = updates.next().await {
            let files: Vec<_> = entries
                .iter()
                .filter_map(|e| match e {
                    TimelineEntry::Event(ev) => match &ev.content {
                        EventContent::Message(m) => m.media.as_ref().map(|media| (ev, media)),
                        _ => None,
                    },
                    _ => None,
                })
                .collect();
            let on_server = files.iter().all(|(ev, media)| {
                ev.send_state == SendState::Sent
                    && media.source.uri().server_name().map(|s| s.as_str())
                        != Ok("send-queue.localhost")
            });
            let shown = |name: &str| files.iter().any(|(_, media)| media.filename == name);
            if on_server && shown("melt-rate-map.png") && shown("ice-flux-draft.pdf") {
                return entries;
            }
            seen = files
                .iter()
                .map(|(ev, media)| {
                    format!(
                        "{} ({}, {:?}, {:?}, {})",
                        media.filename,
                        ev.id,
                        ev.send_state,
                        ev.event_id,
                        media.source.uri()
                    )
                })
                .collect();
        }
        unreachable!()
    })
    .await
    .unwrap_or_else(|_| panic!("the files did not show as the server has them: {seen:#?}"));
    let file = |extension: &str| {
        entries
            .iter()
            .find_map(|e| match e {
                TimelineEntry::Event(ev) => match &ev.content {
                    EventContent::Message(m) => {
                        m.media.clone().filter(|m| m.filename.ends_with(extension))
                    }
                    _ => None,
                },
                _ => None,
            })
            .unwrap()
    };
    let (image, pdf) = (file(".png"), file(".pdf"));

    let session = Arc::new(alice);
    let main_with = async |modal: Option<Modal>| {
        let mut main = Main::new(session.clone());
        main.show_rooms(rooms.clone());
        main.verified = Some(true);
        let mut open = Room::new(
            main.session.room(&room).unwrap(),
            timeline.clone(),
            main.session.draft_keeper(&room),
        );
        show(&mut open, entries.clone());
        let images: Vec<Media> = open.previewed().cloned().collect();
        for media in &images {
            main.previews.load_now(&session, media).await;
        }
        main.pane = Pane::Open(open);
        main.modal = modal;
        Box::new(main)
    };
    for (mode, name) in [(Mode::Light, "media-light"), (Mode::Dark, "media-dark")] {
        snapshot(&app(Screen::Main(main_with(None).await), mode), name);
    }
    // Files attached to a message being written sit in the composer.
    let files = dir.path().join("attach");
    std::fs::create_dir(&files).unwrap();
    let paths = vec![files.join("terminus.png"), files.join("notes.pdf")];
    std::fs::write(
        &paths[0],
        gradient(300, 200, [200, 90, 40], [250, 220, 120]),
    )
    .unwrap();
    std::fs::write(&paths[1], crate::pdf::sample()).unwrap();
    let mut main = main_with(None).await;
    let open = main.pane.room_mut().unwrap();
    open.attachments = crate::media::read_attached(paths)
        .await
        .into_iter()
        .map(Result::unwrap)
        .collect();
    open.composer = text_editor::Content::with_text("The calving front this morning:");
    snapshot(
        &app(Screen::Main(main), Mode::Light),
        "composer-attachments",
    );
    let mut viewer = Viewer::new(image);
    viewer.load_now(&session).await;
    snapshot(
        &app(
            Screen::Main(main_with(Some(Modal::Image(viewer))).await),
            Mode::Light,
        ),
        "image-viewer",
    );
    let mut viewer = pdf_viewer::Viewer::new(pdf.clone(), Size::new(1200.0, 800.0), 2.0);
    viewer.open_now(&session).await;
    snapshot(
        &app(
            Screen::Main(main_with(Some(Modal::Pdf(viewer))).await),
            Mode::Light,
        ),
        "pdf-viewer",
    );
    let mut viewer = pdf_viewer::Viewer::new(pdf, Size::new(1200.0, 800.0), 2.0);
    viewer.set_zoom(0.5);
    viewer.open_now(&session).await;
    snapshot(
        &app(
            Screen::Main(main_with(Some(Modal::Pdf(viewer))).await),
            Mode::Dark,
        ),
        "pdf-viewer-zoomed-out",
    );
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs local Synapse (testserver/run.sh)"]
async fn room_categories() {
    let dir = TempDir::new().unwrap();
    let (_, alice) = user("arrange", &dir).await;
    let alice = Arc::new(alice);
    let (_, bob) = user("colleague", &dir).await;
    let mut ids = HashMap::new();
    for name in ["ice-sheets", "ocean", "admin", "archive", "seminar"] {
        let id = alice
            .create_room(NewRoom {
                name: Some(name.into()),
                ..Default::default()
            })
            .await
            .unwrap();
        ids.insert(name, id);
    }
    alice.direct_chat(&bob.user_id()).await.unwrap();
    (alice.room(&ids["ocean"]).unwrap())
        .set_tag(RoomTag::Favourite)
        .await
        .unwrap();
    (alice.room(&ids["archive"]).unwrap())
        .set_tag(RoomTag::LowPriority)
        .await
        .unwrap();
    let mut projects = String::new();
    alice
        .change_room_categories(|c| {
            let home = c.layout_mut(None);
            projects = home.add_category("Projects");
            home.place(&ids["ice-sheets"], Some(&projects));
            home.place(&ids["seminar"], Some(&projects));
            home.add_category("Teaching");
        })
        .await
        .unwrap();
    let list = alice.room_list().await.unwrap();
    let rooms = tokio::time::timeout(TIMEOUT, async {
        let mut updates = std::pin::pin!(list.updates());
        while let Some(state) = updates.next().await {
            let tag = |name| {
                state
                    .rooms
                    .iter()
                    .find(|r| r.id == ids[name])
                    .map(|r| r.tag)
            };
            if state.loaded
                && state.rooms.len() == 6
                && state.rooms.iter().any(|r| r.kind == RoomKind::Direct)
                && tag("ocean") == Some(RoomTag::Favourite)
                && tag("archive") == Some(RoomTag::LowPriority)
            {
                return state;
            }
        }
        unreachable!()
    })
    .await
    .unwrap();
    let session = alice;

    let main_with = |configure: &dyn Fn(&mut Main)| {
        let mut main = Main::new(session.clone());
        main.show_rooms(rooms.clone());
        main.verified = Some(true);
        configure(&mut main);
        app(Screen::Main(Box::new(main)), Mode::Light)
    };
    let send = |main: &mut Main, message| {
        let _ = main.arrange.update(
            message,
            arrange::Context {
                session: &session,
                space: None,
                rooms: &main.lists.rooms,
                grouping: mtrx_core::Grouping::default(),
                popup: &mut main.popup,
            },
        );
    };

    snapshot(
        &main_with(&|main| {
            send(main, arrange::Message::Row(ids["ice-sheets"].clone(), true));
            send(
                main,
                arrange::Message::RoomMenu(Some(ids["ice-sheets"].clone())),
            );
        }),
        "categories",
    );
    snapshot(
        &main_with(&|main| {
            send(main, arrange::Message::Row(ids["admin"].clone(), true));
            send(main, arrange::Message::Pressed);
            send(
                main,
                arrange::Message::Moved(iced::Point::new(120.0, 330.0)),
            );
            send(
                main,
                arrange::Message::Moved(iced::Point::new(130.0, 250.0)),
            );
            send(
                main,
                arrange::Message::Target(Section::Category(projects.clone()), true),
            );
        }),
        "categories-drag",
    );

    // A heading dragged by its grip, to the place of Favourites.
    let teaching = Section::Category(
        session.room_categories().layout(None).categories[1]
            .id
            .clone(),
    );
    let drag_heading = [
        arrange::Message::SectionHovered(teaching.clone(), true),
        arrange::Message::Grip(teaching.clone(), true),
        arrange::Message::Pressed,
        arrange::Message::Moved(iced::Point::new(400.0, 410.0)),
        arrange::Message::Moved(iced::Point::new(400.0, 300.0)),
        arrange::Message::Target(Section::Favourites, true),
    ];
    snapshot(
        &main_with(&|main| {
            for message in drag_heading.clone() {
                send(main, message);
            }
        }),
        "heading-drag",
    );
    let mut main = Main::new(session.clone());
    main.show_rooms(rooms.clone());
    for message in drag_heading.into_iter().chain([arrange::Message::Released]) {
        send(&mut main, message);
    }
    let order = main.arrange.categories.layout(None).order();
    assert_eq!(order[..2], [Section::People, teaching]);

    // Dropping moves the room into the category; the menu favourites one.
    let mut main = Main::new(session.clone());
    main.show_rooms(rooms.clone());
    for message in [
        arrange::Message::Row(ids["admin"].clone(), true),
        arrange::Message::Pressed,
        arrange::Message::Moved(iced::Point::new(120.0, 330.0)),
        arrange::Message::Moved(iced::Point::new(130.0, 250.0)),
        arrange::Message::Target(Section::Category(projects.clone()), true),
        arrange::Message::Released,
        arrange::Message::MoveTo(ids["seminar"].clone(), Section::Favourites),
    ] {
        send(&mut main, message);
    }
    let home = main.arrange.categories.layout(None);
    let in_projects = |name| home.category_of(&ids[name]).map(|c| c.id.clone());
    assert_eq!(in_projects("admin"), Some(projects.clone()));
    assert_eq!(in_projects("seminar"), None);
    assert!(main.arrange.drag.is_none());
}

/// A space's menu of which sections are expanded: open, and after hiding
/// all (the unread room stays).
#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs local Synapse (testserver/run.sh)"]
async fn show_menu() {
    let dir = TempDir::new().unwrap();
    let (_, alice) = user("shower", &dir).await;
    let alice = Arc::new(alice);
    let (_, bob) = user("colleague", &dir).await;
    let space = alice
        .create_room(NewRoom {
            name: Some("Glaciology Lab".into()),
            space: true,
            ..Default::default()
        })
        .await
        .unwrap();
    let mut ids = HashMap::new();
    for name in ["ice-sheets", "seminars", "bm-ensemble", "coffee"] {
        let id = alice
            .create_room(NewRoom {
                name: Some(name.into()),
                ..Default::default()
            })
            .await
            .unwrap();
        alice.room(&space).unwrap().add_child(&id).await.unwrap();
        ids.insert(name, id);
    }
    let dm = alice.direct_chat(&bob.user_id()).await.unwrap();
    accept_invite(&bob, &dm).await;
    (alice.room(&ids["ice-sheets"]).unwrap())
        .set_tag(RoomTag::Favourite)
        .await
        .unwrap();
    (alice.room(&ids["coffee"]).unwrap())
        .set_tag(RoomTag::LowPriority)
        .await
        .unwrap();
    alice
        .change_room_categories(|c| {
            let layout = c.layout_mut(Some(&space));
            let melt = layout.add_category("Antarctic basal melt sensitivity experiments");
            layout.place(&ids["bm-ensemble"], Some(&melt));
            layout.add_category("Papers");
            layout.set_collapsed(&Section::LowPriority, true);
        })
        .await
        .unwrap();
    let list = alice.room_list().await.unwrap();
    let mut rooms = tokio::time::timeout(TIMEOUT, async {
        let mut updates = std::pin::pin!(list.updates());
        while let Some(state) = updates.next().await {
            let tag = |name| {
                state
                    .rooms
                    .iter()
                    .find(|r| r.id == ids[name])
                    .map(|r| r.tag)
            };
            if state.loaded
                && state.rooms.len() == 6
                && state.rooms.iter().any(|r| r.direct_user.is_some())
                && tag("ice-sheets") == Some(RoomTag::Favourite)
                && tag("coffee") == Some(RoomTag::LowPriority)
            {
                return state;
            }
        }
        unreachable!()
    })
    .await
    .unwrap();
    if let Some(seminars) = rooms.rooms.iter_mut().find(|r| r.id == ids["seminars"]) {
        seminars.unread_messages = 2;
    }
    let session = alice;

    let mut main = Main::new(session.clone());
    // The space's members, as if loaded, for the direct chat with Bob.
    let members = HashMap::from([(space.clone(), HashSet::from([bob.user_id()]))]);
    main.lists = Arc::new(RoomLists::new(
        rooms,
        main.arrange.categories.clone(),
        members,
        &main.own_user,
        BTreeSet::new(),
    ));
    main.verified = Some(true);
    main.space = Some(space.clone());
    let send = |main: &mut Main, message| {
        let _ = main.arrange.update(
            message,
            arrange::Context {
                session: &session,
                space: Some(&space),
                rooms: &main.lists.rooms,
                grouping: mtrx_core::Grouping::default(),
                popup: &mut main.popup,
            },
        );
    };
    send(&mut main, arrange::Message::ShowMenu(true));
    // Rooms dragged by its grip to the place of Papers, above it.
    let papers = main.arrange.categories.layout(Some(&space)).categories[1]
        .id
        .clone();
    for message in [
        arrange::Message::Grip(Section::Rooms, true),
        arrange::Message::Pressed,
        arrange::Message::Moved(iced::Point::new(200.0, 330.0)),
        arrange::Message::Moved(iced::Point::new(200.0, 300.0)),
        arrange::Message::Target(Section::Category(papers.clone()), true),
    ] {
        send(&mut main, message);
    }
    let mut app = app(Screen::Main(Box::new(main)), Mode::Light);
    snapshot(&app, "section-drag");

    let Screen::Main(main) = &mut app.screen else {
        unreachable!()
    };
    send(main, arrange::Message::Released);
    assert!(matches!(main.popup, Some(Popup::ShowMenu)));
    let order = main.arrange.categories.layout(Some(&space)).order();
    assert_eq!(order[3..5], [Section::Rooms, Section::Category(papers)]);
    snapshot(&app, "show-menu");

    let Screen::Main(main) = &mut app.screen else {
        unreachable!()
    };
    let sections = main.arrange.categories.layout(Some(&space)).order();
    send(
        main,
        arrange::Message::Collapse(sections.iter().map(|s| (s.clone(), true)).collect()),
    );
    send(main, arrange::Message::ShowMenu(false));
    let layout = main.arrange.categories.layout(Some(&space));
    assert!(sections.iter().all(|s| layout.is_collapsed(s)));
    snapshot(&app, "show-menu-hidden");
}

/// Screenshots for the website (`site/images/`): a lab's room with math, a
/// reply, reactions and a call; then sending later, and copying a selection.
/// `cargo test -p mtrx-app website -- --ignored`, then copy the `site-*`
/// snapshots from `target/ui-snapshots/`.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs local Synapse (testserver/run.sh)"]
async fn website_screenshots() {
    let dir = TempDir::new().unwrap();
    let (_, ana) = user("ana", &dir).await;
    let (_, ben) = user("ben", &dir).await;
    let (_, chloe) = user("chloe", &dir).await;
    for (session, name) in [
        (&ana, "Ana Sousa"),
        (&ben, "Ben Okafor"),
        (&chloe, "Chloé Martin"),
    ] {
        session.set_display_name(name).await.unwrap();
    }
    ben.set_avatar("image/png", picture([40, 90, 200], [120, 220, 255]))
        .await
        .unwrap();
    chloe
        .set_avatar("image/png", picture([230, 120, 40], [250, 210, 90]))
        .await
        .unwrap();

    let ana = Arc::new(ana);
    let space = ana
        .create_room(NewRoom {
            name: Some("Glaciology Lab".into()),
            space: true,
            ..Default::default()
        })
        .await
        .unwrap();
    let mut rooms_in_space = Vec::new();
    for (name, topic) in [
        (
            "ice-sheets",
            "Basal melt and ice flux: drafts, figures and model runs",
        ),
        ("field-campaign", "Greenland 2027 logistics"),
        ("seminars", "Talks and reading group"),
    ] {
        let room = ana
            .create_room(NewRoom {
                name: Some(name.into()),
                topic: Some(topic.into()),
                encrypted: true,
                invite: vec![ben.user_id(), chloe.user_id()],
                ..Default::default()
            })
            .await
            .unwrap();
        ana.room(&space).unwrap().add_child(&room).await.unwrap();
        accept_invite(&ben, &room).await;
        accept_invite(&chloe, &room).await;
        rooms_in_space.push(room);
    }
    let room = rooms_in_space[0].clone();
    let dm = ana
        .create_room(NewRoom {
            invite: vec![chloe.user_id()],
            direct: true,
            encrypted: true,
            ..Default::default()
        })
        .await
        .unwrap();
    accept_invite(&chloe, &dm).await;
    ana.direct_chat(&ana.user_id()).await.unwrap();

    let ana_timeline = ana.timeline(&room).await.unwrap();
    let mut entries = std::pin::pin!(ana_timeline.updates());
    tokio::time::timeout(TIMEOUT, async {
        while let Some(entries) = entries.next().await {
            let joined = |user: &Session| {
                entries.iter().any(
                    |e| matches!(e, TimelineEntry::Event(ev) if ev.sender.id == user.user_id()),
                )
            };
            if joined(&ben) && joined(&chloe) {
                break;
            }
        }
    })
    .await
    .unwrap();
    let first = send(
        &ana_timeline,
        "Morning! The draft of the **ice flux** section is up.",
    )
    .await;
    send(
        &ana_timeline,
        r"The basal melt rate follows $\dot m = \frac{G + \tau_b u_b}{\rho_i L}$ where $G$ is the geothermal flux.",
    )
    .await;
    let ben_timeline = ben.timeline(&room).await.unwrap();
    send(
        &ben_timeline,
        "Looks good, but check the sign of $\\dot m$ in the mass balance 🙂",
    )
    .await;
    send_reply(
        &ben_timeline,
        "Which geothermal flux dataset is it?",
        Some(&first),
    )
    .await;
    let chloe_timeline = chloe.timeline(&room).await.unwrap();
    send(
        &chloe_timeline,
        "I can run the sensitivity tests tonight. Quick call first? https://meet.jit.si/glaciology-lab",
    )
    .await;
    send(
        &chloe.timeline(&dm).await.unwrap(),
        "Coffee before the seminar?",
    )
    .await;
    for timeline in [&ben_timeline, &chloe_timeline] {
        timeline
            .toggle_reaction(
                &key_of(
                    timeline,
                    "Morning! The draft of the **ice flux** section is up.",
                )
                .await,
                "🎉",
            )
            .await
            .unwrap();
    }
    ana_timeline
        .toggle_reaction(
            &key_of(&ana_timeline, "Which geothermal flux dataset is it?").await,
            "👍",
        )
        .await
        .unwrap();
    ben_timeline.mark_read().await.unwrap();
    chloe_timeline.mark_read().await.unwrap();

    let list = Arc::new(ana.room_list().await.unwrap());
    let rooms = tokio::time::timeout(TIMEOUT, async {
        let mut updates = std::pin::pin!(list.updates());
        while let Some(state) = updates.next().await {
            let in_space = rooms_in_space.iter().all(|id| {
                state.rooms.iter().any(|r| &r.id == id) && state.graph.parents(id).contains(&space)
            });
            let dm_avatar = state.rooms.iter().any(|r| r.id == dm && r.avatar.is_some());
            if state.loaded && in_space && dm_avatar {
                return state;
            }
        }
        unreachable!()
    })
    .await
    .unwrap();
    let timeline = Arc::new(ana.timeline(&room).await.unwrap());
    while !timeline.paginate_back(50).await.unwrap() {}
    let entries = tokio::time::timeout(TIMEOUT, async {
        let mut updates = std::pin::pin!(timeline.updates());
        while let Some(entries) = updates.next().await {
            let messages = || {
                entries.iter().filter_map(|e| match e {
                    TimelineEntry::Event(ev) => match &ev.content {
                        EventContent::Message(m) => Some((ev, m)),
                        _ => None,
                    },
                    _ => None,
                })
            };
            let named = messages().all(|(ev, _)| ev.sender.name.is_some());
            let reply = messages()
                .any(|(_, m)| m.in_reply_to.as_ref().is_some_and(|r| r.preview.is_some()));
            let reactions = messages().filter(|(_, m)| !m.reactions.is_empty()).count() == 2;
            let read = messages().any(|(ev, _)| ev.read_by.len() == 2);
            if messages().count() == 5 && named && reply && reactions && read {
                return entries;
            }
        }
        unreachable!()
    })
    .await
    .unwrap();
    let id_of = |start: &str| {
        entries
            .iter()
            .find_map(|e| match e {
                TimelineEntry::Event(ev) => match &ev.content {
                    EventContent::Message(m) if m.body.starts_with(start) => Some(ev.id.clone()),
                    _ => None,
                },
                _ => None,
            })
            .unwrap()
    };

    let now = SystemTime::now();
    // Tomorrow at nine.
    let tomorrow_nine: SystemTime = (chrono::Local::now().date_naive() + chrono::Days::new(1))
        .and_hms_opt(9, 0, 0)
        .unwrap()
        .and_local_timezone(chrono::Local)
        .unwrap()
        .into();
    let screen = |mode: Mode| {
        let mut main = Main::new(ana.clone());
        main.show_rooms(rooms.clone());
        main.space = Some(space.clone());
        main.verified = Some(true);
        main.now = now;
        main.scheduled = vec![ScheduledMessage {
            id: "a".into(),
            room_id: room.clone(),
            text: "Reminder: figures for Thursday's meeting, please!".into(),
            send_at: tomorrow_nine,
            in_reply_to: None,
            overdue: false,
            files: Vec::new(),
        }];
        let mut open = Room::new(
            main.session.room(&room).unwrap(),
            timeline.clone(),
            main.session.draft_keeper(&room),
        );
        show(&mut open, entries.clone());
        main.pane = Pane::Open(open);
        let mut app = app(Screen::Main(Box::new(main)), mode);
        app.settings.colored_names = mode == Mode::Dark;
        app
    };
    async fn finish(app: &mut App, ana: &Session) {
        let params = app.math_params();
        if let Screen::Main(main) = &mut app.screen {
            for document in main.pane.room().unwrap().math_documents() {
                app.math.render_now(document, &params);
            }
            for (mxc, size) in main.avatars_on_screen(&Default::default()) {
                main.avatars.load_now(ana, &mxc, size).await;
            }
        }
    }

    for (mode, name) in [
        (Mode::Light, "site-main-light"),
        (Mode::Dark, "site-main-dark"),
    ] {
        let mut app = screen(mode);
        finish(&mut app, &ana).await;
        snapshot(&app, name);
    }

    // Sending later: the picker, with a draft.
    let mut app = screen(Mode::Light);
    if let Screen::Main(main) = &mut app.screen
        && let Some(open) = main.pane.room_mut()
    {
        open.composer = text_editor::Content::with_text("Results are in the shared folder 📈");
        main.popup = Some(Popup::SendLater(send_later::SendLater {
            picker: send_later::Picker::new(now, mtrx_core::parse_send_time("tomorrow 8:00", now)),
            rescheduling: None,
        }));
    }
    finish(&mut app, &ana).await;
    snapshot(&app, "site-send-later");

    // Copying: a selection through typeset math into the next message.
    let mut app = screen(Mode::Light);
    if let Screen::Main(main) = &mut app.screen
        && let Some(open) = main.pane.room_mut()
    {
        let math = id_of("The basal melt");
        open.selection = Some(Selection {
            anchor: Point {
                event: math.clone(),
                pos: Pos { unit: 0, offset: 4 },
            },
            focus: Point {
                event: id_of("Looks good"),
                pos: Pos {
                    unit: 0,
                    offset: 10,
                },
            },
            dragging: false,
        });
        open.hover(math.clone(), true);
        main.popup = Some(Popup::MessageMenu {
            row: math,
            at: MenuAt::Toolbar,
        });
    }
    finish(&mut app, &ana).await;
    snapshot(&app, "site-copy");
}

/// Scrolling a long draft scrolls the composer, not the timeline: its
/// height stays, with empty lines at the edges too (#32).
#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs local Synapse (testserver/run.sh)"]
async fn composer_scrolls_alone() {
    use iced::advanced::{clipboard, renderer::Headless};
    use iced_runtime::user_interface::{Cache, UserInterface};

    let dir = TempDir::new().unwrap();
    let (_, alice) = user("scroll", &dir).await;
    let room = alice
        .create_room(NewRoom {
            name: Some("drafts".into()),
            ..Default::default()
        })
        .await
        .unwrap();
    let timeline = Arc::new(alice.timeline(&room).await.unwrap());
    for i in 0..40 {
        send(&timeline, &format!("Message {i}")).await;
    }
    while !timeline.paginate_back(50).await.unwrap() {}
    let mut main = Main::new(Arc::new(alice));
    main.verified = Some(true);
    let mut open = Room::new(
        main.session.room(&room).unwrap(),
        timeline.clone(),
        main.session.draft_keeper(&room),
    );
    show(&mut open, timeline.current());
    // Short lines, then empty ones, as typed.
    let draft: Vec<String> = (0..18)
        .map(|i| {
            if i < 9 {
                format!("Line {i}")
            } else {
                String::new()
            }
        })
        .collect();
    open.composer = typed(&draft.join("\n"));
    main.pane = Pane::Open(open);
    let mut app = app(Screen::Main(Box::new(main)), Mode::Light);

    let mut renderer = iced::futures::executor::block_on(iced::Renderer::new(
        fonts::INTER,
        iced::Pixels(style::TEXT_SIZE),
        None,
    ))
    .unwrap();
    let size = Size::new(1200.0, 800.0);
    let at = iced::Point::new(700.0, 700.0);
    let mut cache = Cache::default();
    let mut before = None;
    for round in 0..24 {
        // Up, then down past the end.
        let delta = match round {
            0..4 => mouse::ScrollDelta::Pixels { x: 0.0, y: 30.0 },
            4..8 => mouse::ScrollDelta::Lines { x: 0.0, y: 1.0 },
            _ => mouse::ScrollDelta::Pixels { x: 0.0, y: -30.0 },
        };
        let mut ui = UserInterface::build(app.view(), size, cache, &mut renderer);
        let mut messages = Vec::new();
        let mut events = vec![iced::Event::Mouse(mouse::Event::CursorMoved {
            position: at,
        })];
        // Typing in it: focused.
        if round == 0 {
            events.push(iced::Event::Mouse(mouse::Event::ButtonPressed(
                mouse::Button::Left,
            )));
            events.push(iced::Event::Mouse(mouse::Event::ButtonReleased(
                mouse::Button::Left,
            )));
        }
        events.push(iced::Event::Mouse(mouse::Event::WheelScrolled { delta }));
        let (_, statuses) = ui.update(
            &events,
            mouse::Cursor::Available(at),
            &mut renderer,
            &mut clipboard::Null,
            &mut messages,
        );
        let mut timeline = TimelineView(None);
        ui.operate(&renderer, &mut timeline);
        cache = ui.into_cache();
        assert_eq!(*statuses.last().unwrap(), iced::event::Status::Captured);
        assert_eq!(
            *before.get_or_insert(timeline.0),
            timeline.0,
            "round {round}"
        );
        for message in messages {
            assert!(
                matches!(message, Message::Room(_, room::Message::Composer(_))),
                "{message:?}"
            );
            let _ = app.update(message);
        }
    }
}

/// The timeline's bounds and scroll translation, once operated.
struct TimelineView(Option<(iced::Rectangle, iced::Vector)>);

impl iced::advanced::widget::Operation for TimelineView {
    fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn iced::advanced::widget::Operation)) {
        operate(self);
    }

    fn scrollable(
        &mut self,
        id: Option<&iced::advanced::widget::Id>,
        bounds: iced::Rectangle,
        _content_bounds: iced::Rectangle,
        translation: iced::Vector,
        _state: &mut dyn iced::advanced::widget::operation::Scrollable,
    ) {
        if id == Some(&iced::advanced::widget::Id::new(room::TIMELINE)) {
            self.0 = Some((bounds, translation));
        }
    }
}

/// A room in the room list.
fn summary(id: &OwnedRoomId, name: &str) -> mtrx_core::RoomSummary {
    mtrx_core::RoomSummary {
        id: id.clone(),
        name: name.into(),
        avatar: None,
        topic: None,
        kind: RoomKind::Room,
        invited: false,
        encrypted: false,
        public: false,
        unread_messages: 0,
        unread_mentions: 0,
        unread_notifications: 0,
        last_activity: None,
        tag: RoomTag::None,
        direct_user: None,
        alias: None,
        joined_members: 1,
        pinned: Vec::new(),
    }
}

fn open_room(app: &mut App) -> &mut Room {
    match &mut app.screen {
        Screen::Main(main) => main.pane.room_mut().unwrap(),
        _ => unreachable!(),
    }
}

fn pane(app: &App) -> &Pane {
    match &app.screen {
        Screen::Main(main) => &main.pane,
        _ => unreachable!(),
    }
}

/// Run `task` to its end, as iced would, for its messages.
async fn messages_of(task: Task<Message>) -> Vec<Message> {
    let Some(stream) = iced_runtime::task::into_stream(task) else {
        return Vec::new();
    };
    stream
        .filter_map(|action| async move {
            match action {
                iced_runtime::Action::Output(message) => Some(message),
                _ => None,
            }
        })
        .collect()
        .await
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs local Synapse (testserver/run.sh)"]
async fn drafts_stay_in_their_room() {
    let dir = TempDir::new().unwrap();
    let (_, alice) = user("drafts", &dir).await;
    let new_room = |name: &str| NewRoom {
        name: Some(name.into()),
        ..Default::default()
    };
    let a = alice.create_room(new_room("a")).await.unwrap();
    let b = alice.create_room(new_room("b")).await.unwrap();
    let timeline = Arc::new(alice.timeline(&a).await.unwrap());
    let question = send(&timeline, "Coffee at three?").await;
    // In the timeline as sent, to be answered.
    key_of(&timeline, "Coffee at three?").await;
    let alice = Arc::new(alice);
    let mut main = Main::new(alice.clone());
    main.verified = Some(true);
    let open = Room::new(
        alice.room(&a).unwrap(),
        timeline.clone(),
        alice.draft_keeper(&a),
    );
    main.pane = Pane::Open(open);
    let mut app = app(Screen::Main(Box::new(main)), Mode::Light);

    // Writing an answer, kept after a pause.
    let target = (open_room(&mut app).rows.iter().filter_map(Row::event))
        .find(|e| e.event_id.as_ref() == Some(&question))
        .and_then(|e| actions::Target::of(e, "alice".into(), "Coffee at three?".into()));
    let _ = app.update(Message::Shown(room::Message::Act(actions::Message::Reply(
        target.unwrap(),
    ))));
    let _ = app.update(Message::Shown(room::Message::Composer(
        text_editor::Action::Edit(text_editor::Edit::Paste(Arc::new("Yes, see you".into()))),
    )));
    let kept = mtrx_core::Draft {
        text: "Yes, see you".into(),
        reply_to: Some(question.clone()),
    };
    draft_kept(&alice, &a, Some(kept.clone())).await;

    // More written, then another room opened at once: kept as it was.
    let _ = app.update(Message::Shown(room::Message::Composer(
        text_editor::Action::Edit(text_editor::Edit::Paste(Arc::new(" there".into()))),
    )));
    let before = app.room_draft();
    let other_timeline = Arc::new(alice.timeline(&b).await.unwrap());
    *open_room(&mut app) = Room::new(
        alice.room(&b).unwrap(),
        other_timeline,
        alice.draft_keeper(&b),
    );
    let _ = messages_of(app.keep_draft(before)).await;
    let kept = mtrx_core::Draft {
        text: "Yes, see you there".into(),
        ..kept
    };
    assert_eq!(
        alice.room(&a).unwrap().draft().await.unwrap(),
        Some(kept.clone())
    );
    let drafted = alice.room(&b).unwrap().draft().await.unwrap();
    assert_eq!(drafted, None, "per room");

    // Back in the room: the text, answering the question once it is found.
    *open_room(&mut app) = Room::new(
        alice.room(&a).unwrap(),
        timeline.clone(),
        alice.draft_keeper(&a),
    );
    let restored = open_room(&mut app).drafts.restore().await;
    let found = messages_of(app.update(room::Message::DraftLoaded(restored).to(&a))).await;
    assert_eq!(
        open_room(&mut app).composer.text().trim_end(),
        "Yes, see you there"
    );
    for message in found {
        let _ = app.update(message);
    }
    assert_eq!(open_room(&mut app).compose.reply_to(), Some(&question));
}

/// Wait until the draft kept for `room` on the device is `draft`.
async fn draft_kept(session: &Arc<Session>, room: &OwnedRoomId, draft: Option<mtrx_core::Draft>) {
    let room = session.room(room).unwrap();
    for _ in 0..50 {
        if room.draft().await.unwrap() == draft {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert_eq!(room.draft().await.unwrap(), draft);
}

/// Typed while another room opens: in neither room, nor in the draft kept
/// for the room left.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs local Synapse (testserver/run.sh)"]
async fn typing_while_a_room_opens_goes_nowhere() {
    let dir = TempDir::new().unwrap();
    let (_, alice) = user("opening", &dir).await;
    let new_room = |name: &str| NewRoom {
        name: Some(name.into()),
        ..Default::default()
    };
    let a = alice.create_room(new_room("a")).await.unwrap();
    let b = alice.create_room(new_room("b")).await.unwrap();
    let timeline = Arc::new(alice.timeline(&a).await.unwrap());
    let alice = Arc::new(alice);
    let mut main = Main::new(alice.clone());
    main.verified = Some(true);
    let mut open = Room::new(
        alice.room(&a).unwrap(),
        timeline.clone(),
        alice.draft_keeper(&a),
    );
    open.set_composer(typed("Hello"));
    main.pane = Pane::Open(open);
    let mut app = app(Screen::Main(Box::new(main)), Mode::Light);

    let opened = messages_of(app.update(Message::OpenRoom(b.clone()))).await;
    assert!(matches!(pane(&app), Pane::Opening(id) if *id == b));
    let _ =
        messages_of(app.update(Message::Shown(room::Message::TypeInComposer("x".into())))).await;
    for message in opened {
        let _ = app.update(message);
    }
    assert_eq!(open_room(&mut app).id, b);
    assert_eq!(open_room(&mut app).composer.text().trim_end(), "");
    let kept = mtrx_core::Draft {
        text: "Hello".into(),
        reply_to: None,
    };
    assert_eq!(alice.room(&a).unwrap().draft().await.unwrap(), Some(kept));
}

/// An invite chosen while a room opens stays shown: the room opening late
/// does not open behind it.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs local Synapse (testserver/run.sh)"]
async fn an_invite_chosen_while_a_room_opens_stays() {
    let dir = TempDir::new().unwrap();
    let (_, alice) = user("invited", &dir).await;
    let a = alice
        .create_room(NewRoom {
            name: Some("a".into()),
            ..Default::default()
        })
        .await
        .unwrap();
    let invite: OwnedRoomId = "!invite:localhost".try_into().unwrap();
    let mut main = Main::new(Arc::new(alice));
    main.verified = Some(true);
    main.show_rooms(RoomListState {
        rooms: vec![
            summary(&a, "a"),
            mtrx_core::RoomSummary {
                invited: true,
                ..summary(&invite, "invite")
            },
        ],
        loaded: false,
        ..Default::default()
    });
    let mut app = app(Screen::Main(Box::new(main)), Mode::Light);

    let opened = messages_of(app.update(Message::OpenRoom(a.clone()))).await;
    let _ = app.update(Message::OpenRoom(invite.clone()));
    for message in opened {
        let _ = app.update(message);
    }
    assert!(matches!(
        pane(&app),
        Pane::Invite { id, answering: false } if *id == invite
    ));
}

/// What finishes for a room after another opened stays with it: a message
/// scheduled there leaves the other's composer alone, and its failures do
/// not show in the other.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs local Synapse (testserver/run.sh)"]
async fn results_stay_with_their_room() {
    let dir = TempDir::new().unwrap();
    let (_, alice) = user("results", &dir).await;
    let new_room = |name: &str| NewRoom {
        name: Some(name.into()),
        ..Default::default()
    };
    let a = alice.create_room(new_room("a")).await.unwrap();
    let b = alice.create_room(new_room("b")).await.unwrap();
    let timeline = Arc::new(alice.timeline(&a).await.unwrap());
    let mut main = Main::new(Arc::new(alice));
    main.verified = Some(true);
    let mut open = Room::new(
        main.session.room(&a).unwrap(),
        timeline,
        main.session.draft_keeper(&a),
    );
    open.set_composer(typed("Later, in a"));
    main.pane = Pane::Open(open);
    let mut app = app(Screen::Main(Box::new(main)), Mode::Light);

    let in_an_hour = SystemTime::now() + Duration::from_secs(3600);
    let later = send_later::Message::At(in_an_hour);
    let scheduled = app.update(room::Message::Later(later).to(&a));
    for message in messages_of(app.update(Message::OpenRoom(b.clone()))).await {
        let _ = app.update(message);
    }
    open_room(&mut app).set_composer(typed("Draft in b"));
    for message in messages_of(scheduled).await {
        let _ = app.update(message);
    }
    let failed = actions::Message::Done(Err("Could not react".into()));
    let _ = app.update(room::Message::Act(failed).to(&a));

    let open = open_room(&mut app);
    assert_eq!(open.id, b);
    assert_eq!(open.composer.text().trim_end(), "Draft in b");
    assert_eq!(open.error, None);
}

/// A room that failed to open says so in the notice bar, offering to open
/// it again, until it opens.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs local Synapse (testserver/run.sh)"]
async fn a_room_that_failed_to_open_opens_again() {
    let dir = TempDir::new().unwrap();
    let (_, alice) = user("reopen", &dir).await;
    let a = alice
        .create_room(NewRoom {
            name: Some("a".into()),
            ..Default::default()
        })
        .await
        .unwrap();
    let timeline = Arc::new(alice.timeline(&a).await.unwrap());
    let mut main = Main::new(Arc::new(alice));
    main.verified = Some(true);
    main.show_rooms(RoomListState {
        rooms: vec![summary(&a, "a")],
        loaded: false,
        ..Default::default()
    });
    let mut app = app(Screen::Main(Box::new(main)), Mode::Light);
    let problem = |app: &App| match &app.screen {
        Screen::Main(main) => main.notice.as_ref().filter(|n| n.error).map(|n| {
            assert_eq!(n.room.as_ref(), Some(&a));
            n.text.clone()
        }),
        _ => None,
    };

    let _ = app.update(Message::OpenRoom(a.clone()));
    let _ = app.update(Message::RoomOpened(a.clone(), Err("timed out".into())));
    assert!(matches!(pane(&app), Pane::Empty));
    assert_eq!(
        problem(&app).as_deref(),
        Some("Could not open the room: timed out")
    );
    let _ = app.update(Message::OpenRoom(a.clone()));
    let _ = app.update(Message::RoomOpened(a.clone(), Ok(Shared(timeline))));
    assert_eq!(open_room(&mut app).id, a);
    assert_eq!(problem(&app), None);
}

/// A dialog's request reports back to that dialog: one closed while it ran
/// says how it went in the notice bar, not in the dialog open since.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs local Synapse (testserver/run.sh)"]
async fn a_cancelled_request_reports_in_the_notice_bar() {
    let dir = TempDir::new().unwrap();
    let (_, alice) = user("cancelled", &dir).await;
    let mut shown = Main::new(Arc::new(alice));
    shown.verified = Some(true);
    let mut app = app(Screen::Main(Box::new(shown)), Mode::Light);
    fn main(app: &mut App) -> &mut Main {
        match &mut app.screen {
            Screen::Main(main) => main,
            _ => unreachable!(),
        }
    }
    let leave = |name: &str| dialog::Kind::Leave {
        room: format!("!{name}:localhost").try_into().unwrap(),
        name: name.into(),
    };

    // Not run: the result is given below.
    let _ = app.update(Message::OpenDialog(leave("first")));
    let _ = app.update(Message::Dialog(dialog::Message::Submit));
    let first = main(&mut app).last_request;
    let _ = app.update(Message::Dialog(dialog::Message::Cancel));
    assert!(main(&mut app).modal.is_none());
    let _ = app.update(Message::OpenDialog(leave("second")));
    let _ = app.update(Message::Dialog(dialog::Message::Submit));
    let second = main(&mut app).last_request;
    assert_ne!(first, second);

    let failed = Err("no such room".into());
    let _ = app.update(Message::Dialog(dialog::Message::Done(first, failed)));
    let notice = main(&mut app).notice.take().expect("a notice");
    assert!(notice.error);
    assert_eq!(notice.text, "Could not leave first: no such room");
    let Some(Modal::Dialog(open)) = &main(&mut app).modal else {
        panic!("the second dialog closed");
    };
    assert!(open.busy(), "still waiting for its own request");

    // Its own result, which only it takes.
    let failed = Err("not a member".into());
    let _ = app.update(Message::Dialog(dialog::Message::Done(second, failed)));
    let Some(Modal::Dialog(open)) = &main(&mut app).modal else {
        panic!("the second dialog closed");
    };
    assert!(!open.busy());
    assert!(main(&mut app).notice.is_none());
}

/// A request finishing after its dialog was closed still does what it
/// did: a room left closes, signing out shows the sign-in.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs local Synapse (testserver/run.sh)"]
async fn a_cancelled_request_still_takes_effect() {
    let dir = TempDir::new().unwrap();
    let (_, alice) = user("cancelleffect", &dir).await;
    let mut shown = Main::new(Arc::new(alice));
    shown.verified = Some(true);
    let mut app = app(Screen::Main(Box::new(shown)), Mode::Light);
    fn main(app: &mut App) -> &mut Main {
        match &mut app.screen {
            Screen::Main(main) => main,
            _ => unreachable!(),
        }
    }
    let lab: OwnedRoomId = "!lab:localhost".try_into().unwrap();
    main(&mut app).set_pane(Pane::Opening(lab.clone()));
    main(&mut app).space = Some(lab.clone());

    // Not run: the results are given below.
    let leave = dialog::Kind::Leave {
        room: lab.clone(),
        name: "Lab".into(),
    };
    let _ = app.update(Message::OpenDialog(leave));
    let _ = app.update(Message::Dialog(dialog::Message::Submit));
    let left = main(&mut app).last_request;
    let _ = app.update(Message::Dialog(dialog::Message::Cancel));
    let done = Ok(dialog::Opened::Left(lab.clone()));
    let _ = app.update(Message::Dialog(dialog::Message::Done(left, done)));
    assert!(main(&mut app).notice.is_some());
    assert!(
        main(&mut app).pane.id().is_none(),
        "the room left still shows"
    );
    assert!(main(&mut app).space.is_none(), "the space left still shows");

    let _ = app.update(Message::OpenDialog(dialog::Kind::SignOut));
    let _ = app.update(Message::Dialog(dialog::Message::Submit));
    let out = main(&mut app).last_request;
    let _ = app.update(Message::Dialog(dialog::Message::Cancel));
    let done = Ok(dialog::Opened::SignedOut);
    let _ = app.update(Message::Dialog(dialog::Message::Done(out, done)));
    assert!(matches!(app.screen, Screen::Login(_)), "still signed in");
}

/// A direct chat that could not be started says so in the notice bar,
/// whatever dialog is open.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs local Synapse (testserver/run.sh)"]
async fn a_chat_that_cannot_start_reports_in_the_notice_bar() {
    let dir = TempDir::new().unwrap();
    let (_, alice) = user("chatwith", &dir).await;
    // Signed out elsewhere: the server refuses to start the chat.
    let alice = Arc::new(alice);
    alice.logout().await.unwrap();
    let mut main = Main::new(alice);
    main.verified = Some(true);
    let mut app = app(Screen::Main(Box::new(main)), Mode::Light);
    let _ = app.update(Message::OpenDialog(dialog::Kind::SignOut));

    let ghost: OwnedUserId = "@ghost:localhost".try_into().unwrap();
    let started = messages_of(app.update(Message::ChatWith(ghost))).await;
    assert!(
        matches!(&started[..], [Message::ChatStarted(Err(_))]),
        "{started:?}"
    );
    for message in started {
        let _ = app.update(message);
    }
    let Screen::Main(main) = &app.screen else {
        unreachable!()
    };
    let notice = main.notice.as_ref().expect("a notice");
    assert!(notice.error);
    assert!(
        notice.text.starts_with("Could not open the direct chat: "),
        "{}",
        notice.text
    );
    let Some(Modal::Dialog(open)) = &main.modal else {
        panic!("the dialog closed");
    };
    assert!(!open.busy());
    assert!(matches!(pane(&app), Pane::Empty));
}

/// A message scheduled in a room left before the server confirmed it is
/// no longer that room's draft.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs local Synapse (testserver/run.sh)"]
async fn scheduled_in_a_room_left_clears_its_draft() {
    let dir = TempDir::new().unwrap();
    let (_, alice) = user("scheduled", &dir).await;
    let new_room = |name: &str| NewRoom {
        name: Some(name.into()),
        ..Default::default()
    };
    let a = alice.create_room(new_room("a")).await.unwrap();
    let b = alice.create_room(new_room("b")).await.unwrap();
    let timeline = Arc::new(alice.timeline(&a).await.unwrap());
    let alice = Arc::new(alice);
    let mut main = Main::new(alice.clone());
    main.verified = Some(true);
    let mut open = Room::new(alice.room(&a).unwrap(), timeline, alice.draft_keeper(&a));
    open.set_composer(typed("Later, in a"));
    main.pane = Pane::Open(open);
    let mut app = app(Screen::Main(Box::new(main)), Mode::Light);

    let in_an_hour = SystemTime::now() + Duration::from_secs(3600);
    let later = send_later::Message::At(in_an_hour);
    let scheduled = app.update(room::Message::Later(later).to(&a));
    for message in messages_of(app.update(Message::OpenRoom(b.clone()))).await {
        let _ = app.update(message);
    }
    let kept = mtrx_core::Draft {
        text: "Later, in a".into(),
        reply_to: None,
    };
    assert_eq!(
        alice.room(&a).unwrap().draft().await.unwrap(),
        Some(kept),
        "kept on leaving"
    );
    for message in messages_of(scheduled).await {
        let _ = messages_of(app.update(message)).await;
    }

    assert_eq!(alice.room(&a).unwrap().draft().await.unwrap(), None);
    let drafts = alice.rooms_with_drafts_updates().get();
    assert!(!drafts.contains(&a), "{drafts:?}");
    assert_eq!(open_room(&mut app).id, b);
}
