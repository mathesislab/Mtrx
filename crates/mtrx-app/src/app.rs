//! Application state, messages and top-level layout.

pub mod account;
pub mod modal;
pub mod pane;
pub mod people;
pub mod start;
pub mod window;

pub use self::window::closing;
use self::window::{Resize, Window};

use std::{
    collections::{BTreeSet, HashMap},
    fmt,
    hash::{Hash, Hasher},
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant, SystemTime},
};

use futures_util::{SinkExt, StreamExt};
use iced::{
    Element, Length, Size, Subscription, Task, Theme, mouse,
    widget::{button, column, container, mouse_area, opaque, row, stack, text, text_input},
};
use mtrx_core::{
    Connection, Contact, DownloadId, Live, Membership, Notification, OwnedEventId, OwnedRoomId,
    OwnedRoomOrAliasId, OwnedServerName, OwnedUserId, Profile, RecoveryStep, RoomKind,
    RoomListColours, RoomListState, RoomLists, RoomOrAliasId, RoomTag, ScheduledMessage,
    SecretStore, Session, SpaceKey,
};
use mtrx_google::Google;

use crate::{
    actions::PickerAt,
    arrange::{self, Arrange},
    avatar::Avatars,
    badge::{self, Badge},
    calls,
    dialog::{self, Dialog},
    directory,
    downloads::{self, Downloads},
    edit_menu, emoji,
    find::{self},
    hint::{Position, hint},
    info, join,
    last_view::LastView,
    login::Login,
    math::Math,
    media::{self, Previews, Viewer},
    members, message,
    message_field::Field,
    message_menu::MenuAt,
    overview, pdf_viewer, pins,
    placement::Placement,
    popover,
    presence::{self, Presences},
    room::{self, Load, Look, Room},
    selection::{self},
    send_later,
    settings::{self, Settings},
    sidebar, spelling, style,
    task::{self, Owned},
    tint::Tint,
    topic,
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
    space: Option<OwnedRoomId>,
    pane: Pane,
    /// The member list is shown beside the open room, whichever it is.
    show_members: bool,
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
    modal: Option<Modal>,
    /// The menu or picker open, anywhere in the window.
    popup: Option<Popup>,
    /// A space just joined, whose overview shows once it is in the room
    /// list.
    joined_space: Option<OwnedRoomId>,
    /// The last dialog request made.
    last_request: dialog::RequestId,
    /// Dialogs closed while their requests run: notices for success and
    /// failure.
    background: HashMap<dialog::RequestId, (String, String)>,
    /// The rooms of our spaces, for the room search.
    space_rooms: directory::SpaceRooms,
    /// The selected space's topic panel, below its name.
    space_topic: topic::Panel,
    /// This desktop's "Notes at the top" was offered to the account.
    notes_moved: bool,
    /// Something finished in the background, or failed.
    notice: Option<Notice>,
    /// The room search text.
    filter: String,
    own_user: OwnedUserId,
    previews: Previews,
    /// Categories, tags and dragging in the room list.
    arrange: Arrange,
    /// The info card shown: a panel to come back to, below a modal.
    card: Option<info::Card>,
    /// People the user knows, closest first; refreshed when a dialog
    /// picking people opens.
    contacts: Vec<Contact>,
    /// The emoji the user reacts with, most used first; refreshed when a
    /// reaction picker opens.
    frequent_emoji: Vec<String>,
    connection: Connection,
    /// The room list's colours, from the account.
    colours: RoomListColours,
    /// Unread rooms scrolled out of the room list's view.
    out_of_view: sidebar::OutOfView,
    /// The view to show again once its room is listed (after starting).
    restore: Option<LastView>,
    /// Others' presence, for the people shown.
    presences: Presences,
    downloads: Downloads,
    /// A message to show once its room opens ("Go to message").
    go_to: Option<(OwnedRoomId, OwnedEventId)>,
}

/// What the room pane shows.
// One, in the boxed [`Main`]: boxing the room would only add indirection.
#[allow(clippy::large_enum_variant)]
enum Pane {
    Empty,
    /// Its timeline loads.
    Opening(OwnedRoomId),
    /// Accept and Decline instead of a timeline; `answering` while it is answered.
    Invite {
        id: OwnedRoomId,
        answering: bool,
    },
    Open(Room),
}

impl Pane {
    /// The open room.
    fn room(&self) -> Option<&Room> {
        match self {
            Pane::Open(room) => Some(room),
            _ => None,
        }
    }

    fn room_mut(&mut self) -> Option<&mut Room> {
        match self {
            Pane::Open(room) => Some(room),
            _ => None,
        }
    }

    /// Room `id`, if it is the one open.
    fn open(&self, id: &OwnedRoomId) -> Option<&Room> {
        self.room().filter(|room| room.id == *id)
    }

    fn open_mut(&mut self, id: &OwnedRoomId) -> Option<&mut Room> {
        self.room_mut().filter(|room| room.id == *id)
    }

    /// The room shown: opening, invited to or open.
    fn id(&self) -> Option<&OwnedRoomId> {
        match self {
            Pane::Empty => None,
            Pane::Opening(id) | Pane::Invite { id, .. } => Some(id),
            Pane::Open(room) => Some(&room.id),
        }
    }
}

/// What shows over the whole window, above the card: one at a time, the
/// one opened last replacing any other.
// One, in the boxed [`Main`].
#[allow(clippy::large_enum_variant)]
enum Modal {
    Settings(settings::Panel),
    Dialog(Dialog),
    /// Joining a space or room from elsewhere.
    Join(join::Join),
    /// An image shown large.
    Image(Viewer),
    /// A PDF shown large.
    Pdf(pdf_viewer::Viewer),
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
    fn of_card(&self) -> bool {
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
    pub google: &'a Arc<Google>,
    /// The menu or picker open in the window.
    pub popup: &'a mut Option<Popup>,
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
            space: None,
            pane: Pane::Empty,
            show_members: false,
            verified: None,
            recovery_key: String::new(),
            recovery_step: None,
            recovery_error: None,
            scheduled: session.scheduled(),
            own_user: session.user_id(),
            arrange: Arrange::new(&session),
            card: None,
            contacts: Vec::new(),
            frequent_emoji: Vec::new(),
            connection: session.connection(),
            colours: session.room_list_colours(),
            out_of_view: Default::default(),
            restore: None,
            presences: Presences::default(),
            downloads: Downloads::default(),
            go_to: None,
            now: SystemTime::now(),
            session,
            avatars: Avatars::new(),
            modal: None,
            popup: None,
            joined_space: None,
            last_request: Default::default(),
            background: HashMap::new(),
            space_rooms: Default::default(),
            space_topic: Default::default(),
            notes_moved: false,
            notice: None,
            filter: String::new(),
            previews: Previews::new(),
        }
    }
}

impl Main {
    /// Say that something failed outside a room (where its status pill
    /// would say it): in the notice bar, until closed or replaced. With
    /// `room`, the room it concerns, offered to open (again).
    fn report(&mut self, problem: String, room: Option<OwnedRoomId>) {
        self.notice = Some(Notice {
            text: problem,
            room,
            error: true,
        });
    }

    /// Show `pane`: the popup of the room shown before closes, and its
    /// draft is kept now.
    fn set_pane(&mut self, pane: Pane) -> Task<Message> {
        let left = std::mem::replace(&mut self.pane, pane);
        if let Some(room) = left.id() {
            self.popup.take_if(|popup| popup.of_room(room));
        }
        match left {
            Pane::Open(room) => room::keep_now(room.drafts.clone(), room.draft()),
            _ => Task::none(),
        }
    }

    /// A message for room `id`: to it while it is open, else what still
    /// concerns it.
    fn room(
        &mut self,
        id: OwnedRoomId,
        message: room::Message,
        google: &Arc<Google>,
    ) -> Task<Message> {
        let Main {
            session,
            own_user,
            lists,
            connection,
            popup,
            pane,
            ..
        } = self;
        let Some(room) = pane.open_mut(&id) else {
            return room::left(session, id, message);
        };
        let mut cx = Cx {
            session,
            own_user,
            lists,
            connection: *connection,
            google,
            popup,
        };
        room.update(message, &mut cx)
    }

    /// Make the open room's pinned messages' timeline, once it has pins.
    fn load_pinned(&mut self) -> Task<Message> {
        match self.pane.room_mut() {
            Some(room) => room.load_pinned(&self.session, &self.lists),
            None => Task::none(),
        }
    }

    fn room_list_view<'a>(&'a self, settings: &'a Settings) -> sidebar::RoomListView<'a> {
        let (grouping, width) = (settings.room_list, settings.room_list_width);
        sidebar::RoomListView {
            lists: &self.lists,
            selected_space: self.space.as_ref(),
            open: self.pane.id(),
            loaded: self.lists.loaded,
            avatars: &self.avatars,
            scheduled: &self.scheduled,
            popup: self.popup.as_ref(),
            filter: &self.filter,
            arrange: &self.arrange,
            grouping,
            out_of_view: self.out_of_view,
            width,
            tint: Tint::of(&self.colours, self.space.as_ref()),
            room_pictures: settings.room_pictures,
            notes: self.pinned_notes(),
            presences: &self.presences,
            space_rooms: &self.space_rooms,
            space_topic: &self.space_topic,
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
        if self.show_members
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
                | Message::ClosePopup
                | Message::Pane(pane::Message::Open(_))
                | Message::Window(window::Message::Resized(_))
        );
        // A menu's choice closes it, as does what opens over it: so an
        // open popup is always on top.
        if let Screen::Main(main) = &mut self.screen
            && matches!(
                message,
                Message::Modal(modal::Message::OpenDialog(_))
                    | Message::Modal(modal::Message::OpenJoin)
                    | Message::Modal(modal::Message::ShowSettings(true))
                    | Message::Modal(modal::Message::OpenImage(_))
                    | Message::Modal(modal::Message::OpenPdf(_))
                    | Message::Card(info::Message::Show(..))
                    | Message::Sidebar(sidebar::Message::SelectSpace(_))
                    | Message::Pane(pane::Message::Open(_))
            )
        {
            main.popup = None;
        }
        let remember = matches!(
            message,
            Message::Pane(pane::Message::Open(_))
                | Message::Sidebar(sidebar::Message::SelectSpace(_))
        );
        let card = self.card_shown();
        let space = self.selected_space();
        let task = self.handle(message);
        // A member's menu and the card's go with the card they were opened
        // on.
        if self.card_shown() != card
            && let Screen::Main(main) = &mut self.screen
        {
            main.popup.take_if(|popup| popup.of_card());
        }
        // Another space, however chosen: its topic, closed.
        let selected = self.selected_space();
        if selected.is_some()
            && selected != space
            && let Screen::Main(main) = &mut self.screen
        {
            main.space_topic = topic::Panel::default();
        }
        let task = Task::batch([task, self.move_notes_at_top()]);
        if remember {
            self.remember_view();
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
            Message::Sidebar(sidebar::Message::Scrolled) => Task::none(),
            Message::Sidebar(sidebar::Message::OutOfView(out_of_view)) => {
                if let Screen::Main(main) = &mut self.screen {
                    main.out_of_view = out_of_view;
                }
                Task::none()
            }
            Message::Sidebar(sidebar::Message::ScrollTo(offset)) => sidebar::scroll_to(offset),
            Message::Sidebar(sidebar::Message::Colours(colours)) => {
                if let Screen::Main(main) = &mut self.screen {
                    main.colours = colours;
                }
                Task::none()
            }
            Message::Sidebar(sidebar::Message::ColoursMoved(result)) => {
                match result {
                    Ok(()) => {
                        self.settings.legacy_home_tint = None;
                        self.settings.legacy_space_tints.clear();
                        if let Some(file) = &self.settings_file
                            && let Err(e) = self.settings.save(file)
                        {
                            tracing::warn!("could not save settings: {e}");
                        }
                    }
                    Err(e) => tracing::warn!("could not move room list colours: {e}"),
                }
                Task::none()
            }
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
            Message::Sidebar(sidebar::Message::SelectSpace(space)) => {
                let Screen::Main(main) = &mut self.screen else {
                    return Task::none();
                };
                main.space = space;
                main.card = None;
                main.restore = None;
                main.arrange.escape();
                Task::none()
            }
            Message::Sidebar(sidebar::Message::Arrange(message)) => {
                let Screen::Main(main) = &mut self.screen else {
                    return Task::none();
                };
                let context = arrange::Context {
                    session: &main.session,
                    space: main.space.as_ref(),
                    rooms: &main.lists.rooms,
                    grouping: self.settings.room_list,
                    popup: &mut main.popup,
                };
                main.arrange.update(message, context)
            }
            Message::Sidebar(sidebar::Message::ArrangeFailed(e)) => {
                if let Screen::Main(main) = &mut self.screen {
                    main.report(format!("Could not rearrange the room list: {e}"), None);
                }
                Task::none()
            }
            Message::Pane(pane::Message::Open(id)) => {
                let Screen::Main(main) = &mut self.screen else {
                    return Task::none();
                };
                main.card = None;
                // Chosen now: last time's view is no longer wanted.
                main.restore = None;
                main.go_to.take_if(|(room, _)| *room != id);
                if main.pane.open(&id).is_some() {
                    return Task::none();
                }
                // An invite shows Accept and Decline instead of a timeline.
                let invited = |r: &mtrx_core::RoomSummary| r.membership == Membership::Invited;
                if main.lists.room(&id).is_some_and(invited) {
                    return main.set_pane(Pane::Invite {
                        id,
                        answering: false,
                    });
                }
                let left = main.set_pane(Pane::Opening(id.clone()));
                let session = main.session.clone();
                let open = task::request(
                    {
                        let id = id.clone();
                        async move { session.timeline(&id).await.map(|t| Shared(Arc::new(t))) }
                    },
                    move |result| Message::Pane(pane::Message::Opened(id, result)),
                );
                Task::batch([left, open])
            }
            Message::Pane(pane::Message::Opened(id, result)) => {
                let Screen::Main(main) = &mut self.screen else {
                    return Task::none();
                };
                if !matches!(&main.pane, Pane::Opening(opening) if *opening == id) {
                    // Another room or an invite was chosen meanwhile.
                    return Task::none();
                }
                let opened = result.and_then(|timeline| {
                    let handle = main.session.room(&id).map_err(|e| e.to_string())?;
                    Ok((handle, timeline))
                });
                match opened {
                    Ok((handle, Shared(timeline))) => {
                        // It opened after all.
                        main.notice
                            .take_if(|notice| notice.error && notice.room.as_ref() == Some(&id));
                        let drafts = main.session.draft_keeper(&id);
                        let mut room = Room::new(handle, timeline, drafts);
                        let paginate = room.fill();
                        let read = room.mark_read();
                        // The member list stays open from the last room.
                        let members = if main.show_members {
                            room.load_members()
                        } else {
                            Task::none()
                        };
                        let draft = room.load_draft();
                        let call_service = room.load_call_service(&main.session);
                        let opening = main.set_pane(Pane::Open(room));
                        let go_to = match main.go_to.take() {
                            Some((room, event)) if room == id => {
                                Task::done(room::Message::GoTo(event).to(&id))
                            }
                            _ => Task::none(),
                        };
                        Task::batch([
                            opening,
                            paginate,
                            read,
                            members,
                            draft,
                            go_to,
                            call_service,
                            main.load_pinned(),
                        ])
                    }
                    Err(e) => {
                        let opening = main.set_pane(Pane::Empty);
                        main.report(format!("Could not open the room: {e}"), Some(id));
                        opening
                    }
                }
            }
            Message::Modal(modal::Message::ShowSettings(false)) => {
                if let Screen::Main(main) = &mut self.screen {
                    main.modal
                        .take_if(|modal| matches!(modal, Modal::Settings(_)));
                }
                Task::none()
            }
            Message::Modal(modal::Message::ShowSettings(true)) => self.open_settings(),
            Message::Modal(modal::Message::Settings(message)) => self.settings_update(message),
            #[cfg(target_os = "macos")]
            Message::CheckForUpdates => {
                let check = (self.updater).update(update::Message::Check, &self.settings.updates);
                Task::batch([self.open_settings(), check])
            }
            Message::Update(message) => {
                let task = self.updater.update(message, &self.settings.updates);
                self.update_seen();
                task
            }
            Message::Room(id, message) => match &mut self.screen {
                Screen::Main(main) => main.room(id, message, &self.google),
                _ => Task::none(),
            },
            // Typed with nothing focused goes to the composer, unless
            // something shows over it.
            Message::Shown(room::Message::TypeInComposer(_)) if self.covered() => Task::none(),
            Message::Shown(message) => match &mut self.screen {
                Screen::Main(main) => match main.pane.room() {
                    Some(room) => {
                        let id = room.id.clone();
                        main.room(id, message, &self.google)
                    }
                    None => Task::none(),
                },
                _ => Task::none(),
            },
            Message::Sidebar(sidebar::Message::SpaceTopic(space, message)) => {
                self.space_topic(space, message)
            }
            Message::Sidebar(sidebar::Message::AddMenu) => {
                if let Screen::Main(main) = &mut self.screen {
                    main.popup = Some(Popup::AddMenu);
                }
                Task::none()
            }
            Message::Sidebar(sidebar::Message::SpaceMenu(menu)) => {
                if let Screen::Main(main) = &mut self.screen {
                    main.popup = Some(Popup::SpaceMenu(menu));
                }
                Task::none()
            }
            Message::ClosePopup => {
                if let Screen::Main(main) = &mut self.screen {
                    main.popup = None;
                }
                Task::none()
            }
            Message::Modal(modal::Message::OpenDialog(kind)) => {
                let Screen::Main(main) = &mut self.screen else {
                    return Task::none();
                };
                // A card stays below, e.g. a space's overview, to come
                // back to.
                let near = near_spaces(&kind, &main.lists.graph, main.space.as_ref());
                main.arrange.escape();
                let mut dialog = Dialog::new(kind);
                dialog.fill_category(&main.arrange.categories);
                if !dialog.picks_people() {
                    main.modal = Some(Modal::Dialog(dialog));
                    return Task::none();
                }
                dialog.prefer_spaces(near);
                dialog.offer_self(main.own_user.clone());
                dialog.update(dialog::Message::Contacts(main.contacts.clone()));
                let session = main.session.clone();
                let owners = if dialog.creates_room() {
                    let session = session.clone();
                    dialog.offer_owners(Task::perform(
                        async move { session.offers_owners().await },
                        |offered| {
                            Message::Modal(modal::Message::Dialog(dialog::Message::OwnersOffered(
                                offered,
                            )))
                        },
                    ))
                } else {
                    Task::none()
                };
                main.modal = Some(Modal::Dialog(dialog));
                Task::batch([
                    Task::perform(
                        async move { session.contacts().await.unwrap_or_default() },
                        |m| Message::People(people::Message::ContactsLoaded(m)),
                    ),
                    owners,
                ])
            }
            Message::People(people::Message::FrequentEmoji(emoji)) => {
                if let Screen::Main(main) = &mut self.screen {
                    main.frequent_emoji = emoji;
                }
                Task::none()
            }
            Message::People(people::Message::ContactsLoaded(contacts)) => {
                if let Screen::Main(main) = &mut self.screen {
                    if let Some(Modal::Dialog(dialog)) = &mut main.modal {
                        dialog.update(dialog::Message::Contacts(contacts.clone()));
                    }
                    main.contacts = contacts;
                }
                Task::none()
            }
            Message::Card(info::Message::Show(target, tab)) => self.show_card(target, tab),
            Message::Modal(modal::Message::OpenJoin) => {
                let Screen::Main(main) = &mut self.screen else {
                    return Task::none();
                };
                main.card = None;
                let session = main.session.clone();
                let (join, action) = join::Join::new(
                    session.user_id().server_name().to_owned(),
                    &session.directory_servers().servers,
                );
                main.modal = Some(Modal::Join(join));
                Task::batch([
                    self.join_action(action),
                    iced::widget::operation::focus(join::SEARCH),
                ])
            }
            Message::Modal(modal::Message::Join(message)) => {
                let Screen::Main(main) = &mut self.screen else {
                    return Task::none();
                };
                let Some(Modal::Join(join)) = &mut main.modal else {
                    return Task::none();
                };
                // A server whose directory answered is remembered.
                let remember = match &message {
                    join::Message::Found(search, Ok(_)) => search.server.clone().map(|server| {
                        let session = main.session.clone();
                        task::background("remember a directory server", async move {
                            session.remember_directory_server(&server).await
                        })
                    }),
                    _ => None,
                };
                let action = join.update(message);
                Task::batch([
                    remember.unwrap_or_else(Task::none),
                    self.join_action(action),
                ])
            }
            Message::Card(message) => {
                let Screen::Main(main) = &mut self.screen else {
                    return Task::none();
                };
                let Some(card) = &mut main.card else {
                    return Task::none();
                };
                let overview_session = main.session.clone();
                let room = card.handle.clone();
                let saved = |result| Message::Card(info::Message::Saved(result));
                // A menu's choice closes it.
                match &message {
                    info::Message::OpenPictureMenu => {
                        main.popup = Some(Popup::CardPicture);
                        return Task::none();
                    }
                    info::Message::OpenSpacesMenu => {
                        main.popup = Some(Popup::CardAddToSpace);
                        return Task::none();
                    }
                    info::Message::ChangePicture | info::Message::RemovePicture => {
                        main.popup
                            .take_if(|popup| matches!(popup, Popup::CardPicture));
                    }
                    info::Message::Change(_) => {
                        main.popup
                            .take_if(|popup| matches!(popup, Popup::CardAddToSpace));
                    }
                    _ => {}
                }
                let task = match (&message, room) {
                    (info::Message::Close, _) => {
                        main.card = None;
                        return Task::none();
                    }
                    (info::Message::Copy(text), _) => iced::clipboard::write(text.clone()),
                    (info::Message::SaveDetails, Some(room)) => {
                        let Some(summary) = main.lists.rooms.iter().find(|r| r.id == room.id())
                        else {
                            return Task::none();
                        };
                        let (name, topic) = card.changed_details(summary);
                        task::request(
                            async move {
                                if let Some(name) = name {
                                    room.set_name(&name).await?;
                                }
                                if let Some(topic) = topic {
                                    room.set_topic(&topic).await?;
                                }
                                Ok(())
                            },
                            saved,
                        )
                    }
                    (info::Message::JoinRule(rule), Some(room)) => {
                        let rule = *rule;
                        task::request(async move { room.set_join_rule(rule).await }, saved)
                    }
                    (info::Message::Notify(mode), Some(room)) => {
                        let mode = *mode;
                        task::request(async move { room.set_notify(mode).await }, saved)
                    }
                    (info::Message::ChangePicture, Some(room)) => task::request(
                        async move {
                            let Some(file) = rfd::AsyncFileDialog::new()
                                .set_title("Room picture")
                                .add_filter("Images", &["png", "jpg", "jpeg", "gif", "webp"])
                                .pick_file()
                                .await
                            else {
                                return Ok(());
                            };
                            room.set_picture(file.path()).await
                        },
                        saved,
                    ),
                    (info::Message::RemovePicture, Some(room)) => {
                        task::request(async move { room.remove_picture().await }, saved)
                    }
                    (info::Message::Change(change), Some(room)) => {
                        let change = change.clone();
                        let session = main.session.clone();
                        task::request(
                            async move {
                                use info::Change;
                                match change {
                                    Change::Address(name) => {
                                        room.set_address(name.as_deref()).await
                                    }
                                    Change::Listed(on) => room.set_listed(on).await,
                                    Change::History(h) => room.set_history(h).await,
                                    Change::Permissions(p) => room.set_permissions(p).await,
                                    Change::Suggested(space, on) => {
                                        session.room(&space)?.set_suggested(room.id(), on).await
                                    }
                                    Change::AddToSpace(space) => {
                                        session.room(&space)?.add_child(room.id()).await
                                    }
                                    Change::RemoveFromSpace(space) => {
                                        session.room(&space)?.remove_child(room.id()).await
                                    }
                                    Change::Unban(user) => room.unban(&user).await,
                                }
                            },
                            saved,
                        )
                    }
                    (info::Message::LetEveryoneEdit, Some(room)) => task::request(
                        async move {
                            let update = room.let_everyone_edit_all().await?;
                            let skipped = match update.not_allowed.len() {
                                0 => String::new(),
                                n => format!(
                                    " {n} could not be changed: you are not an admin there."
                                ),
                            };
                            let opened = update.changed.len();
                            Ok(format!("Opened {opened} room(s).{skipped}"))
                        },
                        |result| Message::Card(info::Message::Opened(result)),
                    ),
                    _ => Task::none(),
                };
                let overview = match card.update(message) {
                    overview::Action::None => Task::none(),
                    action => match &card.overview {
                        Some(o) => overview_task(
                            &overview_session,
                            &mut main.space_rooms,
                            &mut card.tasks,
                            &o.space,
                            action,
                        ),
                        None => Task::none(),
                    },
                };
                Task::batch([task, overview])
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
            Message::Pane(pane::Message::Reload) => {
                let Screen::Main(main) = &mut self.screen else {
                    return Task::none();
                };
                match main.connection {
                    Connection::Offline => {
                        return self.update(Message::Account(account::Message::Reconnect));
                    }
                    // Its bar offers signing in again.
                    Connection::SignedOut => return Task::none(),
                    Connection::Online => {}
                }
                let Some(room) = main.pane.room_mut() else {
                    return Task::none();
                };
                let (session, id) = (main.session.clone(), room.id.clone());
                room.tasks.load(
                    Load::Reload,
                    {
                        let id = id.clone();
                        async move { session.timeline(&id).await.map(|t| Shared(Arc::new(t))) }
                    },
                    move |result| room::Message::Reloaded(result).to(&id),
                )
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
            Message::Modal(modal::Message::Dialog(message)) => self.dialog(message),
            Message::Pane(pane::Message::Answer(id, accept)) => {
                let Screen::Main(main) = &mut self.screen else {
                    return Task::none();
                };
                let answering = main.set_pane(Pane::Invite {
                    id: id.clone(),
                    answering: true,
                });
                let session = main.session.clone();
                let answer = task::request(
                    {
                        let id = id.clone();
                        async move {
                            if accept {
                                session
                                    .join(<&RoomOrAliasId>::from(&*id), &[])
                                    .await
                                    .map(|_| ())
                            } else {
                                session.room(&id)?.leave().await
                            }
                        }
                    },
                    move |result| Message::Pane(pane::Message::Answered(id, accept, result)),
                );
                Task::batch([answering, answer])
            }
            Message::Pane(pane::Message::Answered(id, accept, result)) => {
                let Screen::Main(main) = &mut self.screen else {
                    return Task::none();
                };
                let answered = if matches!(&main.pane, Pane::Invite { id: invite, .. } if *invite == id)
                {
                    main.set_pane(Pane::Empty)
                } else {
                    Task::none()
                };
                let task = match result {
                    Ok(()) if accept => self.update(Message::Pane(pane::Message::Open(id))),
                    Ok(()) => Task::none(),
                    Err(e) => {
                        let answer = if accept { "accept" } else { "decline" };
                        main.report(format!("Could not {answer} the invite: {e}"), None);
                        Task::none()
                    }
                };
                Task::batch([answered, task])
            }
            Message::Sidebar(sidebar::Message::Filter(filter)) => {
                let Screen::Main(main) = &mut self.screen else {
                    return Task::none();
                };
                // A search starting looks through our spaces' rooms anew.
                let load = if main.filter.trim().is_empty() && !filter.trim().is_empty() {
                    load_space_rooms(main)
                } else {
                    Task::none()
                };
                main.filter = filter;
                load
            }
            Message::Sidebar(sidebar::Message::SpaceRoomsLoaded(space, result)) => {
                if let Screen::Main(main) = &mut self.screen {
                    main.space_rooms.loaded(space, result);
                }
                Task::none()
            }
            Message::Sidebar(sidebar::Message::JoinFound(room, via, knock)) => {
                let Screen::Main(main) = &mut self.screen else {
                    return Task::none();
                };
                main.space_rooms.busy.insert(room.clone());
                let join = join_listed(main.session.clone(), room.clone().into(), via, knock);
                task::request(join, move |result| {
                    Message::Sidebar(sidebar::Message::FoundJoined(
                        room,
                        knock,
                        result.map(|_| ()),
                    ))
                })
            }
            Message::Sidebar(sidebar::Message::FoundJoined(room, knock, result)) => {
                let Screen::Main(main) = &mut self.screen else {
                    return Task::none();
                };
                main.space_rooms.busy.remove(&room);
                match result {
                    Ok(()) if knock => Task::none(),
                    Ok(()) => self.update(Message::Pane(pane::Message::Open(room))),
                    Err(e) => {
                        main.report(format!("Could not join: {e}"), None);
                        Task::none()
                    }
                }
            }
            Message::Sidebar(sidebar::Message::OpenFirstMatch) => {
                let Screen::Main(main) = &mut self.screen else {
                    return Task::none();
                };
                let key = (main.space.clone()).map_or(SpaceKey::Home, SpaceKey::Space);
                let found = (main.lists).search(Some(&key), &self.settings.room_list, &main.filter);
                match found.first() {
                    Some(room) => {
                        let id = room.id.clone();
                        main.filter.clear();
                        self.update(Message::Pane(pane::Message::Open(id)))
                    }
                    None => Task::none(),
                }
            }
            Message::Sidebar(sidebar::Message::GoTo(go)) => {
                let Screen::Main(main) = &self.screen else {
                    return Task::none();
                };
                match sidebar::go_to(&main.room_list_view(&self.settings), go) {
                    Some(room) => {
                        let id = room.id.clone();
                        self.update(Message::Pane(pane::Message::Open(id)))
                    }
                    None => Task::none(),
                }
            }
            Message::Sidebar(sidebar::Message::FocusSearch) => {
                iced::widget::operation::focus(sidebar::SEARCH)
            }
            // Closes the topmost thing: the popup (on top, as what opens
            // over it closes it), the modal, the card, then in the open room
            // the mention offered, the find bar, the selection, the reply
            // and the message edited in place; then a drag or category
            // name, else the search.
            Message::Escape => {
                let Screen::Main(main) = &mut self.screen else {
                    return Task::none();
                };
                // The innermost first.
                if let Some(popup) = &mut main.popup {
                    if !popup.escape() {
                        main.popup = None;
                    }
                    return Task::none();
                }
                match &mut main.modal {
                    Some(Modal::Dialog(_)) => {
                        return Task::done(Message::Modal(modal::Message::Dialog(
                            dialog::Message::Cancel,
                        )));
                    }
                    Some(Modal::Join(_)) => {
                        return Task::done(Message::Modal(modal::Message::Join(
                            join::Message::Close,
                        )));
                    }
                    Some(_) => main.modal = None,
                    None if main.card.is_some() => main.card = None,
                    None => {
                        let escaped = (main.pane.room_mut())
                            .and_then(|room| room.escape(&main.own_user, &main.lists));
                        if let Some(task) = escaped {
                            return task;
                        }
                        if !main.arrange.escape() {
                            main.filter.clear();
                        }
                    }
                }
                Task::none()
            }
            Message::Submit => {
                let Screen::Main(main) = &self.screen else {
                    return Task::none();
                };
                let submit = match (&main.modal, &main.card) {
                    (Some(Modal::Dialog(_)), _) => Some(Message::Modal(modal::Message::Dialog(
                        dialog::Message::Submit,
                    ))),
                    (Some(_), _) => None,
                    (None, card) => card
                        .as_ref()
                        .and_then(|card| card.submit(&main.lists.rooms)),
                };
                submit.map_or_else(Task::none, Task::done)
            }
            Message::Pane(pane::Message::MemberMenu(menu)) => {
                if let Screen::Main(main) = &mut self.screen {
                    main.popup = Some(Popup::MemberMenu(menu));
                }
                Task::none()
            }
            Message::Pane(pane::Message::SetLevel(room, user, level)) => {
                let Screen::Main(main) = &mut self.screen else {
                    return Task::none();
                };
                main.popup.take_if(|p| matches!(p, Popup::MemberMenu(_)));
                let session = main.session.clone();
                task::request(
                    {
                        let room = room.clone();
                        async move { session.room(&room)?.set_level(&user, level).await }
                    },
                    move |result| Message::Pane(pane::Message::MembersChanged(room, result)),
                )
            }
            Message::Pane(pane::Message::MembersChanged(room, result)) => {
                if let (Err(e), Screen::Main(main)) = (&result, &mut self.screen) {
                    main.report(format!("Could not change the members: {e}"), None);
                }
                self.refresh_members(&room)
            }
            Message::Pane(pane::Message::ShowMembers(id)) => {
                let Screen::Main(main) = &mut self.screen else {
                    return Task::none();
                };
                main.show_members = true;
                main.card = None;
                match main.pane.open_mut(&id) {
                    Some(room) if room.tasks.running(&Load::Members) => Task::none(),
                    Some(room) => room.load_members(),
                    None => Task::done(Message::Pane(pane::Message::Open(id))),
                }
            }
            Message::Pane(pane::Message::ToggleMembers) => {
                let Screen::Main(main) = &mut self.screen else {
                    return Task::none();
                };
                main.show_members = !main.show_members;
                // Opening it shows the members anew.
                match main.pane.room_mut() {
                    Some(room) if main.show_members && !room.tasks.running(&Load::Members) => {
                        room.load_members()
                    }
                    _ => Task::none(),
                }
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
                    main.report(format!("Could not open the direct chat: {e}"), None);
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
                    main.notice = None;
                }
                Task::none()
            }
            Message::Modal(modal::Message::Progress) => {
                if let Screen::Main(main) = &mut self.screen
                    && let Some(Modal::Dialog(dialog)) = &mut main.modal
                {
                    dialog.tick(Instant::now());
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
            Message::Modal(modal::Message::OpenImage(file)) => {
                let Screen::Main(main) = &mut self.screen else {
                    return Task::none();
                };
                let mut viewer = Viewer::new(file);
                let load = viewer.load(main.session.clone());
                main.modal = Some(Modal::Image(viewer));
                load
            }
            Message::Modal(modal::Message::OpenViewerMenu) => {
                if let Screen::Main(main) = &mut self.screen
                    && let Some(Modal::Image(_)) = &main.modal
                {
                    main.popup = Some(Popup::Viewer);
                }
                Task::none()
            }
            Message::Modal(modal::Message::CloseImage) => {
                if let Screen::Main(main) = &mut self.screen {
                    main.modal
                        .take_if(|modal| matches!(modal, Modal::Image(_) | Modal::Pdf(_)));
                    main.popup.take_if(|popup| matches!(popup, Popup::Viewer));
                }
                Task::none()
            }
            Message::Modal(modal::Message::OpenPdf(file)) => {
                let mut viewer = pdf_viewer::Viewer::new(file, self.viewport(), self.pixel_scale());
                let Screen::Main(main) = &mut self.screen else {
                    return Task::none();
                };
                let open = viewer.open(main.session.clone());
                main.modal = Some(Modal::Pdf(viewer));
                open
            }
            Message::Modal(modal::Message::Pdf(message)) => match &mut self.screen {
                Screen::Main(main) => match &mut main.modal {
                    Some(Modal::Pdf(viewer)) => viewer.update(message),
                    _ => Task::none(),
                },
                _ => Task::none(),
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
                            main.go_to = Some((id, event));
                        }
                        open
                    }
                    downloads::Message::Report(problem) => {
                        main.report(problem, None);
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
                    media::Origin::Viewer => match &mut main.modal {
                        Some(Modal::Image(viewer)) => Some(&mut viewer.error),
                        Some(Modal::Pdf(pdf)) => Some(&mut pdf.error),
                        _ => None,
                    },
                    media::Origin::Room(id) => main.pane.open_mut(id).map(|room| &mut room.error),
                };
                match error {
                    Some(error) => *error = Some(e),
                    None => main.report(e, None),
                }
                Task::none()
            }
            Message::Media(message) => self.media(message),
            Message::Modal(modal::Message::ImageLoaded(media, result)) => {
                if let Screen::Main(main) = &mut self.screen
                    && let Some(Modal::Image(viewer)) = &mut main.modal
                {
                    viewer.loaded(&media, result);
                }
                Task::none()
            }
        }
    }

    fn dialog(&mut self, message: dialog::Message) -> Task<Message> {
        let Screen::Main(main) = &mut self.screen else {
            return Task::none();
        };
        // The request of a dialog closed while it ran: what it changed, and
        // a notice, which opens what it made.
        let message = match message {
            dialog::Message::Done(id, result) if main.background.contains_key(&id) => {
                let (done, failed) = main.background.remove(&id).unwrap_or_default();
                return match result {
                    Ok(opened) => {
                        main.notice = Some(Notice {
                            text: done,
                            room: match &opened {
                                dialog::Opened::Room(id) | dialog::Opened::Space(id) => {
                                    Some(id.clone())
                                }
                                _ => None,
                            },
                            error: false,
                        });
                        self.request_done(opened)
                    }
                    Err(e) => {
                        main.report(format!("{failed}: {e}"), None);
                        Task::none()
                    }
                };
            }
            message => message,
        };
        let Some(Modal::Dialog(dialog)) = &mut main.modal else {
            return Task::none();
        };
        let session = main.session.clone();
        match dialog.update(message) {
            dialog::Action::None => Task::none(),
            dialog::Action::Close => {
                main.modal = None;
                Task::none()
            }
            dialog::Action::Background(id) => {
                main.background.insert(id, dialog.outcome());
                main.modal = None;
                Task::none()
            }
            dialog::Action::Search(query) => dialog.search_users(Task::perform(
                {
                    let query = query.clone();
                    async move { session.search_users(&query, 10).await.unwrap_or_default() }
                },
                move |users| {
                    Message::Modal(modal::Message::Dialog(dialog::Message::Suggestions(
                        query, users,
                    )))
                },
            )),
            dialog::Action::Arrange(message) => {
                main.modal = None;
                self.update(Message::Sidebar(sidebar::Message::Arrange(message)))
            }
            dialog::Action::Submit(request) => {
                let id = main.last_request.next();
                main.last_request = id;
                dialog.sent(id);
                Task::perform(run_request(session, request), move |result| {
                    Message::Modal(modal::Message::Dialog(dialog::Message::Done(id, result)))
                })
            }
            dialog::Action::Finished(opened) => {
                main.modal = None;
                let show = match &opened {
                    dialog::Opened::Room(id) => {
                        self.update(Message::Pane(pane::Message::Open(id.clone())))
                    }
                    dialog::Opened::Space(id) => self.update(Message::Sidebar(
                        sidebar::Message::SelectSpace(Some(id.clone())),
                    )),
                    _ => Task::none(),
                };
                Task::batch([self.request_done(opened), show])
            }
        }
    }

    /// What a dialog's request changed, shown, whether the dialog still
    /// shows or not: a room left closes, signing out ends the session.
    fn request_done(&mut self, opened: dialog::Opened) -> Task<Message> {
        let Screen::Main(main) = &mut self.screen else {
            return Task::none();
        };
        match opened {
            dialog::Opened::Nothing | dialog::Opened::Room(_) | dialog::Opened::Space(_) => {
                Task::none()
            }
            dialog::Opened::Members(room) => self.refresh_members(&room),
            dialog::Opened::Left(id) => {
                let left = if main.pane.id() == Some(&id) {
                    main.set_pane(Pane::Empty)
                } else {
                    Task::none()
                };
                if main.card.as_ref().and_then(|c| c.room()) == Some(&id) {
                    main.card = None;
                }
                if main.space.as_ref() == Some(&id) {
                    main.space = None;
                }
                left
            }
            dialog::Opened::SignedOut => {
                self.screen = Screen::Login(Login::default());
                Task::none()
            }
        }
    }

    fn open_settings(&mut self) -> Task<Message> {
        let mut panel = settings::Panel::default();
        panel.google.account = self.google.account().unwrap_or_else(|e| {
            panel.google.error = Some(e.to_string());
            None
        });
        let mut tasks = Vec::new();
        if let Screen::Main(main) = &self.screen {
            panel.account = Some(main.session.user_id().to_string());
            let name = |id: &OwnedRoomId| {
                (main.lists.room(id)).map_or_else(|| id.to_string(), |r| r.name.clone())
            };
            let rights =
                |id: &OwnedRoomId| main.lists.room(id).map(|r| r.rights).unwrap_or_default();
            if let Some(space) = &main.space {
                panel.space = Some(settings::SpaceCalls::new(
                    space.clone(),
                    name(space),
                    rights(space),
                ));
                let (session, space) = (main.session.clone(), space.clone());
                tasks.push(panel.load_space(Task::perform(
                    async move {
                        let service = match session.room(&space) {
                            Ok(room) => room.call_service().await.ok().flatten(),
                            Err(_) => None,
                        };
                        settings::Message::SpaceLoaded(space, service)
                    },
                    |m| Message::Modal(modal::Message::Settings(m)),
                )));
            }
        }
        if let Screen::Main(main) = &mut self.screen {
            main.modal = Some(Modal::Settings(panel));
        }
        self.update_seen();
        Task::batch(tasks)
    }

    /// An update shown in the open settings is seen: no dot on the gear
    /// until a newer one.
    fn update_seen(&mut self) {
        if self.settings_panel().is_none() || !self.updater.unseen(&self.settings.updates) {
            return;
        }
        let version = (self.updater.available.as_ref()).map(|u| u.version.to_string());
        self.settings.updates.seen = version;
        self.save_settings();
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

    fn settings_update(&mut self, message: settings::Message) -> Task<Message> {
        // Preferences change also from outside the panel, e.g. a card.
        let panel = match &mut self.screen {
            Screen::Main(main) => match &mut main.modal {
                Some(Modal::Settings(panel)) => Some(panel),
                _ => None,
            },
            _ => None,
        };
        let action = match (message, panel) {
            (settings::Message::Preference(preference), _) => {
                settings::change(&mut self.settings, preference)
            }
            (message, Some(panel)) => panel.update(&mut self.settings, message),
            (_, None) => return Task::none(),
        };
        match action {
            settings::Action::None => Task::none(),
            settings::Action::Save => {
                self.save_settings();
                Task::none()
            }
            settings::Action::CheckUpdates => {
                self.save_settings();
                self.updater
                    .update(update::Message::Check, &self.settings.updates)
            }
            settings::Action::Appearance => {
                self.save_settings();
                #[cfg(target_os = "macos")]
                self.settings.appearance.apply();
                Task::none()
            }
            settings::Action::TextSize => {
                self.save_settings();
                self.pdf_resized()
            }
            settings::Action::Spelling => {
                self.save_settings();
                spelling::set_enabled(self.settings.check_spelling);
                Task::none()
            }
            settings::Action::SharePresence => {
                self.save_settings();
                let Screen::Main(main) = &self.screen else {
                    return Task::none();
                };
                // The switch is off while the server does not share presence.
                presence::sharing(&main.session, self.settings.share_presence)
                    .map(|m| Message::People(people::Message::Presence(m)))
            }
            settings::Action::GoogleSignIn(client) => {
                let google = self.google.clone();
                let sign_in = Task::perform(
                    async move {
                        google
                            .sign_in(client, |url| {
                                let _ = open::that_detached(url.as_str());
                            })
                            .await
                    },
                    |result| {
                        Message::Modal(modal::Message::Settings(settings::Message::GoogleSignedIn(
                            result.map_err(|e| e.to_string()),
                        )))
                    },
                );
                // Asked for from the panel, which shows it signing in.
                match self.settings_panel() {
                    Some(panel) => panel.google_sign_in(sign_in),
                    None => Task::none(),
                }
            }
            settings::Action::Colour(space, tint) => {
                let Screen::Main(main) = &self.screen else {
                    return Task::none();
                };
                let session = main.session.clone();
                task::request(
                    async move {
                        session
                            .change_room_list_colours(|colours| {
                                let name = tint.name().to_owned();
                                match space {
                                    Some(space) => {
                                        colours.spaces.insert(space, name);
                                    }
                                    None => colours.home = Some(name),
                                }
                            })
                            .await
                    },
                    |result| {
                        Message::Modal(modal::Message::Settings(settings::Message::ColourSaved(
                            result,
                        )))
                    },
                )
            }
            settings::Action::AtHome(at_home) => self.update(Message::Sidebar(
                sidebar::Message::Arrange(arrange::Message::AtHome(at_home)),
            )),
            settings::Action::NotesAtTop(on) => self.update(Message::Sidebar(
                sidebar::Message::Arrange(arrange::Message::NotesAtTop(on)),
            )),
            settings::Action::GoogleSignOut => {
                let google = self.google.clone();
                Task::perform(async move { google.sign_out().await }, |result| {
                    Message::Modal(modal::Message::Settings(
                        settings::Message::GoogleSignedOut(result.map_err(|e| e.to_string())),
                    ))
                })
            }
            settings::Action::SetSpaceService(space, service) => {
                let Screen::Main(main) = &self.screen else {
                    return Task::none();
                };
                let session = main.session.clone();
                task::request(
                    async move { session.room(&space)?.set_call_service(service).await },
                    |result| {
                        Message::Modal(modal::Message::Settings(settings::Message::SpaceSaved(
                            result,
                        )))
                    },
                )
            }
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

    /// The selected space's topic panel.
    fn space_topic(&mut self, space: OwnedRoomId, message: topic::Message) -> Task<Message> {
        let Screen::Main(main) = &mut self.screen else {
            return Task::none();
        };
        if main.space.as_ref() != Some(&space) {
            return Task::none();
        }
        let listed = (main.lists.room(&space)).and_then(|r| r.topic.as_deref());
        topic::update(
            &main.session,
            &mut main.space_topic,
            space.clone(),
            listed,
            message,
            move |message| Message::Sidebar(sidebar::Message::SpaceTopic(space.clone(), message)),
        )
    }

    /// Something shows over the main view, as [`App::view`] puts it there:
    /// a modal or the card.
    fn covered(&self) -> bool {
        let Screen::Main(main) = &self.screen else {
            return false;
        };
        main.modal.is_some() || main.card.is_some()
    }

    /// The settings panel, while open.
    fn settings_panel(&mut self) -> Option<&mut settings::Panel> {
        match &mut self.screen {
            Screen::Main(main) => match &mut main.modal {
                Some(Modal::Settings(panel)) => Some(panel),
                _ => None,
            },
            _ => None,
        }
    }

    /// The PDF viewer, while it shows.
    fn pdf_viewer(&mut self) -> Option<&mut pdf_viewer::Viewer> {
        match &mut self.screen {
            Screen::Main(main) => match &mut main.modal {
                Some(Modal::Pdf(pdf)) => Some(pdf),
                _ => None,
            },
            _ => None,
        }
    }

    /// The PDF viewer follows the window's size, the display's scale and
    /// the text size. (Math does too, by the keys of its pictures.)
    fn pdf_resized(&mut self) -> Task<Message> {
        let (viewport, scale) = (self.viewport(), self.pixel_scale());
        match &mut self.screen {
            Screen::Main(main) => match &mut main.modal {
                Some(Modal::Pdf(pdf)) => pdf.resized(viewport, scale),
                _ => Task::none(),
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
                main.space = space;
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

    /// Save the space and room shown, to show them again next time.
    fn remember_view(&self) {
        let (Screen::Main(main), Some(file)) = (&self.screen, &self.view_file) else {
            return;
        };
        let room = match &main.pane {
            Pane::Opening(id) => Some(id.clone()),
            Pane::Open(room) => Some(room.id.clone()),
            Pane::Empty | Pane::Invite { .. } => None,
        };
        LastView {
            space: main.space.clone(),
            room,
        }
        .save(file);
    }

    /// Show a room's members anew where they are listed: beside the open
    /// room, or in its space's overview.
    fn refresh_members(&mut self, room: &OwnedRoomId) -> Task<Message> {
        let Screen::Main(main) = &mut self.screen else {
            return Task::none();
        };
        let beside = match main.pane.open_mut(room) {
            Some(open) if main.show_members => open.load_members(),
            _ => Task::none(),
        };
        let overview = match &mut main.card {
            Some(card) if card.overview.as_ref().is_some_and(|o| o.space == *room) => {
                overview_task(
                    &main.session,
                    &mut main.space_rooms,
                    &mut card.tasks,
                    room,
                    overview::Action::LoadMembers,
                )
            }
            _ => Task::none(),
        };
        Task::batch([beside, overview])
    }

    /// Once synced, an account without "Notes at the top" gets this
    /// desktop's earlier choice; asked once a run.
    fn move_notes_at_top(&mut self) -> Task<Message> {
        let Screen::Main(main) = &mut self.screen else {
            return Task::none();
        };
        if main.notes_moved || !main.lists.loaded || main.arrange.categories.notes_at_top.is_some()
        {
            return Task::none();
        }
        main.notes_moved = true;
        Task::done(Message::Sidebar(sidebar::Message::Arrange(
            arrange::Message::NotesAtTop(self.settings.pinned_notes),
        )))
    }

    fn selected_space(&self) -> Option<OwnedRoomId> {
        match &self.screen {
            Screen::Main(main) => main.space.clone(),
            _ => None,
        }
    }

    /// Which card is shown, and on which tab: whether one is, its room, and
    /// a space's tab.
    fn card_shown(&self) -> Option<(Option<OwnedRoomId>, Option<overview::Tab>)> {
        let Screen::Main(main) = &self.screen else {
            return None;
        };
        let card = main.card.as_ref()?;
        Some((card.room().cloned(), card.overview.as_ref().map(|o| o.tab)))
    }

    /// Run what the join dialog asks for.
    fn join_action(&mut self, action: join::Action) -> Task<Message> {
        let Screen::Main(main) = &mut self.screen else {
            return Task::none();
        };
        let session = main.session.clone();
        let done = |room: OwnedRoomOrAliasId| {
            move |result| Message::Modal(modal::Message::Join(join::Message::Done(room, result)))
        };
        let join = match &mut main.modal {
            Some(Modal::Join(join)) => Some(join),
            _ => None,
        };
        match action {
            join::Action::None => Task::none(),
            join::Action::Close => {
                main.modal = None;
                Task::none()
            }
            join::Action::Search(search) => join.map_or_else(Task::none, |join| {
                join.browse(task::request(
                    {
                        let search = search.clone();
                        async move {
                            session
                                .public_rooms(
                                    search.server.as_deref(),
                                    &search.query,
                                    search.kind,
                                    search.page,
                                )
                                .await
                        }
                    },
                    move |result| {
                        Message::Modal(modal::Message::Join(join::Message::Found(search, result)))
                    },
                ))
            }),
            join::Action::Preview(address, via) => join.map_or_else(Task::none, |join| {
                join.browse(task::request(
                    {
                        let address = address.clone();
                        async move { session.preview(&address, via).await }
                    },
                    move |result| {
                        Message::Modal(modal::Message::Join(join::Message::Previewed(
                            address.to_string(),
                            result,
                        )))
                    },
                ))
            }),
            join::Action::Join(room, via, space) => task::request(
                {
                    let join = join_listed(session, room.clone(), via, false);
                    async move {
                        let id = join.await?;
                        Ok(if space {
                            join::Joined::Space(id)
                        } else {
                            join::Joined::Room
                        })
                    }
                },
                done(room),
            ),
            join::Action::Knock(room, via) => task::request(
                {
                    let knock = join_listed(session, room.clone(), via, true);
                    async move { knock.await.map(|_| join::Joined::Asked) }
                },
                done(room),
            ),
            join::Action::ShowSpace(space) => {
                main.modal = None;
                main.joined_space = Some(space.clone());
                let shown = main.lists.rooms.iter().any(|r| r.id == space);
                let select = self.update(Message::Sidebar(sidebar::Message::SelectSpace(Some(
                    space.clone(),
                ))));
                if shown {
                    if let Screen::Main(main) = &mut self.screen {
                        main.joined_space = None;
                    }
                    Task::batch([
                        select,
                        Task::done(info::show_space(space, overview::Tab::Rooms)),
                    ])
                } else {
                    select
                }
            }
        }
    }

    /// Show the card of a person or room; a space's opens on `tab`.
    fn show_card(&mut self, target: info::Target, tab: overview::Tab) -> Task<Message> {
        let Screen::Main(main) = &mut self.screen else {
            return Task::none();
        };
        main.arrange.escape();
        let mut card = info::Card::new(target, &main.lists.rooms, tab);
        let room = card.room().map(|id| (id.clone(), main.session.room(id)));
        card.handle = room
            .as_ref()
            .and_then(|(_, handle)| handle.as_ref().ok().cloned());
        // Fresher than the watcher's, and for people it does not follow.
        let presence = match card.user() {
            Some(user) if *user != main.own_user && !main.presences.unavailable() => {
                presence::look_up(&main.session, user.clone())
                    .map(|m| Message::People(people::Message::Presence(m)))
            }
            _ => Task::none(),
        };
        let load = match card.user() {
            Some(user) => {
                let (session, user) = (main.session.clone(), user.clone());
                card.tasks.load(
                    info::Load::User,
                    {
                        let user = user.clone();
                        async move { session.user_info(&user).await }
                    },
                    move |result| Message::Card(info::Message::UserLoaded(user, result)),
                )
            }
            // A room's settings are followed while its card shows
            // (`subscription`).
            None => match room {
                Some((id, Err(e))) => Task::done(Message::Card(info::Message::Settings(
                    id,
                    Err(e.to_string()),
                ))),
                _ => Task::none(),
            },
        };
        let overview = match &card.overview {
            Some(o) => overview_task(
                &main.session,
                &mut main.space_rooms,
                &mut card.tasks,
                &o.space,
                o.opened(),
            ),
            None => Task::none(),
        };
        main.card = Some(card);
        Task::batch([load, presence, overview])
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
            subscriptions.push(main.arrange.subscription());
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
            if let Some(Modal::Dialog(dialog)) = &main.modal
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
                        pinned_notes: main.arrange.categories.notes_at_top(),
                        call_services: &self.settings.call_services,
                        room_pictures: self.settings.room_pictures,
                        popup: main.popup.as_ref(),
                        space_rooms: &main.space_rooms,
                    });
                    layers = layers.push(panel(card, Message::Card(info::Message::Close), true));
                }
                let modal = match &main.modal {
                    None => return layers.into(),
                    Some(Modal::Settings(settings)) => {
                        let spaces = (main.lists.top_level_spaces().into_iter())
                            .map(|s| (s.id.clone(), s.name.clone()))
                            .collect();
                        let view = settings.view(
                            &self.settings,
                            spaces,
                            &main.colours,
                            &main.arrange.categories,
                            !main.presences.unavailable(),
                            &self.updater,
                        );
                        panel(
                            view,
                            Message::Modal(modal::Message::ShowSettings(false)),
                            true,
                        )
                    }
                    Some(Modal::Dialog(dialog)) => panel(
                        dialog.view(&main.lists, &main.arrange, &self.settings.room_list),
                        Message::Modal(modal::Message::Dialog(dialog::Message::Cancel)),
                        false,
                    ),
                    Some(Modal::Join(join)) => panel(
                        join.view(&main.lists, &main.avatars, self.settings.room_pictures),
                        Message::Modal(modal::Message::Join(join::Message::Close)),
                        true,
                    ),
                    // Fill the window, with their own backdrop.
                    Some(Modal::Image(viewer)) => opaque(
                        viewer.view(&main.previews, matches!(main.popup, Some(Popup::Viewer))),
                    ),
                    Some(Modal::Pdf(pdf)) => opaque(pdf.view()),
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
            main.space.as_ref(),
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
        if let Some(notice) = &main.notice {
            banners = banners.push(notice_bar(notice));
        }
        let mut content = column![banners];
        let room_view: Element<'a, Message> = match &main.pane {
            Pane::Invite { id, answering } => {
                invite_view(rooms.iter().find(|r| &r.id == id), id, *answering)
            }
            Pane::Opening(id) => {
                let opening =
                    container(text("Opening…").style(text::secondary)).center(Length::Fill);
                // The member list stays where it is, loading.
                if main.show_members {
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
            Pane::Open(room) => {
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
                    show_members: main.show_members,
                    popup: main.popup.as_ref(),
                    presences: &main.presences,
                    now: main.now,
                };
                room.view(summary, &look, &scheduled)
            }
            Pane::Empty => container(text("Select a room").style(text::secondary))
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
            main.space.as_ref(),
            &main.arrange,
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

/// An invite, with Accept and Decline.
fn invite_view<'a>(
    summary: Option<&'a mtrx_core::RoomSummary>,
    id: &'a OwnedRoomId,
    answering: bool,
) -> Element<'a, Message> {
    let name = summary.map_or(id.as_str(), |s| s.name.as_str());
    let actions: Element<'a, Message> = if answering {
        text("Answering the invite…").style(text::secondary).into()
    } else {
        row![
            button(text("Decline"))
                .style(style::secondary_button)
                .on_press(Message::Pane(pane::Message::Answer(id.clone(), false))),
            button(text("Accept"))
                .style(style::primary_button)
                .on_press(Message::Pane(pane::Message::Answer(id.clone(), true))),
        ]
        .spacing(8)
        .into()
    };
    container(
        column![
            text(format!("You are invited to {name}"))
                .size(18)
                .font(crate::fonts::SEMIBOLD),
            text("Its messages show once you accept.").style(text::secondary),
            actions,
        ]
        .spacing(12)
        .align_x(iced::Alignment::Center),
    )
    .center(Length::Fill)
    .into()
}

/// Carry out a dialog's request.
async fn run_request(
    session: Arc<Session>,
    request: dialog::Request,
) -> Result<dialog::Opened, String> {
    use dialog::{Opened, Request};
    let result = async {
        Ok(match request {
            Request::CreateRoom(new) => Opened::Room(session.create_room(new).await?),
            Request::CreateSpace(new) => Opened::Space(session.create_room(new).await?),
            Request::DirectChat(user) => Opened::Room(session.direct_chat(&user).await?),
            Request::Invite(room, people) => {
                let room = session.room(&room)?;
                for person in &people {
                    room.invite(person).await?;
                }
                Opened::Nothing
            }
            Request::AddToSpace(space, rooms) => {
                let space = session.room(&space)?;
                for room in &rooms {
                    space.add_child(room).await?;
                }
                Opened::Nothing
            }
            Request::Leave(room) => {
                session.room(&room)?.leave().await?;
                Opened::Left(room)
            }
            Request::RemoveMember(room, user, reason, ban) => {
                (session.room(&room)?)
                    .remove(&user, ban, reason.as_deref())
                    .await?;
                Opened::Members(room)
            }
            Request::SetLevel(room, user, level) => {
                session.room(&room)?.set_level(&user, level).await?;
                Opened::Members(room)
            }
            Request::Encrypt(room) => {
                session.room(&room)?.encrypt().await?;
                Opened::Nothing
            }
            Request::SignOut => {
                // Others see us offline now, not once the server forgets us.
                presence::go_offline(&session, mtrx_core::Activity::SignOut).await;
                session.logout().await?;
                Opened::SignedOut
            }
        })
    }
    .await;
    result.map_err(|e: mtrx_core::Error| e.to_string())
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

/// List the rooms of all our spaces, for the room search.
fn load_space_rooms(main: &mut Main) -> Task<Message> {
    let spaces = directory::searched_spaces(&main.lists, None);
    Task::batch(
        spaces
            .iter()
            .map(|space| list_space_rooms(&main.session, &mut main.space_rooms, space)),
    )
}

/// List `space`'s rooms (the core asks its server at most once a minute),
/// unless being listed.
fn list_space_rooms(
    session: &Arc<Session>,
    space_rooms: &mut directory::SpaceRooms,
    space: &OwnedRoomId,
) -> Task<Message> {
    let session = session.clone();
    space_rooms.list(space, |space| {
        task::request(
            {
                let space = space.clone();
                async move { session.space_children(&space).await }
            },
            move |result| Message::Sidebar(sidebar::Message::SpaceRoomsLoaded(space, result)),
        )
    })
}

/// Load or join what a space's overview needs; the card owns its loads.
fn overview_task(
    session: &Arc<Session>,
    space_rooms: &mut directory::SpaceRooms,
    card: &mut Owned<info::Load>,
    space: &OwnedRoomId,
    action: overview::Action,
) -> Task<Message> {
    use overview::{Action, Message as Overview};
    let session = session.clone();
    let done = |message: Overview| Message::Card(info::Message::Overview(message));
    match action {
        Action::None => Task::none(),
        Action::Load(spaces) => Task::batch(
            spaces
                .iter()
                .map(|space| list_space_rooms(&session, space_rooms, space)),
        ),
        Action::LoadMembers => {
            let space = space.clone();
            card.load(
                info::Load::Members,
                async move { session.room(&space)?.members().await },
                move |result| done(Overview::MembersLoaded(result)),
            )
        }
        Action::Join(room, via) => joined_in_overview(session, room, via, false),
        Action::Knock(room, via) => joined_in_overview(session, room, via, true),
    }
}

/// Join a room a space's overview lists, or ask to (`knock`).
fn joined_in_overview(
    session: Arc<Session>,
    room: OwnedRoomId,
    via: Vec<OwnedServerName>,
    knock: bool,
) -> Task<Message> {
    let join = join_listed(session, room.clone().into(), via, knock);
    task::request(join, move |result| {
        Message::Card(info::Message::Overview(overview::Message::Done(
            room,
            result.map(|_| ()),
        )))
    })
}

/// Join a room or space that a space, a directory or a link lists, or ask
/// to join it (`knock`): its ID. The room lists show it as joined, or asked,
/// at once.
async fn join_listed(
    session: Arc<Session>,
    room: OwnedRoomOrAliasId,
    via: Vec<OwnedServerName>,
    knock: bool,
) -> mtrx_core::Result<OwnedRoomId> {
    if knock {
        session.knock(&room, via).await
    } else {
        session.join(&room, &via).await
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

/// Spaces whose members a dialog suggests first: the space a room is
/// created in, the spaces around a room people are invited to, or the
/// space shown.
fn near_spaces(
    kind: &dialog::Kind,
    graph: &mtrx_core::SpaceGraph,
    shown: Option<&OwnedRoomId>,
) -> Vec<OwnedRoomId> {
    let around = |room: &OwnedRoomId| {
        let mut spaces = graph.ancestors(room);
        spaces.push(room.clone());
        spaces
    };
    match kind {
        dialog::Kind::NewRoom {
            space: Some((space, _)),
        } => around(space),
        dialog::Kind::Invite { room, .. } => around(room),
        _ => shown.map(around).unwrap_or_default(),
    }
}

#[cfg(test)]
mod tests;
