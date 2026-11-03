//! Cards: who someone is, with their address to copy; a room's settings.
//! Opened by clicking a name or from the room and member lists.

use std::{fmt, time::SystemTime};

use iced::{
    Alignment, Element, Length, Theme,
    widget::{button, column, container, pick_list, row, space, stack, text, text_input, toggler},
};
use mtrx_core::{
    Access, CallService, History, MxcUri, NotifyMode, OwnedMxcUri, OwnedRoomId, OwnedUserId,
    Permissions, RoomHandle, RoomKind, RoomSettings, RoomSummary, RoomTag, SpaceGraph, Topic,
    UserInfo, UserSummary,
};

use crate::{
    app, arrange,
    avatar::{Avatars, Shape},
    context_menu,
    dialog::Kind,
    directory, fonts, frame,
    hint::{Position, hint},
    icons,
    overview::{self, Overview},
    popover::{Edge, Side, popover},
    presence::{self, Presences},
    selectable::selectable_text,
    settings::{self, Preference},
    sidebar, spelling, style, topic,
};

/// The picture in the card's header.
const AVATAR: f32 = 40.0;
const WIDTH: f32 = 500.0;
/// A space's card, with its rooms.
const OVERVIEW_WIDTH: f32 = 720.0;
/// A longer topic above a space's tabs scrolls.
const ABOUT_HEIGHT: f32 = 180.0;
/// Height of a person's shared rooms list; more scroll.
const SHARED_HEIGHT: f32 = 200.0;

/// What a card shows, besides its target: the rooms we know, pictures, who
/// we are, people's presence as of `now`, whether the Notes are pinned at
/// the top (our own card switches it), and our call services, naming a
/// room's calls.
#[derive(Clone, Copy)]
pub struct Shown<'a> {
    pub rooms: &'a [RoomSummary],
    /// The joined spaces and what they list.
    pub graph: &'a SpaceGraph,
    pub avatars: &'a Avatars,
    pub own_user: &'a OwnedUserId,
    pub presences: &'a Presences,
    pub now: SystemTime,
    pub pinned_notes: bool,
    pub call_services: &'a [CallService],
    /// Rooms show their pictures, as the room list does.
    pub room_pictures: bool,
    /// The member whose menu is open, and in which room.
    pub member_menu: Option<&'a crate::members::MenuKey>,
    /// The rooms of spaces as their servers listed them.
    pub space_rooms: &'a directory::SpaceRooms,
}

/// What to show a card for.
#[derive(Debug, Clone)]
pub enum Target {
    /// With the name and picture known so far.
    User(UserSummary),
    /// The room's settings.
    Room(OwnedRoomId),
}

#[derive(Debug, Clone)]
pub enum Message {
    UserLoaded(OwnedUserId, Result<UserInfo, String>),
    SettingsLoaded(OwnedRoomId, Result<RoomSettings, String>),
    Copy(String),
    Name(String),
    Topic(topic::Edit),
    /// Save the changed name and topic.
    SaveDetails,
    Notify(NotifyMode),
    Access(Access),
    ChangePicture,
    RemovePicture,
    /// A change was saved on the server, or not.
    Saved(Result<(), String>),
    PictureMenu(bool),
    EditTopic,
    /// Stop editing the topic, back to this one.
    CancelTopic(String),
    /// The address's name, as edited.
    AddressName(String),
    Change(Change),
    /// The menu of spaces to add the room to.
    SpacesMenu(bool),
    Overview(overview::Message),
    Close,
}

/// A setting changed on a room's card, saved at once.
#[derive(Debug, Clone)]
pub enum Change {
    /// The address's name; none without.
    Address(Option<String>),
    Listed(bool),
    History(History),
    Permissions(Permissions),
    Suggested(OwnedRoomId, bool),
    AddToSpace(OwnedRoomId),
    RemoveFromSpace(OwnedRoomId),
    Unban(OwnedUserId),
}

/// A card's parts, for its [`frame::frame`].
struct Framed<'a> {
    title: Element<'a, app::Message>,
    actions: Vec<Element<'a, app::Message>>,
    /// Stays below the header, e.g. a space's tabs.
    bar: Option<Element<'a, app::Message>>,
    body: Element<'a, app::Message>,
    /// A pill over the body's bottom, e.g. while it updates.
    status: Option<Element<'a, app::Message>>,
}

pub struct Card {
    pub target: Target,
    /// The room's, to read and change its settings; set by the app.
    pub handle: Option<RoomHandle>,
    /// Spaces and rooms shared with the person, once loaded.
    shared: Option<Vec<OwnedRoomId>>,
    /// The room's settings, once loaded.
    pub settings: Option<RoomSettings>,
    /// The room's name and topic as edited.
    name: String,
    topic: topic::Editor,
    /// A change is being saved.
    pub saving: bool,
    error: Option<String>,
    /// The text last copied, to confirm it.
    copied: Option<String>,
    /// For a space: its rooms and members too.
    pub overview: Option<Overview>,
    /// The picture's menu is open.
    picture_menu: bool,
    editing_topic: bool,
    /// The address's name, as edited.
    address: String,
    spaces_menu: bool,
}

impl Card {
    /// A space's card opens on `tab`.
    pub fn new(target: Target, rooms: &[RoomSummary], tab: overview::Tab) -> Self {
        let room = match &target {
            Target::Room(id) => rooms.iter().find(|r| &r.id == id),
            Target::User(_) => None,
        };
        Self {
            picture_menu: false,
            editing_topic: false,
            address: String::new(),
            spaces_menu: false,
            overview: room
                .filter(|r| r.kind == RoomKind::Space)
                .map(|r| Overview::new(r.id.clone(), tab)),
            name: room.map(|r| r.name.clone()).unwrap_or_default(),
            topic: topic::Editor::new(room.and_then(|r| r.topic.as_deref()).unwrap_or_default()),
            target,
            handle: None,
            shared: None,
            settings: None,
            saving: false,
            error: None,
            copied: None,
        }
    }

    /// The user whose details still need loading.
    pub fn user(&self) -> Option<&OwnedUserId> {
        match &self.target {
            Target::User(user) => Some(&user.id),
            Target::Room(_) => None,
        }
    }

    /// The room whose settings are shown.
    pub fn room(&self) -> Option<&OwnedRoomId> {
        match &self.target {
            Target::Room(id) => Some(id),
            Target::User(_) => None,
        }
    }

    /// The card's picture, with its size.
    pub fn avatar<'a>(&'a self, rooms: &'a [RoomSummary]) -> Option<(&'a MxcUri, f32)> {
        let mxc = match &self.target {
            Target::User(user) => user.avatar.as_deref(),
            Target::Room(id) => rooms.iter().find(|r| &r.id == id)?.avatar.as_deref(),
        };
        Some((mxc?, AVATAR))
    }

    /// The edited name and topic, if they differ from `room`'s.
    pub fn changed_details(&self, room: &RoomSummary) -> (Option<String>, Option<String>) {
        let name = self.name.trim();
        (
            (!name.is_empty() && name != room.name).then(|| name.to_owned()),
            (self.topic.changed(room.topic.as_deref())).then(|| self.topic.topic()),
        )
    }

    /// What the space's overview needs done, if anything.
    pub fn update(&mut self, message: Message) -> overview::Action {
        match message {
            Message::UserLoaded(id, Ok(info)) => {
                if let Target::User(user) = &mut self.target
                    && user.id == id
                {
                    // Keep what we knew if the profile is not readable.
                    user.name = info.user.name.or(user.name.take());
                    user.avatar = info.user.avatar.or(user.avatar.take());
                    self.shared = Some(info.shared);
                }
            }
            Message::SettingsLoaded(id, Ok(settings)) => {
                if self.room() == Some(&id) {
                    // The address as saved, unless it is being edited.
                    let saved = self.settings.as_ref().map(address_name);
                    if saved.is_none_or(|saved| saved == self.address) {
                        self.address = address_name(&settings);
                    }
                    self.settings = Some(settings);
                }
            }
            Message::UserLoaded(_, Err(e)) | Message::SettingsLoaded(_, Err(e)) => {
                self.error = Some(e)
            }
            Message::Copy(copied) => self.copied = Some(copied),
            Message::Name(name) => self.name = name,
            Message::Topic(edit) => self.topic.update(edit),
            Message::Notify(mode) => {
                if let Some(settings) = &mut self.settings {
                    settings.notify = mode;
                }
                self.saving = true;
                self.error = None;
            }
            Message::Access(access) => {
                if let Some(settings) = &mut self.settings {
                    settings.access = access;
                }
                self.saving = true;
                self.error = None;
            }
            Message::ChangePicture | Message::RemovePicture => {
                self.picture_menu = false;
                self.saving = true;
                self.error = None;
            }
            Message::SaveDetails => {
                self.saving = true;
                self.error = None;
            }
            Message::Saved(result) => {
                self.saving = false;
                if result.is_ok() {
                    self.editing_topic = false;
                }
                self.error = result.err();
            }
            Message::PictureMenu(open) => self.picture_menu = open,
            Message::SpacesMenu(open) => self.spaces_menu = open,
            Message::AddressName(name) => self.address = name,
            Message::Change(change) => {
                self.spaces_menu = false;
                self.saving = true;
                self.error = None;
                if let Some(settings) = &mut self.settings {
                    apply(settings, &change);
                }
            }
            Message::EditTopic => self.editing_topic = true,
            Message::CancelTopic(topic) => {
                self.topic = topic::Editor::new(&topic);
                self.editing_topic = false;
            }
            Message::Overview(message) => {
                if let Some(overview) = &mut self.overview {
                    return overview.update(message);
                }
            }
            Message::Close => {}
        }
        overview::Action::None
    }

    pub fn view<'a>(&'a self, shown: Shown<'a>) -> Element<'a, app::Message> {
        let Framed {
            title,
            actions,
            bar,
            body,
            status,
        } = match &self.target {
            Target::User(user) => self.user_view(user, shown),
            Target::Room(id) => match shown.rooms.iter().find(|r| &r.id == id) {
                Some(room) => self.room_view(room, shown),
                None => Framed {
                    title: frame::title("Room"),
                    actions: Vec::new(),
                    bar: None,
                    body: text("This room is not in your list.").into(),
                    status: None,
                },
            },
        };
        frame::with_bar(
            title,
            actions,
            app::Message::Info(Message::Close),
            bar,
            body,
            status,
            if self.overview.is_some() {
                OVERVIEW_WIDTH
            } else {
                WIDTH
            },
        )
    }

    /// The header's main button for the room: Save once the name or topic
    /// changed, else Open (a room, not a space).
    fn room_action<'a>(&self, room: &RoomSummary) -> Option<Element<'a, app::Message>> {
        let (name, topic) = self.changed_details(room);
        if name.is_some() || topic.is_some() {
            Some(frame::primary(
                "Save",
                (!self.saving && self.topic.valid())
                    .then_some(app::Message::Info(Message::SaveDetails)),
            ))
        } else {
            (room.kind != RoomKind::Space && !room.invited)
                .then(|| frame::primary("Open", Some(app::Message::OpenRoom(room.id.clone()))))
        }
    }

    /// Saves the changed name and topic (⌘↩), if any.
    pub fn submit(&self, rooms: &[RoomSummary]) -> Option<app::Message> {
        let room = rooms.iter().find(|r| Some(&r.id) == self.room())?;
        let (name, topic) = self.changed_details(room);
        ((name.is_some() || topic.is_some()) && !self.saving && self.topic.valid())
            .then_some(app::Message::Info(Message::SaveDetails))
    }

    fn user_view<'a>(&'a self, user: &'a UserSummary, shown: Shown<'a>) -> Framed<'a> {
        let Shown {
            rooms,
            avatars,
            own_user,
            presences,
            now,
            pinned_notes,
            ..
        } = shown;
        let presence = presences.get(&user.id);
        let name = user.name.as_deref().unwrap_or(user.id.localpart());
        // The star favourites the direct chat, made if there is none.
        let chat = rooms
            .iter()
            .find(|r| r.kind == RoomKind::Direct && r.direct_user.as_ref() == Some(&user.id));
        let star = match chat {
            Some(chat) => sidebar::star_button(&chat.id, chat.tag == RoomTag::Favourite),
            None => sidebar::star_for(app::Message::FavouritePerson(user.id.clone()), false),
        };
        let picture = avatars.view(
            user.id.as_str(),
            name,
            user.avatar.as_deref(),
            AVATAR,
            Shape::Circle,
        );
        let picture = presence::with_dot(picture, presence, AVATAR, |theme| {
            theme.extended_palette().background.base.color
        });
        let mut title = column![selectable_text(name).size(18).font(fonts::SEMIBOLD)].spacing(2);
        if let Some(line) = presence.and_then(|p| p.line(now)) {
            title = title.push(text(line).size(13).style(text::secondary));
        }
        let title = row![picture, title.width(Length::Fill), star]
            .spacing(12)
            .align_y(Alignment::Center);
        let mut card = column![self.copyable("User ID", user.id.to_string())].spacing(14);
        let shared: Option<Element<'a, app::Message>> = match &self.shared {
            None if self.error.is_none() => Some(frame::loading()),
            None => None,
            Some(shared) if shared.is_empty() => Some(
                text("You share no rooms or spaces.")
                    .size(13)
                    .style(text::secondary)
                    .into(),
            ),
            Some(_) => None,
        };
        if let Some(shared) = shared {
            card = card.push(shared);
        } else if let Some(shared) = &self.shared {
            card = card.push(shared_view(shared, rooms));
        }
        if let Some(error) = &self.error {
            card = card.push(text(error).size(12).style(text::danger));
        }
        // Our own card is the Notes': where they are pinned too.
        let message = if user.id == *own_user {
            card = card.push(settings::option(
                "Notes at the top",
                "Your notes to self stay at the top of the room list, in every space.",
                toggler(pinned_notes)
                    .on_toggle(|on| settings::preference(Preference::PinnedNotes(on)))
                    .into(),
            ));
            "Notes to self"
        } else {
            "Message"
        };
        Framed {
            title: title.into(),
            actions: vec![frame::primary(
                message,
                Some(app::Message::ChatWith(user.id.clone())),
            )],
            bar: None,
            body: card.into(),
            status: None,
        }
    }

    /// Picture, name (editable where allowed) and address, favourite,
    /// invite and leave; the topic and settings; a space's tabs.
    fn room_view<'a>(&'a self, room: &'a RoomSummary, shown: Shown<'a>) -> Framed<'a> {
        let Shown {
            avatars,
            call_services,
            ..
        } = shown;
        let settings = self.settings.as_ref();
        // They decide what may be changed: until then, only that it loads.
        let loading = settings.is_none() && self.error.is_none();
        let allowed = |can: fn(&RoomSettings) -> bool| settings.is_some_and(can) && !self.saving;
        let star = sidebar::star_button(&room.id, room.tag == RoomTag::Favourite);
        let picture = avatars.view(
            room.id.as_str(),
            &room.name,
            room.avatar.as_deref(),
            AVATAR,
            Shape::Circle,
        );
        let picture = if allowed(|s| s.can_change_picture) {
            self.picture_button(picture, room.avatar.is_some())
        } else {
            picture
        };
        // The name is edited where it is shown.
        let name: Element<'a, app::Message> = if allowed(|s| s.can_rename) {
            hint(
                spelling::checked(text_input("Room name", &self.name), &self.name)
                    .style(style::title_input)
                    .font(fonts::SEMIBOLD)
                    .size(18)
                    .padding([2, 6])
                    .on_input(|value| app::Message::Info(Message::Name(value)))
                    .on_submit(app::Message::Info(Message::SaveDetails)),
                "Rename",
                Position::Bottom,
            )
        } else {
            selectable_text(room.name.as_str())
                .size(18)
                .font(fonts::SEMIBOLD)
                .into()
        };
        // The address under the name, small, to copy.
        let mut named = column![name].spacing(0);
        if let Some(address) = settings
            .and_then(|s| s.address.as_ref().map(ToString::to_string))
            .or_else(|| room.alias.clone())
        {
            named = named.push(
                container(selectable_text(address).size(12).style(text::secondary)).padding([0, 6]),
            );
        }
        let title = row![picture, container(named).width(Length::Fill), star]
            .spacing(12)
            .align_y(Alignment::Center)
            .into();
        let about = self.about(room, call_services, allowed(|s| s.can_change_topic));
        let body = if loading {
            frame::loading()
        } else {
            self.settings_view(room, shown)
        };
        // Invite and Leave on every tab; Invite where the people are.
        let mut actions = Vec::new();
        if !room.invited {
            if settings.is_some_and(|s| s.can_invite) {
                let invite = match &self.overview {
                    Some(_) => app::Message::Info(Message::Overview(overview::Message::Tab(
                        overview::Tab::Members,
                    ))),
                    None => app::Message::ShowMembers(room.id.clone()),
                };
                actions.push(header_button("Invite…", style::soft_button, invite));
            }
            actions.push(header_button(
                "Leave…",
                style::secondary_button,
                app::Message::OpenDialog(Kind::Leave {
                    room: room.id.clone(),
                    name: room.name.clone(),
                }),
            ));
        }
        actions.extend(self.room_action(room));
        let Some(overview) = &self.overview else {
            return Framed {
                title,
                actions,
                bar: None,
                body: if loading {
                    body
                } else {
                    column![about, body].spacing(14).into()
                },
                status: None,
            };
        };
        // A space: its rooms, members and these settings, in tabs. The
        // topic stays above them, scrolling itself if long.
        let bar = column![
            container(frame::scroll(about).height(Length::Shrink)).max_height(ABOUT_HEIGHT),
            overview.tabs(),
        ]
        .spacing(12);
        Framed {
            title,
            actions,
            status: overview.status(shown),
            bar: Some(bar.into()),
            body: overview.view(
                room,
                shown,
                settings.is_some_and(|s| s.can_add_rooms),
                settings.is_some_and(|s| s.can_invite) && !room.invited,
                body,
            ),
        }
    }

    /// The settings below the topic: notifications and the room list, who
    /// can join, find, read and edit it, encryption, its address, the
    /// spaces it is in, and who is banned.
    fn settings_view<'a>(
        &'a self,
        room: &'a RoomSummary,
        shown: Shown<'a>,
    ) -> Element<'a, app::Message> {
        let rooms = shown.rooms;
        let settings = self.settings.as_ref();
        let change = |change| app::Message::Info(Message::Change(change));
        let enabled = |can: bool| can && !self.saving;
        let mut card = column![].spacing(14);
        if let Some(settings) = settings {
            card = card.push(settings::option(
                "Notifications",
                match settings.notify {
                    NotifyMode::All => "Every message notifies you.",
                    NotifyMode::Mentions => "Only mentions of you and your keywords notify you.",
                    NotifyMode::Mute => "Nothing here notifies you.",
                },
                pick_list(
                    Notify::ALL,
                    Some(Notify(settings.notify)),
                    |Notify(mode)| app::Message::Info(Message::Notify(mode)),
                )
                .text_size(13)
                .padding(6)
                .into(),
            ));
        }
        let low_priority = room.tag == RoomTag::LowPriority;
        card = card.push(settings::option(
            "Low priority",
            "Listed after your other rooms, when the room list groups them.",
            toggler(low_priority)
                .on_toggle(|on| {
                    app::Message::Arrange(arrange::Message::Tag(
                        room.id.clone(),
                        if on {
                            RoomTag::LowPriority
                        } else {
                            RoomTag::None
                        },
                    ))
                })
                .into(),
        ));
        let Some(settings) = settings else {
            return card.into();
        };
        let direct = room.kind == RoomKind::Direct;
        if settings.can_change_access && !direct {
            // Space members only for a room in a space.
            let choices: Vec<Join> = [
                Access::Invite,
                Access::Knock,
                Access::SpaceMembers,
                Access::Public,
            ]
            .into_iter()
            .filter(|&access| {
                access != Access::SpaceMembers
                    || !shown.graph.parents(&room.id).is_empty()
                    || settings.access == access
            })
            .map(Join)
            .collect();
            card = card.push(settings::option(
                "Who can join",
                access_description(settings.access, &parent_names(room, shown.graph, rooms)),
                pick_list(choices, Some(Join(settings.access)), |Join(access)| {
                    app::Message::Info(Message::Access(access))
                })
                .text_size(13)
                .padding(6)
                .into(),
            ));
        }
        // Listing a room only others may join would only tease.
        if let Some(listed) = settings.listed
            && settings.can_change_access
            && (settings.access == Access::Public || listed)
        {
            card = card.push(settings::option(
                "Listed in the directory",
                format!(
                    "Found by people browsing the rooms of {}.",
                    shown.own_user.server_name()
                ),
                toggler(listed)
                    .on_toggle_maybe(enabled(true).then_some(move |on| change(Change::Listed(on))))
                    .into(),
            ));
        }
        for link in settings.spaces.iter().filter(|l| l.can_change) {
            let space = rooms
                .iter()
                .find(|r| r.id == link.space)
                .map_or("the space", |r| r.name.as_str());
            let id = link.space.clone();
            card = card.push(settings::option(
                format!("Suggested in {space}"),
                "Offered first to the space's members, and to people who join it.",
                toggler(link.suggested)
                    .on_toggle_maybe(
                        enabled(true)
                            .then_some(move |on| change(Change::Suggested(id.clone(), on))),
                    )
                    .into(),
            ));
        }
        if settings.can_change_history && !direct {
            card = card.push(settings::option(
                "Who can read the history",
                match settings.history {
                    History::Joined => "Members, from when they joined.",
                    History::Invited => "Members, from when they were invited.",
                    History::Members => "Members, all of it, also from before they joined.",
                    History::Anyone => "Anyone, without joining.",
                },
                pick_list(
                    Reading::ALL,
                    Some(Reading(settings.history)),
                    move |Reading(history)| change(Change::History(history)),
                )
                .text_size(13)
                .padding(6)
                .into(),
            ));
        }
        if settings.can_change_permissions && !direct {
            card = card.push(settings::option(
                "Who can edit",
                "Invite, rename, change the topic and picture, and pin messages.",
                pick_list(
                    Editing::ALL,
                    Some(Editing(settings.permissions)),
                    move |Editing(permissions)| change(Change::Permissions(permissions)),
                )
                .text_size(13)
                .padding(6)
                .into(),
            ));
        }
        if room.kind != RoomKind::Space {
            let control: Element<'a, app::Message> = if settings.encrypted {
                text("On").size(13).style(text::secondary).into()
            } else if settings.can_encrypt {
                button(text("Turn on…").size(13))
                    .padding([4, 10])
                    .style(style::secondary_button)
                    .on_press_maybe(enabled(true).then(|| {
                        app::Message::OpenDialog(Kind::Encrypt {
                            room: room.id.clone(),
                            name: room.name.clone(),
                        })
                    }))
                    .into()
            } else {
                text("Off").size(13).style(text::secondary).into()
            };
            card = card.push(settings::option(
                "Encryption",
                if settings.encrypted {
                    "Only members' devices can read the messages."
                } else {
                    "Messages are stored readable on the servers."
                },
                control,
            ));
        }
        card = card.push(self.address_view(room, settings, shown));
        card = card.push(self.spaces_view(room, settings, rooms));
        card = card.push(
            selectable_text(facts(room, settings))
                .size(13)
                .style(text::secondary),
        );
        if !settings.banned.is_empty() {
            card = card.push(banned_view(settings, enabled(settings.can_unban)));
        }
        if let Some(error) = &self.error {
            card = card.push(text(error).size(12).style(text::danger));
        }
        card.into()
    }

    /// The address, edited where we may (its name, on our server), and the
    /// link to share.
    fn address_view<'a>(
        &'a self,
        room: &RoomSummary,
        settings: &RoomSettings,
        shown: Shown<'a>,
    ) -> Element<'a, app::Message> {
        let address = settings
            .address
            .as_ref()
            .map(ToString::to_string)
            .unwrap_or_else(|| room.id.to_string());
        let link = self.copyable("Link", format!("https://matrix.to/#/{address}"));
        if !settings.can_change_address || room.kind == RoomKind::Direct {
            return column![
                self.copyable(
                    if settings.address.is_some() {
                        "Address"
                    } else {
                        "Room ID"
                    },
                    address,
                ),
                link,
            ]
            .spacing(14)
            .into();
        }
        let name = self.address.trim();
        let changed = name != address_name(settings);
        let mut line = row![
            text("#").size(13).font(iced::Font::MONOSPACE),
            text_input("name", &self.address)
                .size(13)
                .font(iced::Font::MONOSPACE)
                .padding([4, 8])
                .style(style::input)
                .on_input(|name| app::Message::Info(Message::AddressName(name)))
                .on_submit_maybe(changed.then(|| self.save_address())),
            text(format!(":{}", shown.own_user.server_name()))
                .size(13)
                .font(iced::Font::MONOSPACE),
        ]
        .spacing(4)
        .align_y(Alignment::Center);
        if changed {
            line = line.push(
                button(text("Save").size(13))
                    .padding([4, 10])
                    .style(style::primary_button)
                    .on_press_maybe((!self.saving).then(|| self.save_address())),
            );
        }
        column![
            labelled(
                "Address",
                column![
                    line,
                    text("People can join by it, and it names the room in links.")
                        .size(12)
                        .style(text::secondary),
                ]
                .spacing(4),
            ),
            link,
        ]
        .spacing(14)
        .into()
    }

    fn save_address(&self) -> app::Message {
        let name = self.address.trim();
        app::Message::Info(Message::Change(Change::Address(
            (!name.is_empty()).then(|| name.to_owned()),
        )))
    }

    /// The spaces the room is in, as chips taken out with ×, and a menu
    /// adding it to others: moving it is both.
    fn spaces_view<'a>(
        &'a self,
        room: &RoomSummary,
        settings: &'a RoomSettings,
        rooms: &'a [RoomSummary],
    ) -> Element<'a, app::Message> {
        let name_of = |id: &OwnedRoomId| {
            rooms
                .iter()
                .find(|r| &r.id == id)
                .map_or_else(|| id.to_string(), |r| r.name.clone())
        };
        let change = |change| app::Message::Info(Message::Change(change));
        let mut chips = row![].spacing(6).align_y(Alignment::Center);
        for link in &settings.spaces {
            let mut chip = row![text(name_of(&link.space)).size(13)]
                .spacing(2)
                .align_y(Alignment::Center);
            if link.can_change {
                chip = chip.push(hint(
                    button(icons::cross(9.0, directory::secondary))
                        .padding(4)
                        .style(style::list_item(false))
                        .on_press_maybe(
                            (!self.saving)
                                .then(|| change(Change::RemoveFromSpace(link.space.clone()))),
                        ),
                    "Take it out of this space",
                    Position::Bottom,
                ));
            }
            chips = chips.push(
                container(chip)
                    .padding(
                        iced::Padding::new(3.0)
                            .left(10.0)
                            .right(if link.can_change { 3.0 } else { 10.0 }),
                    )
                    .style(style::chip),
            );
        }
        let others: Vec<_> = settings
            .can_add_to
            .iter()
            .filter(|space| **space != room.id)
            .map(|space| (name_of(space), change(Change::AddToSpace(space.clone()))))
            .collect();
        if !others.is_empty() {
            let open = self.spaces_menu;
            chips = chips.push(
                popover(
                    button(text("Add to space…").size(13))
                        .padding([4, 10])
                        .style(style::text_button)
                        .on_press_maybe(
                            (!self.saving)
                                .then_some(app::Message::Info(Message::SpacesMenu(!open))),
                        ),
                    open.then(|| context_menu::menu(others)),
                    Side::Below,
                    Edge::Left,
                )
                .on_dismiss(app::Message::Info(Message::SpacesMenu(false))),
            );
        }
        if settings.spaces.is_empty() && settings.can_add_to.is_empty() {
            return space().into();
        }
        let label = if settings.spaces.is_empty() {
            "In no space"
        } else {
            "In"
        };
        labelled(label, chips.wrap())
    }

    /// The picture, which changes or removes it from a menu.
    fn picture_button<'a>(
        &self,
        picture: Element<'a, app::Message>,
        has_picture: bool,
    ) -> Element<'a, app::Message> {
        let badge = container(icons::pencil(10.0, |theme: &Theme| {
            theme.extended_palette().background.base.text
        }))
        .padding(3)
        .style(style::badge_circle);
        let shown = stack![
            picture,
            container(badge).align_right(AVATAR).align_bottom(AVATAR)
        ];
        let open = self.picture_menu;
        let button = button(shown)
            .padding(0)
            .style(style::list_item(open))
            .on_press(app::Message::Info(Message::PictureMenu(!open)));
        let mut items = vec![(
            "Change picture…".to_owned(),
            app::Message::Info(Message::ChangePicture),
        )];
        if has_picture {
            items.push((
                "Remove picture".to_owned(),
                app::Message::Info(Message::RemovePicture),
            ));
        }
        let menu = open.then(|| context_menu::menu(items));
        hint(
            popover(button, menu, Side::Below, Edge::Left)
                .on_dismiss(app::Message::Info(Message::PictureMenu(false))),
            "Change picture",
            Position::Bottom,
        )
    }

    /// The topic and its links in a box, edited there (`can_edit`): an
    /// Edit button in its corner opens the editor, saved with the header's
    /// Save.
    fn about<'a>(
        &'a self,
        room: &RoomSummary,
        services: &'a [CallService],
        can_edit: bool,
    ) -> Element<'a, app::Message> {
        let synced = room.topic.clone().unwrap_or_default();
        let content: Element<'a, app::Message> = if self.editing_topic && can_edit {
            column![
                (self.topic.view(services)).map(|edit| app::Message::Info(Message::Topic(edit))),
                row![
                    space::horizontal(),
                    button(text("Cancel").size(13))
                        .padding([4, 10])
                        .style(style::text_button)
                        .on_press(app::Message::Info(Message::CancelTopic(synced))),
                ],
            ]
            .spacing(8)
            .into()
        } else {
            let topic = Topic::parse(&synced);
            let empty = topic.text.trim().is_empty() && topic.links.is_empty();
            let details: Element<'a, app::Message> = if empty {
                text("No topic").size(13).style(text::secondary).into()
            } else {
                topic::details(&topic, services)
            };
            let mut line = row![container(details).width(Length::Fill)].spacing(12);
            if can_edit {
                line = line.push(
                    button(
                        row![
                            icons::pencil(12.0, |theme: &Theme| {
                                theme
                                    .extended_palette()
                                    .background
                                    .base
                                    .text
                                    .scale_alpha(0.6)
                            }),
                            text(if empty { "Add a topic" } else { "Edit" }).size(13)
                        ]
                        .spacing(5)
                        .align_y(Alignment::Center),
                    )
                    .padding([4, 10])
                    .style(style::secondary_button)
                    .on_press(app::Message::Info(Message::EditTopic)),
                );
            }
            line.into()
        };
        container(content)
            .width(Length::Fill)
            .padding(10)
            .style(style::section)
            .into()
    }

    /// A label, the value and a button copying it.
    fn copyable<'a>(&self, label: &'a str, value: String) -> Element<'a, app::Message> {
        let copied = self.copied.as_ref() == Some(&value);
        labelled(
            label,
            row![
                container(
                    selectable_text(value.clone())
                        .size(13)
                        .font(iced::Font::MONOSPACE)
                        .wrapping(text::Wrapping::Glyph)
                )
                .width(Length::Fill)
                .padding([4, 8])
                .style(style::code_block),
                hint(
                    button(if copied {
                        icons::check(16.0, |theme: &Theme| {
                            theme.extended_palette().success.base.color
                        })
                    } else {
                        icons::copy(16.0, |theme: &Theme| {
                            theme
                                .extended_palette()
                                .background
                                .base
                                .text
                                .scale_alpha(0.7)
                        })
                    })
                    .padding(6)
                    .style(style::list_item(false))
                    .on_press(app::Message::Info(Message::Copy(value))),
                    if copied { "Copied" } else { "Copy" },
                    Position::Left,
                ),
            ]
            .spacing(8)
            .align_y(Alignment::Center),
        )
    }
}

fn labelled<'a>(
    label: &'a str,
    content: impl Into<Element<'a, app::Message>>,
) -> Element<'a, app::Message> {
    column![
        text(label)
            .size(11)
            .font(fonts::SEMIBOLD)
            .style(text::secondary),
        content.into(),
    ]
    .spacing(4)
    .into()
}

/// Spaces, then rooms, shared with someone, all of them scrolling; each
/// opens.
fn shared_view<'a>(shared: &[OwnedRoomId], rooms: &'a [RoomSummary]) -> Element<'a, app::Message> {
    let mut known: Vec<&RoomSummary> = shared
        .iter()
        .filter_map(|id| rooms.iter().find(|r| &r.id == id))
        .collect();
    known.sort_by_key(|r| r.kind != RoomKind::Space);
    let title = format!("Shared spaces and rooms ({})", known.len());
    let mut list = column![].spacing(2);
    for room in known {
        let target = match room.kind {
            RoomKind::Space => app::Message::SelectSpace(Some(room.id.clone())),
            _ => app::Message::OpenRoom(room.id.clone()),
        };
        list = list.push(
            button(text(&room.name).size(13))
                .width(Length::Fill)
                .padding([3, 8])
                .style(style::list_item(false))
                .on_press(target),
        );
    }
    column![
        text(title)
            .size(11)
            .font(fonts::SEMIBOLD)
            .style(text::secondary),
        container(frame::scroll(list).height(Length::Shrink)).max_height(SHARED_HEIGHT),
    ]
    .spacing(4)
    .into()
}

fn header_button<'a>(
    label: &'a str,
    style: fn(&Theme, button::Status) -> button::Style,
    on_press: app::Message,
) -> Element<'a, app::Message> {
    button(text(label).size(13))
        .padding([4, 12])
        .style(style)
        .on_press(on_press)
        .into()
}

/// The name of the room's address (`#name:server`), or nothing.
fn address_name(settings: &RoomSettings) -> String {
    settings
        .address
        .as_ref()
        .map(|a| a.alias().to_owned())
        .unwrap_or_default()
}

/// A change as the card shows it until the settings come back.
fn apply(settings: &mut RoomSettings, change: &Change) {
    match change {
        Change::Address(_) | Change::AddToSpace(_) | Change::RemoveFromSpace(_) => {}
        Change::Listed(listed) => settings.listed = Some(*listed),
        Change::History(history) => settings.history = *history,
        Change::Permissions(permissions) => settings.permissions = *permissions,
        Change::Suggested(space, suggested) => {
            if let Some(link) = settings.spaces.iter_mut().find(|l| &l.space == space) {
                link.suggested = *suggested;
            }
        }
        Change::Unban(user) => settings.banned.retain(|b| &b.id != user),
    }
}

/// "Room · Public: anyone can join · 4 members · encrypted · version 12".
fn facts(room: &RoomSummary, settings: &RoomSettings) -> String {
    let mut facts = vec![
        match room.kind {
            RoomKind::Space => "Space",
            RoomKind::Direct => "Direct chat",
            RoomKind::Room => "Room",
        }
        .to_owned(),
    ];
    if room.kind != RoomKind::Direct {
        facts.push(
            match settings.access {
                Access::Public => "Public: anyone can join",
                Access::SpaceMembers => "Private: space members can join",
                Access::Knock => "Private: people ask to join",
                Access::Invite => "Private: by invitation",
            }
            .to_owned(),
        );
    }
    facts.push(match room.joined_members {
        1 => "1 member".to_owned(),
        n => format!("{n} members"),
    });
    if room.kind != RoomKind::Space {
        facts.push(
            if settings.encrypted {
                "encrypted"
            } else {
                "not encrypted"
            }
            .to_owned(),
        );
    }
    if let Some(version) = &settings.version {
        facts.push(format!("room version {version}"));
    }
    facts.join(" · ")
}

/// Who is banned and why, each to let back where we may (`can_unban`).
fn banned_view(settings: &RoomSettings, can_unban: bool) -> Element<'_, app::Message> {
    let rows = settings.banned.iter().map(|banned| {
        let mut who = column![text(banned.name.as_deref().unwrap_or(banned.id.as_str())).size(13)];
        let detail = match &banned.reason {
            Some(reason) => format!("{} · {reason}", banned.id),
            None => banned.id.to_string(),
        };
        who = who.push(selectable_text(detail).size(12).style(text::secondary));
        let mut line = row![who.width(Length::Fill)].align_y(Alignment::Center);
        if settings.can_unban {
            line = line.push(
                button(text("Unban").size(12))
                    .padding([3, 10])
                    .style(style::secondary_button)
                    .on_press_maybe(can_unban.then(|| {
                        app::Message::Info(Message::Change(Change::Unban(banned.id.clone())))
                    })),
            );
        }
        line.into()
    });
    labelled("Banned", column(rows).spacing(6))
}

/// The names of the joined spaces that list `room`.
fn parent_names<'a>(
    room: &RoomSummary,
    graph: &SpaceGraph,
    rooms: &'a [RoomSummary],
) -> Vec<&'a str> {
    (graph.parents(&room.id).iter())
        .filter_map(|p| rooms.iter().find(|r| &r.id == p))
        .map(|r| r.name.as_str())
        .collect()
}

fn access_description(access: Access, spaces: &[&str]) -> String {
    match access {
        Access::Invite => "Only people invited can join.".into(),
        Access::SpaceMembers if spaces.is_empty() => {
            "Members of the room's spaces can find and join it.".into()
        }
        Access::SpaceMembers => format!("Members of {} can find and join it.", spaces.join(", ")),
        Access::Public => "Anyone can find and join it.".into(),
        Access::Knock => "Anyone can ask to join; members let them in.".into(),
    }
}

/// A room's notifications, as its menu names them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Notify(NotifyMode);

impl Notify {
    const ALL: [Self; 3] = [
        Self(NotifyMode::All),
        Self(NotifyMode::Mentions),
        Self(NotifyMode::Mute),
    ];
}

impl fmt::Display for Notify {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self.0 {
            NotifyMode::All => "All messages",
            NotifyMode::Mentions => "Mentions",
            NotifyMode::Mute => "Mute",
        })
    }
}

/// Who can read a room's history, as its menu names them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Reading(History);

impl Reading {
    const ALL: [Self; 4] = [
        Self(History::Joined),
        Self(History::Invited),
        Self(History::Members),
        Self(History::Anyone),
    ];
}

impl fmt::Display for Reading {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self.0 {
            History::Joined => "Since joining",
            History::Invited => "Since invited",
            History::Members => "Members",
            History::Anyone => "Anyone",
        })
    }
}

/// Who can edit a room, as its menu names them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Editing(Permissions);

impl Editing {
    const ALL: [Self; 2] = [Self(Permissions::Everyone), Self(Permissions::Moderators)];
}

impl fmt::Display for Editing {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self.0 {
            Permissions::Everyone => "Everyone",
            Permissions::Moderators => "Moderators",
        })
    }
}

/// Who can join a room, as its menu names them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Join(Access);

impl fmt::Display for Join {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self.0 {
            Access::Invite => "Invited people",
            Access::SpaceMembers => "Space members",
            Access::Knock => "Ask to join",
            Access::Public => "Anyone",
        })
    }
}

/// A card target for a room in the list: the person for a direct chat.
pub fn target_of(room: &RoomSummary) -> Target {
    match (&room.direct_user, room.kind) {
        (Some(user), RoomKind::Direct) => Target::User(UserSummary {
            id: user.clone(),
            name: Some(room.name.clone()),
            avatar: room.avatar.clone(),
        }),
        _ => Target::Room(room.id.clone()),
    }
}

/// A card target for a message's sender.
pub fn sender(id: OwnedUserId, name: Option<String>, avatar: Option<OwnedMxcUri>) -> Target {
    Target::User(UserSummary { id, name, avatar })
}
