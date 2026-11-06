//! App preferences, shared by all accounts, in `settings.json` next to the
//! profiles, and the settings panel.

use std::{
    collections::BTreeMap,
    fmt, io,
    path::{Path, PathBuf},
};

use iced::{
    Alignment, Element, Length, Theme,
    task::Handle,
    widget::{
        button, column, container, pick_list, progress_bar, row, space, text, text_input, toggler,
    },
};
use mtrx_core::{
    AtHome, CallService, Grouping, OwnedRoomId, Rights, RoomCategories, RoomListColours,
    ServiceKind,
};
use mtrx_google::GoogleClient;
use serde::{Deserialize, Serialize};

use crate::{
    app,
    appearance::Appearance,
    dialog::Kind,
    feedback, fonts, frame,
    hint::{Position, hint, shortcut},
    spelling, style,
    text_size::TextSize,
    tint::Tint,
    update::{self, Updater},
};

const FILE: &str = "settings.json";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Light, dark, or as the system is.
    pub appearance: Appearance,
    /// The scale of text, pictures and spacing.
    pub text_size: TextSize,
    /// Senders' names in their colour (as in Element) instead of the text colour.
    pub colored_names: bool,
    /// Show new messages as system notifications.
    pub desktop_notifications: bool,
    /// The count of unread messages that notify on the app's icon.
    pub icon_badge: bool,
    /// Services for starting calls; the first is the default.
    pub call_services: Vec<CallService>,
    /// The room list's built-in sections.
    pub room_list: Grouping,
    /// The room list's width, as dragged.
    pub room_list_width: f32,
    /// Rooms show their picture (or initial) rather than a symbol saying
    /// whether they are public or private.
    pub room_pictures: bool,
    /// The Notes to self at the top, as this desktop had it before the
    /// choice moved to the account ([`RoomCategories::notes_at_top`]):
    /// saved there once, if the account has none yet.
    pub pinned_notes: bool,
    /// Others see when we are online or idle; otherwise we are offline.
    pub share_presence: bool,
    /// Misspelled words are marked where one writes.
    pub check_spelling: bool,
    /// Looking for newer versions of the app.
    pub updates: update::Preferences,
    /// Room list colours chosen before they were kept in the account
    /// ([`mtrx_core::RoomListColours`]): moved there once, then dropped.
    #[serde(rename = "home_tint", skip_serializing)]
    pub legacy_home_tint: Option<Tint>,
    #[serde(rename = "space_tints", skip_serializing)]
    pub legacy_space_tints: BTreeMap<OwnedRoomId, Tint>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            appearance: Appearance::default(),
            text_size: TextSize::default(),
            colored_names: false,
            desktop_notifications: true,
            icon_badge: true,
            call_services: vec![
                CallService::jitsi("Jitsi Meet", "https://meet.jit.si").expect("valid"),
            ],
            room_list: Grouping::default(),
            room_list_width: style::ROOM_LIST_WIDTH,
            room_pictures: false,
            pinned_notes: true,
            share_presence: true,
            check_spelling: true,
            updates: update::Preferences::default(),
            legacy_home_tint: None,
            legacy_space_tints: BTreeMap::new(),
        }
    }
}

impl Settings {
    /// Colours from before they were kept in the account, to move there.
    pub fn legacy_colours(&self) -> Option<RoomListColours> {
        (self.legacy_home_tint.is_some() || !self.legacy_space_tints.is_empty()).then(|| {
            RoomListColours {
                home: self.legacy_home_tint.map(|t| t.name().to_owned()),
                spaces: self
                    .legacy_space_tints
                    .iter()
                    .map(|(space, tint)| (space.clone(), tint.name().to_owned()))
                    .collect(),
            }
        })
    }

    pub fn path(data_root: &Path) -> PathBuf {
        data_root.join(FILE)
    }

    /// The saved settings; defaults if there are none or they cannot be read.
    pub fn load(path: &Path) -> Self {
        std::fs::read(path)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, path: &Path) -> io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(path, serde_json::to_vec_pretty(self)?)
    }
}

/// A preference changed, which the Settings panel need not be open for
/// (see [`change`]).
#[derive(Debug, Clone)]
pub enum Preference {
    Appearance(Appearance),
    TextSize(TextSize),
    ColoredNames(bool),
    DesktopNotifications(bool),
    IconBadge(bool),
    RoomList(Grouping),
    /// Which rooms of spaces Home lists too.
    AtHome(AtHome),
    RoomPictures(bool),
    PinnedNotes(bool),
    SharePresence(bool),
    CheckSpelling(bool),
    /// Look for updates at start and once a day.
    AutomaticUpdates(bool),
    UpdateChannel(update::Channel),
    /// The room list's colour in a space, or at home.
    Tint(Option<OwnedRoomId>, Tint),
}

#[derive(Debug, Clone)]
pub enum Message {
    Preference(Preference),
    FormKind(FormKind),
    FormName(String),
    FormUrl(String),
    AddService,
    MakeDefault(usize),
    RemoveService(usize),
    /// The selected space's call service.
    SpaceLoaded(OwnedRoomId, Option<CallService>),
    SetSpaceService(SpaceChoice),
    SpaceSaved(Result<(), String>),
    /// A room list colour was saved in the account, or not.
    ColourSaved(Result<(), String>),
    GoogleClientId(String),
    GoogleClientSecret(String),
    GoogleSignIn,
    GoogleCancel,
    /// The signed-in account's email.
    GoogleSignedIn(Result<String, String>),
    GoogleSignOut,
    GoogleSignedOut(Result<(), String>),
}

/// What the app should do after a [`Message`].
pub enum Action {
    None,
    /// The settings changed; save them.
    Save,
    /// The appearance changed; save the settings and draw in it.
    Appearance,
    /// The text size changed; save the settings and draw at it.
    TextSize,
    /// Sharing our presence was turned on or off; save the settings and
    /// set it.
    SharePresence,
    /// Spell checking was turned on or off; save the settings and mark
    /// words or not.
    Spelling,
    /// The update channel changed; save the settings and look for updates
    /// on it.
    CheckUpdates,
    SetSpaceService(OwnedRoomId, Option<CallService>),
    GoogleSignIn(GoogleClient),
    GoogleSignOut,
    /// Set the room list's colour in a space, or at home, in the account.
    Colour(Option<OwnedRoomId>, Tint),
    /// Set which rooms of spaces Home lists too, in the account.
    AtHome(AtHome),
    /// Keep the Notes to self at the top, in the account.
    NotesAtTop(bool),
}

/// State of the open settings panel.
#[derive(Default)]
pub struct Panel {
    /// Saving failed.
    pub error: Option<String>,
    form: ServiceForm,
    /// The selected space, while one is.
    pub space: Option<SpaceCalls>,
    pub google: GoogleState,
    /// The signed-in Matrix account.
    pub account: Option<String>,
}

/// The Google connection, for Meet links.
#[derive(Default)]
pub struct GoogleState {
    /// The signed-in account's email.
    pub account: Option<String>,
    client_id: String,
    client_secret: String,
    /// Browser sign-in running; aborting the handle cancels it.
    pub signing_in: Option<Handle>,
    pub error: Option<String>,
}

/// A space's call service in the panel.
pub struct SpaceCalls {
    pub id: OwnedRoomId,
    pub name: String,
    /// `None` until loaded.
    service: Option<Option<CallService>>,
    /// We may change the call service.
    can_set: bool,
    error: Option<String>,
}

impl SpaceCalls {
    /// For a space where we have `rights`.
    pub fn new(id: OwnedRoomId, name: String, rights: Rights) -> Self {
        Self {
            id,
            name,
            service: None,
            can_set: rights.set_call_service,
            error: None,
        }
    }
}

#[derive(Default)]
struct ServiceForm {
    kind: FormKind,
    name: String,
    url: String,
    error: Option<String>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FormKind {
    #[default]
    Jitsi,
    Rooms,
    Fixed,
    GoogleMeet,
}

impl FormKind {
    const ALL: [Self; 4] = [Self::Jitsi, Self::Rooms, Self::Fixed, Self::GoogleMeet];

    fn url_placeholder(self) -> &'static str {
        match self {
            Self::Jitsi => "Server, e.g. https://meet.jit.si",
            Self::Rooms => "Link with {room}, e.g. https://p2p.mirotalk.com/join/{room}",
            Self::Fixed => "Meeting link, e.g. https://zoom.us/j/123456789",
            Self::GoogleMeet => "",
        }
    }
}

impl fmt::Display for FormKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Jitsi => "Jitsi server",
            Self::Rooms => "New room per call",
            Self::Fixed => "Fixed link",
            Self::GoogleMeet => "Google Meet",
        })
    }
}

/// An entry of the space service picker.
#[derive(Clone, Debug, PartialEq)]
pub struct SpaceChoice(pub Option<CallService>);

impl fmt::Display for SpaceChoice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.0 {
            Some(service) => f.write_str(&service.name),
            None => f.write_str("None"),
        }
    }
}

/// Change a preference, which needs no open panel.
pub fn change(settings: &mut Settings, preference: Preference) -> Action {
    match preference {
        Preference::Appearance(appearance) => {
            settings.appearance = appearance;
            Action::Appearance
        }
        Preference::TextSize(size) => {
            settings.text_size = size;
            Action::TextSize
        }
        Preference::ColoredNames(colored) => {
            settings.colored_names = colored;
            Action::Save
        }
        Preference::DesktopNotifications(on) => {
            settings.desktop_notifications = on;
            Action::Save
        }
        Preference::CheckSpelling(on) => {
            settings.check_spelling = on;
            Action::Spelling
        }
        Preference::IconBadge(on) => {
            settings.icon_badge = on;
            Action::Save
        }
        Preference::RoomList(grouping) => {
            settings.room_list = grouping;
            Action::Save
        }
        Preference::AtHome(at_home) => Action::AtHome(at_home),
        Preference::RoomPictures(on) => {
            settings.room_pictures = on;
            Action::Save
        }
        Preference::PinnedNotes(on) => Action::NotesAtTop(on),
        Preference::SharePresence(on) => {
            settings.share_presence = on;
            Action::SharePresence
        }
        Preference::AutomaticUpdates(on) => {
            settings.updates.automatic = on;
            Action::Save
        }
        Preference::UpdateChannel(channel) => {
            settings.updates.channel = channel;
            Action::CheckUpdates
        }
        Preference::Tint(space, tint) => Action::Colour(space, tint),
    }
}

impl Panel {
    pub fn update(&mut self, settings: &mut Settings, message: Message) -> Action {
        match message {
            Message::Preference(preference) => return change(settings, preference),
            Message::FormKind(kind) => self.form.kind = kind,
            Message::FormName(name) => self.form.name = name,
            Message::FormUrl(url) => self.form.url = url,
            Message::AddService => match self.form.service() {
                Ok(service) => {
                    settings.call_services.push(service);
                    self.form = ServiceForm::default();
                    return Action::Save;
                }
                Err(e) => self.form.error = Some(e),
            },
            Message::MakeDefault(index) if index < settings.call_services.len() => {
                let service = settings.call_services.remove(index);
                settings.call_services.insert(0, service);
                return Action::Save;
            }
            Message::RemoveService(index) if index < settings.call_services.len() => {
                settings.call_services.remove(index);
                return Action::Save;
            }
            Message::MakeDefault(_) | Message::RemoveService(_) => {}
            Message::SpaceLoaded(id, service) => {
                if let Some(space) = self.space.as_mut().filter(|s| s.id == id) {
                    space.service = Some(service);
                }
            }
            Message::SetSpaceService(SpaceChoice(service)) => {
                if let Some(space) = &mut self.space {
                    space.error = None;
                    space.service = Some(service.clone());
                    return Action::SetSpaceService(space.id.clone(), service);
                }
            }
            Message::ColourSaved(result) => self.error = result.err(),
            Message::SpaceSaved(result) => {
                if let Some(space) = &mut self.space {
                    space.error = result.err();
                }
            }
            Message::GoogleClientId(id) => self.google.client_id = id,
            Message::GoogleClientSecret(secret) => self.google.client_secret = secret,
            Message::GoogleSignIn if self.google.can_sign_in() => {
                self.google.error = None;
                return Action::GoogleSignIn(GoogleClient {
                    client_id: self.google.client_id.trim().to_owned(),
                    client_secret: self.google.client_secret.trim().to_owned(),
                });
            }
            Message::GoogleSignIn => {}
            Message::GoogleCancel => {
                if let Some(handle) = self.google.signing_in.take() {
                    handle.abort();
                }
            }
            Message::GoogleSignedIn(result) => {
                self.google.signing_in = None;
                match result {
                    Ok(account) => {
                        self.google.account = Some(account);
                        self.google.client_secret.clear();
                        // Offer Meet links right away.
                        if !settings
                            .call_services
                            .iter()
                            .any(|s| s.kind == ServiceKind::GoogleMeet)
                        {
                            settings.call_services.push(CallService::google_meet());
                            return Action::Save;
                        }
                    }
                    Err(e) => self.google.error = Some(e),
                }
            }
            Message::GoogleSignOut => {
                self.google.error = None;
                return Action::GoogleSignOut;
            }
            Message::GoogleSignedOut(result) => match result {
                Ok(()) => self.google.account = None,
                Err(e) => self.google.error = Some(e),
            },
        }
        Action::None
    }

    /// `spaces`: the top-level spaces, for their room list colours;
    /// `arranged`: the room list's arrangement, from the account;
    /// `presence_shared`: the server shares presence (as far as known);
    /// `updater`: whether there is a newer version, first in the panel.
    pub fn view<'a>(
        &'a self,
        settings: &'a Settings,
        spaces: Vec<(OwnedRoomId, String)>,
        colours: &RoomListColours,
        arranged: &RoomCategories,
        presence_shared: bool,
        updater: &'a Updater,
    ) -> Element<'a, app::Message> {
        let mut panel = column![
            heading("Updates"),
            updates(&settings.updates, updater),
            heading("Feedback"),
            option(
                "Send feedback",
                "Report a problem or suggest an idea, on GitHub.",
                button(text("Send Feedback…").size(13))
                    .padding([4, 10])
                    .style(style::secondary_button)
                    .on_press(app::Message::LinkClicked(feedback::url()))
                    .into(),
            ),
            heading("General"),
            option(
                "Appearance",
                "Light or dark; Follow system changes with your computer's setting.",
                pick_list(Appearance::ALL, Some(settings.appearance), |a| {
                    preference(Preference::Appearance(a))
                })
                .text_size(13)
                .padding(6)
                .into(),
            ),
            option(
                "Text size",
                format!(
                    "Text, pictures and spacing together; also {} and {}, and {} for Default.",
                    shortcut("+"),
                    shortcut("−"),
                    shortcut("0"),
                ),
                pick_list(TextSize::ALL, Some(settings.text_size), |s| {
                    preference(Preference::TextSize(s))
                })
                .text_size(13)
                .padding(6)
                .into(),
            ),
            option(
                "Share my presence",
                if presence_shared {
                    "Others see when you are online or idle."
                } else {
                    "This server does not share presence."
                },
                toggler(settings.share_presence)
                    .on_toggle_maybe(
                        presence_shared.then_some(|on| preference(Preference::SharePresence(on)))
                    )
                    .into(),
            ),
            heading("Chat"),
            option(
                "Coloured names",
                "Show people's names in their own colour, as Element does.",
                toggler(settings.colored_names)
                    .on_toggle(|on| preference(Preference::ColoredNames(on)))
                    .into(),
            ),
            option(
                "Check spelling",
                spelling_detail(),
                toggler(settings.check_spelling)
                    .on_toggle(|on| preference(Preference::CheckSpelling(on)))
                    .into(),
            ),
            option(
                "Desktop notifications",
                "New messages that notify by your Matrix notification settings, \
                 unless you are looking at the room.",
                toggler(settings.desktop_notifications)
                    .on_toggle(|on| preference(Preference::DesktopNotifications(on)))
                    .into(),
            ),
            option(
                "Unread count on the app icon",
                "How many unread messages notify by your Matrix notification \
                 settings, on the app's icon in the Dock.",
                toggler(settings.icon_badge)
                    .on_toggle(|on| preference(Preference::IconBadge(on)))
                    .into(),
            ),
            heading("Room list"),
            room_list_options(settings.room_list),
            home_options(arranged.at_home, settings.room_list),
            option(
                "Room pictures",
                "Rooms show their picture or initial, instead of # for public \
                 rooms and a lock for private ones.",
                toggler(settings.room_pictures)
                    .on_toggle(|on| preference(Preference::RoomPictures(on)))
                    .into(),
            ),
            option(
                "Notes at the top",
                "Your notes to self stay at the top of the room list, in every space.",
                toggler(arranged.notes_at_top())
                    .on_toggle(|on| preference(Preference::PinnedNotes(on)))
                    .into(),
            ),
            tints(colours, spaces),
            heading("Calls"),
            text(
                "Start call uses the space's call service if it has one, \
                 otherwise the first one here."
            )
            .size(12)
            .style(text::secondary),
            self.services_view(settings),
            self.form.view(),
        ]
        .spacing(14);
        panel = panel.push(heading("Google Meet")).push(self.google.view());
        if let Some(space) = &self.space {
            panel = panel
                .push(heading(&format!("Space: {}", space.name)))
                .push(space.view(settings));
        }
        if let Some(account) = &self.account {
            panel = panel.push(heading("Account")).push(
                row![
                    text(account).width(Length::Fill),
                    button(text("Sign out…").size(13))
                        .padding([4, 10])
                        .style(style::secondary_button)
                        .on_press(app::Message::OpenDialog(Kind::SignOut)),
                ]
                .align_y(Alignment::Center),
            );
        }
        if let Some(error) = &self.error {
            panel = panel.push(
                text(format!("Could not save settings: {error}"))
                    .size(12)
                    .style(text::danger),
            );
        }
        frame::frame(
            frame::title("Settings"),
            [],
            app::Message::ShowSettings(false),
            panel,
            560.0,
        )
    }

    fn services_view<'a>(&self, settings: &'a Settings) -> Element<'a, app::Message> {
        if settings.call_services.is_empty() {
            return text("No call services.").size(13).into();
        }
        column(
            settings
                .call_services
                .iter()
                .enumerate()
                .map(|(i, service)| {
                    let detail = match &service.kind {
                        ServiceKind::Rooms { template } => template.as_str(),
                        ServiceKind::Fixed { url } => url.as_str(),
                        ServiceKind::GoogleMeet => "A new meeting with your Google account",
                    };
                    let mut line = row![
                        column![
                            text(&service.name),
                            text(detail).size(12).style(text::secondary)
                        ]
                        .width(Length::Fill),
                    ]
                    .spacing(4)
                    .align_y(Alignment::Center);
                    if i == 0 {
                        line = line.push(text("Default").size(12).style(text::secondary));
                    } else {
                        line = line.push(small_button(
                            "Make default",
                            message(Message::MakeDefault(i)),
                        ));
                    }
                    line.push(small_button("Remove", message(Message::RemoveService(i))))
                        .into()
                }),
        )
        .spacing(8)
        .into()
    }
}

impl GoogleState {
    fn can_sign_in(&self) -> bool {
        self.signing_in.is_none()
            && !self.client_id.trim().is_empty()
            && !self.client_secret.trim().is_empty()
    }

    fn view(&self) -> Element<'_, app::Message> {
        let mut section = column![].spacing(8);
        if let Some(account) = &self.account {
            section = section.push(
                row![
                    text(format!("Connected as {account}")).width(Length::Fill),
                    small_button("Disconnect", message(Message::GoogleSignOut)),
                ]
                .align_y(Alignment::Center),
            );
        } else if self.signing_in.is_some() {
            section = section.push(
                row![
                    text("Continue in your browser…").width(Length::Fill),
                    small_button("Cancel", message(Message::GoogleCancel)),
                ]
                .align_y(Alignment::Center),
            );
        } else {
            let field = |placeholder, value: &str, on_input: fn(String) -> Message| {
                text_input(placeholder, value)
                    .on_input(move |v| message(on_input(v)))
                    .on_submit(message(Message::GoogleSignIn))
                    .style(style::input)
                    .padding(6)
                    .size(13)
            };
            section = section
                .push(
                    text(
                        "Creates Meet links with your Google account. Needs an OAuth client \
                         of your own Google Cloud project (see the README).",
                    )
                    .size(12)
                    .style(text::secondary),
                )
                .push(
                    row![
                        field("Client ID", &self.client_id, Message::GoogleClientId),
                        field(
                            "Client secret",
                            &self.client_secret,
                            Message::GoogleClientSecret
                        )
                        .secure(true),
                        button(text("Connect").size(13))
                            .padding([5, 12])
                            .style(style::secondary_button)
                            .on_press_maybe(
                                self.can_sign_in().then(|| message(Message::GoogleSignIn))
                            ),
                    ]
                    .spacing(6)
                    .align_y(Alignment::Center),
                );
        }
        if let Some(error) = &self.error {
            section = section.push(text(error).size(12).style(text::danger));
        }
        section.into()
    }
}

impl ServiceForm {
    /// The service described by the form.
    fn service(&self) -> Result<CallService, String> {
        let url = self.url.trim();
        let name = match self.name.trim() {
            "" => self.kind.to_string(),
            name => name.to_owned(),
        };
        let service = match self.kind {
            FormKind::Jitsi => CallService::jitsi(&name, url),
            FormKind::Rooms => CallService::rooms(&name, url),
            FormKind::Fixed => CallService::fixed(&name, url),
            FormKind::GoogleMeet => Ok(CallService::google_meet()),
        };
        service.map_err(|e| e.to_string())
    }

    fn view(&self) -> Element<'_, app::Message> {
        let field = |placeholder, value: &str, on_input: fn(String) -> Message| {
            text_input(placeholder, value)
                .on_input(move |v| message(on_input(v)))
                .on_submit(message(Message::AddService))
                .style(style::input)
                .padding(6)
                .size(13)
        };
        let mut inputs = row![
            pick_list(FormKind::ALL, Some(self.kind), |k| message(
                Message::FormKind(k)
            ))
            .text_size(13)
            .padding(6),
        ]
        .spacing(6)
        .align_y(Alignment::Center);
        if self.kind != FormKind::GoogleMeet {
            inputs = inputs
                .push(field("Name", &self.name, Message::FormName).width(120))
                .push(field(
                    self.kind.url_placeholder(),
                    &self.url,
                    Message::FormUrl,
                ));
        } else {
            inputs = inputs.push(
                text("Needs the Google connection below.")
                    .size(12)
                    .style(text::secondary)
                    .width(Length::Fill),
            );
        }
        inputs = inputs.push(
            button(text("Add").size(13))
                .padding([5, 12])
                .style(style::secondary_button)
                .on_press(message(Message::AddService)),
        );
        let mut form = column![inputs].spacing(4);
        if let Some(error) = &self.error {
            form = form.push(text(error).size(12).style(text::danger));
        }
        form.into()
    }
}

impl SpaceCalls {
    fn view<'a>(&'a self, settings: &'a Settings) -> Element<'a, app::Message> {
        let mut section =
            column![text(format!("Call service for everyone in {}", self.name))].spacing(6);
        match &self.service {
            None => section = section.push(text("Loading…").size(12).style(text::secondary)),
            Some(current) => {
                let mut choices = vec![SpaceChoice(None)];
                choices.extend(current.iter().cloned().map(|s| SpaceChoice(Some(s))));
                choices.extend(
                    settings
                        .call_services
                        .iter()
                        .filter(|s| Some(*s) != current.as_ref())
                        .cloned()
                        .map(|s| SpaceChoice(Some(s))),
                );
                let allowed = self.can_set;
                let picker =
                    pick_list(choices, Some(SpaceChoice(current.clone())), move |choice| {
                        message(Message::SetSpaceService(choice))
                    })
                    .text_size(13)
                    .padding(6);
                section = section.push(if allowed {
                    Element::from(picker)
                } else {
                    row![
                        text(SpaceChoice(current.clone()).to_string()),
                        text("Only moderators can change it.")
                            .size(12)
                            .style(text::secondary),
                    ]
                    .spacing(12)
                    .into()
                });
            }
        }
        if let Some(error) = &self.error {
            section = section.push(text(error).size(12).style(text::danger));
        }
        section.into()
    }
}

/// This version or a newer one, and how to look for them.
fn updates<'a>(
    preferences: &'a update::Preferences,
    updater: &'a Updater,
) -> Element<'a, app::Message> {
    let current = update::current();
    let action = |label, message, style: fn(&Theme, button::Status) -> button::Style| {
        button(text(label).size(13))
            .padding([4, 10])
            .style(style)
            .on_press(app::Message::Update(message))
    };
    let state = |line: String, detail: Option<String>| {
        let mut lines = column![text(line)].spacing(2);
        if let Some(detail) = detail {
            lines = lines.push(text(detail).size(12).style(text::secondary));
        }
        lines.width(Length::Fill)
    };
    let status = match &updater.available {
        Some(update) => row![
            {
                let line = state(
                    format!("Mtrx {} is available", update.version),
                    Some(
                        updater
                            .error
                            .clone()
                            .unwrap_or(format!("You have {current}.")),
                    ),
                );
                // How much has come, below what is available.
                match updater.installing.map(|step| step.downloaded()) {
                    Some(done) => line.push(
                        container(
                            progress_bar(0.0..=1.0, done.unwrap_or(0.0))
                                .girth(4)
                                .style(progress_bar::primary),
                        )
                        .padding(iced::Padding::ZERO.top(4).right(12)),
                    ),
                    None => line,
                }
            },
            action(
                "What's new",
                update::Message::Notes,
                style::secondary_button
            ),
            if let Some(step) = updater.installing {
                button(
                    text(match step {
                        update::Step::Downloading { .. } => "Downloading…",
                        update::Step::Installing => "Installing…",
                    })
                    .size(13),
                )
                .padding([4, 10])
                .style(style::primary_button)
            } else if update.installable() {
                action(
                    "Install and restart",
                    update::Message::Download,
                    style::primary_button,
                )
            } else {
                action("Download", update::Message::Download, style::primary_button)
            },
        ],
        None => {
            let line = if updater.checking {
                "Looking for updates…".to_owned()
            } else if updater.checked.is_some() && updater.error.is_none() {
                format!("Mtrx {current} is the newest version.")
            } else {
                format!("This is Mtrx {current}.")
            };
            let mut check = button(text("Check now").size(13))
                .padding([4, 10])
                .style(style::secondary_button);
            if !updater.checking {
                check = check.on_press(app::Message::Update(update::Message::Check));
            }
            row![state(line, updater.error.clone()), check]
        }
    };
    column![
        status.spacing(8).align_y(Alignment::Center),
        option(
            "Check automatically",
            "At start and once a day.",
            toggler(preferences.automatic)
                .on_toggle(|on| preference(Preference::AutomaticUpdates(on)))
                .into(),
        ),
        option(
            "Channel",
            "Beta also offers pre-releases, to try new things first.",
            pick_list(update::Channel::ALL, Some(preferences.channel), |c| {
                preference(Preference::UpdateChannel(c))
            })
            .text_size(13)
            .padding(6)
            .into(),
        ),
    ]
    .spacing(14)
    .into()
}

/// What "Check spelling" says: the languages checked, and where to add one.
fn spelling_detail() -> String {
    /// Languages named, the rest counted.
    const NAMED: usize = 4;
    let languages = spelling::languages();
    let named = match languages.len() {
        0 => "none yet".to_owned(),
        n if n <= NAMED => languages.join(", "),
        n => format!(
            "{} and {} more",
            languages[..NAMED - 1].join(", "),
            n - (NAMED - 1)
        ),
    };
    let add = if cfg!(target_os = "macos") {
        "Choose them in System Settings › Keyboard › Text Input › Edit › Spelling."
    } else if cfg!(windows) {
        "Add one in Settings › Time & language › Language & region."
    } else {
        "Add one by installing its Hunspell dictionary, such as hunspell-es."
    };
    format!("Marks misspelled words where you write. Languages: {named}. {add}")
}

fn heading<'a>(label: &str) -> Element<'a, app::Message> {
    text(label.to_uppercase())
        .size(11)
        .font(fonts::SEMIBOLD)
        .style(text::secondary)
        .into()
}

/// The room list's colour at home and in each space.
fn tints<'a>(
    colours: &RoomListColours,
    spaces: Vec<(OwnedRoomId, String)>,
) -> Element<'a, app::Message> {
    let line = |label: String, space: Option<OwnedRoomId>, chosen: Tint| {
        let swatches = Tint::ALL.map(|tint| {
            let selected = tint == chosen;
            hint(
                button(space::horizontal())
                    .width(22)
                    .height(22)
                    .padding(0)
                    .style(move |theme: &iced::Theme, status| {
                        style::swatch(theme, status, tint, selected)
                    })
                    .on_press(preference(Preference::Tint(space.clone(), tint))),
                tint.name(),
                Position::Top,
            )
        });
        row![
            text(label)
                .width(Length::Fill)
                .wrapping(text::Wrapping::None),
            row(swatches).spacing(6),
        ]
        .spacing(12)
        .align_y(Alignment::Center)
        .into()
    };
    let mut lines = vec![line("Home".into(), None, Tint::of(colours, None))];
    for (space, name) in spaces {
        let chosen = Tint::of(colours, Some(&space));
        lines.push(line(name, Some(space), chosen));
    }
    column![
        column![
            text("Colour"),
            text("The room list's colour; spaces without their own use Home's. Synced to your other devices.")
                .size(12)
                .style(text::secondary),
        ]
        .spacing(2),
        column(lines).spacing(8),
    ]
    .spacing(8)
    .into()
}

/// Which built-in sections the room list has.
fn room_list_options<'a>(grouping: Grouping) -> Element<'a, app::Message> {
    let toggle = |on: bool, set: fn(&mut Grouping, bool)| {
        toggler(on)
            .on_toggle(move |on| {
                let mut grouping = grouping;
                set(&mut grouping, on);
                preference(Preference::RoomList(grouping))
            })
            .into()
    };
    column![
        option(
            "Favourites",
            "Rooms marked as favourite on top. Element shows the same favourites.",
            toggle(grouping.favourites, |g, on| g.favourites = on),
        ),
        option(
            "Direct messages",
            "Direct chats in their own section; in a space, those with its members.",
            toggle(grouping.people, |g, on| g.people = on),
        ),
        option(
            "Low priority",
            "Rooms marked as low priority at the bottom.",
            toggle(grouping.low_priority, |g, on| g.low_priority = on),
        ),
        option(
            "Unread first",
            "Rooms with unread messages at the top of each section.",
            toggle(grouping.unread_first, |g, on| g.unread_first = on),
        ),
    ]
    .spacing(14)
    .into()
}

/// Which rooms of spaces Home lists too. Favourites only matter while Home
/// leaves out the rest and the room list has its Favourites section.
fn home_options<'a>(at_home: AtHome, grouping: Grouping) -> Element<'a, app::Message> {
    let favourites = if at_home.all_rooms {
        "Home already shows every room."
    } else if !grouping.favourites {
        "Needs the Favourites section."
    } else {
        "Favourites from your spaces also show at Home, in Favourites."
    };
    let can_favourite = !at_home.all_rooms && grouping.favourites;
    column![
        option(
            "Show all rooms at Home",
            "Also the rooms of your spaces; otherwise Home has your chats, \
             invites and rooms in no space. Synced to your other devices.",
            toggler(at_home.all_rooms)
                .on_toggle(move |all_rooms| preference(Preference::AtHome(AtHome {
                    all_rooms,
                    ..at_home
                })))
                .into(),
        ),
        option(
            "Show favourites at Home",
            favourites,
            toggler(at_home.favourites)
                .on_toggle_maybe(can_favourite.then_some(move |favourites| {
                    preference(Preference::AtHome(AtHome {
                        favourites,
                        ..at_home
                    }))
                }))
                .into(),
        ),
    ]
    .spacing(14)
    .into()
}

/// A setting: its name and what it does, beside its control.
pub fn option<'a>(
    label: impl text::IntoFragment<'a>,
    description: impl text::IntoFragment<'a>,
    control: Element<'a, app::Message>,
) -> Element<'a, app::Message> {
    row![
        column![
            text(label),
            text(description).size(12).style(text::secondary)
        ]
        .spacing(2)
        .width(Length::Fill),
        control,
    ]
    .spacing(16)
    .align_y(Alignment::Center)
    .into()
}

fn small_button<'a>(label: &'a str, on_press: app::Message) -> Element<'a, app::Message> {
    button(text(label).size(12))
        .padding([2, 8])
        .style(style::text_button)
        .on_press(on_press)
        .into()
}

fn message(message: Message) -> app::Message {
    app::Message::Settings(message)
}

pub fn preference(preference: Preference) -> app::Message {
    message(Message::Preference(preference))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saves_and_loads() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = Settings::path(dir.path());
        assert_eq!(Settings::load(&path), Settings::default(), "missing file");
        let settings = Settings {
            appearance: Appearance::Dark,
            text_size: TextSize::Larger,
            colored_names: true,
            desktop_notifications: false,
            icon_badge: false,
            call_services: vec![CallService::fixed("Zoom", "https://zoom.us/j/1").unwrap()],
            room_list: Grouping {
                unread_first: true,
                ..Grouping::default()
            },
            room_list_width: 320.0,
            room_pictures: true,
            pinned_notes: false,
            share_presence: false,
            check_spelling: false,
            updates: crate::update::Preferences {
                automatic: false,
                channel: crate::update::Channel::Beta,
                seen: Some("0.4.0".into()),
            },
            legacy_home_tint: None,
            legacy_space_tints: BTreeMap::new(),
        };
        settings.save(&path).unwrap();
        assert_eq!(Settings::load(&path), settings);
        let saved = std::fs::read_to_string(&path).unwrap();
        assert!(saved.contains(r#""appearance": "Dark""#), "{saved}");
        assert!(saved.contains(r#""text_size": "Larger""#), "{saved}");
        assert!(saved.contains(r#""share_presence": false"#), "{saved}");
        assert!(saved.contains(r#""channel": "beta""#), "{saved}");
        std::fs::write(&path, "{}").unwrap();
        assert_eq!(Settings::load(&path), Settings::default(), "older file");

        // Colours from before the account kept them: read, to move there,
        // and not written again.
        std::fs::write(
            &path,
            r#"{"home_tint": "Blue", "space_tints": {"!s:example.org": "Rose"}}"#,
        )
        .unwrap();
        let old = Settings::load(&path);
        let colours = old.legacy_colours().unwrap();
        assert_eq!(colours.home.as_deref(), Some("Blue"));
        assert_eq!(
            colours.spaces.values().next().map(String::as_str),
            Some("Rose")
        );
        old.save(&path).unwrap();
        assert_eq!(Settings::load(&path).legacy_colours(), None);
    }

    #[test]
    fn chooses_the_appearance() {
        let mut settings = Settings::default();
        assert_eq!(settings.appearance, Appearance::System);
        let action = change(&mut settings, Preference::Appearance(Appearance::Light));
        assert!(matches!(action, Action::Appearance));
        assert_eq!(settings.appearance, Appearance::Light);
    }

    #[test]
    fn chooses_the_text_size() {
        let mut settings = Settings::default();
        assert_eq!(settings.text_size, TextSize::Default);
        let action = change(&mut settings, Preference::TextSize(TextSize::Smaller));
        assert!(matches!(action, Action::TextSize));
        assert_eq!(settings.text_size, TextSize::Smaller);
    }

    #[test]
    fn shares_presence_unless_turned_off() {
        let mut settings = Settings::default();
        assert!(settings.share_presence);
        let action = change(&mut settings, Preference::SharePresence(false));
        assert!(matches!(action, Action::SharePresence));
        assert!(!settings.share_presence);
    }

    #[test]
    fn what_home_lists_goes_to_the_account() {
        let mut settings = Settings::default();
        let at_home = AtHome {
            all_rooms: true,
            favourites: false,
        };
        let action = change(&mut settings, Preference::AtHome(at_home));
        assert!(matches!(action, Action::AtHome(sent) if sent == at_home));
        // Not in the settings file.
        assert_eq!(settings, Settings::default());
    }

    #[test]
    fn adds_services_from_the_form() {
        let mut settings = Settings::default();
        let mut panel = Panel::default();
        for message in [
            Message::FormKind(FormKind::Fixed),
            Message::FormName("My Zoom".into()),
            Message::FormUrl("https://zoom.us/j/42".into()),
        ] {
            panel.update(&mut settings, message);
        }
        assert!(matches!(
            panel.update(&mut settings, Message::AddService),
            Action::Save
        ));
        assert_eq!(settings.call_services[1].name, "My Zoom");
        panel.update(&mut settings, Message::MakeDefault(1));
        assert_eq!(settings.call_services[0].name, "My Zoom");

        panel.update(&mut settings, Message::FormUrl("not a link".into()));
        assert!(matches!(
            panel.update(&mut settings, Message::AddService),
            Action::None
        ));
        assert!(panel.form.error.is_some());
    }
}
