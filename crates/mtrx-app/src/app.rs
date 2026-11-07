//! Application state, messages and top-level layout.

pub mod account;
pub mod modal;
pub mod pane;
pub mod people;
pub mod start;
pub mod window;

pub use self::window::closing;
use self::{
    modal::Modal,
    pane::{Pane, Shown},
    window::{Resize, Window},
};

use std::{
    collections::{BTreeSet, HashMap},
    fmt,
    hash::{Hash, Hasher},
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, SystemTime},
};

use futures_util::{SinkExt, StreamExt};
use iced::{
    Element, Length, Size, Subscription, Task, Theme, mouse,
    widget::{button, column, container, mouse_area, opaque, row, stack, text, text_input},
};
use mtrx_core::{
    Connection, Contact, DownloadId, Live, Notification, OwnedRoomId, OwnedUserId, Profile,
    RecoveryStep, RoomKind, RoomListState, RoomLists, RoomTag, ScheduledMessage, SecretStore,
    Session,
};
use mtrx_google::Google;

use crate::{
    actions::PickerAt,
    arrange::{self},
    avatar::Avatars,
    badge::{self, Badge},
    calls,
    dialog::{self},
    downloads::{self, Downloads},
    edit_menu, emoji,
    find::{self},
    hint::{Position, hint},
    info, join,
    last_view::LastView,
    login::Login,
    math::Math,
    media::{self, Previews},
    members, message,
    message_field::Field,
    message_menu::MenuAt,
    overview, pins,
    placement::Placement,
    popover,
    presence::{self, Presences},
    room::{self, Look},
    selection::{self},
    send_later,
    settings::{self, Settings},
    sidebar::{self, Sidebar},
    spelling, style,
    task::{self},
    tint::Tint,
    update::{self, Updater},
};

/// How often times shown relative to now are refreshed.
const TICK: Duration = Duration::from_secs(20);
/// How often progress (elapsed seconds) is refreshed while something runs.
const PROGRESS_TICK: Duration = Duration::from_secs(1);
/// Where cards, the join dialog and Settings start, below the window's top.
const PANEL_TOP: f32 = 72.0;

/// The room lists, kept up to date while held.
type LiveLists = Live<Arc<RoomLists>>;

/// An `Arc` that identifies by pointer: lets live core handles be used as
/// subscription keys and carried in messages.
pub struct Shared<T>(pub Arc<T>);

impl<T> Clone for Shared<T> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<T> fmt::Debug for Shared<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Shared<{}>", std::any::type_name::<T>())
    }
}

impl<T> Hash for Shared<T> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        (Arc::as_ptr(&self.0) as usize).hash(state);
    }
}

#[derive(Debug, Clone)]
pub enum Message {
    Window(window::Message),
    Start(start::Message),
    /// The room lists changed.
    Rooms(Arc<RoomLists>),
    Sidebar(sidebar::Message),
    Pane(pane::Message),
    /// For room `.0`: built by its views, or by a task it started. Dropped
    /// unless that room is open.
    Room(OwnedRoomId, room::Message),
    /// For the room shown, whichever it is: input to the whole window, such
    /// as shortcuts and dropped files. Dropped while no room is open.
    Shown(room::Message),
    /// The info card: who someone is or what a room is.
    Card(info::Message),
    Modal(modal::Message),
    Account(account::Message),
    People(people::Message),
    /// Pictures and typeset math, copying images, and downloads.
    Media(media::Message),
    Update(update::Message),
    /// Open the settings and look for updates: Check for Updates… in the
    /// app menu, which only macOS has.
    #[cfg(target_os = "macos")]
    CheckForUpdates,
    Escape,
    /// ⌘↩: the open dialog's main action, or saving the card's changes.
    Submit,
    LinkClicked(String),
    /// Close the popup: a click outside it, or its Cancel.
    ClosePopup,
    /// We left a room: what shows it closes.
    Left(OwnedRoomId),
    /// A space just joined: selected, with its overview once it is listed.
    ShowJoined(OwnedRoomId),
    CloseNotice,
}

pub struct App {
    profile: Profile,
    /// Connection for creating Google Meet links, shared by all accounts.
    google: Arc<Google>,
    settings: Settings,
    /// Where settings are saved; `None` keeps them in memory (tests).
    settings_file: Option<PathBuf>,
    /// Where the last space and room shown are saved; `None` in tests.
    view_file: Option<PathBuf>,
    window: Window,
    screen: Screen,
    math: Math,
    badge: Badge,
    updater: Updater,
}

enum Screen {
    Starting,
    Login(Login),
    Main(Box<Main>),
    /// The window closes: the session or sign-in has ended.
    Closing,
}

struct Main {
    session: Arc<Session>,
    room_lists: Option<Arc<LiveLists>>,
    lists: Arc<RoomLists>,
    sidebar: Sidebar,
    pane: Pane,
    /// `None` until known.
    verified: Option<bool>,
    recovery_key: String,
    /// Set while recovery runs.
    recovery_step: Option<RecoveryStep>,
    recovery_error: Option<String>,
    /// Scheduled messages of all rooms, earliest first.
    scheduled: Vec<ScheduledMessage>,
    /// When scheduled times were last compared to the clock.
    now: SystemTime,
    avatars: Avatars,
    /// The panel over the window, above the card.
    modal: Modal,
    /// The menu or picker open, anywhere in the window.
    popup: Option<Popup>,
    /// A space just joined, whose overview shows once it is in the room
    /// list.
    joined_space: Option<OwnedRoomId>,
    /// This desktop's "Notes at the top" was offered to the account.
    notes_moved: bool,
    notices: Notices,
    own_user: OwnedUserId,
    previews: Previews,
    /// The info card shown: a panel to come back to, below a modal.
    card: Option<info::Card>,
    /// People the user knows, closest first; refreshed when a dialog
    /// picking people opens.
    contacts: Vec<Contact>,
    /// The emoji the user reacts with, most used first; refreshed when a
    /// reaction picker opens.
    frequent_emoji: Vec<String>,
    connection: Connection,
    /// The view to show again once its room is listed (after starting).
    restore: Option<LastView>,
    /// The space and room shown, as last saved to show them again.
    shown: LastView,
    /// Others' presence, for the people shown.
    presences: Presences,
    downloads: Downloads,
}

/// A menu or picker: one at a time in the whole window, the one opened last
/// replacing any other. Those of the open room close with it, see
/// [`Main::set_pane`].
pub enum Popup {
    /// A timeline row's menu, and where.
    MessageMenu { row: String, at: MenuAt },
    /// A timeline row's reaction window, and where; with all emoji while
    /// shown.
    Reactions {
        row: String,
        at: PickerAt,
        all: Option<emoji::Panel>,
    },
    /// Everyone who read up to a timeline row's message.
    Readers(String),
    /// Everyone who reacted to a timeline row's message with a key.
    Reactors { row: String, key: String },
    /// Retry or delete a timeline row's message that was not sent.
    Unsent(String),
    /// Confirm deleting a timeline row's message.
    ConfirmDelete(String),
    /// The menu of a field one writes in.
    FieldMenu(Field, edit_menu::Menu),
    /// The emoji panel of a field one writes in.
    FieldEmoji(Field, emoji::Panel),
    /// The room's calls and links.
    Calls(calls::Menu),
    /// A send time, for the composer's text or a scheduled message.
    SendLater(send_later::SendLater),
    /// The room list's "+".
    AddMenu,
    /// The space bar's "+", or a space's.
    SpaceMenu(sidebar::SpaceMenu),
    /// A member's, in a room's member list or a space's overview.
    MemberMenu(members::MenuKey),
    /// A room's in the room list.
    RoomMenu(OwnedRoomId),
    /// A category's in the room list.
    CategoryMenu(String),
    /// Which of a space's sections are expanded.
    ShowMenu,
    /// The card's picture.
    CardPicture,
    /// The card's spaces to add its room to.
    CardAddToSpace,
    /// The image's in the viewer.
    Viewer,
    /// The downloads list, and a download's menu in it.
    Downloads { menu: Option<DownloadId> },
}

impl Popup {
    /// It belongs to room `room`, the open one: to its timeline, fields or
    /// header, or a member's menu in its member list.
    pub fn of_room(&self, room: &OwnedRoomId) -> bool {
        match self {
            Popup::MemberMenu((of, ..)) => of == room,
            Popup::AddMenu
            | Popup::SpaceMenu(_)
            | Popup::RoomMenu(_)
            | Popup::CategoryMenu(_)
            | Popup::ShowMenu
            | Popup::CardPicture
            | Popup::CardAddToSpace
            | Popup::Viewer
            | Popup::Downloads { .. } => false,
            _ => true,
        }
    }

    /// It goes with the card: a member's menu, or the card's own.
    pub fn of_card(&self) -> bool {
        matches!(
            self,
            Popup::MemberMenu(_) | Popup::CardPicture | Popup::CardAddToSpace
        )
    }

    /// It belongs to `field`.
    pub fn in_field(&self, field: Field) -> bool {
        matches!(self, Popup::FieldMenu(f, _) | Popup::FieldEmoji(f, _) if *f == field)
    }

    /// Esc closes what is open inside it, if anything: a download's menu,
    /// leaving the list. False if nothing was.
    fn escape(&mut self) -> bool {
        match self {
            Popup::Downloads { menu } => menu.take().is_some(),
            _ => false,
        }
    }

    /// The member whose menu it is.
    pub fn member_menu(popup: Option<&Self>) -> Option<&members::MenuKey> {
        match popup {
            Some(Popup::MemberMenu(key)) => Some(key),
            _ => None,
        }
    }
}

/// What finished in the background, or failed: in the notice bar until
/// closed or replaced.
#[derive(Default)]
pub struct Notices {
    shown: Option<Notice>,
}

impl Notices {
    /// Say that something failed outside a room, where its status pill
    /// would say it. With `room`, the room it concerns, offered to open
    /// (again).
    pub fn report(&mut self, problem: String, room: Option<OwnedRoomId>) {
        self.shown = Some(Notice {
            text: problem,
            room,
            error: true,
        });
    }

    /// Say that something finished; with `room`, what it made, offered to
    /// open.
    pub fn done(&mut self, text: String, room: Option<OwnedRoomId>) {
        self.shown = Some(Notice {
            text,
            room,
            error: false,
        });
    }

    /// `room` opened after all: a problem with it goes.
    pub fn solved(&mut self, room: &OwnedRoomId) {
        (self.shown).take_if(|notice| notice.error && notice.room.as_ref() == Some(room));
    }

    pub fn close(&mut self) {
        self.shown = None;
    }
}

/// Something finished in the background, or failed.
struct Notice {
    text: String,
    /// The room it concerns, offered to open.
    room: Option<OwnedRoomId>,
    /// A problem, not news.
    error: bool,
}

/// What a component's update may read and change besides its own state.
pub struct Cx<'a> {
    pub session: &'a Arc<Session>,
    pub own_user: &'a OwnedUserId,
    pub lists: &'a Arc<RoomLists>,
    pub connection: Connection,
    /// Saved after the update if [`Cx::save_settings`] was called.
    pub settings: &'a mut Settings,
    pub google: &'a Arc<Google>,
    /// The viewport and pixel scale.
    pub window: &'a Window,
    /// The menu or picker open in the window.
    pub popup: &'a mut Option<Popup>,
    pub notices: &'a mut Notices,
    /// The settings changed: `App` saves them.
    save: bool,
    /// The card goes: `Main` drops it.
    close_card: bool,
}

impl Cx<'_> {
    /// Something opens over the window: the popup closes.
    pub fn cover(&mut self) {
        *self.popup = None;
    }

    /// Something is chosen for the main view: the card over it closes.
    pub fn close_card(&mut self) {
        self.close_card = true;
    }

    /// The settings changed: saved once the update is done.
    pub fn save_settings(&mut self) {
        self.save = true;
    }
}

/// What `Main` borrows from `App` for an update.
struct AppParts<'a> {
    settings: &'a mut Settings,
    google: &'a Arc<Google>,
    window: &'a Window,
    updater: &'a mut Updater,
    /// Set when the update changed the settings, for `App` to save.
    save: bool,
}

impl Main {
    fn new(session: Arc<Session>) -> Self {
        Self {
            room_lists: None,
            lists: Arc::new(RoomLists::new(
                RoomListState::default(),
                session.room_categories(),
                HashMap::new(),
                &session.user_id(),
                BTreeSet::new(),
            )),
            sidebar: Sidebar::new(&session),
            pane: Pane::default(),
            verified: None,
            recovery_key: String::new(),
            recovery_step: None,
            recovery_error: None,
            scheduled: session.scheduled(),
            own_user: session.user_id(),
            card: None,
            contacts: Vec::new(),
            frequent_emoji: Vec::new(),
            connection: session.connection(),
            restore: None,
            shown: LastView::default(),
            presences: Presences::default(),
            downloads: Downloads::default(),
            now: SystemTime::now(),
            session,
            avatars: Avatars::new(),
            modal: Modal::default(),
            popup: None,
            joined_space: None,
            notes_moved: false,
            notices: Notices::default(),
            previews: Previews::new(),
        }
    }
}

impl Main {
    /// A message for a component, with what it may read and change
    /// besides its own state.
    fn update(&mut self, message: Message, parts: &mut AppParts<'_>) -> Task<Message> {
        match message {
            // Typed with nothing focused goes to the composer, unless
            // something shows over it.
            Message::Shown(room::Message::TypeInComposer(_))
                if self.modal.is_some() || self.card.is_some() =>
            {
                return Task::none();
            }
            // The room list as shown leads to the room.
            Message::Sidebar(sidebar::Message::GoTo(go)) => {
                return match sidebar::go_to(&self.room_list_view(parts.settings), go) {
                    Some(room) => {
                        let open = pane::Message::Open(room.id.clone());
                        self.update(Message::Pane(open), parts)
                    }
                    None => Task::none(),
                };
            }
            _ => {}
        }
        let Main {
            session,
            own_user,
            lists,
            connection,
            popup,
            notices,
            sidebar,
            pane,
            card,
            modal,
            joined_space,
            ..
        } = self;
        let mut cx = Cx {
            session,
            own_user,
            lists,
            connection: *connection,
            settings: &mut *parts.settings,
            google: parts.google,
            window: parts.window,
            popup,
            notices,
            save: false,
            close_card: false,
        };
        // A card shown, or closed.
        let mut shown_card = None;
        let task = match message {
            Message::Sidebar(message) => sidebar.update(message, &mut cx),
            Message::Pane(message) => pane.update(message, &mut cx),
            Message::Room(id, message) => pane.room_message(id, message, &mut cx),
            Message::Shown(message) => pane.shown_message(message, &mut cx),
            Message::Card(info::Message::Show(target, tab)) => {
                let (card, task) = info::Card::open(target, tab, &mut cx);
                shown_card = Some(Some(card));
                task
            }
            Message::Card(info::Message::Close) => {
                shown_card = Some(None);
                Task::none()
            }
            Message::Card(message) => match card {
                Some(card) => card.update(message, &mut cx),
                None => Task::none(),
            },
            Message::Modal(message) => modal.update(message, &mut cx, parts.updater, sidebar),
            Message::Left(id) => {
                let left = if pane.id() == Some(&id) {
                    pane.set(Shown::Empty, cx.popup)
                } else {
                    Task::none()
                };
                if card.as_ref().and_then(|c| c.room()) == Some(&id) {
                    cx.close_card();
                }
                if sidebar.space.as_ref() == Some(&id) {
                    sidebar.select(None);
                }
                left
            }
            Message::ShowJoined(space) => {
                let select = sidebar.select_space(Some(space.clone()), &mut cx);
                // Its overview once the room list has it.
                let show = if cx.lists.room(&space).is_some() {
                    Task::done(info::show_space(space, overview::Tab::Rooms))
                } else {
                    *joined_space = Some(space);
                    Task::none()
                };
                Task::batch([select, show])
            }
            // `App::handle`'s.
            _ => Task::none(),
        };
        let (save, close_card) = (cx.save, cx.close_card);
        parts.save |= save;
        if close_card {
            self.set_card(None);
        }
        if let Some(card) = shown_card {
            self.set_card(card);
        }
        task
    }

    /// Show `card`, or none: the popups of the card shown before close.
    fn set_card(&mut self, card: Option<info::Card>) {
        self.popup.take_if(|popup| popup.of_card());
        self.card = card;
    }

    /// Esc closes the topmost thing: the popup (on top, as what opens over
    /// it closes it), its innermost part first; the modal, the card, then in
    /// the open room the mention offered, the find bar, the selection, the
    /// reply and the message edited in place; then a drag or category
    /// name, else the search.
    fn escape(&mut self) -> Task<Message> {
        if let Some(popup) = &mut self.popup {
            if !popup.escape() {
                self.popup = None;
            }
            return Task::none();
        }
        if let Some(task) = self.modal.escape() {
            return task;
        }
        if self.card.is_some() {
            self.set_card(None);
            return Task::none();
        }
        let room = self.pane.room_mut();
        if let Some(task) = room.and_then(|room| room.escape(&self.own_user, &self.lists)) {
            return task;
        }
        self.sidebar.escape();
        Task::none()
    }

    /// ⌘↩: the open dialog's main action, or saving the card's changes.
    fn submit(&self) -> Task<Message> {
        let submit = match (&self.modal.shown, &self.card) {
            (Some(modal::Shown::Dialog(_)), _) => Some(Message::Modal(modal::Message::Dialog(
                dialog::Message::Submit,
            ))),
            (Some(_), _) => None,
            (None, card) => card
                .as_ref()
                .and_then(|card| card.submit(&self.lists.rooms)),
        };
        submit.map_or_else(Task::none, Task::done)
    }

    /// Save the space and room shown when they change, to show them again
    /// next time; a view chosen ends waiting for last time's.
    fn remember_view(&mut self, file: Option<&Path>) {
        let (space, room) = (self.sidebar.space.as_ref(), self.pane.remembered());
        if self.shown.space.as_ref() == space && self.shown.room.as_ref() == room {
            return;
        }
        self.shown = LastView {
            space: space.cloned(),
            room: room.cloned(),
        };
        self.restore = None;
        if let Some(file) = file {
            self.shown.save(file);
        }
    }

    /// Make the open room's pinned messages' timeline, once it has pins.
    fn load_pinned(&mut self) -> Task<Message> {
        self.pane.load_pinned(&self.session, &self.lists)
    }

    fn room_list_view<'a>(&'a self, settings: &'a Settings) -> sidebar::RoomListView<'a> {
        let (grouping, width) = (settings.room_list, settings.room_list_width);
        sidebar::RoomListView {
            lists: &self.lists,
            selected_space: self.sidebar.space.as_ref(),
            open: self.pane.id(),
            loaded: self.lists.loaded,
            avatars: &self.avatars,
            scheduled: &self.scheduled,
            popup: self.popup.as_ref(),
            filter: &self.sidebar.filter,
            arrange: &self.sidebar.arrange,
            grouping,
            out_of_view: self.sidebar.out_of_view,
            width,
            tint: Tint::of(&self.sidebar.colours, self.sidebar.space.as_ref()),
            room_pictures: settings.room_pictures,
            notes: self.pinned_notes(),
            presences: &self.presences,
            space_rooms: &self.sidebar.space_rooms,
            space_topic: &self.sidebar.space_topic,
            call_services: &settings.call_services,
            topic_width: popover::MIN_SHARED_WIDTH,
        }
    }

    /// The people whose presence shows, first shown first: the open direct
    /// chat's, those of the direct chats listed, and the member list's.
    fn shown_people(&self, settings: &Settings) -> Vec<OwnedUserId> {
        let rooms = &self.lists.rooms;
        let open = self
            .pane
            .room()
            .and_then(|room| rooms.iter().find(|r| r.id == room.id));
        let mut people: Vec<&OwnedUserId> = open
            .filter(|r| r.kind == RoomKind::Direct)
            .and_then(|r| r.direct_user.as_ref().map(|u| &u.id))
            .into_iter()
            .collect();
        people.extend(sidebar::direct_users(
            &self.room_list_view(settings),
            &self.own_user,
        ));
        if self.pane.show_members
            && let Some(Ok(members)) = self.pane.room().and_then(|r| r.member_list.as_ref())
        {
            people.extend(members.iter().map(|m| &m.user.id));
        }
        let mut unique: Vec<OwnedUserId> = Vec::with_capacity(people.len());
        for person in people {
            if *person != self.own_user && !unique.contains(person) {
                unique.push(person.clone());
            }
        }
        unique
    }

    /// Follow the presence of the people shown once they settle.
    fn follow_shown(&mut self, settings: &Settings) -> Task<Message> {
        let people = self.shown_people(settings);
        match self.presences.show(people) {
            Some(generation) => Task::perform(tokio::time::sleep(presence::SETTLE), move |()| {
                Message::People(people::Message::Presence(presence::Message::Settle(
                    generation,
                )))
            }),
            None => Task::none(),
        }
    }

    /// What the app is doing, which sets our presence, unless the server
    /// does not share it.
    fn activity(&self, activity: mtrx_core::Activity) -> Task<Message> {
        if self.presences.unavailable() {
            return Task::none();
        }
        presence::activity(&self.session, activity)
            .map(|m| Message::People(people::Message::Presence(m)))
    }

    /// The Notes to self if pinned; missing ones only once the room list
    /// is complete, so they are not started twice.
    fn pinned_notes(&self) -> Option<sidebar::Notes<'_>> {
        if !self.lists.notes_at_top() {
            return None;
        }
        match self.lists.notes.as_ref().and_then(|id| self.lists.room(id)) {
            Some(notes) => Some(sidebar::Notes::Room(notes)),
            None => self.lists.loaded.then_some(sidebar::Notes::Missing),
        }
    }
}

/// What the app starts with: profile and data directory from the same
/// environment variables the CLI reads, and the saved settings and window
/// placement, read before the window opens as its size depends on the text
/// size.
#[derive(Clone)]
pub struct Environment {
    profile: Profile,
    data_root: PathBuf,
    secrets: SecretStore,
    settings: Settings,
    placement: Placement,
}

impl Environment {
    pub fn load() -> Result<Self, String> {
        let name = std::env::var("MTRX_PROFILE").unwrap_or_else(|_| "default".into());
        let data_root = match std::env::var_os("MTRX_DATA_DIR") {
            Some(dir) => PathBuf::from(dir),
            None => Profile::default_data_root().map_err(|e| e.to_string())?,
        };
        let secrets = match std::env::var("MTRX_SECRET_STORE").as_deref() {
            Ok("file") => SecretStore::File,
            _ => SecretStore::Keyring,
        };
        let profile =
            Profile::new(&name, &data_root, secrets.clone()).map_err(|e| e.to_string())?;
        let settings = Settings::load(&Settings::path(&data_root));
        let placement = Placement::load(&Placement::path(&profile));
        Ok(Self {
            profile,
            data_root,
            secrets,
            settings,
            placement,
        })
    }

    /// The window's size, position and whether it is maximised on opening,
    /// as it was left. Its size is in layout units, which iced scales by the
    /// text size: the same in logical pixels at any text size.
    pub fn window(&self) -> iced::window::Settings {
        self.placement.settings(self.settings.text_size.scale())
    }

    /// Where the settings, the profiles and the crash log are.
    pub fn data_root(&self) -> &Path {
        &self.data_root
    }
}

impl App {
    pub fn boot(environment: Environment) -> (Self, Task<Message>) {
        let Environment {
            profile,
            data_root,
            secrets,
            settings,
            placement,
        } = environment;
        let settings_file = Settings::path(&data_root);
        #[cfg(target_os = "macos")]
        settings.appearance.apply();
        spelling::set_enabled(settings.check_spelling);
        update::clean_up();
        let app = Self {
            profile: profile.clone(),
            google: Arc::new(Google::new(&data_root, secrets)),
            settings,
            settings_file: Some(settings_file),
            view_file: Some(LastView::path(&profile)),
            window: Window::new(placement, Some(Placement::path(&profile))),
            screen: Screen::Starting,
            math: Math::new(),
            badge: Badge::default(),
            updater: Updater::default(),
        };
        let restore = app.restore();
        // Once the app has launched, over winit's own.
        #[cfg(target_os = "macos")]
        let menu_bar = iced::window::oldest().then(|id| match id {
            Some(id) => iced::window::run(id, |_| crate::menu_bar::install()).discard(),
            None => Task::none(),
        });
        #[cfg(not(target_os = "macos"))]
        let menu_bar = Task::none();
        let update = Task::done(Message::Update(update::Message::Tick));
        (
            app,
            Task::batch([restore, Window::boot(), menu_bar, update, ask_to_notify()]),
        )
    }

    /// The theme the app is drawn in.
    pub fn theme(&self) -> Theme {
        self.window.theme(&self.settings)
    }

    /// Layout units per logical pixel, which iced applies on top of the
    /// display's scale (see [`crate::text_size`]).
    pub fn scale_factor(&self) -> f32 {
        self.settings.text_size.scale()
    }

    /// The window's size in layout units.
    fn viewport(&self) -> Size {
        self.window.viewport(&self.settings)
    }

    /// Physical pixels per layout unit, for drawing images sharp.
    fn pixel_scale(&self) -> f32 {
        self.window.pixel_scale(&self.settings)
    }

    /// The theme iced fixes the window to, if any.
    pub fn window_theme(&self) -> Option<Theme> {
        self.settings.appearance.window_theme()
    }

    pub fn update(&mut self, message: Message) -> Task<Message> {
        // What messages still bring, e.g. a session restored meanwhile, is
        // dropped here, while the runtime runs (see `window::Message::Close`).
        if let Screen::Closing = self.screen {
            return Task::none();
        }
        // Unread rooms may have moved into or out of the room list's view.
        let moved = matches!(
            message,
            Message::Rooms(_)
                | Message::Start(start::Message::Ready(_))
                | Message::Sidebar(sidebar::Message::Scrolled)
                | Message::Sidebar(sidebar::Message::SelectSpace(_))
                | Message::Sidebar(sidebar::Message::Filter(_))
                | Message::Sidebar(sidebar::Message::Arrange(_))
                | Message::Sidebar(sidebar::Message::AddMenu)
                | Message::Sidebar(sidebar::Message::GoTo(_))
                | Message::ClosePopup
                | Message::ShowJoined(_)
                | Message::Pane(pane::Message::Open(_) | pane::Message::Answered(..))
                | Message::Window(window::Message::Resized(_))
        );
        let task = self.handle(message);
        let task = Task::batch([task, self.move_notes_at_top()]);
        if let Screen::Main(main) = &mut self.screen {
            main.remember_view(self.view_file.as_deref());
        }
        let badge = self.badge.show(match &self.screen {
            Screen::Main(main) if self.settings.icon_badge => badge::count(&main.lists.rooms),
            _ => 0,
        });
        let out_of_view = match &self.screen {
            Screen::Main(main) if moved => sidebar::find_out_of_view(&main.lists.rooms)
                .map(|m| Message::Sidebar(sidebar::Message::OutOfView(m))),
            _ => Task::none(),
        };
        let presence = match &mut self.screen {
            Screen::Main(main) => main.follow_shown(&self.settings),
            _ => Task::none(),
        };
        Task::batch([task, badge, out_of_view, presence])
    }

    fn handle(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Window(message) => self.window_update(message),
            Message::Start(message) => self.start(message),
            Message::Rooms(lists) => {
                let Screen::Main(main) = &mut self.screen else {
                    return Task::none();
                };
                main.lists = lists;
                let joined = main
                    .joined_space
                    .take_if(|space| main.lists.rooms.iter().any(|r| r.id == *space))
                    .map_or(Task::none(), |space| {
                        Task::done(info::show_space(space, overview::Tab::Rooms))
                    });
                Task::batch([joined, main.load_pinned(), self.restore_view()])
            }
            #[cfg(target_os = "macos")]
            Message::CheckForUpdates => {
                let check = (self.updater).update(update::Message::Check, &self.settings.updates);
                let open = self.update(Message::Modal(modal::Message::ShowSettings(true)));
                Task::batch([open, check])
            }
            Message::Update(message) => {
                let task = self.updater.update(message, &self.settings.updates);
                if let Screen::Main(main) = &mut self.screen
                    && main.modal.see_update(&self.updater, &mut self.settings)
                {
                    self.save_settings();
                }
                task
            }
            Message::Escape => match &mut self.screen {
                Screen::Main(main) => main.escape(),
                _ => Task::none(),
            },
            Message::Submit => match &self.screen {
                Screen::Main(main) => main.submit(),
                _ => Task::none(),
            },
            Message::Sidebar(_)
            | Message::Pane(_)
            | Message::Room(..)
            | Message::Shown(_)
            | Message::Card(_)
            | Message::Modal(_)
            | Message::Left(_)
            | Message::ShowJoined(_) => {
                let Screen::Main(main) = &mut self.screen else {
                    return Task::none();
                };
                let mut parts = AppParts {
                    settings: &mut self.settings,
                    google: &self.google,
                    window: &self.window,
                    updater: &mut self.updater,
                    save: false,
                };
                let task = main.update(message, &mut parts);
                if parts.save {
                    self.save_settings();
                }
                task
            }
            Message::ClosePopup => {
                if let Screen::Main(main) = &mut self.screen {
                    main.popup = None;
                }
                Task::none()
            }
            Message::People(people::Message::FrequentEmoji(emoji)) => {
                if let Screen::Main(main) = &mut self.screen {
                    main.frequent_emoji = emoji;
                }
                Task::none()
            }
            Message::People(people::Message::ContactsLoaded(contacts)) => {
                if let Screen::Main(main) = &mut self.screen {
                    if let Some(modal::Shown::Dialog(dialog)) = &mut main.modal.shown {
                        dialog.update(dialog::Message::Contacts(contacts.clone()));
                    }
                    main.contacts = contacts;
                }
                Task::none()
            }
            // Those known so far at once, then afresh.
            Message::People(people::Message::LoadContacts) => {
                let Screen::Main(main) = &mut self.screen else {
                    return Task::none();
                };
                if let Some(modal::Shown::Dialog(dialog)) = &mut main.modal.shown {
                    dialog.update(dialog::Message::Contacts(main.contacts.clone()));
                }
                let session = main.session.clone();
                Task::perform(
                    async move { session.contacts().await.unwrap_or_default() },
                    |contacts| Message::People(people::Message::ContactsLoaded(contacts)),
                )
            }
            Message::People(people::Message::LookUp(user)) => {
                let Screen::Main(main) = &self.screen else {
                    return Task::none();
                };
                if main.presences.unavailable() {
                    return Task::none();
                }
                presence::look_up(&main.session, user)
                    .map(|message| Message::People(people::Message::Presence(message)))
            }
            Message::People(people::Message::FavouritePerson(user)) => {
                let Screen::Main(main) = &self.screen else {
                    return Task::none();
                };
                let session = main.session.clone();
                Task::future(async move {
                    let room = session.direct_chat(&user).await?;
                    session.room(&room)?.set_tag(RoomTag::Favourite).await
                })
                .then(|result| match result {
                    Ok(()) => Task::none(),
                    Err(e) => Task::done(Message::Sidebar(sidebar::Message::ArrangeFailed(
                        e.to_string(),
                    ))),
                })
            }
            Message::Account(account::Message::Connection(connection)) => {
                let Screen::Main(main) = &mut self.screen else {
                    return Task::none();
                };
                let back =
                    main.connection == Connection::Offline && connection == Connection::Online;
                main.connection = connection;
                // History that could not load while offline.
                match main.pane.room_mut() {
                    Some(room) if back => room.fill(),
                    _ => Task::none(),
                }
            }
            Message::Account(account::Message::Reconnect) => {
                let Screen::Main(main) = &self.screen else {
                    return Task::none();
                };
                let session = main.session.clone();
                Task::future(async move { session.reconnect().await }).discard()
            }
            Message::People(people::Message::StartNotes) => {
                let Screen::Main(main) = &self.screen else {
                    return Task::none();
                };
                let own_user = main.own_user.clone();
                self.update(Message::People(people::Message::ChatWith(own_user)))
            }
            Message::People(people::Message::ChatWith(user)) => {
                let Screen::Main(main) = &mut self.screen else {
                    return Task::none();
                };
                main.card = None;
                let session = main.session.clone();
                task::request(async move { session.direct_chat(&user).await }, |m| {
                    Message::People(people::Message::ChatStarted(m))
                })
            }
            Message::People(people::Message::ChatStarted(Ok(room))) => {
                self.update(Message::Pane(pane::Message::Open(room)))
            }
            Message::People(people::Message::ChatStarted(Err(e))) => {
                if let Screen::Main(main) = &mut self.screen {
                    main.notices
                        .report(format!("Could not open the direct chat: {e}"), None);
                }
                Task::none()
            }
            Message::Account(account::Message::Notified(notification)) => {
                let Screen::Main(main) = &self.screen else {
                    return Task::none();
                };
                let looking = self.window.focused
                    && main
                        .pane
                        .room()
                        .is_some_and(|r| r.id == notification.room_id);
                if !self.settings.desktop_notifications || looking {
                    return Task::none();
                }
                let room = notification.room_id.clone();
                Task::future(notify(notification)).then(move |clicked| {
                    if clicked {
                        Task::done(Message::Account(account::Message::NotificationClicked(
                            room.clone(),
                        )))
                    } else {
                        Task::none()
                    }
                })
            }
            Message::Account(account::Message::NotificationClicked(room)) => {
                // Shown again if closing hid it (macOS).
                let raise = iced::window::oldest().then(|id| match id {
                    Some(id) => iced::window::mode(id).then(move |mode| {
                        if mode == iced::window::Mode::Hidden {
                            iced::window::set_mode(id, iced::window::Mode::Windowed)
                                .chain(iced::window::gain_focus(id))
                        } else {
                            iced::window::gain_focus(id)
                        }
                    }),
                    None => Task::none(),
                });
                Task::batch([raise, self.update(Message::Pane(pane::Message::Open(room)))])
            }
            Message::People(people::Message::Presence(message)) => self.presence(message),
            // First among the most used here at once, and in account data
            // (shared with Element).
            Message::People(people::Message::Used(used)) => {
                let Screen::Main(main) = &mut self.screen else {
                    return Task::none();
                };
                for emoji in &used {
                    main.frequent_emoji.retain(|e| e != emoji);
                    main.frequent_emoji.insert(0, emoji.clone());
                }
                let session = main.session.clone();
                task::background("record the emoji used", async move {
                    session.record_emoji(&used).await
                })
            }
            Message::CloseNotice => {
                if let Screen::Main(main) = &mut self.screen {
                    main.notices.close();
                }
                Task::none()
            }
            Message::Account(account::Message::Scheduled(scheduled)) => {
                if let Screen::Main(main) = &mut self.screen {
                    if let Some(room) = main.pane.room_mut() {
                        room.scheduled_changed(&scheduled, &mut main.popup);
                    }
                    // A message given a new time may have been sent (it
                    // became due) or cancelled elsewhere.
                    let gone = |id: &str| !scheduled.iter().any(|m| m.id == id);
                    main.popup.take_if(|popup| {
                        matches!(popup, Popup::SendLater(later)
                            if later.rescheduling.as_ref().is_some_and(|m| gone(&m.id)))
                    });
                    main.scheduled = scheduled;
                    main.now = SystemTime::now();
                }
                Task::none()
            }
            Message::Account(account::Message::Tick) => {
                if let Screen::Main(main) = &mut self.screen {
                    main.now = SystemTime::now();
                }
                Task::none()
            }
            Message::Account(account::Message::RecoveryKey(key)) => {
                if let Screen::Main(main) = &mut self.screen {
                    main.recovery_key = key;
                }
                Task::none()
            }
            Message::Account(account::Message::Recover) => {
                let Screen::Main(main) = &mut self.screen else {
                    return Task::none();
                };
                if main.recovery_step.is_some() || main.recovery_key.trim().is_empty() {
                    return Task::none();
                }
                main.recovery_step = Some(RecoveryStep::CheckingKey);
                main.recovery_error = None;
                let (session, key) = (main.session.clone(), main.recovery_key.trim().to_owned());
                Task::run(
                    iced::stream::channel(16, async move |mut tx| {
                        let progress = |step| {
                            let _ = tx.clone().try_send(Message::Account(
                                account::Message::RecoveryProgress(step),
                            ));
                        };
                        let result = async {
                            session.recover(&key, progress).await?;
                            session.is_device_verified().await
                        }
                        .await;
                        let _ = tx
                            .send(Message::Account(account::Message::Recovered(
                                result.map_err(|e| e.to_string()),
                            )))
                            .await;
                    }),
                    |message| message,
                )
            }
            Message::Account(account::Message::RecoveryProgress(step)) => {
                if let Screen::Main(main) = &mut self.screen
                    && main.recovery_step.is_some()
                {
                    main.recovery_step = Some(step);
                }
                Task::none()
            }
            Message::Account(account::Message::Recovered(result)) => {
                if let Screen::Main(main) = &mut self.screen {
                    main.recovery_step = None;
                    match result {
                        Ok(verified) => {
                            main.verified = Some(verified);
                            main.recovery_key.clear();
                        }
                        Err(e) => main.recovery_error = Some(e),
                    }
                }
                Task::none()
            }
            // A mention shows the person's card, with the way to the direct
            // chat; other links open in the browser.
            Message::LinkClicked(url) => match message::mentioned_user(&url) {
                Some(user) => {
                    let room = match &self.screen {
                        Screen::Main(main) => main.pane.room(),
                        _ => None,
                    };
                    let card = match room {
                        Some(room) => room.mentioned(user),
                        None => info::Target::User(mtrx_core::User::from(user)),
                    };
                    Task::done(info::show(card))
                }
                None => {
                    let _ = open::that_detached(url);
                    Task::none()
                }
            },
            Message::Media(media::Message::Downloads(message)) => {
                let Screen::Main(main) = &mut self.screen else {
                    return Task::none();
                };
                match message {
                    downloads::Message::GoTo(id, event) => {
                        main.popup
                            .take_if(|popup| matches!(popup, Popup::Downloads { .. }));
                        if main.pane.open(&id).is_some() {
                            return Task::done(room::Message::GoTo(event).to(&id));
                        }
                        let open = self.update(Message::Pane(pane::Message::Open(id.clone())));
                        if let Screen::Main(main) = &mut self.screen {
                            main.pane.go_to = Some((id, event));
                        }
                        open
                    }
                    downloads::Message::Report(problem) => {
                        main.notices.report(problem, None);
                        Task::none()
                    }
                    downloads::Message::OpenList => {
                        main.popup = Some(Popup::Downloads { menu: None });
                        Task::none()
                    }
                    downloads::Message::Menu(menu) => {
                        if let Some(Popup::Downloads { menu: open }) = &mut main.popup {
                            *open = menu;
                        }
                        Task::none()
                    }
                    message => {
                        // Chosen in a message's menu, or the viewer's.
                        if let downloads::Message::SaveAs(_) = &message {
                            main.popup.take_if(|popup| {
                                matches!(popup, Popup::MessageMenu { .. } | Popup::Viewer)
                            });
                        }
                        // A row's choice closes its menu.
                        if Downloads::chosen(&message)
                            && let Some(Popup::Downloads { menu }) = &mut main.popup
                        {
                            *menu = None;
                        }
                        let updated = matches!(message, downloads::Message::Updated(_));
                        let task = main.downloads.update(&main.session, message);
                        if updated && let Some(Popup::Downloads { menu }) = &main.popup {
                            main.popup =
                                (main.downloads.kept(*menu)).map(|menu| Popup::Downloads { menu });
                        }
                        task.map(|m| Message::Media(media::Message::Downloads(m)))
                    }
                }
            }
            Message::Media(media::Message::CopyImage(file, from)) => {
                let Screen::Main(main) = &mut self.screen else {
                    return Task::none();
                };
                close_menu(main, &from);
                Task::perform(
                    media::copy_image(main.session.clone(), file),
                    move |result| Message::Media(media::Message::Done(from.clone(), result)),
                )
            }
            Message::Media(media::Message::CopyMath(document, from)) => {
                let Screen::Main(main) = &mut self.screen else {
                    return Task::none();
                };
                close_menu(main, &from);
                Task::perform(self.math.copy_image(&document), move |result| {
                    Message::Media(media::Message::Done(from.clone(), result))
                })
            }
            Message::Media(media::Message::SaveMath(document, from)) => {
                let Screen::Main(main) = &mut self.screen else {
                    return Task::none();
                };
                close_menu(main, &from);
                let png = self.math.png(&document);
                Task::perform(
                    async move {
                        let png = png.await?;
                        let Some(file) = rfd::AsyncFileDialog::new()
                            .set_file_name("message.png")
                            .save_file()
                            .await
                        else {
                            return Ok(());
                        };
                        file.write(&png)
                            .await
                            .map_err(|e| format!("Could not save {}: {e}", file.file_name()))
                    },
                    move |result| Message::Media(media::Message::Done(from.clone(), result)),
                )
            }
            Message::Media(media::Message::Done(from, result)) => {
                let (Screen::Main(main), Err(e)) = (&mut self.screen, result) else {
                    return Task::none();
                };
                // Shown where it was asked from, else as a notice.
                let error = match &from {
                    media::Origin::Viewer => match &mut main.modal.shown {
                        Some(modal::Shown::Image(viewer)) => Some(&mut viewer.error),
                        Some(modal::Shown::Pdf(pdf)) => Some(&mut pdf.error),
                        _ => None,
                    },
                    media::Origin::Room(id) => main.pane.open_mut(id).map(|room| &mut room.error),
                };
                match error {
                    Some(error) => *error = Some(e),
                    None => main.notices.report(e, None),
                }
                Task::none()
            }
            Message::Media(message) => self.media(message),
        }
    }

    /// Saves the settings; the open panel says if that failed.
    fn save_settings(&mut self) {
        let error = self
            .settings_file
            .as_ref()
            .and_then(|file| self.settings.save(file).err())
            .map(|e| e.to_string());
        if let Some(panel) = self.settings_panel() {
            panel.error = error;
        }
    }

    fn presence(&mut self, message: presence::Message) -> Task<Message> {
        let Screen::Main(main) = &mut self.screen else {
            return Task::none();
        };
        match message {
            presence::Message::Input => main.activity(mtrx_core::Activity::Input),
            message => {
                main.presences.update(message);
                Task::none()
            }
        }
    }

    /// The settings panel, while open.
    fn settings_panel(&mut self) -> Option<&mut settings::Panel> {
        match &mut self.screen {
            Screen::Main(main) => main.modal.settings(),
            _ => None,
        }
    }

    /// The PDF viewer follows the window's size, the display's scale and
    /// the text size. (Math does too, by the keys of its pictures.)
    fn pdf_resized(&mut self) -> Task<Message> {
        let (viewport, scale) = (self.viewport(), self.pixel_scale());
        match &mut self.screen {
            Screen::Main(main) => match main.modal.pdf() {
                Some(pdf) => pdf.resized(viewport, scale),
                None => Task::none(),
            },
            _ => Task::none(),
        }
    }

    /// Show the space and room of last time once the room is listed, until
    /// the first sync shows it is gone.
    fn restore_view(&mut self) -> Task<Message> {
        let Screen::Main(main) = &mut self.screen else {
            return Task::none();
        };
        let Some(view) = &main.restore else {
            return Task::none();
        };
        match view.found(&main.lists.rooms) {
            Some((space, room)) => {
                main.restore = None;
                main.sidebar.select(space);
                self.update(Message::Pane(pane::Message::Open(room)))
            }
            None => {
                if main.lists.loaded {
                    main.restore = None;
                }
                Task::none()
            }
        }
    }

    /// Once synced, an account without "Notes at the top" gets this
    /// desktop's earlier choice; asked once a run.
    fn move_notes_at_top(&mut self) -> Task<Message> {
        let Screen::Main(main) = &mut self.screen else {
            return Task::none();
        };
        if main.notes_moved
            || !main.lists.loaded
            || main.sidebar.arrange.categories.notes_at_top.is_some()
        {
            return Task::none();
        }
        main.notes_moved = true;
        Task::done(Message::Sidebar(sidebar::Message::Arrange(
            arrange::Message::NotesAtTop(self.settings.pinned_notes),
        )))
    }

    /// Load what a view asks for, unless it is loaded or loading, and keep
    /// what was loaded.
    fn media(&mut self, message: media::Message) -> Task<Message> {
        use media::Message::{
            AvatarLoaded, AvatarWanted, MathRendered, MathWanted, PreviewLoaded, PreviewWanted,
        };
        match (message, &mut self.screen) {
            (MathWanted(request), _) => self.math.request(request, |key, rendered| {
                Message::Media(MathRendered(key, rendered))
            }),
            (MathRendered(key, rendered), _) => {
                self.math.insert(key, rendered);
                Task::none()
            }
            (AvatarWanted(key), Screen::Main(main)) => {
                main.avatars.request(&main.session, key, |key, handle| {
                    Message::Media(AvatarLoaded(key, handle))
                })
            }
            (AvatarLoaded(key, handle), Screen::Main(main)) => {
                main.avatars.insert(key, handle);
                Task::none()
            }
            (PreviewWanted(media), Screen::Main(main)) => {
                main.previews
                    .request(&main.session, &media, |media, preview| {
                        Message::Media(PreviewLoaded(media, preview))
                    })
            }
            (PreviewLoaded(media, preview), Screen::Main(main)) => {
                main.previews.insert(media, preview);
                Task::none()
            }
            // Signed out since.
            (AvatarWanted(_) | AvatarLoaded(..) | PreviewWanted(_) | PreviewLoaded(..), _) => {
                Task::none()
            }
            // See `App::handle`.
            (
                media::Message::CopyImage(..)
                | media::Message::CopyMath(..)
                | media::Message::SaveMath(..)
                | media::Message::Done(..)
                | media::Message::Downloads(_),
                _,
            ) => Task::none(),
        }
    }

    pub fn subscription(&self) -> Subscription<Message> {
        let mut subscriptions = vec![
            self.window.subscription(),
            iced::time::every(update::TICK).map(|_| Message::Update(update::Message::Tick)),
        ];
        // Settings… and Check for Updates… in the app menu.
        #[cfg(target_os = "macos")]
        subscriptions.push(crate::menu_bar::chosen().map(|chosen| match chosen {
            crate::menu_bar::Chosen::Settings => Message::Modal(modal::Message::ShowSettings(true)),
            crate::menu_bar::Chosen::CheckForUpdates => Message::CheckForUpdates,
        }));
        if let Screen::Main(main) = &self.screen {
            subscriptions.push(
                Subscription::run_with(Shared(main.session.clone()), |s| {
                    s.0.scheduled_updates().updates()
                })
                .map(|m| Message::Account(account::Message::Scheduled(m))),
            );
            subscriptions.push(
                Subscription::run_with(
                    (Shared(main.session.clone()), "notifications"),
                    |(s, _)| s.0.notifications(),
                )
                .map(|m| Message::Account(account::Message::Notified(m))),
            );
            subscriptions.push(iced::event::listen_with(shortcuts));
            // A selection being dragged ends wherever the button is released.
            if main
                .pane
                .room()
                .and_then(|r| r.selection.as_ref())
                .is_some_and(|s| s.dragging)
            {
                subscriptions.push(iced::event::listen_with(|event, _, _| match event {
                    iced::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => Some(
                        Message::Shown(room::Message::Select(selection::Message::End)),
                    ),
                    _ => None,
                }));
            }
            subscriptions.push(
                Subscription::run_with(Shared(main.session.clone()), |s| {
                    s.0.connection_updates().updates()
                })
                .map(|m| Message::Account(account::Message::Connection(m))),
            );
            subscriptions.push(
                Subscription::run_with(Shared(main.session.clone()), |s| {
                    s.0.room_categories_updates().updates()
                })
                .map(|c| {
                    Message::Sidebar(sidebar::Message::Arrange(arrange::Message::Categories(c)))
                }),
            );
            subscriptions.push(main.sidebar.arrange.subscription());
            subscriptions.push(
                Subscription::run_with(Shared(main.session.clone()), |s| {
                    s.0.room_list_colours_updates().updates()
                })
                .map(|m| Message::Sidebar(sidebar::Message::Colours(m))),
            );
            // Times shown relative to now: scheduled messages, last activity.
            if !main.scheduled.is_empty() || main.presences.following() {
                subscriptions.push(
                    iced::time::every(TICK).map(|_| Message::Account(account::Message::Tick)),
                );
            }
            if let Some(watcher) = main.presences.subscription(&main.session) {
                subscriptions.push(watcher.map(|m| Message::People(people::Message::Presence(m))));
            }
            if self.settings.share_presence && !main.presences.unavailable() {
                subscriptions.push(iced::event::listen_with(input));
            }
            if let Some(modal::Shown::Dialog(dialog)) = &main.modal.shown
                && dialog.busy()
            {
                subscriptions.push(
                    iced::time::every(PROGRESS_TICK)
                        .map(|_| Message::Modal(modal::Message::Progress)),
                );
            }
            if let Some(lists) = &main.room_lists {
                subscriptions.push(
                    Subscription::run_with(Shared(lists.clone()), |lists| lists.0.updates())
                        .map(Message::Rooms),
                );
            }
            if let Some(card) = &main.card {
                subscriptions.push(card.subscription());
            }
            if let Some(room) = main.pane.room() {
                subscriptions.push(Subscription::run_with(
                    (Shared(room.timeline.clone()), "typing"),
                    |(t, _)| {
                        let id = t.0.room_id().clone();
                        t.0.typing_updates()
                            .updates()
                            .map(move |names| room::Message::Typing(names).to(&id))
                    },
                ));
                subscriptions.push(Subscription::run_with(Shared(room.timeline.clone()), |t| {
                    let id = t.0.room_id().clone();
                    t.0.rows()
                        .updates()
                        .map(move |rows| room::Message::Timeline(rows).to(&id))
                }));
                subscriptions.push(Subscription::run_with(
                    (Shared(room.timeline.clone()), "uploads"),
                    |(t, _)| {
                        let (timeline, id) = (t.0.clone(), t.0.room_id().clone());
                        iced::futures::stream::once(async move { timeline.uploads().await })
                            .filter_map(|uploads| async move { uploads.ok() })
                            .flat_map(|uploads| uploads.updates())
                            .map(move |uploads| room::Message::Uploads(uploads).to(&id))
                    },
                ));
                if let Some(pinned) = &room.pins.timeline {
                    subscriptions.push(Subscription::run_with(Shared(pinned.clone()), |t| {
                        let id = t.0.room_id().clone();
                        t.0.updates().map(move |entries| {
                            room::Message::Pins(pins::Message::Updated(entries)).to(&id)
                        })
                    }));
                }
                subscriptions.push(media::dropped_files());
            }
            subscriptions.push(
                downloads::subscription(&main.session)
                    .map(|m| Message::Media(media::Message::Downloads(m))),
            );
        }
        Subscription::batch(subscriptions)
    }

    pub fn view(&self) -> Element<'_, Message> {
        match &self.screen {
            Screen::Starting => container(text("Starting…")).center(Length::Fill).into(),
            Screen::Closing => container("").into(),
            Screen::Login(login) => login
                .view()
                .map(|m| Message::Start(start::Message::Login(m))),
            // The main view is always the stack's first layer, so opening
            // and closing what floats over it keeps its widgets' state.
            Screen::Main(main) => {
                let mut layers = stack![self.main_view(main)];
                if let Some(card) = &main.card {
                    let card = card.view(info::Shown {
                        lists: &main.lists,
                        avatars: &main.avatars,
                        own_user: &main.own_user,
                        presences: &main.presences,
                        now: main.now,
                        pinned_notes: main.sidebar.arrange.categories.notes_at_top(),
                        call_services: &self.settings.call_services,
                        room_pictures: self.settings.room_pictures,
                        popup: main.popup.as_ref(),
                        space_rooms: &main.sidebar.space_rooms,
                    });
                    layers = layers.push(panel(card, Message::Card(info::Message::Close), true));
                }
                let modal = match &main.modal.shown {
                    None => return layers.into(),
                    Some(modal::Shown::Settings(settings)) => {
                        let spaces = (main.lists.top_level_spaces().into_iter())
                            .map(|s| (s.id.clone(), s.name.clone()))
                            .collect();
                        let view = settings.view(
                            &self.settings,
                            spaces,
                            &main.sidebar.colours,
                            &main.sidebar.arrange.categories,
                            !main.presences.unavailable(),
                            &self.updater,
                        );
                        panel(
                            view,
                            Message::Modal(modal::Message::ShowSettings(false)),
                            true,
                        )
                    }
                    Some(modal::Shown::Dialog(dialog)) => panel(
                        dialog.view(&main.lists, &main.sidebar.arrange, &self.settings.room_list),
                        Message::Modal(modal::Message::Dialog(dialog::Message::Cancel)),
                        false,
                    ),
                    Some(modal::Shown::Join(join)) => panel(
                        join.view(&main.lists, &main.avatars, self.settings.room_pictures),
                        Message::Modal(modal::Message::Join(join::Message::Close)),
                        true,
                    ),
                    // Fill the window, with their own backdrop.
                    Some(modal::Shown::Image(viewer)) => opaque(
                        viewer.view(&main.previews, matches!(main.popup, Some(Popup::Viewer))),
                    ),
                    Some(modal::Shown::Pdf(pdf)) => opaque(pdf.view()),
                };
                layers.push(modal).into()
            }
        }
    }

    fn main_view<'a>(&'a self, main: &'a Main) -> Element<'a, Message> {
        let theme = self.theme();
        let rooms = &main.lists.rooms;
        let space_bar = sidebar::space_bar(
            &main.lists,
            main.sidebar.space.as_ref(),
            &self.settings.room_list,
            &main.avatars,
            match &main.popup {
                Some(Popup::SpaceMenu(menu)) => Some(menu),
                _ => None,
            },
            self.updater.unseen(&self.settings.updates),
            match &main.popup {
                Some(Popup::Downloads { menu }) => main.downloads.view(true, *menu),
                _ => main.downloads.view(false, None),
            },
        );
        let room_list = sidebar::room_list(sidebar::RoomListView {
            // As wide as a room's below its name.
            topic_width: (self.window.room_width(&self.settings) * room::TOPIC_SHARE)
                .max(popover::MIN_SHARED_WIDTH),
            ..main.room_list_view(&self.settings)
        });

        // Banners come and go inside a column that is always there, so the
        // room view keeps its place and its widgets their state (the
        // composer its focus).
        let mut banners = column![];
        if main.verified == Some(false) {
            banners = banners.push(recovery_banner(main));
        }
        if let Some(banner) = overdue_banner(main) {
            banners = banners.push(banner);
        }
        if let Some(notice) = &main.notices.shown {
            banners = banners.push(notice_bar(notice));
        }
        let mut content = column![banners];
        let room_view: Element<'a, Message> = match &main.pane.shown {
            Shown::Invite { id, answering } => {
                pane::invite_view(rooms.iter().find(|r| &r.id == id), id, *answering)
            }
            Shown::Opening(id) => {
                let opening =
                    container(text("Opening…").style(text::secondary)).center(Length::Fill);
                // The member list stays where it is, loading.
                if main.pane.show_members {
                    let listed = rooms.iter().find(|r| &r.id == id);
                    row![
                        opening,
                        members::view(
                            None,
                            listed.map_or("", |r| r.name.as_str()),
                            listed.is_some_and(|r| r.rights.invite),
                            members::Shown {
                                room: id,
                                avatars: &main.avatars,
                                presences: &main.presences,
                                menu: Popup::member_menu(main.popup.as_ref()),
                            },
                        )
                    ]
                    .into()
                } else {
                    opening.into()
                }
            }
            Shown::Open(room) => {
                let summary = rooms.iter().find(|r| r.id == room.id);
                let scheduled = send_later::Pending {
                    messages: main
                        .scheduled
                        .iter()
                        .filter(|m| m.room_id == room.id)
                        .collect(),
                    now: main.now,
                };
                let look = Look {
                    own_user: &main.own_user,
                    theme: theme.clone(),
                    math: &self.math,
                    params: self.window.math_params(&self.settings),
                    avatars: &main.avatars,
                    colored_names: self.settings.colored_names,
                    call_services: &self.settings.call_services,
                    previews: &main.previews,
                    frequent_emoji: &main.frequent_emoji,
                    show_members: main.pane.show_members,
                    popup: main.popup.as_ref(),
                    presences: &main.presences,
                    now: main.now,
                };
                room.view(summary, &look, &scheduled)
            }
            Shown::Empty => container(text("Select a room").style(text::secondary))
                .center(Length::Fill)
                .into(),
        };
        content = content.push(room_view);

        // The room list's edge, dragged to resize it.
        let handle = mouse_area(
            container(iced::widget::space())
                .width(window::RESIZE_HANDLE)
                .height(Length::Fill),
        )
        .interaction(mouse::Interaction::ResizingHorizontally)
        .on_press(Message::Window(window::Message::Resize(Resize::Start)));
        let main_row = row![space_bar, room_list, handle, content.width(Length::Fill)];
        // Bars and the drag preview come after the rooms, keeping them in
        // place.
        let mut window = column![main_row];
        match main.connection {
            Connection::Online => {}
            Connection::Offline => window = window.push(offline_bar()),
            Connection::SignedOut => window = window.push(signed_out_bar()),
        }
        let mut layers = stack![window];
        if let Some(preview) = sidebar::drag_preview(
            &main.lists,
            main.sidebar.space.as_ref(),
            &main.sidebar.arrange,
            &main.avatars,
        ) {
            layers = layers.push(preview);
        }
        layers.into()
    }
}

/// A panel over what is below, on a backdrop that closes it with `close`.
/// Panels whose content loads and changes (`anchored`) keep their top
/// where it is; dialogs are centred.
fn panel<'a>(panel: Element<'a, Message>, close: Message, anchored: bool) -> Element<'a, Message> {
    let placed = if anchored {
        container(opaque(panel))
            .center_x(Length::Fill)
            .height(Length::Fill)
            .align_top(Length::Fill)
            .padding(iced::Padding::new(24.0).top(PANEL_TOP))
    } else {
        container(opaque(panel)).center(Length::Fill)
    };
    opaque(mouse_area(placed.style(style::backdrop)).on_press(close))
}

fn recovery_banner(main: &Main) -> Element<'_, Message> {
    let busy = main.recovery_step.is_some();
    let mut input = text_input("Recovery key", &main.recovery_key)
        .style(style::input)
        .secure(true)
        .padding(8)
        .width(320);
    if !busy {
        input = input
            .on_input(|m| Message::Account(account::Message::RecoveryKey(m)))
            .on_submit(Message::Account(account::Message::Recover));
    }
    let unlock = button(text(if busy { "Unlocking…" } else { "Unlock" }))
        .style(style::primary_button)
        .on_press_maybe((!busy).then_some(Message::Account(account::Message::Recover)));
    let mut banner = column![
        text("This device is not verified, so older encrypted messages cannot be read."),
        row![input, unlock].spacing(8),
    ]
    .spacing(8);
    if let Some(step) = main.recovery_step {
        let status = match step {
            RecoveryStep::CheckingKey => "Checking the recovery key…".to_owned(),
            RecoveryStep::ImportingIdentity => {
                "Verifying this device and connecting to the key backup…".to_owned()
            }
            RecoveryStep::DownloadingKeys { done, total } => {
                format!("Downloading message keys for recent rooms: {done} of {total}…")
            }
        };
        banner = banner.push(text(status).size(13));
    }
    if let Some(error) = &main.recovery_error {
        banner = banner.push(text(error).style(text::danger));
    }
    container(
        container(banner)
            .padding(12)
            .width(Length::Fill)
            .style(style::banner),
    )
    .padding([8, 24])
    .into()
}

/// Points to rooms (other than the open one) with overdue scheduled
/// messages; their composer bar offers sending or cancelling them.
fn overdue_banner(main: &Main) -> Option<Element<'_, Message>> {
    let open = main.pane.id();
    let mut rooms: Vec<(&OwnedRoomId, usize)> = Vec::new();
    for message in main.scheduled.iter().filter(|m| m.overdue) {
        if Some(&message.room_id) == open {
            continue;
        }
        match rooms.iter_mut().find(|(id, _)| *id == &message.room_id) {
            Some((_, count)) => *count += 1,
            None => rooms.push((&message.room_id, 1)),
        }
    }
    if rooms.is_empty() {
        return None;
    }
    let mut banner = row![text("Scheduled messages were not sent on time:")]
        .spacing(8)
        .align_y(iced::Alignment::Center);
    for (id, count) in rooms {
        let name = (main.lists.room(id)).map_or(id.as_str(), |r| r.name.as_str());
        let label = if count > 1 {
            format!("{name} ({count})")
        } else {
            name.to_owned()
        };
        banner = banner.push(
            button(text(label).size(13))
                .padding([2, 8])
                .style(style::secondary_button)
                .on_press(Message::Pane(pane::Message::Open(id.clone()))),
        );
    }
    Some(
        container(
            container(banner)
                .padding([8, 12])
                .width(Length::Fill)
                .style(style::banner),
        )
        .padding([8, 24])
        .into(),
    )
}

/// Something finished in the background, or failed.
/// Along the bottom, near the composer, while the server is unreachable.
fn offline_bar<'a>() -> Element<'a, Message> {
    container(
        row![
            text("Offline – reconnecting…")
                .size(13)
                .font(crate::fonts::SEMIBOLD),
            text("Messages are sent once the server is back.")
                .size(13)
                .width(Length::Fill),
            button(text("Retry now").size(13))
                .padding([3, 10])
                .style(style::problem_button)
                .on_press(Message::Account(account::Message::Reconnect)),
        ]
        .spacing(10)
        .align_y(iced::Alignment::Center),
    )
    .padding([6, 16])
    .width(Length::Fill)
    .style(style::offline_bar)
    .into()
}

/// Along the bottom once the server no longer accepts the session.
fn signed_out_bar<'a>() -> Element<'a, Message> {
    container(
        row![
            text("Your session has ended")
                .size(13)
                .font(crate::fonts::SEMIBOLD),
            text("The server no longer accepts this login: it expired or was signed out elsewhere.")
                .size(13)
                .width(Length::Fill),
            hint(
                button(text("Sign in again").size(13))
                    .padding([3, 10])
                    .style(style::problem_button)
                    .on_press(Message::Start(start::Message::SignInAgain)),
                "Remove this device's data and log in again (your recovery key unlocks older messages)",
                Position::Top,
            ),
        ]
        .spacing(10)
        .align_y(iced::Alignment::Center),
    )
    .padding([6, 16])
    .width(Length::Fill)
    .style(style::offline_bar)
    .into()
}

fn notice_bar(notice: &Notice) -> Element<'_, Message> {
    let mut bar = row![
        text(&notice.text)
            .size(13)
            .style(if notice.error {
                text::danger
            } else {
                text::default
            })
            .width(Length::Fill)
    ]
    .spacing(8)
    .align_y(iced::Alignment::Center);
    if let Some(room) = &notice.room {
        bar = bar.push(
            button(text("Open").size(13))
                .padding([3, 10])
                .style(style::primary_button)
                .on_press(Message::Pane(pane::Message::Open(room.clone()))),
        );
    }
    bar = bar.push(
        button(text("Dismiss").size(13))
            .padding([3, 8])
            .style(style::text_button)
            .on_press(Message::CloseNotice),
    );
    container(container(bar).padding([6, 12]).style(style::code_block))
        .padding([8, 24])
        .into()
}

/// ⌘K / Ctrl+K focuses the room search, Esc closes what floats on top or
/// clears the search, ⌘↩ submits a dialog, ⌘C, ⌘A and Shift with an arrow act on the timeline's selection;
/// window focus.
fn shortcuts(
    event: iced::Event,
    status: iced::event::Status,
    _window: iced::window::Id,
) -> Option<Message> {
    use iced::keyboard::{self, Key, key::Named};
    match as_key(event) {
        iced::Event::Keyboard(keyboard::Event::KeyPressed {
            key,
            physical_key,
            modifiers,
            ..
        }) if let Some(go) = sidebar::Go::from_key(&key, physical_key, modifiers) => {
            Some(Message::Sidebar(sidebar::Message::GoTo(go)))
        }
        iced::Event::Keyboard(keyboard::Event::KeyPressed {
            key,
            modifiers,
            text,
            ..
        }) => match key {
            // Unless a text field copies its own selection.
            Key::Character(c)
                if c == "c" && modifiers.command() && status == iced::event::Status::Ignored =>
            {
                Some(Message::Shown(room::Message::Select(
                    selection::Message::Copy,
                )))
            }
            Key::Character(c)
                if c == "a" && modifiers.command() && status == iced::event::Status::Ignored =>
            {
                Some(Message::Shown(room::Message::Select(
                    selection::Message::SelectAll,
                )))
            }
            Key::Named(
                arrow @ (Named::ArrowLeft | Named::ArrowRight | Named::ArrowUp | Named::ArrowDown),
            ) if status == iced::event::Status::Ignored => arrow_key(arrow, modifiers),
            Key::Named(page @ (Named::PageUp | Named::PageDown))
                if status == iced::event::Status::Ignored =>
            {
                Some(Message::Shown(room::Message::ScrollKey(
                    if page == Named::PageUp {
                        room::Scroll::PageUp
                    } else {
                        room::Scroll::PageDown
                    },
                )))
            }
            Key::Character(c) if c == "k" && modifiers.command() => {
                Some(Message::Sidebar(sidebar::Message::FocusSearch))
            }
            Key::Character(c) if c == "f" && modifiers.command() => {
                Some(Message::Shown(room::Message::Find(find::Message::Open)))
            }
            Key::Character(c) if c == "g" && modifiers.command() => {
                Some(Message::Shown(room::Message::Find(if modifiers.shift() {
                    find::Message::Previous
                } else {
                    find::Message::Next
                })))
            }
            Key::Character(c) if c == "r" && modifiers.command() => {
                Some(Message::Pane(pane::Message::Reload))
            }
            Key::Named(Named::Escape) => Some(Message::Escape),
            Key::Named(Named::Enter) if modifiers.command() => Some(Message::Submit),
            // Typed with nothing focused: a character, not a shortcut.
            _ if status == iced::event::Status::Ignored
                && !modifiers.command()
                && !modifiers.control() =>
            {
                text.filter(|text| !text.is_empty() && !text.chars().any(char::is_control))
                    .map(|text| Message::Shown(room::Message::TypeInComposer(text.to_string())))
            }
            _ => None,
        },
        iced::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => Some(
            Message::Sidebar(sidebar::Message::Arrange(arrange::Message::Pressed)),
        ),
        iced::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => Some(
            Message::Sidebar(sidebar::Message::Arrange(arrange::Message::Released)),
        ),
        iced::Event::Window(iced::window::Event::Focused) => {
            Some(Message::Window(window::Message::Focused(true)))
        }
        iced::Event::Window(iced::window::Event::Unfocused) => {
            Some(Message::Window(window::Message::Focused(false)))
        }
        _ => None,
    }
}

/// A standard action of the menus as the key press it stands for (macOS's
/// menu takes the key, see `menu_bar`): ⌘C for Copy, ⌘F for Find, ⌘= for
/// Bigger. Other events as they are.
fn as_key(event: iced::Event) -> iced::Event {
    match event {
        iced::Event::StandardAction(action) => iced::Event::Keyboard(action.key_press()),
        event => event,
    }
}

/// An arrow key no field took. ⌥↑ and ⌥↓ scroll the timeline a page, ⌘↑
/// and ⌘↓ to the oldest and newest message. With Shift it moves the end of
/// the timeline's selection: by a character (⌥: a word) or a line, and with
/// ⌘ to the edge of its line or of the screen.
fn arrow_key(
    arrow: iced::keyboard::key::Named,
    modifiers: iced::keyboard::Modifiers,
) -> Option<Message> {
    use iced::keyboard::key::Named;
    use selection::Message::{Extend, ExtendLine, ExtendToLineEdge, ExtendToScreenEdge};
    let forward = matches!(arrow, Named::ArrowRight | Named::ArrowDown);
    let across = matches!(arrow, Named::ArrowLeft | Named::ArrowRight);
    if !modifiers.shift() {
        let scroll = match (modifiers.alt(), modifiers.command(), forward) {
            (true, false, false) => room::Scroll::PageUp,
            (true, false, true) => room::Scroll::PageDown,
            (false, true, false) => room::Scroll::Oldest,
            (false, true, true) => room::Scroll::Newest,
            _ => return None,
        };
        return (!across).then_some(Message::Shown(room::Message::ScrollKey(scroll)));
    }
    let message = match (across, modifiers.command()) {
        (true, true) => ExtendToLineEdge(forward),
        (false, true) => ExtendToScreenEdge(!forward),
        (true, false) => Extend(match (forward, modifiers.alt()) {
            (false, false) => selection::Step::Back,
            (true, false) => selection::Step::Forward,
            (false, true) => selection::Step::WordBack,
            (true, true) => selection::Step::WordForward,
        }),
        (false, false) => ExtendLine(!forward),
    };
    Some(Message::Shown(room::Message::Select(message)))
}

/// Keys, clicks and scrolling: we are here (see [`mtrx_core::Activity::Input`]).
fn input(
    event: iced::Event,
    _status: iced::event::Status,
    _window: iced::window::Id,
) -> Option<Message> {
    use iced::keyboard;
    match event {
        iced::Event::Keyboard(keyboard::Event::KeyPressed { .. })
        | iced::Event::StandardAction(_)
        | iced::Event::Mouse(mouse::Event::ButtonPressed(_) | mouse::Event::WheelScrolled { .. }) => {
            Some(Message::People(people::Message::Presence(
                presence::Message::Input,
            )))
        }
        _ => None,
    }
}

/// A menu's choice closes it: the menu of a message in the room, or the
/// viewer's.
fn close_menu(main: &mut Main, from: &media::Origin) {
    match from {
        media::Origin::Viewer => {
            main.popup.take_if(|popup| matches!(popup, Popup::Viewer));
        }
        media::Origin::Room(id) => {
            if main.pane.open(id).is_some() {
                main.popup
                    .take_if(|popup| matches!(popup, Popup::MessageMenu { .. }));
            }
        }
    }
}

/// macOS asks once whether Mtrx may show notifications; only for the app
/// bundle (`Mtrx.app`), as macOS sends them for bundles only.
fn ask_to_notify() -> Task<Message> {
    #[cfg(target_os = "macos")]
    return Task::future(async {
        if let Err(e) = notify_rust::request_auth().await {
            tracing::warn!("notifications not allowed: {e}");
        }
    })
    .discard();
    #[cfg(not(target_os = "macos"))]
    Task::none()
}

/// How long a notification waits to be clicked (macOS): it may be clicked
/// in Notification Center long after its banner, but "Clear All" there
/// never answers, and each waits on a thread of its own.
#[cfg(target_os = "macos")]
const NOTIFICATION_WAIT: u32 = 12 * 60 * 60 * 1000;

/// Show a system notification; true when it was clicked, also later from
/// Notification Center (macOS). The calls block, so they run off the UI
/// thread.
async fn notify(notification: Notification) -> bool {
    const MAX_BODY: usize = 200;
    let sender = notification.sender.display_name();
    let title = if notification.direct {
        sender
    } else {
        format!("{sender} in {}", notification.room_name)
    };
    let body: String = notification.body.chars().take(MAX_BODY).collect();
    let shown = tokio::task::spawn_blocking(move || {
        let mut desktop = notify_rust::Notification::new();
        desktop.appname("Mtrx").summary(&title).body(&body);
        #[cfg(windows)]
        crate::windows::notification_sender(&mut desktop);
        #[cfg(target_os = "macos")]
        desktop.timeout(notify_rust::Timeout::Milliseconds(NOTIFICATION_WAIT));
        // Freedesktop servers report clicks only for a "default" action.
        #[cfg(all(unix, not(target_os = "macos")))]
        desktop.action("default", "Open");
        let mut clicked = false;
        desktop
            .show()?
            .wait_for_response(|response: &notify_rust::NotificationResponse| {
                clicked = matches!(response, notify_rust::NotificationResponse::Default);
            })?;
        Ok::<_, notify_rust::error::Error>(clicked)
    })
    .await;
    match shown {
        Ok(Ok(clicked)) => clicked,
        Ok(Err(e)) => {
            tracing::warn!("could not show a notification: {e}");
            false
        }
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests;
