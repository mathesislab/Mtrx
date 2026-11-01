//! Application state, messages and top-level layout.

use std::{
    collections::BTreeSet,
    fmt,
    hash::{Hash, Hasher},
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant, SystemTime},
};

use futures_util::{SinkExt, StreamExt};
use iced::{
    Element, Length, Point, Rectangle, Size, Subscription, Task, Theme, mouse,
    theme::Mode,
    widget::{
        button, column, container, image, mouse_area, opaque, row, stack, text, text_editor,
        text_input,
    },
    window,
};
use mtrx_core::{
    CallService, Connection, Contact, Draft, Google, Media, MediaRef, Notification, OwnedEventId,
    OwnedMxcUri, OwnedRoomId, OwnedRoomOrAliasId, OwnedServerName, OwnedUserId, PresenceState,
    Profile, RecoveryStep, RoomKind, RoomList, RoomListColours, RoomListState, RoomOrAliasId,
    RoomTag, RoomTimeline, ScheduledMessage, SecretStore, ServiceKind, Session, Topic, Url,
};

use crate::{
    actions::{self, Compose, Editing, InlineEdit},
    arrange::{self, Arrange},
    avatar::{self, Avatars},
    badge::{self, Badge},
    calls,
    clipboard::{self, Copied, Field, Pasted},
    dialog::{self, Dialog},
    directory, edit_menu, emoji,
    find::{self, Find},
    hint::{Position, hint},
    info, join,
    last_view::LastView,
    login::{self, Login},
    math::{self, Math, Rendered},
    media::{self, Previews, Viewer},
    members, mention, message, overview, pdf_viewer, pins,
    placement::{self, Frame, Placement},
    popover,
    presence::{self, Activity, Presences},
    room::{self, Look, Older, Room},
    selection::{self, Selection},
    send_later,
    settings::{self, Settings},
    sidebar, spelling,
    style::{self, MESSAGE_INSET, SPACE_BAR_WIDTH},
    text_size::Zoom,
    tint::Tint,
    topic,
    update::{self, Updater},
};

/// How often times shown relative to now are refreshed.
const TICK: Duration = Duration::from_secs(20);
/// How often progress (elapsed seconds) is refreshed while something runs.
const PROGRESS_TICK: Duration = Duration::from_secs(1);
/// How long writing pauses before the draft is kept.
const DRAFT_PAUSE: Duration = Duration::from_secs(1);
/// Where cards, the join dialog and Settings start, below the window's top.
const PANEL_TOP: f32 = 72.0;

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
    /// The saved session, or why there is none to open.
    Restored(Result<Shared<Session>, Shared<mtrx_core::Error>>),
    Login(login::Message),
    LoggedIn(Result<Shared<Session>, String>),
    Ready(Result<(Shared<RoomList>, bool), String>),
    Rooms(RoomListState),
    SelectSpace(Option<OwnedRoomId>),
    OpenRoom(OwnedRoomId),
    RoomOpened(OwnedRoomId, Result<Shared<RoomTimeline>, String>),
    /// For room `.0`: built by its views, or by a task it started. Dropped
    /// unless that room is open.
    Room(OwnedRoomId, room::Message),
    /// For the room shown, whichever it is: input to the whole window, such
    /// as shortcuts and dropped files. Dropped while no room is open.
    Shown(room::Message),
    /// The rooms with a draft on this device.
    Drafts(BTreeSet<OwnedRoomId>),
    /// The room list's colours changed, here or on another device.
    Colours(RoomListColours),
    /// Colours from the settings file were moved to the account, or not.
    ColoursMoved(Result<(), String>),
    Scheduled(Vec<ScheduledMessage>),
    /// Refreshes times shown relative to now.
    Tick,
    ShowSettings(bool),
    Settings(settings::Message),
    Update(update::Message),
    /// Open the settings and look for updates: Check for Updates… in the
    /// app menu, which only macOS has.
    #[cfg(target_os = "macos")]
    CheckForUpdates,
    AddMenu(bool),
    SpaceMenu(Option<sidebar::SpaceMenu>),
    OpenDialog(dialog::Kind),
    /// Join a space or room from elsewhere.
    OpenJoin,
    Join(join::Message),
    Dialog(dialog::Message),
    /// Accept (`true`) or decline an invite.
    AnswerInvite(OwnedRoomId, bool),
    InviteAnswered(OwnedRoomId, bool, Result<(), String>),
    CloseNotice,
    /// Fast ticks while something is in progress.
    Progress,
    RoomFilter(String),
    /// Open the first room matching the search.
    OpenFirstMatch,
    /// Open the room above or below in the room list, or one by its row.
    GoToRoom(sidebar::Go),
    /// The rooms of our spaces, as their servers list them.
    /// A space's rooms as its server listed them, or why not.
    SpaceRoomsLoaded(OwnedRoomId, Result<Vec<mtrx_core::DirectoryRoom>, String>),
    /// Join a room the search found in our spaces, or ask to (`true`).
    JoinFound(OwnedRoomId, Vec<OwnedServerName>, bool),
    /// Joined it (or asked to, `true`), or not.
    FoundJoined(OwnedRoomId, bool, Result<(), String>),
    FocusSearch,
    Escape,
    /// ⌘↩: the open dialog's main action, or saving the card's changes.
    Submit,
    ToggleMembers,
    /// Open the room with its members shown (where Invite is).
    ShowMembers(OwnedRoomId),
    MemberMenu(Option<members::MenuKey>),
    /// Give a member a power level, without asking.
    SetLevel(OwnedRoomId, OwnedUserId, i64),
    /// A room's members were changed by us, or not.
    MembersChanged(OwnedRoomId, Result<(), String>),
    /// A space's card, on one of its tabs.
    ShowSpace(OwnedRoomId, overview::Tab),
    /// Start or open a direct chat with a member.
    ChatWith(OwnedUserId),
    /// Start the Notes to self and open them.
    StartNotes,
    Notified(Notification),
    /// A desktop notification about the room was clicked.
    NotificationClicked(OwnedRoomId),
    WindowFocused(bool),
    /// Quit (⌘Q, in the Dock, logging out) on macOS; elsewhere the window's
    /// close button (or Ctrl+W, Alt+F4, ...) was pressed. See [`closing`].
    CloseWindow(window::Id),
    /// Others' presence, and ours.
    Presence(presence::Message),
    /// The selected space's topic and links, in its topic panel.
    SpaceTopic(OwnedRoomId, topic::Message),
    RecoveryKey(String),
    Recover,
    RecoveryProgress(RecoveryStep),
    Recovered(Result<bool, String>),
    MathRendered(math::Key, Rendered),
    LinkClicked(String),
    WindowResized(Size),
    /// The window opened at `.2` (its outer position), with these displays
    /// attached (`None` where the platform does not say).
    WindowOpened(window::Id, Option<Vec<Rectangle>>, Option<Point>),
    /// The window was moved or resized.
    WindowMoved(window::Id),
    /// Where the window now is, to open there next time.
    WindowPlaced {
        position: Option<Point>,
        maximized: bool,
        fullscreen: bool,
    },
    ScaleFactor(f32),
    /// Text size, or the PDF viewer's zoom while it is open.
    Zoom(Zoom),
    ThemeMode(Mode),
    AvatarLoaded(avatar::Key, Option<image::Handle>),
    PreviewLoaded(MediaRef, Option<media::Preview>),
    /// Show an image large.
    OpenImage(Media),
    CloseImage,
    /// Open or close the menu of the image in the viewer.
    ViewerMenu(bool),
    /// The whole image for the viewer.
    ImageLoaded(MediaRef, Result<media::Picture, String>),
    /// A PDF in the viewer was loaded, with its first page drawn.
    OpenPdf(Media),
    Pdf(pdf_viewer::Message),
    /// Ask where to save a message's file, then save it.
    SaveMedia(Media, media::Origin),
    /// Put a message's image on the clipboard.
    CopyImage(Media, media::Origin),
    /// Saving or copying a file finished, or failed: shown where it was
    /// asked from.
    MediaDone(media::Origin, Result<(), String>),
    Arrange(arrange::Message),
    ArrangeFailed(String),
    /// Show who someone is or what a room is.
    ShowInfo(info::Target),
    Info(info::Message),
    /// Star the direct chat with someone, starting it if there is none.
    FavouritePerson(OwnedUserId),
    /// Reload the open room's timeline (⌘R / Ctrl+R); while offline, try
    /// the server now.
    Reload,
    /// The server became reachable or not.
    Connection(Connection),
    RoomListScrolled,
    OutOfView(sidebar::OutOfView),
    /// Scroll the room list to this offset.
    ScrollRoomList(f32),
    /// Dragging the room list's edge.
    Resize(Resize),
    /// Try the server now.
    Reconnect,
    /// The server no longer accepts the session: forget it and log in again.
    SignInAgain,
    /// The ended session's local data is gone.
    Forgotten(Result<(), String>),
    /// People the user knows, for suggestions.
    ContactsLoaded(Vec<Contact>),
    /// The emoji the user reacts with, most used first.
    FrequentEmoji(Vec<String>),
}

pub struct App {
    profile: Profile,
    /// The window has the keyboard focus (notifications only when not
    /// looking at the room).
    focused: bool,
    /// Connection for creating Google Meet links, shared by all accounts.
    google: Arc<Google>,
    settings: Settings,
    /// Where settings are saved; `None` keeps them in memory (tests).
    settings_file: Option<PathBuf>,
    /// Where the last space and room shown are saved; `None` in tests.
    view_file: Option<PathBuf>,
    /// The window's size and position, to open it there again.
    placement: Placement,
    /// Where the placement is saved; `None` in tests.
    placement_file: Option<PathBuf>,
    /// The room list's edge is being dragged.
    resizing: bool,
    /// The settings panel, while open.
    settings_panel: Option<settings::Panel>,
    screen: Screen,
    /// Light or dark as iced reports the system (on macOS, the app's
    /// appearance, see [`crate::appearance`]).
    system_mode: Mode,
    /// The window's size in logical pixels, whatever the text size.
    window: Size,
    /// The display's scale: physical pixels per logical pixel.
    scale: f32,
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
    room_list: Option<Arc<RoomList>>,
    rooms: RoomListState,
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
    /// The rooms with a draft on this device, marked in the room list.
    drafts: BTreeSet<OwnedRoomId>,
    /// When scheduled times were last compared to the clock.
    now: SystemTime,
    error: Option<String>,
    avatars: Avatars,
    dialog: Option<Dialog>,
    /// The dialog for joining a space or room from elsewhere.
    join: Option<join::Join>,
    /// A space just joined, whose overview shows once it is in the room
    /// list.
    joined_space: Option<OwnedRoomId>,
    /// A dialog closed while its request runs: notices for success and failure.
    background: Option<(String, String)>,
    /// The room list's "+" menu is open.
    add_menu: bool,
    /// The space bar's "+" menu, or a space's, is open.
    space_menu: Option<sidebar::SpaceMenu>,
    /// The rooms of our spaces, for the room search.
    space_rooms: directory::SpaceRooms,
    /// The selected space's topic panel, below its name.
    space_topic: topic::Panel,
    /// This desktop's "Notes at the top" was offered to the account.
    notes_moved: bool,
    /// The member whose menu is open, and in which room.
    member_menu: Option<members::MenuKey>,
    /// Something finished in the background, or failed.
    notice: Option<Notice>,
    /// The room search text.
    filter: String,
    own_user: OwnedUserId,
    previews: Previews,
    /// An image shown large.
    viewer: Option<Viewer>,
    /// A PDF shown over the window.
    pdf: Option<pdf_viewer::Viewer>,
    /// Categories, tags and dragging in the room list.
    arrange: Arrange,
    /// The info card shown.
    info: Option<info::Card>,
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
    /// Our presence, from the window's focus and input.
    activity: Activity,
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

/// The room list's width limits, and its draggable edge's.
const MIN_ROOM_LIST_WIDTH: f32 = 200.0;
const MAX_ROOM_LIST_WIDTH: f32 = 520.0;
const RESIZE_HANDLE: f32 = 5.0;

#[derive(Debug, Clone, Copy)]
pub enum Resize {
    Start,
    /// The cursor's x in the window.
    Move(f32),
    End,
}

struct Notice {
    text: String,
    room: Option<OwnedRoomId>,
    error: bool,
}

impl Main {
    fn new(session: Arc<Session>) -> Self {
        Self {
            room_list: None,
            rooms: RoomListState::default(),
            space: None,
            pane: Pane::Empty,
            show_members: false,
            verified: None,
            recovery_key: String::new(),
            recovery_step: None,
            recovery_error: None,
            scheduled: session.scheduled(),
            drafts: BTreeSet::new(),
            own_user: session.user_id(),
            arrange: Arrange::new(&session),
            info: None,
            contacts: Vec::new(),
            frequent_emoji: Vec::new(),
            connection: session.connection(),
            colours: session.room_list_colours(),
            out_of_view: Default::default(),
            restore: None,
            presences: Presences::default(),
            activity: Activity::new(true, Instant::now()),
            now: SystemTime::now(),
            session,
            error: None,
            avatars: Avatars::new(),
            dialog: None,
            join: None,
            joined_space: None,
            background: None,
            add_menu: false,
            space_menu: None,
            space_rooms: Default::default(),
            space_topic: Default::default(),
            notes_moved: false,
            member_menu: None,
            notice: None,
            filter: String::new(),
            previews: Previews::new(),
            viewer: None,
            pdf: None,
        }
    }
}

impl Main {
    /// Pictures of the spaces, listed rooms, and the open room's header and
    /// timeline senders, with their sizes.
    fn avatars_on_screen(&self, settings: &Settings) -> Vec<(OwnedMxcUri, f32)> {
        let rooms = &self.rooms.rooms;
        let mut wanted = sidebar::avatars(&self.room_list_view(settings));
        wanted.extend(self.info.as_ref().and_then(|c| c.avatar(rooms)));
        if let Some(overview) = self.info.as_ref().and_then(|c| c.overview.as_ref()) {
            wanted.extend(overview.avatars());
            wanted.extend(self.space_rooms.avatars());
        }
        if let Some(join) = &self.join {
            wanted.extend(join.avatars());
        }
        if let Some(room) = self.pane.room() {
            wanted.extend(room.avatars(
                rooms.iter().find(|r| r.id == room.id),
                &self.own_user,
                self.show_members,
            ));
        }
        wanted
            .into_iter()
            .map(|(mxc, size)| (mxc.to_owned(), size))
            .collect()
    }

    fn room_list_view<'a>(&'a self, settings: &'a Settings) -> sidebar::RoomListView<'a> {
        let (grouping, width) = (settings.room_list, settings.room_list_width);
        sidebar::RoomListView {
            rooms: &self.rooms.rooms,
            selected_space: self.space.as_ref(),
            open: self.pane.id(),
            loaded: self.rooms.loaded,
            avatars: &self.avatars,
            scheduled: &self.scheduled,
            drafts: &self.drafts,
            add_menu: self.add_menu,
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
        let rooms = &self.rooms.rooms;
        let open = self
            .pane
            .room()
            .and_then(|room| rooms.iter().find(|r| r.id == room.id));
        let mut people: Vec<&OwnedUserId> = open
            .filter(|r| r.kind == RoomKind::Direct)
            .and_then(|r| r.direct_user.as_ref())
            .into_iter()
            .collect();
        people.extend(sidebar::direct_users(
            &self.room_list_view(settings),
            &self.own_user,
        ));
        if self.show_members
            && let Some(Ok(members)) = self.pane.room().and_then(|r| r.member_list.as_ref())
        {
            people.extend(members.iter().map(|m| &m.id));
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
                Message::Presence(presence::Message::Settle(generation))
            }),
            None => Task::none(),
        }
    }

    /// Set our presence, unless the server does not share it.
    fn set_presence(&self, state: PresenceState) -> Task<Message> {
        if self.presences.unavailable() {
            return Task::none();
        }
        presence::set_own(&self.session, state).map(Message::Presence)
    }

    /// The Notes to self if pinned; missing ones only once the room list
    /// is complete, so they are not started twice.
    fn pinned_notes(&self) -> Option<sidebar::Notes<'_>> {
        if !self.arrange.categories.notes_at_top() {
            return None;
        }
        let rooms = &self.rooms.rooms;
        match rooms.iter().find(|r| r.is_notes(&self.own_user)) {
            Some(notes) => Some(sidebar::Notes::Room(notes)),
            None => self.rooms.loaded.then_some(sidebar::Notes::Missing),
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
    pub fn window(&self) -> window::Settings {
        self.placement.settings(self.settings.text_size.scale())
    }

    /// Where the settings, the profiles and the crash log are.
    pub fn data_root(&self) -> &Path {
        &self.data_root
    }
}

/// The window's settings for closing it (`docs/native-behaviour.md`, A1). On
/// macOS closing it hides it, as a Mac app's main window, and the app goes
/// on (synced, notifying); clicking its Dock icon shows it again (our
/// winit). Quitting, and elsewhere closing, requests to close it: the app
/// goes offline and ends its session first, see [`Message::CloseWindow`].
pub fn closing(mut window: window::Settings) -> window::Settings {
    window.exit_on_close_request = false;
    #[cfg(target_os = "macos")]
    {
        window.platform_specific.hides_on_close = true;
    }
    window
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
            focused: true,
            google: Arc::new(Google::new(&data_root, secrets)),
            settings,
            settings_file: Some(settings_file),
            view_file: Some(LastView::path(&profile)),
            placement_file: Some(Placement::path(&profile)),
            window: placement
                .frame
                .map_or(placement::DEFAULT_SIZE, |frame| frame.size()),
            placement,
            resizing: false,
            settings_panel: None,
            screen: Screen::Starting,
            system_mode: Mode::Light,
            scale: 1.0,
            math: Math::new(),
            badge: Badge::default(),
            updater: Updater::default(),
        };
        let restore = Task::perform(
            async move { Session::restore(profile).await.map(|s| Shared(Arc::new(s))) },
            |result| Message::Restored(result.map_err(|e| Shared(Arc::new(e)))),
        );
        let scale = window::oldest().then(|id| match id {
            Some(id) => window::scale_factor(id).map(Message::ScaleFactor),
            None => Task::none(),
        });
        let mode = iced::system::theme().map(Message::ThemeMode);
        // Once the app has launched, over winit's own.
        #[cfg(target_os = "macos")]
        let menu_bar = window::oldest().then(|id| match id {
            Some(id) => window::run(id, |_| crate::menu_bar::install()).discard(),
            None => Task::none(),
        });
        #[cfg(not(target_os = "macos"))]
        let menu_bar = Task::none();
        // Whether the window opened on a display, see `Message::WindowOpened`.
        let opened = window::oldest().then(|id| match id {
            Some(id) => window::run(id, placement::displays).then(move |displays| {
                window::position(id)
                    .map(move |position| Message::WindowOpened(id, displays.clone(), position))
            }),
            None => Task::none(),
        });
        let update = Task::done(Message::Update(update::Message::Tick));
        (
            app,
            Task::batch([
                restore,
                scale,
                mode,
                opened,
                menu_bar,
                update,
                ask_to_notify(),
            ]),
        )
    }

    /// The theme the app is drawn in.
    pub fn theme(&self) -> Theme {
        self.settings.appearance.theme(self.system_mode)
    }

    /// Layout units per logical pixel, which iced applies on top of the
    /// display's scale (see [`crate::text_size`]).
    pub fn scale_factor(&self) -> f32 {
        self.settings.text_size.scale()
    }

    /// The window's size in layout units.
    fn viewport(&self) -> Size {
        self.window / self.scale_factor()
    }

    /// Physical pixels per layout unit, for drawing images sharp.
    fn pixel_scale(&self) -> f32 {
        self.scale * self.scale_factor()
    }

    /// The theme iced fixes the window to, if any.
    pub fn window_theme(&self) -> Option<Theme> {
        self.settings.appearance.window_theme()
    }

    pub fn update(&mut self, message: Message) -> Task<Message> {
        // What messages still bring, e.g. a session restored meanwhile, is
        // dropped here, while the runtime runs (see `Message::CloseWindow`).
        if let Screen::Closing = self.screen {
            return Task::none();
        }
        // Unread rooms may have moved into or out of the room list's view.
        let moved = matches!(
            message,
            Message::Rooms(_)
                | Message::Ready(_)
                | Message::RoomListScrolled
                | Message::SelectSpace(_)
                | Message::RoomFilter(_)
                | Message::Arrange(_)
                | Message::AddMenu(_)
                | Message::OpenRoom(_)
                | Message::WindowResized(_)
        );
        // A menu's choice closes it.
        if let Screen::Main(main) = &mut self.screen
            && main.space_menu.is_some()
            && matches!(
                message,
                Message::OpenDialog(_)
                    | Message::OpenJoin
                    | Message::ShowSpace(..)
                    | Message::SelectSpace(_)
            )
        {
            main.space_menu = None;
        }
        if let Screen::Main(main) = &mut self.screen
            && main.member_menu.is_some()
            && matches!(message, Message::OpenDialog(_) | Message::ShowInfo(_))
        {
            main.member_menu = None;
        }
        let remember = matches!(message, Message::OpenRoom(_) | Message::SelectSpace(_));
        let card = self.card_shown();
        let space = self.selected_space();
        let draft = self.room_draft();
        let task = self.handle(message);
        let keep_draft = self.keep_draft(draft);
        // A member's menu goes with the card it was opened on.
        if self.card_shown() != card
            && let Screen::Main(main) = &mut self.screen
        {
            main.member_menu = None;
        }
        // Another space, however chosen: its topic, closed, and whether we
        // may edit it.
        let topic = match self
            .selected_space()
            .filter(|_| self.selected_space() != space)
        {
            Some(space) => match &mut self.screen {
                Screen::Main(main) => {
                    main.space_topic = topic::Panel::default();
                    load_can_edit_topic(&main.session, space.clone(), move |message| {
                        Message::SpaceTopic(space, message)
                    })
                }
                _ => Task::none(),
            },
            None => Task::none(),
        };
        let task = Task::batch([task, topic, self.move_notes_at_top()]);
        if remember {
            self.remember_view();
        }
        let badge = self.badge.show(match &self.screen {
            Screen::Main(main) if self.settings.icon_badge => badge::count(&main.rooms.rooms),
            _ => 0,
        });
        let out_of_view = match &self.screen {
            Screen::Main(main) if moved => {
                sidebar::find_out_of_view(&main.rooms.rooms).map(Message::OutOfView)
            }
            _ => Task::none(),
        };
        let presence = match &mut self.screen {
            Screen::Main(main) => main.follow_shown(&self.settings),
            _ => Task::none(),
        };
        Task::batch([task, badge, out_of_view, presence, keep_draft])
    }

    /// The open room, and its draft if it is to be kept.
    fn room_draft(&self) -> Option<(OwnedRoomId, Option<Draft>)> {
        match &self.screen {
            Screen::Main(main) => main
                .pane
                .room()
                .map(|room| (room.id.clone(), room.draft_to_keep())),
            _ => None,
        }
    }

    /// Keep the draft of the room open `before` a message, as it was then,
    /// if another room is open now (or none); else the open room's, after
    /// a pause in writing ([`DRAFT_PAUSE`]).
    fn keep_draft(&mut self, before: Option<(OwnedRoomId, Option<Draft>)>) -> Task<Message> {
        let Screen::Main(main) = &mut self.screen else {
            return Task::none();
        };
        match (before, main.pane.room_mut()) {
            (Some((id, _)), Some(room)) if room.id == id => {
                if room.draft_to_keep().is_none() {
                    return Task::none();
                }
                room.draft_changes += 1;
                let changes = room.draft_changes;
                Task::perform(tokio::time::sleep(DRAFT_PAUSE), move |()| {
                    room::Message::KeepDraft(changes).to(&id)
                })
            }
            (Some((id, Some(draft))), _) => save_draft(&main.session, id, draft),
            _ => Task::none(),
        }
    }

    fn handle(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::RoomListScrolled => Task::none(),
            Message::OutOfView(out_of_view) => {
                if let Screen::Main(main) = &mut self.screen {
                    main.out_of_view = out_of_view;
                }
                Task::none()
            }
            Message::ScrollRoomList(offset) => sidebar::scroll_to(offset),
            Message::Resize(Resize::Start) => {
                self.resizing = true;
                Task::none()
            }
            Message::Resize(Resize::Move(x)) => {
                self.settings.room_list_width =
                    (x - SPACE_BAR_WIDTH).clamp(MIN_ROOM_LIST_WIDTH, MAX_ROOM_LIST_WIDTH);
                Task::none()
            }
            Message::Resize(Resize::End) => {
                self.resizing = false;
                if let Some(file) = &self.settings_file
                    && let Err(e) = self.settings.save(file)
                {
                    tracing::warn!("could not save settings: {e}");
                }
                // Math is laid out for the room's width.
                self.render_math()
            }
            Message::Restored(Ok(Shared(session))) | Message::LoggedIn(Ok(Shared(session))) => {
                let mut main = Main::new(session.clone());
                main.restore = self.view_file.as_deref().map(LastView::load);
                main.activity = Activity::new(self.focused, Instant::now());
                let share = self.settings.share_presence;
                let online = if share {
                    main.set_presence(PresenceState::Online)
                } else {
                    Task::none()
                };
                self.screen = Screen::Main(Box::new(main));
                let migrate = self.move_colours_to_account(&session);
                let start = Task::perform(
                    async move {
                        // Before syncing, whose requests would say online.
                        if !share {
                            presence::go_offline(&session).await;
                        }
                        session.start_sync().await;
                        session.start_scheduler();
                        let list = session.room_list().await?;
                        let verified = session.is_device_verified().await?;
                        Ok((Shared(Arc::new(list)), verified))
                    },
                    |result: mtrx_core::Result<_>| {
                        Message::Ready(result.map_err(|e| e.to_string()))
                    },
                );
                Task::batch([start, migrate, online])
            }
            Message::Colours(colours) => {
                if let Screen::Main(main) = &mut self.screen {
                    main.colours = colours;
                }
                Task::none()
            }
            Message::ColoursMoved(result) => {
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
            Message::Restored(Err(Shared(error))) => {
                let mut login = Login::default();
                // Usually nobody is signed in yet. Without a keyring signing
                // in would fail too, so say so before the user tries.
                if let mtrx_core::Error::NoSecretStore(_) = *error {
                    login.error = Some(error.to_string());
                }
                self.screen = Screen::Login(login);
                Task::none()
            }
            Message::Login(message) => {
                let Screen::Login(login) = &mut self.screen else {
                    return Task::none();
                };
                let (profile, homeserver) = (self.profile.clone(), login.homeserver.clone());
                match login.update(message) {
                    login::Action::None => Task::none(),
                    login::Action::Password => {
                        let (user, password) = (login.user.clone(), login.password.clone());
                        Task::perform(
                            async move {
                                Session::login(profile, &homeserver, &user, &password)
                                    .await
                                    .map(|s| Shared(Arc::new(s)))
                            },
                            |result| Message::LoggedIn(result.map_err(|e| e.to_string())),
                        )
                    }
                    login::Action::Browser => {
                        let (task, handle) = Task::perform(
                            async move {
                                Session::login_oauth(profile, &homeserver, |url| {
                                    let _ = open::that_detached(url.as_str());
                                })
                                .await
                                .map(|s| Shared(Arc::new(s)))
                            },
                            |result| Message::LoggedIn(result.map_err(|e| e.to_string())),
                        )
                        .abortable();
                        login.browser_started(handle);
                        task
                    }
                }
            }
            Message::LoggedIn(Err(error)) => {
                if let Screen::Login(login) = &mut self.screen {
                    login.failed(error);
                }
                Task::none()
            }
            Message::Ready(result) => {
                let Screen::Main(main) = &mut self.screen else {
                    return Task::none();
                };
                match result {
                    Ok((Shared(list), verified)) => {
                        main.rooms = list.current();
                        main.arrange.rooms_updated(&mut main.rooms.rooms);
                        main.room_list = Some(list);
                        main.verified = Some(verified);
                    }
                    Err(e) => main.error = Some(e),
                }
                let members = main
                    .arrange
                    .load_space_members(&main.session, &main.rooms.rooms);
                Task::batch([members, self.load_avatars(), self.restore_view()])
            }
            Message::Rooms(state) => {
                let Screen::Main(main) = &mut self.screen else {
                    return Task::none();
                };
                main.rooms = state;
                main.arrange.rooms_updated(&mut main.rooms.rooms);
                let joined = main
                    .joined_space
                    .take_if(|space| main.rooms.rooms.iter().any(|r| r.id == *space))
                    .map_or(Task::none(), |space| {
                        Task::done(Message::ShowSpace(space, overview::Tab::Rooms))
                    });
                let members = main
                    .arrange
                    .load_space_members(&main.session, &main.rooms.rooms);
                Task::batch([
                    members,
                    joined,
                    self.load_pinned(),
                    self.load_avatars(),
                    self.restore_view(),
                ])
            }
            Message::SelectSpace(space) => {
                let Screen::Main(main) = &mut self.screen else {
                    return Task::none();
                };
                main.space = space;
                main.info = None;
                main.restore = None;
                main.arrange.escape();
                self.load_avatars()
            }
            Message::Arrange(message) => {
                let Screen::Main(main) = &mut self.screen else {
                    return Task::none();
                };
                let context = arrange::Context {
                    session: &main.session,
                    space: main.space.as_ref(),
                    rooms: &mut main.rooms.rooms,
                    grouping: self.settings.room_list,
                };
                let task = main.arrange.update(message, context);
                Task::batch([task, self.load_avatars()])
            }
            Message::ArrangeFailed(e) => {
                if let Screen::Main(main) = &mut self.screen {
                    main.notice = Some(Notice {
                        text: format!("Could not rearrange the room list: {e}"),
                        room: None,
                        error: true,
                    });
                }
                Task::none()
            }
            Message::OpenRoom(id) => {
                let Screen::Main(main) = &mut self.screen else {
                    return Task::none();
                };
                main.add_menu = false;
                main.info = None;
                // Chosen now: last time's view is no longer wanted.
                main.restore = None;
                if main.pane.open(&id).is_some() {
                    return Task::none();
                }
                // An invite shows Accept and Decline instead of a timeline.
                if main.rooms.rooms.iter().any(|r| r.id == id && r.invited) {
                    main.pane = Pane::Invite {
                        id,
                        answering: false,
                    };
                    return Task::none();
                }
                main.pane = Pane::Opening(id.clone());
                let session = main.session.clone();
                Task::perform(
                    async move {
                        let timeline = session.timeline(&id).await.map(|t| Shared(Arc::new(t)));
                        (id, timeline.map_err(|e| e.to_string()))
                    },
                    |(id, result)| Message::RoomOpened(id, result),
                )
            }
            Message::RoomOpened(id, result) => {
                let Screen::Main(main) = &mut self.screen else {
                    return Task::none();
                };
                if !matches!(&main.pane, Pane::Opening(opening) if *opening == id) {
                    // Another room or an invite was chosen meanwhile.
                    return Task::none();
                }
                match result {
                    Ok(Shared(timeline)) => {
                        let mut room = Room::new(id.clone(), timeline);
                        let paginate = fill(&mut room);
                        let read = mark_read(&room);
                        // The member list stays open from the last room.
                        let members = if main.show_members {
                            load_members(&main.session, &mut room)
                        } else {
                            Task::none()
                        };
                        main.pane = Pane::Open(room);
                        let (session, room) = (main.session.clone(), id.clone());
                        let draft = Task::perform(
                            async move {
                                let draft = session.draft(&room).await;
                                (room, draft)
                            },
                            |(id, draft)| {
                                // One that cannot be read is none to restore,
                                // and one to replace.
                                let draft = draft.inspect_err(
                                    |e| tracing::warn!(room = %id, "could not read the draft: {e}"),
                                );
                                room::Message::DraftLoaded(draft.ok().flatten()).to(&id)
                            },
                        );
                        Task::batch([
                            paginate,
                            read,
                            members,
                            draft,
                            self.render_math(),
                            self.load_avatars(),
                            self.load_previews(),
                            self.load_room_calls(&id),
                            self.load_pinned(),
                        ])
                    }
                    Err(e) => {
                        main.pane = Pane::Empty;
                        main.error = Some(e);
                        Task::none()
                    }
                }
            }
            Message::ShowSettings(false) => {
                self.settings_panel = None;
                Task::none()
            }
            Message::ShowSettings(true) => self.open_settings(),
            Message::Settings(message) => self.settings_update(message),
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
            Message::Room(id, message) => self.room(id, message),
            Message::Shown(message) => {
                let Screen::Main(main) = &self.screen else {
                    return Task::none();
                };
                match main.pane.room() {
                    Some(room) => {
                        let id = room.id.clone();
                        self.room(id, message)
                    }
                    None => Task::none(),
                }
            }
            Message::SpaceTopic(space, message) => self.space_topic(space, message),
            Message::AddMenu(open) => {
                if let Screen::Main(main) = &mut self.screen {
                    main.add_menu = open;
                }
                Task::none()
            }
            Message::SpaceMenu(menu) => {
                if let Screen::Main(main) = &mut self.screen {
                    main.space_menu = menu;
                }
                Task::none()
            }
            Message::OpenDialog(kind) => {
                self.settings_panel = None;
                let Screen::Main(main) = &mut self.screen else {
                    return Task::none();
                };
                main.add_menu = false;
                // A card stays below, e.g. a space's overview, to come
                // back to.
                let near = near_spaces(&kind, &main.rooms.rooms, main.space.as_ref());
                main.arrange.escape();
                let mut dialog = Dialog::new(kind);
                dialog.fill_category(&main.arrange.categories);
                if !dialog.picks_people() {
                    main.dialog = Some(dialog);
                    return Task::none();
                }
                dialog.prefer_spaces(near);
                dialog.offer_self(main.own_user.clone());
                let new_room = dialog.creates_room();
                dialog.update(dialog::Message::Contacts(main.contacts.clone()));
                main.dialog = Some(dialog);
                let session = main.session.clone();
                let owners = if new_room {
                    let session = session.clone();
                    Task::perform(async move { session.offers_owners().await }, |offered| {
                        Message::Dialog(dialog::Message::OwnersOffered(offered))
                    })
                } else {
                    Task::none()
                };
                Task::batch([
                    Task::perform(
                        async move { session.contacts().await.unwrap_or_default() },
                        Message::ContactsLoaded,
                    ),
                    owners,
                ])
            }
            Message::FrequentEmoji(emoji) => {
                if let Screen::Main(main) = &mut self.screen {
                    main.frequent_emoji = emoji;
                }
                Task::none()
            }
            Message::ContactsLoaded(contacts) => {
                if let Screen::Main(main) = &mut self.screen {
                    if let Some(dialog) = &mut main.dialog {
                        dialog.update(dialog::Message::Contacts(contacts.clone()));
                    }
                    main.contacts = contacts;
                }
                Task::none()
            }
            Message::ShowInfo(target) => self.show_card(target, overview::Tab::Rooms),
            Message::OpenJoin => {
                let Screen::Main(main) = &mut self.screen else {
                    return Task::none();
                };
                main.add_menu = false;
                main.info = None;
                let session = main.session.clone();
                let (join, action) = join::Join::new(
                    session.user_id().server_name().to_owned(),
                    &session.directory_servers().servers,
                );
                main.join = Some(join);
                Task::batch([
                    self.join_action(action),
                    iced::widget::operation::focus(join::SEARCH),
                ])
            }
            Message::Join(message) => {
                let Screen::Main(main) = &mut self.screen else {
                    return Task::none();
                };
                let Some(join) = &mut main.join else {
                    return Task::none();
                };
                // A server whose directory answered is remembered.
                let remember = match &message {
                    join::Message::Found(search, Ok(_)) => search.server.clone().map(|server| {
                        let session = main.session.clone();
                        Task::future(async move {
                            if let Err(e) = session.remember_directory_server(&server).await {
                                tracing::warn!("could not remember {server}: {e}");
                            }
                        })
                        .discard()
                    }),
                    _ => None,
                };
                let action = join.update(message);
                Task::batch([
                    remember.unwrap_or_else(Task::none),
                    self.join_action(action),
                    self.load_avatars(),
                ])
            }
            Message::ShowSpace(space, tab) => self.show_card(info::Target::Room(space), tab),
            Message::Info(message) => {
                let Screen::Main(main) = &mut self.screen else {
                    return Task::none();
                };
                let Some(card) = &mut main.info else {
                    return Task::none();
                };
                let session = main.session.clone();
                let overview_session = main.session.clone();
                let room = card.room().cloned();
                let saved = |result: mtrx_core::Result<()>| {
                    Message::Info(info::Message::Saved(result.map_err(|e| e.to_string())))
                };
                let task = match (&message, room) {
                    (info::Message::Close, _) => {
                        main.info = None;
                        return Task::none();
                    }
                    (info::Message::Copy(text), _) => iced::clipboard::write(text.clone()),
                    (info::Message::SaveDetails, Some(room)) => {
                        let Some(summary) = main.rooms.rooms.iter().find(|r| r.id == room) else {
                            return Task::none();
                        };
                        let (name, topic) = card.changed_details(summary);
                        Task::perform(
                            async move {
                                if let Some(name) = name {
                                    session.set_room_name(&room, &name).await?;
                                }
                                if let Some(topic) = topic {
                                    session.set_room_topic(&room, &topic).await?;
                                }
                                Ok(())
                            },
                            saved,
                        )
                    }
                    (info::Message::Access(access), Some(room)) => {
                        let access = *access;
                        Task::perform(
                            async move { session.set_room_access(&room, access).await },
                            saved,
                        )
                    }
                    (info::Message::Notify(mode), Some(room)) => {
                        let mode = *mode;
                        Task::perform(
                            async move { session.set_notify_mode(&room, mode).await },
                            saved,
                        )
                    }
                    (info::Message::ChangePicture, Some(room)) => Task::perform(
                        async move {
                            let Some(file) = rfd::AsyncFileDialog::new()
                                .set_title("Room picture")
                                .add_filter("Images", &["png", "jpg", "jpeg", "gif", "webp"])
                                .pick_file()
                                .await
                            else {
                                return Ok(());
                            };
                            session.set_room_picture(&room, file.path()).await
                        },
                        saved,
                    ),
                    (info::Message::RemovePicture, Some(room)) => Task::perform(
                        async move { session.remove_room_picture(&room).await },
                        saved,
                    ),
                    (info::Message::Change(change), Some(room)) => {
                        let change = change.clone();
                        Task::perform(
                            async move {
                                use info::Change;
                                match change {
                                    Change::Address(name) => {
                                        session.set_address(&room, name.as_deref()).await
                                    }
                                    Change::Listed(on) => session.set_listed(&room, on).await,
                                    Change::History(h) => session.set_history(&room, h).await,
                                    Change::Permissions(p) => {
                                        session.set_permissions(&room, p).await
                                    }
                                    Change::Suggested(space, on) => {
                                        session.set_suggested(&space, &room, on).await
                                    }
                                    Change::AddToSpace(space) => {
                                        session.add_to_space(&space, &room).await
                                    }
                                    Change::RemoveFromSpace(space) => {
                                        session.remove_from_space(&space, &room).await
                                    }
                                    Change::Unban(user) => session.unban(&room, &user).await,
                                }
                            },
                            saved,
                        )
                    }
                    // What was saved, or the settings as they are after a failure.
                    (info::Message::Saved(_), Some(room)) => load_settings(&session, &room),
                    _ => Task::none(),
                };
                let overview = match card.update(message) {
                    overview::Action::None => Task::none(),
                    action => card.overview.as_ref().map_or(Task::none(), |o| {
                        overview_task(&overview_session, &mut main.space_rooms, &o.space, action)
                    }),
                };
                Task::batch([task, overview, self.load_avatars()])
            }
            Message::FavouritePerson(user) => {
                let Screen::Main(main) = &self.screen else {
                    return Task::none();
                };
                let session = main.session.clone();
                Task::future(async move {
                    let room = session.direct_chat(&user).await?;
                    session.set_room_tag(&room, RoomTag::Favourite).await
                })
                .then(|result| match result {
                    Ok(()) => Task::none(),
                    Err(e) => Task::done(Message::ArrangeFailed(e.to_string())),
                })
            }
            Message::Reload => {
                let Screen::Main(main) = &mut self.screen else {
                    return Task::none();
                };
                match main.connection {
                    Connection::Offline => return self.update(Message::Reconnect),
                    // Its bar offers signing in again.
                    Connection::SignedOut => return Task::none(),
                    Connection::Online => {}
                }
                let Some(room) = main.pane.room_mut() else {
                    return Task::none();
                };
                room.reloading = true;
                let (session, id) = (main.session.clone(), room.id.clone());
                Task::perform(
                    async move {
                        let timeline = session.timeline(&id).await.map(|t| Shared(Arc::new(t)));
                        (id, timeline.map_err(|e| e.to_string()))
                    },
                    |(id, result)| room::Message::Reloaded(result).to(&id),
                )
            }
            Message::Connection(connection) => {
                let Screen::Main(main) = &mut self.screen else {
                    return Task::none();
                };
                let back =
                    main.connection == Connection::Offline && connection == Connection::Online;
                main.connection = connection;
                // Our presence, if it could not be set while offline.
                let presence = if back && self.settings.share_presence {
                    main.set_presence(main.activity.state())
                } else {
                    Task::none()
                };
                // History that could not load while offline.
                let history = match main.pane.room_mut() {
                    Some(room) if back => fill(room),
                    _ => Task::none(),
                };
                Task::batch([presence, history])
            }
            Message::SignInAgain => {
                let Screen::Main(main) = &self.screen else {
                    return Task::none();
                };
                let session = main.session.clone();
                Task::perform(
                    async move { session.forget().await.map_err(|e| e.to_string()) },
                    Message::Forgotten,
                )
            }
            Message::Forgotten(result) => {
                let Screen::Main(main) = &self.screen else {
                    return Task::none();
                };
                // Ready to log in as before.
                let mut login = Login::default();
                login.homeserver = main.own_user.server_name().to_string();
                login.user = main.own_user.localpart().to_owned();
                login.error = result.err();
                self.screen = Screen::Login(login);
                Task::none()
            }
            Message::Reconnect => {
                let Screen::Main(main) = &self.screen else {
                    return Task::none();
                };
                let session = main.session.clone();
                Task::future(async move { session.reconnect().await }).discard()
            }
            Message::Dialog(message) => self.dialog(message),
            Message::AnswerInvite(id, accept) => {
                let Screen::Main(main) = &mut self.screen else {
                    return Task::none();
                };
                main.pane = Pane::Invite {
                    id: id.clone(),
                    answering: true,
                };
                let session = main.session.clone();
                Task::perform(
                    {
                        let id = id.clone();
                        async move {
                            if accept {
                                session
                                    .join(<&RoomOrAliasId>::from(&*id), &[])
                                    .await
                                    .map(|_| ())
                            } else {
                                session.leave(&id).await
                            }
                        }
                    },
                    move |result| {
                        Message::InviteAnswered(
                            id.clone(),
                            accept,
                            result.map_err(|e| e.to_string()),
                        )
                    },
                )
            }
            Message::InviteAnswered(id, accept, result) => {
                let Screen::Main(main) = &mut self.screen else {
                    return Task::none();
                };
                if matches!(&main.pane, Pane::Invite { id: invite, .. } if *invite == id) {
                    main.pane = Pane::Empty;
                }
                match result {
                    Ok(()) if accept => self.update(Message::OpenRoom(id)),
                    Ok(()) => Task::none(),
                    Err(e) => {
                        main.notice = Some(Notice {
                            text: format!(
                                "Could not {} the invite: {e}",
                                if accept { "accept" } else { "decline" }
                            ),
                            room: None,
                            error: true,
                        });
                        Task::none()
                    }
                }
            }
            Message::RoomFilter(filter) => {
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
                Task::batch([load, self.load_avatars()])
            }
            Message::SpaceRoomsLoaded(space, result) => {
                if let Screen::Main(main) = &mut self.screen {
                    main.space_rooms.loaded(space, result);
                }
                self.load_avatars()
            }
            Message::JoinFound(room, via, knock) => {
                let Screen::Main(main) = &mut self.screen else {
                    return Task::none();
                };
                main.space_rooms.busy.insert(room.clone());
                let session = main.session.clone();
                Task::perform(
                    {
                        let room = room.clone();
                        async move {
                            if knock {
                                session.knock(&room.into(), via).await.map(|_| ())
                            } else {
                                session
                                    .join(<&RoomOrAliasId>::from(&*room), &via)
                                    .await
                                    .map(|_| ())
                            }
                        }
                    },
                    move |result| {
                        Message::FoundJoined(room.clone(), knock, result.map_err(|e| e.to_string()))
                    },
                )
            }
            Message::FoundJoined(room, knock, result) => {
                let Screen::Main(main) = &mut self.screen else {
                    return Task::none();
                };
                main.space_rooms.busy.remove(&room);
                match result {
                    Ok(()) if knock => {
                        main.space_rooms.asked.insert(room);
                        Task::none()
                    }
                    Ok(()) => self.update(Message::OpenRoom(room)),
                    Err(e) => {
                        main.notice = Some(Notice {
                            text: format!("Could not join: {e}"),
                            room: None,
                            error: true,
                        });
                        Task::none()
                    }
                }
            }
            Message::OpenFirstMatch => {
                let Screen::Main(main) = &mut self.screen else {
                    return Task::none();
                };
                match sidebar::first_match(
                    &main.rooms.rooms,
                    main.space.as_ref(),
                    &main.arrange,
                    &self.settings.room_list,
                    &main.filter,
                ) {
                    Some(room) => {
                        let id = room.id.clone();
                        main.filter.clear();
                        self.update(Message::OpenRoom(id))
                    }
                    None => Task::none(),
                }
            }
            Message::GoToRoom(go) => {
                let Screen::Main(main) = &self.screen else {
                    return Task::none();
                };
                match sidebar::go_to(&main.room_list_view(&self.settings), go) {
                    Some(room) => {
                        let id = room.id.clone();
                        self.update(Message::OpenRoom(id))
                    }
                    None => Task::none(),
                }
            }
            Message::FocusSearch => iced::widget::operation::focus(sidebar::SEARCH),
            // Closes the topmost thing: the settings, a dialog, the image
            // viewer's menu, the viewer, a card, a drag, menu or category
            // name, else the search.
            Message::Escape => {
                if self.settings_panel.is_some() {
                    self.settings_panel = None;
                    return Task::none();
                }
                if let Screen::Main(main) = &self.screen {
                    if main.dialog.is_some() {
                        return Task::done(Message::Dialog(dialog::Message::Cancel));
                    }
                    if main.join.is_some() {
                        return Task::done(Message::Join(join::Message::Close));
                    }
                }
                if let Screen::Main(main) = &mut self.screen {
                    if let Some(viewer) = main.viewer.as_mut().filter(|v| v.menu) {
                        viewer.menu = false;
                    } else if main.viewer.is_some() || main.pdf.is_some() {
                        main.viewer = None;
                        main.pdf = None;
                    } else if main.info.is_some() {
                        main.info = None;
                    } else if let Some(room) = main.pane.room_mut().filter(|r| r.calls.is_open()) {
                        room.calls.close();
                    } else if let Some(room) = main
                        .pane
                        .room_mut()
                        .filter(|r| !r.candidates(&main.own_user).is_empty())
                    {
                        room.completion = None;
                    } else if let Some(room) = main.pane.room_mut().filter(|r| r.menu.is_some()) {
                        room.menu = None;
                    } else if let Some(room) =
                        main.pane.room_mut().filter(|r| r.composer_menu.is_some())
                    {
                        room.composer_menu = None;
                    } else if let Some(edit) = main
                        .pane
                        .room_mut()
                        .and_then(|r| r.editing.as_mut())
                        .filter(|e| e.menu.is_some())
                    {
                        edit.menu = None;
                    } else if let Some(room) = main.pane.room_mut().filter(|r| r.readers.is_some())
                    {
                        room.readers = None;
                    } else if let Some(room) = main.pane.room_mut().filter(|r| r.reactors.is_some())
                    {
                        room.reactors = None;
                    } else if let Some(room) = main.pane.room_mut().filter(|r| r.unsent.is_some()) {
                        room.unsent = None;
                    } else if let Some(room) = main.pane.room_mut().filter(|r| r.find.is_some()) {
                        room.find = None;
                    } else if let Some(room) =
                        main.pane.room_mut().filter(|r| r.selection.is_some())
                    {
                        room.selection = None;
                    } else if let Some(room) =
                        main.pane.room_mut().filter(|r| r.composer_emoji.is_some())
                    {
                        room.composer_emoji = None;
                    } else if main.pane.room().is_some_and(|r| r.compose.cancellable()) {
                        // From the composer its own Esc does it first.
                        return Task::done(Message::Shown(room::Message::Act(
                            actions::Message::CancelCompose,
                        )));
                    } else if let Some(edit) = main
                        .pane
                        .room_mut()
                        .and_then(|r| r.editing.as_mut())
                        .filter(|e| e.emoji.is_some())
                    {
                        edit.emoji = None;
                    } else if let Some(room) = main.pane.room_mut().filter(|r| r.editing.is_some())
                    {
                        room.editing = None;
                    } else if let Some(room) =
                        main.pane.room_mut().filter(|r| r.later.picker.is_some())
                    {
                        room.later = Default::default();
                    } else if let Some(room) = main.pane.room_mut().filter(|r| r.picker.is_some()) {
                        room.picker = None;
                        room.emoji = None;
                    } else if !main.arrange.escape() {
                        main.filter.clear();
                    }
                }
                Task::none()
            }
            Message::Submit => {
                let Screen::Main(main) = &self.screen else {
                    return Task::none();
                };
                let submit = if self.settings_panel.is_some() {
                    None
                } else if main.dialog.is_some() {
                    Some(Message::Dialog(dialog::Message::Submit))
                } else {
                    main.info
                        .as_ref()
                        .and_then(|card| card.submit(&main.rooms.rooms))
                };
                submit.map_or_else(Task::none, Task::done)
            }
            Message::MemberMenu(menu) => {
                if let Screen::Main(main) = &mut self.screen {
                    main.member_menu = menu;
                }
                Task::none()
            }
            Message::SetLevel(room, user, level) => {
                let Screen::Main(main) = &mut self.screen else {
                    return Task::none();
                };
                main.member_menu = None;
                let session = main.session.clone();
                Task::perform(
                    {
                        let room = room.clone();
                        async move { session.set_level(&room, &user, level).await }
                    },
                    move |result| {
                        Message::MembersChanged(room.clone(), result.map_err(|e| e.to_string()))
                    },
                )
            }
            Message::MembersChanged(room, result) => {
                if let (Err(e), Screen::Main(main)) = (&result, &mut self.screen) {
                    main.notice = Some(Notice {
                        text: format!("Could not change the members: {e}"),
                        room: None,
                        error: true,
                    });
                }
                self.refresh_members(&room)
            }
            Message::ShowMembers(id) => {
                let Screen::Main(main) = &mut self.screen else {
                    return Task::none();
                };
                main.show_members = true;
                main.info = None;
                match main.pane.open_mut(&id) {
                    Some(room) => {
                        if room.loading_members {
                            Task::none()
                        } else {
                            load_members(&main.session, room)
                        }
                    }
                    _ => Task::done(Message::OpenRoom(id)),
                }
            }
            Message::ToggleMembers => {
                let Screen::Main(main) = &mut self.screen else {
                    return Task::none();
                };
                main.show_members = !main.show_members;
                // Opening it shows the members anew.
                match main.pane.room_mut() {
                    Some(room) if main.show_members && !room.loading_members => {
                        load_members(&main.session, room)
                    }
                    _ => Task::none(),
                }
            }
            Message::StartNotes => {
                let Screen::Main(main) = &self.screen else {
                    return Task::none();
                };
                let own_user = main.own_user.clone();
                self.update(Message::ChatWith(own_user))
            }
            Message::ChatWith(user) => {
                let Screen::Main(main) = &mut self.screen else {
                    return Task::none();
                };
                main.info = None;
                let session = main.session.clone();
                Task::perform(
                    async move { session.direct_chat(&user).await.map_err(|e| e.to_string()) },
                    |result| match result {
                        Ok(room) => Message::OpenRoom(room),
                        Err(e) => Message::Dialog(dialog::Message::Done(Err(e))),
                    },
                )
            }
            Message::Notified(notification) => {
                let Screen::Main(main) = &self.screen else {
                    return Task::none();
                };
                let looking = self.focused
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
                        Task::done(Message::NotificationClicked(room.clone()))
                    } else {
                        Task::none()
                    }
                })
            }
            Message::NotificationClicked(room) => {
                // Shown again if closing hid it (macOS).
                let raise = window::oldest().then(|id| match id {
                    Some(id) => window::mode(id).then(move |mode| {
                        if mode == window::Mode::Hidden {
                            window::set_mode(id, window::Mode::Windowed)
                                .chain(window::gain_focus(id))
                        } else {
                            window::gain_focus(id)
                        }
                    }),
                    None => Task::none(),
                });
                Task::batch([raise, self.update(Message::OpenRoom(room))])
            }
            Message::WindowFocused(focused) => {
                self.focused = focused;
                let Screen::Main(main) = &mut self.screen else {
                    return Task::none();
                };
                match main.activity.focus(focused, Instant::now()) {
                    Some(state) if self.settings.share_presence => main.set_presence(state),
                    _ => Task::none(),
                }
            }
            Message::Drafts(drafts) => {
                if let Screen::Main(main) = &mut self.screen {
                    main.drafts = drafts;
                }
                Task::none()
            }
            Message::CloseWindow(id) => {
                // What is being written is kept before the session ends.
                let draft = match &self.screen {
                    Screen::Main(main) => main.pane.room().and_then(|room| {
                        let draft = room.draft_to_keep()?;
                        Some(save_draft(&main.session, room.id.clone(), draft))
                    }),
                    _ => None,
                }
                .unwrap_or_else(Task::none);
                // Others see us offline now, not once the server forgets us.
                let offline = match &self.screen {
                    Screen::Main(main)
                        if self.settings.share_presence && !main.presences.unavailable() =>
                    {
                        let session = main.session.clone();
                        Task::future(async move { presence::go_offline(&session).await }).discard()
                    }
                    _ => Task::none(),
                };
                // iced drops the app only after its runtime has stopped, when
                // the SDK's stores can no longer close: end the session (once
                // it is offline), or a browser sign-in, now.
                self.screen = Screen::Closing;
                Task::batch([offline, draft]).chain(window::close(id))
            }
            Message::Presence(message) => self.presence(message),
            Message::CloseNotice => {
                if let Screen::Main(main) = &mut self.screen {
                    main.notice = None;
                }
                Task::none()
            }
            Message::Progress => {
                if let Screen::Main(main) = &mut self.screen
                    && let Some(dialog) = &mut main.dialog
                {
                    dialog.tick(Instant::now());
                }
                Task::none()
            }
            Message::Scheduled(scheduled) => {
                if let Screen::Main(main) = &mut self.screen {
                    if let Some(room) = main.pane.room_mut() {
                        room.scheduled_changed(&scheduled);
                    }
                    main.scheduled = scheduled;
                    main.now = SystemTime::now();
                }
                Task::none()
            }
            Message::Tick => {
                if let Screen::Main(main) = &mut self.screen {
                    main.now = SystemTime::now();
                }
                Task::none()
            }
            Message::RecoveryKey(key) => {
                if let Screen::Main(main) = &mut self.screen {
                    main.recovery_key = key;
                }
                Task::none()
            }
            Message::Recover => {
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
                            let _ = tx.clone().try_send(Message::RecoveryProgress(step));
                        };
                        let result = async {
                            session.recover(&key, progress).await?;
                            session.is_device_verified().await
                        }
                        .await;
                        let _ = tx
                            .send(Message::Recovered(result.map_err(|e| e.to_string())))
                            .await;
                    }),
                    |message| message,
                )
            }
            Message::RecoveryProgress(step) => {
                if let Screen::Main(main) = &mut self.screen
                    && main.recovery_step.is_some()
                {
                    main.recovery_step = Some(step);
                }
                Task::none()
            }
            Message::Recovered(result) => {
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
            Message::MathRendered(key, rendered) => {
                self.math.insert(key, rendered);
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
                        None => info::sender(user, None, None),
                    };
                    Task::done(Message::ShowInfo(card))
                }
                None => {
                    let _ = open::that_detached(url);
                    Task::none()
                }
            },
            Message::WindowResized(size) => {
                // Reported in layout units.
                self.window = size * self.scale_factor();
                self.rescaled()
            }
            Message::WindowOpened(id, Some(displays), Some(position)) => {
                // Left on a display since unplugged, the window opens on the
                // main one instead, as the first time.
                let Some(left) = self.placement.frame else {
                    return Task::none();
                };
                if Frame::new(position, left.size()).reachable(&displays) {
                    return Task::none();
                }
                let Some(frame) = Frame::fallback(&displays) else {
                    return Task::none();
                };
                Task::batch([
                    window::maximize(id, false),
                    window::resize(id, frame.size() / self.scale_factor()),
                    window::move_to(id, frame.position()),
                ])
            }
            Message::WindowOpened(..) => Task::none(),
            // Not while closing hid it (macOS): where it is not shown.
            Message::WindowMoved(id) => window::mode(id).then(move |mode| {
                if mode == window::Mode::Hidden {
                    return Task::none();
                }
                window::is_maximized(id).then(move |maximized| {
                    window::position(id).map(move |position| Message::WindowPlaced {
                        position,
                        maximized,
                        fullscreen: mode == window::Mode::Fullscreen,
                    })
                })
            }),
            Message::WindowPlaced {
                position: Some(position),
                maximized,
                fullscreen,
            } => {
                let frame = Frame::new(position, self.window);
                if self.placement.follow(frame, maximized, fullscreen)
                    && let Some(file) = &self.placement_file
                {
                    self.placement.save(file);
                }
                Task::none()
            }
            Message::WindowPlaced { position: None, .. } => Task::none(),
            Message::ScaleFactor(scale) => {
                self.scale = scale;
                self.rescaled()
            }
            Message::Zoom(zoom) => {
                if let Screen::Main(main) = &mut self.screen
                    && let Some(pdf) = &mut main.pdf
                {
                    return pdf.update(pdf_viewer::Message::Zoom(match zoom {
                        Zoom::In => pdf_viewer::Zoom::In,
                        Zoom::Out => pdf_viewer::Zoom::Out,
                        Zoom::Reset => pdf_viewer::Zoom::Fit,
                    }));
                }
                self.settings.text_size = self.settings.text_size.zoom(zoom);
                self.save_settings();
                self.rescaled()
            }
            Message::ThemeMode(mode) => {
                self.system_mode = mode;
                self.render_math()
            }
            Message::AvatarLoaded(key, handle) => {
                if let Screen::Main(main) = &mut self.screen {
                    main.avatars.insert(key, handle);
                }
                Task::none()
            }
            Message::PreviewLoaded(media, picture) => {
                if let Screen::Main(main) = &mut self.screen {
                    main.previews.insert(media, picture);
                }
                Task::none()
            }
            Message::OpenImage(media) => {
                let Screen::Main(main) = &mut self.screen else {
                    return Task::none();
                };
                let viewer = Viewer::new(media);
                let load = viewer.load(main.session.clone());
                main.viewer = Some(viewer);
                load
            }
            Message::ViewerMenu(open) => {
                if let Screen::Main(main) = &mut self.screen
                    && let Some(viewer) = &mut main.viewer
                {
                    viewer.menu = open;
                }
                Task::none()
            }
            Message::CloseImage => {
                if let Screen::Main(main) = &mut self.screen {
                    main.viewer = None;
                    main.pdf = None;
                }
                Task::none()
            }
            Message::OpenPdf(media) => {
                let viewer = pdf_viewer::Viewer::new(media, self.viewport(), self.pixel_scale());
                let Screen::Main(main) = &mut self.screen else {
                    return Task::none();
                };
                let open = viewer.open(main.session.clone());
                main.pdf = Some(viewer);
                open
            }
            Message::Pdf(message) => match &mut self.screen {
                Screen::Main(main) => main
                    .pdf
                    .as_mut()
                    .map_or_else(Task::none, |viewer| viewer.update(message)),
                _ => Task::none(),
            },
            Message::SaveMedia(file, from) => {
                let Screen::Main(main) = &mut self.screen else {
                    return Task::none();
                };
                close_menu(main, &from);
                Task::perform(media::save(main.session.clone(), file), move |result| {
                    Message::MediaDone(from.clone(), result)
                })
            }
            Message::CopyImage(file, from) => {
                let Screen::Main(main) = &mut self.screen else {
                    return Task::none();
                };
                close_menu(main, &from);
                Task::perform(
                    media::copy_image(main.session.clone(), file),
                    move |result| Message::MediaDone(from.clone(), result),
                )
            }
            Message::MediaDone(from, result) => {
                let (Screen::Main(main), Err(e)) = (&mut self.screen, result) else {
                    return Task::none();
                };
                // Shown where it was asked from, else as a notice.
                let error = match &from {
                    media::Origin::Viewer => match (&mut main.viewer, &mut main.pdf) {
                        (Some(viewer), _) => Some(&mut viewer.error),
                        (None, Some(pdf)) => Some(&mut pdf.error),
                        (None, None) => None,
                    },
                    media::Origin::Room(id) => main.pane.open_mut(id).map(|room| &mut room.error),
                };
                match error {
                    Some(error) => *error = Some(e),
                    None => {
                        main.notice = Some(Notice {
                            text: e,
                            room: None,
                            error: true,
                        })
                    }
                }
                Task::none()
            }
            Message::ImageLoaded(media, result) => {
                if let Screen::Main(main) = &mut self.screen
                    && let Some(viewer) = &mut main.viewer
                {
                    viewer.loaded(&media, result);
                }
                Task::none()
            }
        }
    }

    /// A message for room `id`. One for a room no longer open goes to
    /// [`App::room_left`] first.
    fn room(&mut self, id: OwnedRoomId, message: room::Message) -> Task<Message> {
        if !matches!(&self.screen, Screen::Main(main) if main.pane.open(&id).is_some()) {
            return self.room_left(id, message);
        }
        match message {
            room::Message::Timeline(entries) => {
                let Screen::Main(main) = &mut self.screen else {
                    return Task::none();
                };
                let Some(room) = main.pane.open_mut(&id) else {
                    return Task::none();
                };
                let grew = entries.len() != room.entries.len();
                room.entries = entries;
                room.find_again();
                room.resolve_reply();
                let read = if grew { mark_read(room) } else { Task::none() };
                let clocks = room.sending_changed().into_iter().map(|row| {
                    let id = id.clone();
                    Task::perform(tokio::time::sleep(room::SENDING_CLOCK), move |()| {
                        room::Message::SendingSlow(row.clone()).to(&id)
                    })
                });
                Task::batch([
                    Task::batch(clocks),
                    read,
                    fill(room),
                    self.render_math(),
                    self.load_avatars(),
                    self.load_previews(),
                ])
            }
            room::Message::Scrolled(viewport) => {
                let Screen::Main(main) = &mut self.screen else {
                    return Task::none();
                };
                let Some(room) = main.pane.open_mut(&id) else {
                    return Task::none();
                };
                match room.older(&viewport) {
                    Older::None => Task::none(),
                    Older::ShowMore => {
                        room.show_more();
                        Task::batch([
                            self.render_math(),
                            self.load_avatars(),
                            self.load_previews(),
                        ])
                    }
                    Older::Paginate => paginate(room),
                }
            }
            room::Message::Paginated(result) => {
                let Screen::Main(main) = &mut self.screen else {
                    return Task::none();
                };
                let Some(room) = main.pane.open_mut(&id) else {
                    return Task::none();
                };
                room.paginating = false;
                let seeking = room.seeking.take();
                let more = match result {
                    Ok(()) => {
                        room.show_more();
                        match seeking {
                            Some((event, pages)) => go_to(room, event, pages),
                            None => fill(room),
                        }
                    }
                    // Loaded once the server is back.
                    Err(_) if main.connection != Connection::Online => Task::none(),
                    Err(e) => {
                        room.error = Some(e);
                        Task::none()
                    }
                };
                Task::batch([
                    more,
                    self.render_math(),
                    self.load_avatars(),
                    self.load_previews(),
                ])
            }
            room::Message::Composer(action) => {
                let Screen::Main(main) = &mut self.screen else {
                    return Task::none();
                };
                let Some(room) = main.pane.open_mut(&id) else {
                    return Task::none();
                };
                let edit = action.is_edit();
                room.composer.perform(action);
                room.update_completion();
                // Mentions need the members: loaded on the first `@`.
                let members = if room.completion.is_some()
                    && room.member_list.is_none()
                    && !room.loading_members
                {
                    load_members(&main.session, room)
                } else {
                    Task::none()
                };
                let typing = if edit {
                    let timeline = room.timeline.clone();
                    let typing = !room.composer.text().trim().is_empty();
                    Task::future(async move {
                        let _ = timeline.typing(typing).await;
                    })
                    .discard()
                } else {
                    Task::none()
                };
                // The pictures of those offered.
                let avatars = if room.completion.is_some() {
                    self.load_avatars()
                } else {
                    Task::none()
                };
                Task::batch([members, typing, avatars])
            }
            room::Message::TypeInComposer(text) => {
                if self.covered() {
                    return Task::none();
                }
                // Typed, so that it undoes with what is typed next.
                let typed: Vec<_> = text
                    .chars()
                    .map(|c| {
                        let edit = text_editor::Action::Edit(text_editor::Edit::Insert(c));
                        self.update(room::Message::Composer(edit).to(&id))
                    })
                    .collect();
                Task::batch(
                    typed
                        .into_iter()
                        .chain([iced::widget::operation::focus(room::COMPOSER)]),
                )
            }
            room::Message::Mention(message) => {
                let Screen::Main(main) = &mut self.screen else {
                    return Task::none();
                };
                let Some(room) = main.pane.open_mut(&id) else {
                    return Task::none();
                };
                let count = room.candidates(&main.own_user).len();
                let mention = match message {
                    mention::Message::Close => {
                        room.completion = None;
                        return Task::none();
                    }
                    mention::Message::Up | mention::Message::Down => {
                        let step = if matches!(message, mention::Message::Up) {
                            -1
                        } else {
                            1
                        };
                        if let Some(completion) = &mut room.completion {
                            completion.step(step, count);
                        }
                        return Task::none();
                    }
                    mention::Message::Choose => {
                        let selected = room.completion.as_ref().map_or(0, |c| c.selected);
                        match room.candidates(&main.own_user).get(selected) {
                            Some(candidate) => candidate.mention(),
                            None => return Task::none(),
                        }
                    }
                    mention::Message::Pick(mention) => mention,
                };
                room.complete(&mention);
                iced::widget::operation::focus(room::COMPOSER)
            }
            room::Message::MembersLoaded(result) => {
                if let Screen::Main(main) = &mut self.screen
                    && let Some(room) = main.pane.open_mut(&id)
                {
                    room.members_loaded(result);
                }
                self.load_avatars()
            }
            room::Message::Send => {
                let Screen::Main(main) = &mut self.screen else {
                    return Task::none();
                };
                let Some(room) = main.pane.open_mut(&id) else {
                    return Task::none();
                };
                let text = room.composer.text();
                if text.trim().is_empty() && room.attachments.is_empty() {
                    return Task::none();
                }
                let compose = std::mem::take(&mut room.compose);
                room.set_composer(text_editor::Content::new());
                let files = std::mem::take(&mut room.attachments)
                    .into_iter()
                    .map(|attached| attached.file)
                    .collect();
                room.error = None;
                let timeline = room.timeline.clone();
                let used = emoji::used(&text);
                // Queue only: the local echo shows the send state. Queued
                // first, so it shows before the server hears of anything.
                let send = Task::perform(
                    async move {
                        let sent = timeline
                            .send(text.trim_end(), files, compose.reply_to().map(|id| &**id))
                            .await
                            .map(|_| ());
                        let _ = timeline.typing(false).await;
                        sent
                    },
                    move |result| room::Message::Sent(result.map_err(|e| e.to_string())).to(&id),
                );
                Task::batch([send, used_emoji(main, used)])
            }
            room::Message::SendingSlow(row) => {
                if let Screen::Main(main) = &mut self.screen
                    && let Some(room) = main.pane.open_mut(&id)
                {
                    room.sending_slow(&row);
                }
                Task::none()
            }
            room::Message::Sent(result) => {
                if let Screen::Main(main) = &mut self.screen
                    && let Some(room) = main.pane.open_mut(&id)
                    && let Err(e) = result
                {
                    room.error = Some(e);
                }
                Task::none()
            }
            room::Message::Later(message) => self.later(id, message),
            room::Message::Act(message) => self.act(id, message),
            // The timeline is anchored at the bottom, so its offset counts
            // upwards.
            room::Message::ScrollTimeline(dy) => iced::widget::operation::scroll_by(
                room::TIMELINE,
                iced::widget::scrollable::AbsoluteOffset { x: 0.0, y: -dy },
            ),
            room::Message::ScrollKey(scroll) => {
                let Screen::Main(main) = &mut self.screen else {
                    return Task::none();
                };
                let Some(room) = main.pane.open_mut(&id) else {
                    return Task::none();
                };
                if scroll != room::Scroll::Oldest {
                    return room::scroll(scroll);
                }
                room.show_all();
                Task::batch([
                    room::scroll(scroll),
                    self.render_math(),
                    self.load_avatars(),
                    self.load_previews(),
                ])
            }
            room::Message::Select(message) => {
                let Screen::Main(main) = &mut self.screen else {
                    return Task::none();
                };
                let Some(room) = main.pane.open_mut(&id) else {
                    return Task::none();
                };
                match message {
                    // What reply previews show first, as they are cut to fit.
                    selection::Message::Copy => {
                        return room.read_shown().map(move |shown| {
                            room::Message::Select(selection::Message::CopyAsShown(shown)).to(&id)
                        });
                    }
                    selection::Message::CopyAsShown(shown) => {
                        return room
                            .selected(&shown)
                            .map_or_else(Task::none, |copied| copy(&id, copied));
                    }
                    selection::Message::SelectAll => {
                        return room.on_screen().and_then(move |(first, last)| {
                            Task::done(
                                room::Message::Select(selection::Message::All(first, last)).to(&id),
                            )
                        });
                    }
                    selection::Message::Extend(step) => {
                        room.extend(step);
                        return Task::none();
                    }
                    selection::Message::ExtendLine(up) => return room.extend_line(up),
                    selection::Message::ExtendToLineEdge(forward) => {
                        return room.extend_to_line_edge(forward);
                    }
                    selection::Message::ExtendToScreenEdge(up) => {
                        return room.extend_to_screen_edge(up);
                    }
                    selection::Message::ExtendToPart(event, start) => {
                        room.extend_to_part(event, start);
                        return Task::none();
                    }
                    selection::Message::LineMoved(from, up, offset) => {
                        room.line_moved(from, up, offset);
                        return Task::none();
                    }
                    _ => {}
                }
                Selection::update(&mut room.selection, message);
                Task::none()
            }
            room::Message::Find(message) => self.find(id, message),
            room::Message::Typing(names) => {
                if let Screen::Main(main) = &mut self.screen
                    && let Some(room) = main.pane.open_mut(&id)
                {
                    room.typing = names;
                }
                Task::none()
            }
            room::Message::Calls(message) => self.calls(id, message),
            room::Message::Topic(message) => self.room_topic(id, message),
            room::Message::Uploads(uploads) => {
                if let Screen::Main(main) = &mut self.screen
                    && let Some(room) = main.pane.open_mut(&id)
                {
                    room.set_uploads(uploads);
                }
                Task::none()
            }
            room::Message::DismissError | room::Message::StopSeeking => {
                if let Screen::Main(main) = &mut self.screen
                    && let Some(room) = main.pane.open_mut(&id)
                {
                    match message {
                        room::Message::DismissError => room.error = None,
                        _ => room.seeking = None,
                    }
                }
                Task::none()
            }
            room::Message::Pins(message) => {
                let Screen::Main(main) = &mut self.screen else {
                    return Task::none();
                };
                let Some(room) = main.pane.open_mut(&id) else {
                    return Task::none();
                };
                if let pins::Message::GoTo(event) = message {
                    room.topic.open = false;
                    return Task::done(room::Message::GoTo(event).to(&id));
                }
                room.pins.update(message);
                Task::none()
            }
            room::Message::ComposerEmoji(message) => {
                let Screen::Main(main) = &mut self.screen else {
                    return Task::none();
                };
                let Some(room) = main.pane.open_mut(&id) else {
                    return Task::none();
                };
                match message {
                    emoji::Message::Close => {
                        room.composer_emoji = None;
                        Task::none()
                    }
                    emoji::Message::Open => {
                        room.composer_emoji = Some(emoji::Panel::default());
                        Task::batch([
                            iced::widget::operation::focus(emoji::SEARCH),
                            load_frequent_emoji(&main.session),
                        ])
                    }
                    message => {
                        if let Some(panel) = &mut room.composer_emoji {
                            panel.update(message);
                        }
                        Task::none()
                    }
                }
            }
            room::Message::InsertEmoji(emoji) => {
                let Screen::Main(main) = &mut self.screen else {
                    return Task::none();
                };
                let Some(room) = main.pane.open_mut(&id) else {
                    return Task::none();
                };
                emoji::insert(&mut room.composer, &emoji);
                room.update_completion();
                // Counted as used once sent.
                iced::widget::operation::focus(room::COMPOSER)
            }
            room::Message::Reloaded(result) => {
                let Screen::Main(main) = &mut self.screen else {
                    return Task::none();
                };
                let Some(room) = main.pane.open_mut(&id) else {
                    return Task::none();
                };
                match result {
                    Ok(Shared(timeline)) => {
                        room.reload(timeline);
                        Task::batch([
                            fill(room),
                            mark_read(room),
                            self.render_math(),
                            self.load_avatars(),
                            self.load_previews(),
                            self.load_room_calls(&id),
                        ])
                    }
                    Err(e) => {
                        room.reloading = false;
                        room.error = Some(e);
                        Task::none()
                    }
                }
            }
            room::Message::DraftLoaded(draft) => {
                if let Screen::Main(main) = &mut self.screen
                    && let Some(room) = main.pane.open_mut(&id)
                {
                    room.restore_draft(draft);
                }
                Task::none()
            }
            room::Message::KeepDraft(changes) => {
                let Screen::Main(main) = &mut self.screen else {
                    return Task::none();
                };
                let Some(room) = (main.pane.open_mut(&id)).filter(|r| r.draft_changes == changes)
                else {
                    return Task::none();
                };
                match room.draft_to_keep() {
                    Some(draft) => {
                        room.kept_draft = Some(draft.clone());
                        save_draft(&main.session, id, draft)
                    }
                    None => Task::none(),
                }
            }
            room::Message::Hover(row, entered) => {
                if let Screen::Main(main) = &mut self.screen
                    && let Some(room) = main.pane.open_mut(&id)
                {
                    room.hover(row, entered);
                }
                Task::none()
            }
            room::Message::GoTo(event) => {
                let Screen::Main(main) = &mut self.screen else {
                    return Task::none();
                };
                let Some(room) = main.pane.open_mut(&id) else {
                    return Task::none();
                };
                room.error = None;
                let go = go_to(room, event, 0);
                Task::batch([
                    go,
                    self.render_math(),
                    self.load_avatars(),
                    self.load_previews(),
                ])
            }
            room::Message::Unmark(row) => {
                if let Screen::Main(main) = &mut self.screen
                    && let Some(room) = main.pane.open_mut(&id)
                    && room.marked.as_ref() == Some(&row)
                {
                    room.marked = None;
                }
                Task::none()
            }
            room::Message::Attach => {
                Task::future(media::pick_files()).then(move |paths| attach(id.clone(), paths))
            }
            room::Message::FileDropped(path) => attach(id, vec![path]),
            room::Message::Paste(field, how) => {
                Task::perform(clipboard::read(how, field), move |pasted| {
                    room::Message::Pasted(field, pasted).to(&id)
                })
            }
            room::Message::Pasted(field, pasted) => match pasted {
                // As if dropped.
                Some(Pasted::Files(paths)) => attach(id, paths),
                Some(Pasted::Image(file)) => {
                    Task::perform(media::attached(file), move |attached| {
                        room::Message::Attached(vec![attached]).to(&id)
                    })
                }
                Some(Pasted::Text(text)) => self.update(in_field(
                    &id,
                    field,
                    text_editor::Action::Edit(text_editor::Edit::Paste(Arc::new(text))),
                )),
                None => Task::none(),
            },
            room::Message::EditMenu(field, message) => {
                use edit_menu::Message as Menu;
                use text_editor::{Action, Edit};

                let Screen::Main(main) = &mut self.screen else {
                    return Task::none();
                };
                let Some(room) = main.pane.open_mut(&id) else {
                    return Task::none();
                };
                let (content, menu, focus) = match field {
                    Field::Composer => (
                        &mut room.composer,
                        &mut room.composer_menu,
                        iced::widget::operation::focus(room::COMPOSER),
                    ),
                    Field::Edit => {
                        let Some(edit) = room.editing.as_mut() else {
                            return Task::none();
                        };
                        let (content, menu) = edit.with_menu();
                        (content, menu, InlineEdit::focus())
                    }
                };
                // Any choice closes the menu.
                let misspelled = menu.take().and_then(|menu| menu.misspelled);
                if message == Menu::Open {
                    *menu = Some(edit_menu::Menu::at(content));
                    return Task::none();
                }
                let selection = content.selection();
                // The spelling replaces the word as typed over it.
                let correction = match (&message, &misspelled) {
                    (Menu::Correct(i), Some(misspelled)) => {
                        misspelled.select(content);
                        misspelled.suggestions.get(*i).cloned()
                    }
                    _ => None,
                };
                // The field keeps the focus, as a text field does.
                let done = match message {
                    Menu::Open | Menu::Close => return Task::none(),
                    Menu::Correct(_) => correction.map_or_else(Task::none, |spelling| {
                        self.update(in_field(
                            &id,
                            field,
                            Action::Edit(Edit::Paste(Arc::new(spelling))),
                        ))
                    }),
                    Menu::LearnSpelling => {
                        if let Some(misspelled) = misspelled {
                            spelling::learn(&misspelled.word);
                        }
                        Task::none()
                    }
                    Menu::IgnoreSpelling => {
                        if let Some(misspelled) = misspelled {
                            spelling::ignore(&misspelled.word);
                        }
                        Task::none()
                    }
                    Menu::Cut => Task::batch([
                        selection.map_or_else(Task::none, iced::clipboard::write),
                        self.update(in_field(&id, field, Action::Edit(Edit::Delete))),
                    ]),
                    Menu::Copy => selection.map_or_else(Task::none, iced::clipboard::write),
                    Menu::Paste => {
                        self.update(room::Message::Paste(field, clipboard::Paste::Rich).to(&id))
                    }
                    Menu::SelectAll => self.update(in_field(&id, field, Action::SelectAll)),
                    #[cfg(target_os = "macos")]
                    Menu::EmojiAndSymbols => {
                        edit_menu::emoji_and_symbols();
                        Task::none()
                    }
                };
                Task::batch([focus, done])
            }
            room::Message::Attached(attached) => {
                if let Screen::Main(main) = &mut self.screen
                    && let Some(room) = main.pane.open_mut(&id)
                {
                    for attached in attached {
                        match attached {
                            Ok(attached) => room.attachments.push(attached),
                            Err(e) => room.error = Some(e),
                        }
                    }
                }
                Task::none()
            }
            room::Message::Unattach(index) => {
                if let Screen::Main(main) = &mut self.screen
                    && let Some(room) = main.pane.open_mut(&id)
                    && index < room.attachments.len()
                {
                    room.attachments.remove(index);
                }
                Task::none()
            }
        }
    }

    /// A message for room `id`, which is not open: the results that still
    /// concern it act on it; anything else is dropped.
    fn room_left(&mut self, id: OwnedRoomId, message: room::Message) -> Task<Message> {
        let Screen::Main(main) = &self.screen else {
            return Task::none();
        };
        match message {
            // The draft kept on leaving holds the text scheduled since.
            room::Message::Later(send_later::Message::Scheduled(Ok(()))) => {
                save_draft(&main.session, id, Draft::default())
            }
            _ => Task::none(),
        }
    }

    fn dialog(&mut self, message: dialog::Message) -> Task<Message> {
        let Screen::Main(main) = &mut self.screen else {
            return Task::none();
        };
        let Some(dialog) = &mut main.dialog else {
            // The dialog was closed while its request ran.
            if let dialog::Message::Done(result) = message {
                let (done, failed) = main.background.take().unwrap_or_default();
                main.notice = Some(match result {
                    Ok(opened) => Notice {
                        text: done,
                        room: match opened {
                            dialog::Opened::Room(id) | dialog::Opened::Space(id) => Some(id),
                            _ => None,
                        },
                        error: false,
                    },
                    Err(e) => Notice {
                        text: format!("{failed}: {e}"),
                        room: None,
                        error: true,
                    },
                });
            }
            return Task::none();
        };
        let session = main.session.clone();
        match dialog.update(message) {
            dialog::Action::None => Task::none(),
            dialog::Action::Close => {
                main.dialog = None;
                Task::none()
            }
            dialog::Action::Background => {
                main.background = Some(dialog.outcome());
                main.dialog = None;
                Task::none()
            }
            dialog::Action::Search(query) => {
                let (task, handle) = Task::perform(
                    {
                        let query = query.clone();
                        async move { session.search_users(&query, 10).await.unwrap_or_default() }
                    },
                    move |users| {
                        Message::Dialog(dialog::Message::Suggestions(query.clone(), users))
                    },
                )
                .abortable();
                dialog.search = Some(handle);
                task
            }
            dialog::Action::Arrange(message) => {
                main.dialog = None;
                self.update(Message::Arrange(message))
            }
            dialog::Action::Submit(request) => Task::perform(run_request(session, request), |r| {
                Message::Dialog(dialog::Message::Done(r))
            }),
            dialog::Action::Finished(opened) => {
                main.dialog = None;
                match opened {
                    dialog::Opened::Nothing => Task::none(),
                    dialog::Opened::Members(room) => self.refresh_members(&room),
                    dialog::Opened::Settings(room) => self.reload_card(&room),
                    dialog::Opened::Room(id) => self.update(Message::OpenRoom(id)),
                    dialog::Opened::Space(id) => self.update(Message::SelectSpace(Some(id))),
                    dialog::Opened::Left(id) => {
                        if main.pane.id() == Some(&id) {
                            main.pane = Pane::Empty;
                        }
                        if main.info.as_ref().and_then(|c| c.room()) == Some(&id) {
                            main.info = None;
                        }
                        if main.space.as_ref() == Some(&id) {
                            main.space = None;
                        }
                        Task::none()
                    }
                    dialog::Opened::SignedOut => {
                        self.screen = Screen::Login(Login::default());
                        Task::none()
                    }
                }
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
                main.rooms
                    .rooms
                    .iter()
                    .find(|r| &r.id == id)
                    .map_or_else(|| id.to_string(), |r| r.name.clone())
            };
            if let Some(room) = main.pane.room() {
                panel.room = Some(settings::Access::new(
                    room.id.clone(),
                    name(&room.id),
                    false,
                ));
                tasks.push(load_access(main.session.clone(), room.id.clone()));
            }
            if let Some(space) = &main.space {
                panel.space = Some(settings::SpaceCalls::new(space.clone(), name(space)));
                tasks.push(load_access(main.session.clone(), space.clone()));
                let (session, space) = (main.session.clone(), space.clone());
                tasks.push(Task::perform(
                    async move {
                        let service = session.space_call_service(&space).await.ok().flatten();
                        let can_set = session.can_set_space_call_service(&space).await.ok();
                        settings::Message::SpaceLoaded(space, service, can_set)
                    },
                    Message::Settings,
                ));
            }
        }
        self.settings_panel = Some(panel);
        self.update_seen();
        Task::batch(tasks)
    }

    /// An update shown in the open settings is seen: no dot on the gear
    /// until a newer one.
    fn update_seen(&mut self) {
        if self.settings_panel.is_none() || !self.updater.unseen(&self.settings.updates) {
            return;
        }
        let version = (self.updater.available.as_ref()).map(|u| u.version.to_string());
        self.settings.updates.seen = version;
        self.save_settings();
    }

    /// Room list colours chosen before the account kept them, moved there
    /// unless the account has colours already.
    fn move_colours_to_account(&self, session: &Arc<Session>) -> Task<Message> {
        let Some(legacy) = self.settings.legacy_colours() else {
            return Task::none();
        };
        let session = session.clone();
        Task::perform(
            async move {
                if session.room_list_colours() != RoomListColours::default() {
                    return Ok(());
                }
                session
                    .change_room_list_colours(|colours| *colours = legacy)
                    .await
                    .map_err(|e| e.to_string())
            },
            Message::ColoursMoved,
        )
    }

    /// Saves the settings; the open panel says if that failed.
    fn save_settings(&mut self) {
        let error = self
            .settings_file
            .as_ref()
            .and_then(|file| self.settings.save(file).err())
            .map(|e| e.to_string());
        if let Some(panel) = &mut self.settings_panel {
            panel.error = error;
        }
    }

    fn settings_update(&mut self, message: settings::Message) -> Task<Message> {
        // Preferences change also from outside the panel, e.g. a card.
        let action = match (message, &mut self.settings_panel) {
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
                self.render_math()
            }
            settings::Action::TextSize => {
                self.save_settings();
                self.rescaled()
            }
            settings::Action::Spelling => {
                self.save_settings();
                spelling::set_enabled(self.settings.check_spelling);
                Task::none()
            }
            settings::Action::SharePresence => {
                self.save_settings();
                let Screen::Main(main) = &mut self.screen else {
                    return Task::none();
                };
                let state = if self.settings.share_presence {
                    main.activity.restart(Instant::now())
                } else {
                    PresenceState::Offline
                };
                main.set_presence(state)
            }
            settings::Action::GoogleSignIn(client) => {
                let google = self.google.clone();
                let (task, handle) = Task::perform(
                    async move {
                        google
                            .sign_in(client, |url| {
                                let _ = open::that_detached(url.as_str());
                            })
                            .await
                    },
                    |result| {
                        Message::Settings(settings::Message::GoogleSignedIn(
                            result.map_err(|e| e.to_string()),
                        ))
                    },
                )
                .abortable();
                // Asked for from the panel, which shows it signing in.
                if let Some(panel) = &mut self.settings_panel {
                    panel.google.signing_in = Some(handle);
                }
                task
            }
            settings::Action::Colour(space, tint) => {
                let Screen::Main(main) = &self.screen else {
                    return Task::none();
                };
                let session = main.session.clone();
                Task::perform(
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
                        Message::Settings(settings::Message::ColourSaved(
                            result.map_err(|e| e.to_string()),
                        ))
                    },
                )
            }
            settings::Action::AtHome(at_home) => {
                self.update(Message::Arrange(arrange::Message::AtHome(at_home)))
            }
            settings::Action::NotesAtTop(on) => {
                self.update(Message::Arrange(arrange::Message::NotesAtTop(on)))
            }
            settings::Action::GoogleSignOut => {
                let google = self.google.clone();
                Task::perform(async move { google.sign_out().await }, |result| {
                    Message::Settings(settings::Message::GoogleSignedOut(
                        result.map_err(|e| e.to_string()),
                    ))
                })
            }
            settings::Action::LetEveryoneEdit { id, space } => {
                let Screen::Main(main) = &self.screen else {
                    return Task::none();
                };
                let session = main.session.clone();
                Task::perform(
                    {
                        let id = id.clone();
                        async move {
                            if space {
                                session.let_everyone_edit_space(&id).await.map(|update| {
                                    let skipped = match update.not_allowed.len() {
                                        0 => String::new(),
                                        n => format!(
                                            " {n} could not be changed: you are not an admin there."
                                        ),
                                    };
                                    format!("Opened {} room(s).{skipped}", update.changed.len())
                                })
                            } else {
                                session
                                    .let_everyone_edit(&id)
                                    .await
                                    .map(|()| "Done.".to_owned())
                            }
                        }
                    },
                    move |result| {
                        Message::Settings(settings::Message::Opened(
                            id.clone(),
                            result.map_err(|e| e.to_string()),
                        ))
                    },
                )
            }
            settings::Action::SetSpaceService(space, service) => {
                let Screen::Main(main) = &self.screen else {
                    return Task::none();
                };
                let session = main.session.clone();
                Task::perform(
                    async move { session.set_space_call_service(&space, service).await },
                    |result| {
                        Message::Settings(settings::Message::SpaceSaved(
                            result.map_err(|e| e.to_string()),
                        ))
                    },
                )
            }
        }
    }

    fn presence(&mut self, message: presence::Message) -> Task<Message> {
        let Screen::Main(main) = &mut self.screen else {
            return Task::none();
        };
        let now = Instant::now();
        let ours = match message {
            presence::Message::Input => main.activity.input(now),
            presence::Message::CheckIdle => main.activity.check(now),
            message => {
                main.presences.update(message);
                None
            }
        };
        match ours {
            Some(state) if self.settings.share_presence => main.set_presence(state),
            _ => Task::none(),
        }
    }

    fn calls(&mut self, id: OwnedRoomId, message: calls::Message) -> Task<Message> {
        use calls::Message as Calls;
        let Screen::Main(main) = &mut self.screen else {
            return Task::none();
        };
        let Some(room) = main.pane.open_mut(&id) else {
            return Task::none();
        };
        let session = main.session.clone();
        let google = self.google.clone();
        let state = &mut room.calls;
        let moved = match message {
            Calls::SpaceService(service) => {
                state.space_service = service;
                return Task::none();
            }
            Calls::OpenMenu => {
                state.menu = true;
                state.error = None;
                return Task::none();
            }
            Calls::Close => {
                state.close();
                return Task::none();
            }
            Calls::Open(url) => {
                state.close();
                return Task::done(Message::LinkClicked(url));
            }
            Calls::Start(service) => {
                state.busy = true;
                state.menu = false;
                state.error = None;
                let room = room.id.clone();
                return Task::perform(
                    async move {
                        let link = new_call_link(&service, &google).await?;
                        session
                            .post_call(&room, &service.name, &link)
                            .await
                            .map_err(|e| e.to_string())
                    },
                    move |result| room::Message::Calls(Calls::Started(result)).to(&id),
                );
            }
            Calls::Started(result) => {
                state.busy = false;
                // Shown below the composer, like other sending errors.
                if let Err(e) = result {
                    room.error = Some(e);
                }
                return Task::none();
            }
            Calls::Calls(order) => state.calls.update(order).map(|moved| (true, moved)),
            Calls::Links(order) => state.links.update(order).map(|moved| (false, moved)),
        };
        let Some((calls, moved)) = moved else {
            return Task::none();
        };
        // A call moved to the top becomes the room's: the topic changes.
        let synced = (main.rooms.rooms.iter())
            .find(|r| r.id == id)
            .and_then(|r| r.topic.as_deref());
        let shown = Topic::parse(room.topic.shown(synced));
        let topic = topic::moved(shown, calls, moved).to_topic();
        save_topic(
            &main.session,
            &mut room.topic,
            id.clone(),
            topic,
            move |message| room::Message::Topic(message).to(&id),
        )
    }

    /// The open room's topic panel.
    fn room_topic(&mut self, id: OwnedRoomId, message: topic::Message) -> Task<Message> {
        let Screen::Main(main) = &mut self.screen else {
            return Task::none();
        };
        let Some(room) = main.pane.open_mut(&id) else {
            return Task::none();
        };
        // From the call menu too.
        if let topic::Message::Edit = message {
            room.calls.close();
        }
        let synced = (main.rooms.rooms.iter())
            .find(|r| r.id == id)
            .and_then(|r| r.topic.as_deref());
        topic_update(
            &main.session,
            &mut room.topic,
            id.clone(),
            synced,
            message,
            move |message| room::Message::Topic(message).to(&id),
        )
    }

    /// The selected space's topic panel.
    fn space_topic(&mut self, space: OwnedRoomId, message: topic::Message) -> Task<Message> {
        let Screen::Main(main) = &mut self.screen else {
            return Task::none();
        };
        if main.space.as_ref() != Some(&space) {
            return Task::none();
        }
        let synced = (main.rooms.rooms.iter())
            .find(|r| r.id == space)
            .and_then(|r| r.topic.as_deref());
        topic_update(
            &main.session,
            &mut main.space_topic,
            space.clone(),
            synced,
            message,
            move |message| Message::SpaceTopic(space.clone(), message),
        )
    }

    /// Something shows over the main view, as [`App::view`] puts it there:
    /// the settings, a dialog, an info card or a viewer.
    fn covered(&self) -> bool {
        let Screen::Main(main) = &self.screen else {
            return false;
        };
        self.settings_panel.is_some()
            || main.dialog.is_some()
            || main.join.is_some()
            || main.info.is_some()
            || main.viewer.is_some()
            || main.pdf.is_some()
    }

    /// Whether we may change a newly opened room's topic, and the call
    /// service of its space; its old call link moved into its topic.
    fn load_room_calls(&self, room_id: &OwnedRoomId) -> Task<Message> {
        let Screen::Main(main) = &self.screen else {
            return Task::none();
        };
        let session = main.session.clone();
        let spaces = sidebar::ancestors(&main.rooms.rooms, room_id);
        let room = room_id.clone();
        let can_edit = load_can_edit_topic(&session, room.clone(), {
            let room = room.clone();
            move |message| room::Message::Topic(message).to(&room)
        });
        let old_link = {
            let (session, room) = (session.clone(), room.clone());
            Task::future(async move {
                if let Err(e) = session.move_call_link_to_topic(&room).await {
                    tracing::warn!("moving the call link of {room} into its topic: {e}");
                }
            })
            .discard()
        };
        let can_pin = {
            let (session, room, reply) = (session.clone(), room.clone(), room.clone());
            Task::perform(
                async move { session.can_pin(&room).await.ok() },
                move |can| room::Message::Pins(pins::Message::CanPin(can)).to(&reply),
            )
        };
        let space_service = Task::perform(
            async move {
                for space in spaces {
                    if let Ok(Some(service)) = session.space_call_service(&space).await {
                        return Some(service);
                    }
                }
                None
            },
            move |service| room::Message::Calls(calls::Message::SpaceService(service)).to(&room),
        );
        Task::batch([can_edit, can_pin, old_link, space_service])
    }

    /// Make the open room's pinned messages' timeline, once it has pins.
    fn load_pinned(&mut self) -> Task<Message> {
        let Screen::Main(main) = &mut self.screen else {
            return Task::none();
        };
        let Some(room) = main.pane.room_mut() else {
            return Task::none();
        };
        let pinned = (main.rooms.rooms.iter())
            .find(|r| r.id == room.id)
            .map_or(&[][..], |r| &r.pinned);
        if !room.pins.request(pinned) {
            return Task::none();
        }
        let (session, id, reply) = (main.session.clone(), room.id.clone(), room.id.clone());
        Task::perform(
            async move {
                (session.pinned_timeline(&id).await)
                    .map(|t| Shared(Arc::new(t)))
                    .map_err(|e| e.to_string())
            },
            move |result| room::Message::Pins(pins::Message::Loaded(result)).to(&reply),
        )
    }

    fn act(&mut self, id: OwnedRoomId, message: actions::Message) -> Task<Message> {
        use actions::Message as Act;
        let Screen::Main(main) = &mut self.screen else {
            return Task::none();
        };
        let Some(room) = main.pane.open_mut(&id) else {
            return Task::none();
        };
        let timeline = room.timeline.clone();
        let done = {
            let id = id.clone();
            move |result: mtrx_core::Result<()>| {
                actions::act(&id, Act::Done(result.map_err(|e| e.to_string())))
            }
        };
        match message {
            Act::Pin(event, pin) => {
                let (session, room_id) = (main.session.clone(), room.id.clone());
                Task::perform(
                    async move { session.set_pinned(&room_id, &event, pin).await },
                    done,
                )
            }
            Act::React(key, emoji) => {
                room.picker = None;
                room.emoji = None;
                let react = Task::perform(
                    {
                        let emoji = emoji.clone();
                        async move { timeline.toggle_reaction(&key, &emoji).await }
                    },
                    done,
                );
                Task::batch([react, used_emoji(main, vec![emoji])])
            }
            Act::EditEmoji(message) => {
                let Some(edit) = &mut room.editing else {
                    return Task::none();
                };
                match message {
                    emoji::Message::Close => {
                        edit.emoji = None;
                        Task::none()
                    }
                    emoji::Message::Open => {
                        edit.emoji = Some(emoji::Panel::default());
                        Task::batch([
                            iced::widget::operation::focus(emoji::SEARCH),
                            load_frequent_emoji(&main.session),
                        ])
                    }
                    message => {
                        if let Some(panel) = &mut edit.emoji {
                            panel.update(message);
                        }
                        Task::none()
                    }
                }
            }
            Act::EditInsertEmoji(emoji) => {
                if let Some(edit) = &mut room.editing {
                    edit.insert_emoji(&emoji);
                }
                // Counted as used once saved.
                InlineEdit::focus()
            }
            Act::OpenMenu(id, at) => {
                room.menu = Some((id, at));
                room.picker = None;
                room.emoji = None;
                Task::none()
            }
            Act::CloseMenu => {
                room.menu = None;
                Task::none()
            }
            Act::OpenReaders(id) => {
                room.readers = Some(id);
                Task::none()
            }
            Act::CloseReaders => {
                room.readers = None;
                Task::none()
            }
            Act::OpenReactors(id, key) => {
                room.reactors = Some((id, key));
                Task::none()
            }
            Act::CloseReactors => {
                room.reactors = None;
                Task::none()
            }
            Act::OpenUnsent(id) => {
                room.unsent = Some(id);
                Task::none()
            }
            Act::CloseUnsent => {
                room.unsent = None;
                Task::none()
            }
            Act::Retry(key) => {
                room.unsent = None;
                Task::perform(async move { timeline.retry(&key).await }, done)
            }
            Act::Discard(key) => {
                room.unsent = None;
                Task::perform(async move { timeline.discard(&key).await }, done)
            }
            Act::OpenLink(url) => {
                room.menu = None;
                Task::done(Message::LinkClicked(url))
            }
            Act::Copy(copied) => {
                room.menu = None;
                copy(&id, copied)
            }
            Act::CopyLink(url) => {
                room.menu = None;
                iced::clipboard::write(url)
            }
            Act::CopySelection => {
                room.menu = None;
                Task::done(room::Message::Select(selection::Message::Copy).to(&id))
            }
            Act::CopyImage(document) => {
                room.menu = None;
                Task::perform(self.math.copy_image(&document), move |result| {
                    actions::act(&id, Act::Done(result))
                })
            }
            Act::SaveImage(document) => {
                room.menu = None;
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
                    move |result| actions::act(&id, Act::Done(result)),
                )
            }
            Act::TogglePicker(id, at) => {
                let picker = (id, at);
                room.picker = (room.picker.as_ref() != Some(&picker)).then_some(picker);
                room.emoji = None;
                if room.picker.is_none() {
                    return Task::none();
                }
                load_frequent_emoji(&main.session)
            }
            Act::ClosePicker => {
                room.picker = None;
                room.emoji = None;
                Task::none()
            }
            Act::Emoji(emoji::Message::Open) => {
                room.emoji = Some(emoji::Panel::default());
                iced::widget::operation::focus(emoji::SEARCH)
            }
            Act::Emoji(emoji::Message::Close) => {
                room.emoji = None;
                Task::none()
            }
            Act::Emoji(message) => {
                if let Some(panel) = &mut room.emoji {
                    panel.update(message);
                }
                Task::none()
            }
            Act::Reply(target) => {
                room.compose = Compose::Reply(target);
                room.later.picker = None;
                Task::none()
            }
            Act::Edit(key, text) => {
                room.editing = Some(InlineEdit::new(Editing::Sent(key), &text));
                InlineEdit::focus()
            }
            Act::EditLast => {
                let Some((row, key, text)) = room.newest_editable() else {
                    return Task::none();
                };
                room.editing = Some(InlineEdit::new(Editing::Sent(key), &text));
                Task::batch([InlineEdit::focus(), room::scroll_to_row(&row)])
            }
            Act::EditAction(action) => {
                if let Some(edit) = &mut room.editing {
                    edit.perform(action);
                }
                Task::none()
            }
            Act::SaveEdit => {
                let Some(edit) = &room.editing else {
                    return Task::none();
                };
                let text = edit.text();
                if text.trim().is_empty() {
                    return Task::none();
                }
                let session = main.session.clone();
                let target = edit.target.clone();
                let added = edit.added_emoji();
                let save = Task::perform(
                    async move {
                        match target {
                            Editing::Sent(key) => timeline.edit_markdown(&key, &text).await,
                            Editing::Scheduled(id) => session.edit_scheduled(&id, &text).await,
                        }
                    },
                    move |result| {
                        actions::act(&id, Act::EditSaved(result.map_err(|e| e.to_string())))
                    },
                );
                Task::batch([save, used_emoji(main, added)])
            }
            Act::EditSaved(Ok(())) | Act::CancelEdit => {
                room.editing = None;
                Task::none()
            }
            Act::EditSaved(Err(e)) => {
                room.error = Some(e);
                Task::none()
            }
            Act::AskDelete(id) => {
                room.confirm_delete = Some(id);
                Task::none()
            }
            Act::CancelDelete => {
                room.confirm_delete = None;
                Task::none()
            }
            Act::Delete(key) => {
                room.confirm_delete = None;
                Task::perform(async move { timeline.redact(&key).await }, done)
            }
            Act::CancelCompose => {
                // The reply's text stays as a draft.
                room.compose = Compose::New;
                room.error = None;
                Task::none()
            }
            Act::Done(result) => {
                if let Err(e) = result {
                    room.error = Some(e);
                }
                Task::none()
            }
        }
    }

    /// The find bar of room `id`.
    fn find(&mut self, id: OwnedRoomId, message: find::Message) -> Task<Message> {
        let Screen::Main(main) = &mut self.screen else {
            return Task::none();
        };
        let Some(room) = main.pane.open_mut(&id) else {
            return Task::none();
        };
        let mut focus = Task::none();
        match message {
            // With a selection, what reply previews show first, as it may
            // be in one.
            find::Message::Open if room.selection.is_some() => {
                return room
                    .read_shown()
                    .map(move |shown| room::Message::Find(find::Message::Opened(shown)).to(&id));
            }
            find::Message::Open | find::Message::Opened(_) => {
                let shown = match message {
                    find::Message::Opened(shown) => shown,
                    _ => selection::Shown::new(),
                };
                // Text selected on one line is looked for, as in Safari;
                // else what was last.
                let selected = room
                    .selected(&shown)
                    .map(|copied| copied.text)
                    .filter(|text| !text.contains('\n'));
                match (&mut room.find, selected) {
                    (Some(find), Some(text)) => {
                        find.query = text;
                        find.search(&room.entries);
                    }
                    (Some(_), None) => {}
                    (None, selected) => {
                        room.find = Some(Find::new(selected.unwrap_or_default(), &room.entries));
                    }
                }
                focus = Task::batch([
                    iced::widget::operation::focus(find::FIELD),
                    iced::widget::operation::select_all(find::FIELD),
                ]);
            }
            find::Message::Query(query) => {
                if let Some(find) = &mut room.find {
                    find.query = query;
                    find.search(&room.entries);
                }
            }
            find::Message::Next | find::Message::Previous => {
                let next = matches!(message, find::Message::Next);
                if let Some(find) = &mut room.find {
                    find.step(next);
                }
            }
            find::Message::Close => {
                room.find = None;
                return Task::none();
            }
        }
        let scroll = room.show_found();
        // The match may be in a message not rendered before.
        Task::batch([
            focus,
            scroll,
            self.render_math(),
            self.load_avatars(),
            self.load_previews(),
        ])
    }

    fn later(&mut self, id: OwnedRoomId, message: send_later::Message) -> Task<Message> {
        use send_later::Message as Later;
        let Screen::Main(main) = &mut self.screen else {
            return Task::none();
        };
        let Some(room) = main.pane.open_mut(&id) else {
            return Task::none();
        };
        let session = main.session.clone();
        // How a request went, for this room.
        let reply = |message: fn(Result<(), String>) -> Later| {
            let id = id.clone();
            move |result: mtrx_core::Result<()>| {
                room::Message::Later(message(result.map_err(|e| e.to_string()))).to(&id)
            }
        };
        match message {
            Later::Close => {
                room.later = Default::default();
                Task::none()
            }
            Later::Open => {
                room.composer_emoji = None;
                let picker = send_later::Picker::new(SystemTime::now(), None);
                let scroll = picker.scroll();
                room.later.picker = Some(picker);
                scroll
            }
            Later::ChangeTime(message) => {
                // From its time, unless that has passed.
                let picker = send_later::Picker::new(SystemTime::now(), Some(message.send_at));
                let scroll = picker.scroll();
                room.later.picker = Some(picker);
                room.later.rescheduling = Some(message);
                scroll
            }
            Later::Month(_) | Later::Day(_) | Later::Hour(_) | Later::Minute(_) => {
                if let Some(picker) = &mut room.later.picker {
                    picker.update(&message, SystemTime::now());
                }
                Task::none()
            }
            Later::Schedule => match room
                .later
                .picker
                .as_ref()
                .map(|p| p.time(SystemTime::now()))
            {
                Some(Ok(at)) => self.later(id.clone(), Later::At(at)),
                _ => Task::none(),
            },
            Later::At(send_at) => {
                room.error = None;
                if let Some(message) = &room.later.rescheduling {
                    let id = message.id.clone();
                    return Task::perform(
                        async move { session.reschedule(&id, send_at).await },
                        reply(Later::Saved),
                    );
                }
                let text = room.composer.text().trim_end().to_owned();
                if text.trim().is_empty() && room.attachments.is_empty() {
                    return Task::none();
                }
                let room_id = room.id.clone();
                let reply_to = room.compose.reply_to().cloned();
                let files = room.attachments.iter().map(|a| a.file.clone()).collect();
                // The composer is cleared once saved, so nothing is lost on errors.
                Task::perform(
                    async move {
                        session
                            .schedule(&room_id, &text, files, send_at, reply_to.as_deref())
                            .await
                            .map(|_| ())
                    },
                    reply(Later::Scheduled),
                )
            }
            Later::Edit(message) => {
                room.editing = Some(InlineEdit::new(
                    Editing::Scheduled(message.id.clone()),
                    &message.text,
                ));
                InlineEdit::focus()
            }
            Later::SendNow(id) => Task::perform(
                async move { session.send_scheduled_now(&id).await.map(|_| ()) },
                reply(Later::Done),
            ),
            Later::Cancel(id) => Task::perform(
                async move { session.unschedule(&id).await.map(|_| ()) },
                reply(Later::Done),
            ),
            // Scheduling took the composer's text; a new time leaves it alone.
            Later::Scheduled(Ok(())) => {
                room.compose = Compose::New;
                room.set_composer(text_editor::Content::new());
                room.attachments.clear();
                room.later = Default::default();
                Task::none()
            }
            Later::Saved(Ok(())) => {
                room.later = Default::default();
                Task::none()
            }
            Later::Scheduled(Err(e)) | Later::Saved(Err(e)) | Later::Done(Err(e)) => {
                room.error = Some(e);
                Task::none()
            }
            Later::Done(Ok(())) => Task::none(),
        }
    }

    /// Math and the PDF viewer follow the window's size, the display's
    /// scale and the text size.
    fn rescaled(&mut self) -> Task<Message> {
        Task::batch([self.render_math(), self.pdf_resized()])
    }

    fn pdf_resized(&mut self) -> Task<Message> {
        let (viewport, scale) = (self.viewport(), self.pixel_scale());
        match &mut self.screen {
            Screen::Main(main) => main
                .pdf
                .as_mut()
                .map_or_else(Task::none, |pdf| pdf.resized(viewport, scale)),
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
        match view.found(&main.rooms.rooms) {
            Some((space, room)) => {
                main.restore = None;
                main.space = space;
                self.update(Message::OpenRoom(room))
            }
            None => {
                if main.rooms.loaded {
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

    /// The width of the open room, right of the room list.
    fn room_header_width(&self) -> f32 {
        self.viewport().width - SPACE_BAR_WIDTH - self.settings.room_list_width - RESIZE_HANDLE
    }

    fn math_params(&self) -> math::Params {
        let theme = self.theme();
        let palette = theme.extended_palette();
        let max_width = self.room_header_width() - MESSAGE_INSET;
        math::Params {
            max_width: max_width.max(200.0),
            scale: self.pixel_scale(),
            text: palette.background.base.text,
            link: palette.primary.base.color,
            muted: style::muted(&theme),
        }
    }

    /// Request images for all math messages of the open room.
    fn render_math(&mut self) -> Task<Message> {
        let params = self.math_params();
        let Screen::Main(main) = &self.screen else {
            return Task::none();
        };
        let Some(room) = main.pane.room() else {
            return Task::none();
        };
        let tasks: Vec<_> = room
            .math_documents()
            .map(|doc| self.math.request(doc, &params, Message::MathRendered))
            .collect();
        Task::batch(tasks)
    }

    /// Request the pictures on screen.
    /// Show a room's members anew where they are listed: beside the open
    /// room, or in its space's overview.
    fn refresh_members(&mut self, room: &OwnedRoomId) -> Task<Message> {
        let Screen::Main(main) = &mut self.screen else {
            return Task::none();
        };
        let beside = match main.pane.open_mut(room) {
            Some(open) if main.show_members => load_members(&main.session, open),
            _ => Task::none(),
        };
        let overview = match main.info.as_ref().and_then(|c| c.overview.as_ref()) {
            Some(overview) if overview.space == *room => overview_task(
                &main.session,
                &mut main.space_rooms,
                room,
                overview::Action::LoadMembers,
            ),
            _ => Task::none(),
        };
        Task::batch([beside, overview, self.reload_card(room)])
    }

    /// Once synced, an account without "Notes at the top" gets this
    /// desktop's earlier choice; asked once a run.
    fn move_notes_at_top(&mut self) -> Task<Message> {
        let Screen::Main(main) = &mut self.screen else {
            return Task::none();
        };
        if main.notes_moved || !main.rooms.loaded || main.arrange.categories.notes_at_top.is_some()
        {
            return Task::none();
        }
        main.notes_moved = true;
        Task::done(Message::Arrange(arrange::Message::NotesAtTop(
            self.settings.pinned_notes,
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
        let card = main.info.as_ref()?;
        Some((card.room().cloned(), card.overview.as_ref().map(|o| o.tab)))
    }

    /// The settings again on `room`'s card, if it is open.
    fn reload_card(&self, room: &OwnedRoomId) -> Task<Message> {
        match &self.screen {
            Screen::Main(main) if main.info.as_ref().and_then(|c| c.room()) == Some(room) => {
                load_settings(&main.session, room)
            }
            _ => Task::none(),
        }
    }

    /// Run what the join dialog asks for.
    fn join_action(&mut self, action: join::Action) -> Task<Message> {
        let Screen::Main(main) = &mut self.screen else {
            return Task::none();
        };
        let session = main.session.clone();
        let done = |room: OwnedRoomOrAliasId| {
            move |result: mtrx_core::Result<join::Joined>| {
                Message::Join(join::Message::Done(
                    room.clone(),
                    result.map_err(|e| e.to_string()),
                ))
            }
        };
        match action {
            join::Action::None => Task::none(),
            join::Action::Close => {
                main.join = None;
                Task::none()
            }
            join::Action::Search(search) => {
                let (task, handle) = Task::perform(
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
                        Message::Join(join::Message::Found(
                            search.clone(),
                            result.map_err(|e| e.to_string()),
                        ))
                    },
                )
                .abortable();
                if let Some(join) = &mut main.join
                    && let Some(old) = join.request.replace(handle)
                {
                    old.abort();
                }
                task
            }
            join::Action::Preview(address, via) => {
                let (task, handle) = Task::perform(
                    {
                        let address = address.clone();
                        async move { session.preview(&address, via).await }
                    },
                    move |result| {
                        Message::Join(join::Message::Previewed(
                            address.to_string(),
                            result.map_err(|e| e.to_string()),
                        ))
                    },
                )
                .abortable();
                if let Some(join) = &mut main.join
                    && let Some(old) = join.request.replace(handle)
                {
                    old.abort();
                }
                task
            }
            join::Action::Join(room, via, space) => Task::perform(
                {
                    let room = room.clone();
                    async move {
                        let id = session.join(&room, &via).await?;
                        Ok(if space {
                            join::Joined::Space(id)
                        } else {
                            join::Joined::Room
                        })
                    }
                },
                done(room),
            ),
            join::Action::Knock(room, via) => Task::perform(
                {
                    let room = room.clone();
                    async move { Ok(join::Joined::Asked(session.knock(&room, via).await?)) }
                },
                done(room),
            ),
            join::Action::ShowSpace(space) => {
                main.join = None;
                main.joined_space = Some(space.clone());
                let shown = main.rooms.rooms.iter().any(|r| r.id == space);
                let select = self.update(Message::SelectSpace(Some(space.clone())));
                if shown {
                    if let Screen::Main(main) = &mut self.screen {
                        main.joined_space = None;
                    }
                    Task::batch([
                        select,
                        Task::done(Message::ShowSpace(space, overview::Tab::Rooms)),
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
        main.add_menu = false;
        // Opened from the list of a message's readers, say.
        if let Some(room) = main.pane.room_mut() {
            room.readers = None;
            room.reactors = None;
        }
        let card = info::Card::new(target, &main.rooms.rooms, tab);
        // Fresher than the watcher's, and for people it does not follow.
        let presence = match card.user() {
            Some(user) if *user != main.own_user && !main.presences.unavailable() => {
                presence::look_up(&main.session, user.clone()).map(Message::Presence)
            }
            _ => Task::none(),
        };
        let load = match card.user() {
            Some(user) => {
                let (session, user) = (main.session.clone(), user.clone());
                Task::perform(
                    {
                        let user = user.clone();
                        async move { session.user_info(&user).await.map_err(|e| e.to_string()) }
                    },
                    move |result| Message::Info(info::Message::UserLoaded(user.clone(), result)),
                )
            }
            None => card
                .room()
                .map_or_else(Task::none, |room| load_settings(&main.session, room)),
        };
        let overview = card.overview.as_ref().map_or(Task::none(), |o| {
            overview_task(&main.session, &mut main.space_rooms, &o.space, o.opened())
        });
        main.info = Some(card);
        Task::batch([load, presence, overview, self.load_avatars()])
    }

    fn load_avatars(&mut self) -> Task<Message> {
        let Screen::Main(main) = &mut self.screen else {
            return Task::none();
        };
        let tasks: Vec<_> = main
            .avatars_on_screen(&self.settings)
            .into_iter()
            .map(|(mxc, size)| {
                main.avatars
                    .request(&main.session, &mxc, size, Message::AvatarLoaded)
            })
            .collect();
        Task::batch(tasks)
    }

    /// Request the previews of the open room's rendered images.
    fn load_previews(&mut self) -> Task<Message> {
        let Screen::Main(main) = &mut self.screen else {
            return Task::none();
        };
        let Some(room) = main.pane.room() else {
            return Task::none();
        };
        let tasks: Vec<_> = room
            .previewed()
            .map(|media| {
                main.previews
                    .request(&main.session, media, Message::PreviewLoaded)
            })
            .collect();
        Task::batch(tasks)
    }

    pub fn subscription(&self) -> Subscription<Message> {
        let mut subscriptions = vec![
            window::resize_events().map(|(_, size)| Message::WindowResized(size)),
            window::events().filter_map(|(id, event)| match event {
                window::Event::Moved(_) | window::Event::Resized(_) => {
                    Some(Message::WindowMoved(id))
                }
                _ => None,
            }),
            window::close_requests().map(Message::CloseWindow),
            iced::system::theme_changes().map(Message::ThemeMode),
            // Also on the sign-in screen.
            iced::event::listen_with(window_keys),
            iced::time::every(update::TICK).map(|_| Message::Update(update::Message::Tick)),
        ];
        // Settings… and Check for Updates… in the app menu.
        #[cfg(target_os = "macos")]
        subscriptions.push(crate::menu_bar::chosen().map(|chosen| match chosen {
            crate::menu_bar::Chosen::Settings => Message::ShowSettings(true),
            crate::menu_bar::Chosen::CheckForUpdates => Message::CheckForUpdates,
        }));
        if let Screen::Main(main) = &self.screen {
            subscriptions.push(
                Subscription::run_with(Shared(main.session.clone()), |s| s.0.scheduled_updates())
                    .map(Message::Scheduled),
            );
            subscriptions.push(
                Subscription::run_with(
                    (Shared(main.session.clone()), "notifications"),
                    |(s, _)| s.0.notifications(),
                )
                .map(Message::Notified),
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
            if self.resizing {
                subscriptions.push(iced::event::listen_with(|event, _, _| match event {
                    iced::Event::Mouse(mouse::Event::CursorMoved { position }) => {
                        Some(Message::Resize(Resize::Move(position.x)))
                    }
                    iced::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
                        Some(Message::Resize(Resize::End))
                    }
                    _ => None,
                }));
            }
            subscriptions.push(
                Subscription::run_with(Shared(main.session.clone()), |s| s.0.connection_updates())
                    .map(Message::Connection),
            );
            subscriptions.push(
                Subscription::run_with(Shared(main.session.clone()), |s| {
                    s.0.room_categories_updates()
                })
                .map(|c| Message::Arrange(arrange::Message::Categories(c))),
            );
            subscriptions.push(
                Subscription::run_with(Shared(main.session.clone()), |s| {
                    s.0.rooms_with_drafts_updates()
                })
                .map(Message::Drafts),
            );
            subscriptions.push(main.arrange.subscription());
            subscriptions.push(
                Subscription::run_with(Shared(main.session.clone()), |s| {
                    s.0.room_list_colours_updates()
                })
                .map(Message::Colours),
            );
            // Times shown relative to now: scheduled messages, last activity.
            if !main.scheduled.is_empty() || main.presences.following() {
                subscriptions.push(iced::time::every(TICK).map(|_| Message::Tick));
            }
            if let Some(watcher) = main.presences.subscription(&main.session) {
                subscriptions.push(watcher.map(Message::Presence));
            }
            if self.settings.share_presence && !main.presences.unavailable() {
                subscriptions.push(iced::event::listen_with(input));
                if main.activity.may_idle() {
                    subscriptions.push(
                        iced::time::every(presence::IDLE_CHECK)
                            .map(|_| Message::Presence(presence::Message::CheckIdle)),
                    );
                }
            }
            if main.dialog.as_ref().is_some_and(Dialog::busy) {
                subscriptions.push(iced::time::every(PROGRESS_TICK).map(|_| Message::Progress));
            }
            if let Some(list) = &main.room_list {
                subscriptions.push(
                    Subscription::run_with(Shared(list.clone()), |list| list.0.updates())
                        .map(Message::Rooms),
                );
            }
            if let Some(room) = main.pane.room() {
                subscriptions.push(Subscription::run_with(
                    (Shared(room.timeline.clone()), "typing"),
                    |(t, _)| {
                        let id = t.0.room_id().clone();
                        t.0.typing_updates()
                            .map(move |names| room::Message::Typing(names).to(&id))
                    },
                ));
                subscriptions.push(Subscription::run_with(Shared(room.timeline.clone()), |t| {
                    let id = t.0.room_id().clone();
                    t.0.updates()
                        .map(move |entries| room::Message::Timeline(entries).to(&id))
                }));
                subscriptions.push(Subscription::run_with(
                    (Shared(room.timeline.clone()), "uploads"),
                    |(t, _)| {
                        let (timeline, id) = (t.0.clone(), t.0.room_id().clone());
                        iced::futures::stream::once(async move { timeline.uploads().await })
                            .filter_map(|uploads| async move { uploads.ok() })
                            .flatten()
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
        }
        Subscription::batch(subscriptions)
    }

    pub fn view(&self) -> Element<'_, Message> {
        match &self.screen {
            Screen::Starting => container(text("Starting…")).center(Length::Fill).into(),
            Screen::Closing => container("").into(),
            Screen::Login(login) => login.view().map(Message::Login),
            // The main view is always the stack's first layer, so opening
            // and closing what floats over it keeps its widgets' state.
            Screen::Main(main) => {
                let mut layers = stack![self.main_view(main)];
                // Panels whose content loads and changes keep their top where
                // it is; dialogs are centred.
                let (panel, close, anchored) = if let Some(panel) = &self.settings_panel {
                    let spaces = mtrx_core::top_level_spaces(&main.rooms.rooms)
                        .into_iter()
                        .map(|s| (s.id.clone(), s.name.clone()))
                        .collect();
                    (
                        panel.view(
                            &self.settings,
                            spaces,
                            &main.colours,
                            &main.arrange.categories,
                            !main.presences.unavailable(),
                            &self.updater,
                        ),
                        Message::ShowSettings(false),
                        true,
                    )
                } else if let Some(dialog) = &main.dialog {
                    (
                        dialog.view(&main.rooms.rooms, &main.arrange, &self.settings.room_list),
                        Message::Dialog(dialog::Message::Cancel),
                        false,
                    )
                } else if let Some(join) = &main.join {
                    (
                        join.view(
                            &main.rooms.rooms,
                            &main.avatars,
                            self.settings.room_pictures,
                        ),
                        Message::Join(join::Message::Close),
                        true,
                    )
                } else if let Some(card) = &main.info {
                    (
                        card.view(info::Shown {
                            rooms: &main.rooms.rooms,
                            avatars: &main.avatars,
                            own_user: &main.own_user,
                            presences: &main.presences,
                            now: main.now,
                            pinned_notes: main.arrange.categories.notes_at_top(),
                            call_services: &self.settings.call_services,
                            room_pictures: self.settings.room_pictures,
                            member_menu: main.member_menu.as_ref(),
                            space_rooms: &main.space_rooms,
                        }),
                        Message::Info(info::Message::Close),
                        true,
                    )
                } else if let Some(viewer) = &main.viewer {
                    // Fills the window, with its own backdrop.
                    return layers.push(opaque(viewer.view(&main.previews))).into();
                } else if let Some(pdf) = &main.pdf {
                    return layers.push(opaque(pdf.view())).into();
                } else {
                    return layers.into();
                };
                let placed = if anchored {
                    container(opaque(panel))
                        .center_x(Length::Fill)
                        .height(Length::Fill)
                        .align_top(Length::Fill)
                        .padding(iced::Padding::new(24.0).top(PANEL_TOP))
                } else {
                    container(opaque(panel)).center(Length::Fill)
                };
                let backdrop = mouse_area(placed.style(style::backdrop)).on_press(close);
                layers = layers.push(opaque(backdrop));
                layers.into()
            }
        }
    }

    fn main_view<'a>(&'a self, main: &'a Main) -> Element<'a, Message> {
        let theme = self.theme();
        let rooms = &main.rooms.rooms;
        let space_bar = sidebar::space_bar(
            rooms,
            main.space.as_ref(),
            &main.arrange,
            &self.settings.room_list,
            &main.avatars,
            main.space_menu.as_ref(),
            self.updater.unseen(&self.settings.updates),
        );
        let room_list = sidebar::room_list(sidebar::RoomListView {
            // As wide as a room's below its name.
            topic_width: (self.room_header_width() * room::TOPIC_SHARE)
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
        if let Some(error) = &main.error {
            banners = banners.push(container(text(error).style(text::danger)).padding(12));
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
                    let name = rooms
                        .iter()
                        .find(|r| &r.id == id)
                        .map_or("", |r| r.name.as_str());
                    row![
                        opening,
                        members::view(
                            None,
                            name,
                            members::Shown {
                                room: id,
                                own: &main.own_user,
                                avatars: &main.avatars,
                                presences: &main.presences,
                                menu: main.member_menu.as_ref(),
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
                    params: self.math_params(),
                    avatars: &main.avatars,
                    colored_names: self.settings.colored_names,
                    call_services: &self.settings.call_services,
                    previews: &main.previews,
                    frequent_emoji: &main.frequent_emoji,
                    show_members: main.show_members,
                    member_menu: main.member_menu.as_ref(),
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
                .width(RESIZE_HANDLE)
                .height(Length::Fill),
        )
        .interaction(mouse::Interaction::ResizingHorizontally)
        .on_press(Message::Resize(Resize::Start));
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
        if let Some(preview) =
            sidebar::drag_preview(rooms, main.space.as_ref(), &main.arrange, &main.avatars)
        {
            layers = layers.push(preview);
        }
        layers.into()
    }
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
            .on_input(Message::RecoveryKey)
            .on_submit(Message::Recover);
    }
    let unlock = button(text(if busy { "Unlocking…" } else { "Unlock" }))
        .style(style::primary_button)
        .on_press_maybe((!busy).then_some(Message::Recover));
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
        let name = main
            .rooms
            .rooms
            .iter()
            .find(|r| &r.id == id)
            .map_or(id.as_str(), |r| r.name.as_str());
        let label = if count > 1 {
            format!("{name} ({count})")
        } else {
            name.to_owned()
        };
        banner = banner.push(
            button(text(label).size(13))
                .padding([2, 8])
                .style(style::secondary_button)
                .on_press(Message::OpenRoom(id.clone())),
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
                .on_press(Message::Reconnect),
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
                    .on_press(Message::SignInAgain),
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
                .on_press(Message::OpenRoom(room.clone())),
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
                .on_press(Message::AnswerInvite(id.clone(), false)),
            button(text("Accept"))
                .style(style::primary_button)
                .on_press(Message::AnswerInvite(id.clone(), true)),
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
                for person in &people {
                    session.invite(&room, person).await?;
                }
                Opened::Nothing
            }
            Request::AddToSpace(space, rooms) => {
                for room in &rooms {
                    session.add_to_space(&space, room).await?;
                }
                Opened::Nothing
            }
            Request::Leave(room) => {
                session.leave(&room).await?;
                Opened::Left(room)
            }
            Request::RemoveMember(room, user, reason, ban) => {
                session
                    .remove_member(&room, &user, reason.as_deref(), ban)
                    .await?;
                Opened::Members(room)
            }
            Request::SetLevel(room, user, level) => {
                session.set_level(&room, &user, level).await?;
                Opened::Members(room)
            }
            Request::Encrypt(room) => {
                session.encrypt_room(&room).await?;
                Opened::Settings(room)
            }
            Request::SignOut => {
                // Others see us offline now, not once the server forgets us.
                presence::go_offline(&session).await;
                session.logout().await?;
                Opened::SignedOut
            }
        })
    }
    .await;
    result.map_err(|e: mtrx_core::Error| e.to_string())
}

/// Keys on every screen, whatever is focused: ⌘+ ⌘− ⌘0 zoom (Ctrl on Windows
/// and Linux), and Ctrl+W closes the window as its close button does. On
/// macOS ⌘W is the Window menu's Close, which hides it (see [`closing`]).
#[cfg_attr(target_os = "macos", allow(unused_variables))]
fn window_keys(
    event: iced::Event,
    _status: iced::event::Status,
    window: window::Id,
) -> Option<Message> {
    use iced::keyboard;
    match as_key(event) {
        iced::Event::Keyboard(keyboard::Event::KeyPressed { key, modifiers, .. }) => {
            #[cfg(not(target_os = "macos"))]
            if key.as_ref() == keyboard::Key::Character("w") && modifiers.command() {
                return Some(Message::CloseWindow(window));
            }
            Zoom::from_key(&key, modifiers).map(Message::Zoom)
        }
        _ => None,
    }
}

/// ⌘K / Ctrl+K focuses the room search, Esc closes what floats on top or
/// clears the search, ⌘↩ submits a dialog, ⌘C, ⌘A and Shift with an arrow act on the timeline's selection;
/// window focus.
fn shortcuts(
    event: iced::Event,
    status: iced::event::Status,
    _window: window::Id,
) -> Option<Message> {
    use iced::keyboard::{self, Key, key::Named};
    match as_key(event) {
        iced::Event::Keyboard(keyboard::Event::KeyPressed {
            key,
            physical_key,
            modifiers,
            ..
        }) if let Some(go) = sidebar::Go::from_key(&key, physical_key, modifiers) => {
            Some(Message::GoToRoom(go))
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
            Key::Character(c) if c == "k" && modifiers.command() => Some(Message::FocusSearch),
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
            Key::Character(c) if c == "r" && modifiers.command() => Some(Message::Reload),
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
        iced::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
            Some(Message::Arrange(arrange::Message::Pressed))
        }
        iced::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
            Some(Message::Arrange(arrange::Message::Released))
        }
        iced::Event::Window(window::Event::Focused) => Some(Message::WindowFocused(true)),
        iced::Event::Window(window::Event::Unfocused) => Some(Message::WindowFocused(false)),
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

/// Keys, clicks and scrolling: we are here (see [`presence::Activity`]).
fn input(event: iced::Event, _status: iced::event::Status, _window: window::Id) -> Option<Message> {
    use iced::keyboard;
    match event {
        iced::Event::Keyboard(keyboard::Event::KeyPressed { .. })
        | iced::Event::StandardAction(_)
        | iced::Event::Mouse(mouse::Event::ButtonPressed(_) | mouse::Event::WheelScrolled { .. }) => {
            Some(Message::Presence(presence::Message::Input))
        }
        _ => None,
    }
}

/// `action` performed in `field` of `room`.
fn in_field(room: &OwnedRoomId, field: Field, action: text_editor::Action) -> Message {
    match field {
        Field::Composer => room::Message::Composer(action).to(room),
        Field::Edit => actions::act(room, actions::Message::EditAction(action)),
    }
}

/// Put `copied` on the clipboard; a failure shows in the room.
/// Keep `draft` for room `id` on this device; an empty one clears it.
fn save_draft(session: &Arc<Session>, id: OwnedRoomId, draft: Draft) -> Task<Message> {
    let session = session.clone();
    Task::future(async move {
        if let Err(e) = session.save_draft(&id, &draft).await {
            tracing::warn!(room = %id, "could not keep the draft: {e}");
        }
    })
    .discard()
}

fn copy(room: &OwnedRoomId, copied: Copied) -> Task<Message> {
    let room = room.clone();
    Task::perform(clipboard::write(copied), move |result| {
        actions::act(&room, actions::Message::Done(result))
    })
}

/// A menu's choice closes it: the menu of a message in the room, or the
/// viewer's.
fn close_menu(main: &mut Main, from: &media::Origin) {
    match from {
        media::Origin::Viewer => {
            if let Some(viewer) = &mut main.viewer {
                viewer.menu = false;
            }
        }
        media::Origin::Room(id) => {
            if let Some(room) = main.pane.open_mut(id) {
                room.menu = None;
            }
        }
    }
}

/// Read files to attach to the message being written in `room`.
fn attach(room: OwnedRoomId, paths: Vec<PathBuf>) -> Task<Message> {
    if paths.is_empty() {
        return Task::none();
    }
    Task::perform(media::read_attached(paths), move |attached| {
        room::Message::Attached(attached).to(&room)
    })
}

/// Load the room's members, for its member list and mentions, and whether
/// we may mention everyone.
/// A message to `panel`, the topic panel of room or space `room` with
/// topic `synced` in the room list, answered with `reply`.
fn topic_update(
    session: &Arc<Session>,
    panel: &mut topic::Panel,
    room: OwnedRoomId,
    synced: Option<&str>,
    message: topic::Message,
    reply: impl Fn(topic::Message) -> Message + Clone + Send + 'static,
) -> Task<Message> {
    if let topic::Message::Save = message {
        let Some(editor) = &panel.editor else {
            return Task::none();
        };
        let topic = editor.topic();
        return save_topic(session, panel, room, topic, reply);
    }
    let shown = panel.shown(synced).to_owned();
    let opening = matches!(message, topic::Message::Toggle) && !panel.open;
    // Not known when it was asked (the room's permissions not synced yet):
    // asked again as the panel opens.
    let ask = opening && panel.can_edit.is_none();
    let task = panel.update(message, &shown).map(reply.clone());
    if ask {
        Task::batch([task, load_can_edit_topic(session, room, reply)])
    } else {
        task
    }
}

/// Save `topic` as `room`'s, shown in its topic `panel` until the room list
/// has it; `reply` says how it went.
fn save_topic(
    session: &Arc<Session>,
    panel: &mut topic::Panel,
    room: OwnedRoomId,
    topic: String,
    reply: impl FnOnce(topic::Message) -> Message + Send + 'static,
) -> Task<Message> {
    panel.save(topic.clone());
    let session = session.clone();
    Task::perform(
        async move {
            session
                .set_room_topic(&room, &topic)
                .await
                .map_err(|e| e.to_string())
        },
        move |result| reply(topic::Message::Saved(result)),
    )
}

/// Whether we may change `room`'s topic, for its topic panel, answered
/// with `reply`.
fn load_can_edit_topic(
    session: &Arc<Session>,
    room: OwnedRoomId,
    reply: impl FnOnce(topic::Message) -> Message + Send + 'static,
) -> Task<Message> {
    let session = session.clone();
    Task::perform(
        async move { session.can_change_topic(&room).await.ok() },
        move |can| reply(topic::Message::CanEdit(can)),
    )
}

/// List the rooms of all our spaces, for the room search.
fn load_space_rooms(main: &mut Main) -> Task<Message> {
    let spaces = directory::searched_spaces(&main.rooms.rooms, None);
    Task::batch(
        spaces
            .iter()
            .map(|space| list_space_rooms(&main.session, &mut main.space_rooms, space, false)),
    )
}

/// Ask `space`'s server for its rooms, unless listed lately (any time if
/// `again`) or being listed.
fn list_space_rooms(
    session: &Arc<Session>,
    space_rooms: &mut directory::SpaceRooms,
    space: &OwnedRoomId,
    again: bool,
) -> Task<Message> {
    if !space_rooms.start(space, again) {
        return Task::none();
    }
    let (session, space) = (session.clone(), space.clone());
    Task::perform(
        {
            let space = space.clone();
            async move { session.space_children(&space).await }
        },
        move |result| Message::SpaceRoomsLoaded(space.clone(), result.map_err(|e| e.to_string())),
    )
}

/// Load or join what a space's overview needs.
fn overview_task(
    session: &Arc<Session>,
    space_rooms: &mut directory::SpaceRooms,
    space: &OwnedRoomId,
    action: overview::Action,
) -> Task<Message> {
    use overview::{Action, Message as Overview};
    let session = session.clone();
    let done = |message: Overview| Message::Info(info::Message::Overview(message));
    match action {
        Action::None => Task::none(),
        Action::Load(space) => list_space_rooms(&session, space_rooms, &space, false),
        Action::Reload(spaces) => Task::batch(
            spaces
                .iter()
                .map(|space| list_space_rooms(&session, space_rooms, space, true)),
        ),
        Action::LoadMembers => {
            let space = space.clone();
            Task::perform(
                async move { session.members(&space).await },
                move |result| done(Overview::MembersLoaded(result.map_err(|e| e.to_string()))),
            )
        }
        Action::Join(room, via) => Task::perform(
            {
                let room = room.clone();
                async move { session.join(<&RoomOrAliasId>::from(&*room), &via).await }
            },
            move |result| {
                done(Overview::Done(
                    room.clone(),
                    result.map(|_| ()).map_err(|e| e.to_string()),
                ))
            },
        ),
        Action::Knock(room, via) => Task::perform(
            {
                let room = room.clone();
                async move { session.knock(&room.into(), via).await.map(|_| ()) }
            },
            move |result| {
                done(Overview::Done(
                    room.clone(),
                    result.map_err(|e| e.to_string()),
                ))
            },
        ),
    }
}

fn load_members(session: &Arc<Session>, room: &mut Room) -> Task<Message> {
    room.loading_members = true;
    let (session, id) = (session.clone(), room.id.clone());
    Task::perform(
        {
            let id = id.clone();
            async move {
                let (members, everyone) =
                    futures_util::join!(session.members(&id), session.can_mention_room(&id));
                Ok((
                    members.map_err(|e| e.to_string())?,
                    everyone.unwrap_or(false),
                ))
            }
        },
        move |result| room::Message::MembersLoaded(result).to(&id),
    )
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
    let title = if notification.direct {
        notification.sender_name.clone()
    } else {
        format!("{} in {}", notification.sender_name, notification.room_name)
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

/// Who may edit `room`, and whether we may change that, for the settings panel.
fn load_access(session: Arc<Session>, room: OwnedRoomId) -> Task<Message> {
    Task::perform(
        async move {
            let permissions = session.permissions(&room).await.ok();
            let can_change = session.can_change_permissions(&room).await.ok();
            settings::Message::AccessLoaded(room, permissions, can_change)
        },
        Message::Settings,
    )
}

/// A link for a new call with `service`.
async fn new_call_link(service: &CallService, google: &Google) -> Result<Url, String> {
    match service.kind {
        ServiceKind::GoogleMeet => google.create_meeting().await.map_err(|e| match e {
            mtrx_core::Error::GoogleNotSignedIn => {
                "Connect your Google account in Settings to create Meet links.".to_owned()
            }
            e => e.to_string(),
        }),
        _ => service
            .new_link()
            .ok_or_else(|| format!("{} cannot make call links", service.name)),
    }
}

/// Load older events. Whether the start was reached shows as
/// [`TimelineEntry::Start`] in the entries.
fn paginate(room: &mut Room) -> Task<Message> {
    if room.paginating || room.at_start() {
        return Task::none();
    }
    room.paginating = true;
    let (id, timeline) = (room.id.clone(), room.timeline.clone());
    Task::perform(
        async move { timeline.paginate_back(30).await.map(|_| ()) },
        move |result| room::Message::Paginated(result.map_err(|e| e.to_string())).to(&id),
    )
}

fn load_settings(session: &Arc<Session>, room: &OwnedRoomId) -> Task<Message> {
    let (session, room) = (session.clone(), room.clone());
    Task::perform(
        {
            let room = room.clone();
            async move {
                session
                    .room_settings(&room)
                    .await
                    .map_err(|e| e.to_string())
            }
        },
        move |result| Message::Info(info::Message::SettingsLoaded(room.clone(), result)),
    )
}

/// The user's most used emoji, from account data.
fn load_frequent_emoji(session: &Arc<Session>) -> Task<Message> {
    let session = session.clone();
    Task::perform(
        async move { session.frequent_emoji().await },
        Message::FrequentEmoji,
    )
}

/// Count the uses of `used` (reacted with, sent): first among the most
/// used here at once, and in account data (shared with Element).
fn used_emoji(main: &mut Main, used: Vec<String>) -> Task<Message> {
    if used.is_empty() {
        return Task::none();
    }
    for emoji in &used {
        main.frequent_emoji.retain(|e| e != emoji);
        main.frequent_emoji.insert(0, emoji.clone());
    }
    let session = main.session.clone();
    Task::future(async move {
        if let Err(e) = session.record_emoji(&used).await {
            tracing::warn!("could not record the emoji used: {e}");
        }
    })
    .discard()
}

/// Spaces whose members a dialog suggests first: the space a room is
/// created in, the spaces around a room people are invited to, or the
/// space shown.
fn near_spaces(
    kind: &dialog::Kind,
    rooms: &[mtrx_core::RoomSummary],
    shown: Option<&OwnedRoomId>,
) -> Vec<OwnedRoomId> {
    let around = |room: &OwnedRoomId| {
        let mut spaces = sidebar::ancestors(rooms, room);
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

/// Older events loaded at most to find a message gone to.
const GO_TO_PAGES: usize = 20;

/// Scroll to message `event` and mark it for a moment, first loading older
/// events (`pages` were already) if needed.
fn go_to(room: &mut Room, event: OwnedEventId, pages: usize) -> Task<Message> {
    if let Some(id) = room.reveal(&event) {
        room.marked = Some(id.clone());
        let (unmark, room) = (id.clone(), room.id.clone());
        return Task::batch([
            room::scroll_to_row(&id),
            Task::perform(tokio::time::sleep(Duration::from_secs(2)), move |()| {
                room::Message::Unmark(unmark.clone()).to(&room)
            }),
        ]);
    }
    if room.at_start() || pages >= GO_TO_PAGES {
        room.error = Some("The original message was not found in the room's history".into());
        return Task::none();
    }
    room.seeking = Some((event, pages + 1));
    paginate(room)
}

/// Load older events while too few are loaded to fill the view.
fn fill(room: &mut Room) -> Task<Message> {
    if room.wants_older() {
        paginate(room)
    } else {
        Task::none()
    }
}

fn mark_read(room: &Room) -> Task<Message> {
    let timeline = room.timeline.clone();
    Task::future(async move {
        let _ = timeline.mark_read().await;
    })
    .discard()
}

#[cfg(test)]
mod tests;
