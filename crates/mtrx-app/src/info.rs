//! Cards: who someone is, with their address to copy; a room's settings.
//! Opened by clicking a name or from the room and member lists.

use std::{
    fmt,
    hash::{Hash, Hasher},
    time::SystemTime,
};

use iced::{
    Alignment, Element, Length, Subscription, Theme,
    futures::{StreamExt, stream},
    widget::{button, column, container, pick_list, row, space, stack, text, text_input, toggler},
};
use mtrx_core::{
    CallService, History, JoinRule, Membership, NotifyMode, OwnedRoomId, OwnedUserId, Permissions,
    RoomHandle, RoomKind, RoomLists, RoomSettings, RoomSummary, RoomTag, Topic, User, UserInfo,
};

use crate::{
    app, arrange,
    avatar::{Avatars, Shape},
    dialog::Kind,
    directory, fonts, frame,
    hint::{Position, hint},
    icons,
    menu::{self, Entry, Item, Width},
    overview::{self, Overview},
    popover::{Edge, Side, popover},
    presence::{self, Presences},
    selectable::text::selectable_text,
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
    pub lists: &'a RoomLists,
    pub avatars: &'a Avatars,
    pub own_user: &'a OwnedUserId,
    pub presences: &'a Presences,
    pub now: SystemTime,
    pub pinned_notes: bool,
    pub call_services: &'a [CallService],
    /// Rooms show their pictures, as the room list does.
    pub room_pictures: bool,
    /// The window's, which may be a menu of the card or of a member.
    pub popup: Option<&'a app::Popup>,
    /// The rooms of spaces as their servers listed them.
    pub space_rooms: &'a directory::SpaceRooms,
}

/// What to show a card for.
#[derive(Debug, Clone)]
pub enum Target {
    /// With the name and picture known so far.
    User(User),
    /// The room's settings.
    Room(OwnedRoomId),
}

#[derive(Debug, Clone)]
pub enum Message {
    UserLoaded(OwnedUserId, Result<UserInfo, String>),
    /// The room's settings, once read and after each change (those made
    /// here at once); or why they can't be read.
    Settings(OwnedRoomId, Result<RoomSettings, String>),
    Copy(String),
    Name(String),
    Topic(topic::Edit),
    /// Save the changed name and topic.
    SaveDetails,
    Notify(NotifyMode),
    JoinRule(JoinRule),
    ChangePicture,
    RemovePicture,
    /// A change was saved on the server, or not.
    Saved(Result<(), String>),
    /// Open the picture's menu.
    OpenPictureMenu,
    EditTopic,
    /// Stop editing the topic, back to this one.
    CancelTopic(String),
    /// The address's name, as edited.
    AddressName(String),
    Change(Change),
    /// Open the menu of spaces to add the room to.
    OpenSpacesMenu,
    /// Let every member invite and edit the space and the rooms in it.
    LetEveryoneEdit,
    /// What that changed, or why it failed.
    Opened(Result<String, String>),
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

/// A room whose settings a card follows, as a subscription's key: one per
/// room.
pub struct Followed(pub RoomHandle);

impl Hash for Followed {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.0.id().hash(state);
    }
}

impl Card {
    /// The room's settings, once read and after each change, while the
    /// card shows them.
    pub fn subscription(&self) -> Subscription<app::Message> {
        let Some(handle) = &self.handle else {
            return Subscription::none();
        };
        Subscription::run_with(Followed(handle.clone()), |room| {
            let (room, id) = (room.0.clone(), room.0.id().to_owned());
            stream::once(async move { room.settings().await })
                .flat_map(|settings| settings.updates())
                .map(move |settings| {
                    app::Message::Info(Message::Settings(id.clone(), Ok(settings)))
                })
        })
    }
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
    /// The room's, to follow and change its settings; set by the app.
    pub handle: Option<RoomHandle>,
    /// Spaces and rooms shared with the person, once loaded.
    shared: Option<Vec<OwnedRoomId>>,
    /// The room's settings, once read.
    pub settings: Option<RoomSettings>,
    /// The room's name and topic as edited.
    name: String,
    topic: topic::Editor,
    error: Option<String>,
    /// What letting everyone edit the space and its rooms changed.
    opened: Option<String>,
    /// The text last copied, to confirm it.
    copied: Option<String>,
    /// For a space: its rooms and members too.
    pub overview: Option<Overview>,
    editing_topic: bool,
    /// The address's name, as edited.
    address: String,
}

impl Card {
    /// A space's card opens on `tab`.
    pub fn new(target: Target, rooms: &[RoomSummary], tab: overview::Tab) -> Self {
        let room = match &target {
            Target::Room(id) => rooms.iter().find(|r| &r.id == id),
            Target::User(_) => None,
        };
        Self {
            editing_topic: false,
            address: String::new(),
            overview: room
                .filter(|r| r.kind == RoomKind::Space)
                .map(|r| Overview::new(r.id.clone(), tab)),
            name: room.map(|r| r.name.clone()).unwrap_or_default(),
            topic: topic::Editor::new(room.and_then(|r| r.topic.as_deref()).unwrap_or_default()),
            target,
            handle: None,
            shared: None,
            settings: None,
            error: None,
            opened: None,
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
            Message::Settings(id, Ok(settings)) => {
                if self.room() == Some(&id) {
                    // The address as shown, unless it is being edited.
                    let shown = self.settings.as_ref().map(address_name);
                    if shown.is_none_or(|shown| shown == self.address) {
                        self.address = address_name(&settings);
                    }
                    self.settings = Some(settings);
                }
            }
            Message::UserLoaded(_, Err(e)) | Message::Settings(_, Err(e)) => self.error = Some(e),
            Message::Copy(copied) => self.copied = Some(copied),
            Message::Name(name) => self.name = name,
            Message::Topic(edit) => self.topic.update(edit),
            Message::Notify(_) | Message::JoinRule(_) | Message::SaveDetails => self.error = None,
            Message::ChangePicture | Message::RemovePicture => self.error = None,
            Message::Saved(result) => {
                if result.is_ok() {
                    self.editing_topic = false;
                }
                self.error = result.err();
            }
            // The app's popup.
            Message::OpenPictureMenu | Message::OpenSpacesMenu => {}
            Message::AddressName(name) => self.address = name,
            Message::Change(_) => self.error = None,
            Message::LetEveryoneEdit => {
                self.opened = None;
                self.error = None;
            }
            Message::Opened(Ok(summary)) => self.opened = Some(summary),
            Message::Opened(Err(e)) => self.error = Some(e),
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
            Target::Room(id) => match shown.lists.room(id) {
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
                self.topic
                    .valid()
                    .then_some(app::Message::Info(Message::SaveDetails)),
            ))
        } else {
            (room.kind != RoomKind::Space && room.membership == Membership::Joined)
                .then(|| frame::primary("Open", Some(app::Message::OpenRoom(room.id.clone()))))
        }
    }

    /// Saves the changed name and topic (⌘↩), if any.
    pub fn submit(&self, rooms: &[RoomSummary]) -> Option<app::Message> {
        let room = rooms.iter().find(|r| Some(&r.id) == self.room())?;
        let (name, topic) = self.changed_details(room);
        ((name.is_some() || topic.is_some()) && self.topic.valid())
            .then_some(app::Message::Info(Message::SaveDetails))
    }

    fn user_view<'a>(&'a self, user: &'a User, shown: Shown<'a>) -> Framed<'a> {
        let Shown {
            lists,
            avatars,
            own_user,
            presences,
            now,
            pinned_notes,
            ..
        } = shown;
        let rooms: &'a [RoomSummary] = &lists.rooms;
        let presence = presences.get(&user.id);
        let name = user.display_name();
        // The star favourites the direct chat, made if there is none.
        let chat = rooms.iter().find(|r| {
            r.kind == RoomKind::Direct && r.direct_user.as_ref().map(|u| &u.id) == Some(&user.id)
        });
        let star = match chat {
            Some(chat) => sidebar::star_button(&chat.id, chat.tag == RoomTag::Favourite),
            None => sidebar::star_for(app::Message::FavouritePerson(user.id.clone()), false),
        };
        let picture = avatars.view(
            user.id.as_str(),
            &name,
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
        let rights = room.rights;
        let allowed = |can: bool| settings.is_some() && can;
        let star = sidebar::star_button(&room.id, room.tag == RoomTag::Favourite);
        let picture = avatars.view(
            room.id.as_str(),
            &room.name,
            room.avatar.as_deref(),
            AVATAR,
            Shape::Circle,
        );
        let picture = if allowed(rights.change_picture) {
            let open = matches!(shown.popup, Some(app::Popup::CardPicture));
            self.picture_button(picture, room.avatar.is_some(), open)
        } else {
            picture
        };
        // The name is edited where it is shown.
        let name: Element<'a, app::Message> = if allowed(rights.rename) {
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
        let about = self.about(room, call_services, allowed(rights.change_topic));
        let body = if loading {
            frame::loading()
        } else {
            self.settings_view(room, shown)
        };
        // Invite and Leave on every tab; Invite where the people are.
        let mut actions = Vec::new();
        let joined = room.membership == Membership::Joined;
        if joined {
            if rights.invite {
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
            body: overview.view(room, shown, rights.add_rooms, rights.invite && joined, body),
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
        let rooms: &'a [RoomSummary] = &shown.lists.rooms;
        let settings = self.settings.as_ref();
        let change = |change| app::Message::Info(Message::Change(change));
        let mut card = column![].spacing(14);
        if let Some(settings) = settings {
            card = card.push(settings::option(
                "Notifications",
                settings.notify.explanation(),
                pick_list(
                    NotifyMode::ALL.map(Notify),
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
        let rights = room.rights;
        if rights.change_join_rule && !direct {
            let choices: Vec<Join> = (settings.join_rule_choices.iter().copied())
                .map(Join)
                .collect();
            card = card.push(settings::option(
                "Who can join",
                settings.join_rule.fact(),
                pick_list(choices, Some(Join(settings.join_rule)), |Join(rule)| {
                    app::Message::Info(Message::JoinRule(rule))
                })
                .text_size(13)
                .padding(6)
                .into(),
            ));
        }
        // Listing a room only others may join would only tease.
        if let Some(listed) = settings.listed
            && rights.change_join_rule
            && (settings.join_rule == JoinRule::Public || listed)
        {
            card = card.push(settings::option(
                "Listed in the directory",
                format!(
                    "Found by people browsing the rooms of {}.",
                    shown.own_user.server_name()
                ),
                toggler(listed)
                    .on_toggle(move |on| change(Change::Listed(on)))
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
                    .on_toggle(move |on| change(Change::Suggested(id.clone(), on)))
                    .into(),
            ));
        }
        if rights.change_history && !direct {
            card = card.push(settings::option(
                "Who can read the history",
                settings.history.explanation(),
                pick_list(
                    History::ALL.map(Reading),
                    Some(Reading(settings.history)),
                    move |Reading(history)| change(Change::History(history)),
                )
                .text_size(13)
                .padding(6)
                .into(),
            ));
        }
        if rights.change_levels && !direct {
            card = card.push(settings::option(
                "Who can edit",
                settings.permissions.explanation(),
                pick_list(
                    Permissions::ALL.map(Editing),
                    Some(Editing(settings.permissions)),
                    move |Editing(permissions)| change(Change::Permissions(permissions)),
                )
                .text_size(13)
                .padding(6)
                .into(),
            ));
        }
        // A space is opened room by room, so offered even if it is open.
        if rights.change_levels && room.kind == RoomKind::Space {
            card = card.push(settings::option(
                "Its rooms",
                self.opened.as_deref().unwrap_or(
                    "Let everyone invite and edit this space and the rooms in it you have joined.",
                ),
                button(text("Let everyone edit").size(13))
                    .padding([4, 10])
                    .style(style::secondary_button)
                    .on_press(app::Message::Info(Message::LetEveryoneEdit))
                    .into(),
            ));
        }
        if room.kind != RoomKind::Space {
            let control: Element<'a, app::Message> = if settings.encrypted {
                text("On").size(13).style(text::secondary).into()
            } else if rights.encrypt {
                button(text("Turn on…").size(13))
                    .padding([4, 10])
                    .style(style::secondary_button)
                    .on_press(app::Message::OpenDialog(Kind::Encrypt {
                        room: room.id.clone(),
                        name: room.name.clone(),
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
        let adding = matches!(shown.popup, Some(app::Popup::CardAddToSpace));
        card = card.push(self.spaces_view(room, settings, rooms, adding));
        card = card.push(
            selectable_text(facts(room, settings))
                .size(13)
                .style(text::secondary),
        );
        if !settings.banned.is_empty() {
            card = card.push(banned_view(settings, rights.ban));
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
        if !room.rights.change_address || room.kind == RoomKind::Direct {
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
                    .on_press(self.save_address()),
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
    /// adding it to others, open while `adding`: moving it is both.
    fn spaces_view<'a>(
        &'a self,
        room: &RoomSummary,
        settings: &'a RoomSettings,
        rooms: &'a [RoomSummary],
        adding: bool,
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
                        .on_press(change(Change::RemoveFromSpace(link.space.clone()))),
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
        let others = add_to_space_entries(settings, &room.id, name_of);
        if !others.is_empty() {
            // While open, a click on it closes the menu as any outside click.
            chips = chips.push(
                popover(
                    button(text("Add to space…").size(13))
                        .padding([4, 10])
                        .style(style::text_button)
                        .on_press_maybe(
                            (!adding).then_some(app::Message::Info(Message::OpenSpacesMenu)),
                        ),
                    adding.then(|| menu::view(others, Width::Menu)),
                    Side::Below,
                    Edge::Left,
                )
                .on_dismiss(app::Message::ClosePopup),
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

    /// The picture, which changes or removes it from a menu, `open` or
    /// not.
    fn picture_button<'a>(
        &self,
        picture: Element<'a, app::Message>,
        has_picture: bool,
        open: bool,
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
        // While open, a click on it closes the menu as any outside click.
        let button = button(shown)
            .padding(0)
            .style(style::list_item(open))
            .on_press_maybe((!open).then_some(app::Message::Info(Message::OpenPictureMenu)));
        let menu = open.then(|| menu::view(picture_entries(has_picture), Width::Menu));
        hint(
            popover(button, menu, Side::Below, Edge::Left).on_dismiss(app::Message::ClosePopup),
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

/// The picture's menu: change it, and remove it if there is one.
fn picture_entries(has_picture: bool) -> Vec<Entry<'static, app::Message>> {
    let mut entries = vec![
        Item::new(
            "Change picture…",
            app::Message::Info(Message::ChangePicture),
        )
        .into(),
    ];
    if has_picture {
        entries
            .push(Item::new("Remove picture", app::Message::Info(Message::RemovePicture)).into());
    }
    entries
}

/// The spaces `room` may be added to, besides itself, named by `name_of`.
fn add_to_space_entries(
    settings: &RoomSettings,
    room: &OwnedRoomId,
    name_of: impl Fn(&OwnedRoomId) -> String,
) -> Vec<Entry<'static, app::Message>> {
    (settings.can_add_to.iter())
        .filter(|space| *space != room)
        .map(|space| {
            let add = Message::Change(Change::AddToSpace(space.clone()));
            Item::new(name_of(space), app::Message::Info(add)).into()
        })
        .collect()
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
        facts.push(settings.join_rule.fact().to_owned());
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

/// Who is banned and why, each to let back if we may (`can_unban`).
fn banned_view(settings: &RoomSettings, can_unban: bool) -> Element<'_, app::Message> {
    let rows = settings.banned.iter().map(|banned| {
        let mut who = column![text(banned.user.display_name()).size(13)];
        let detail = match &banned.reason {
            Some(reason) => format!("{} · {reason}", banned.user.id),
            None => banned.user.id.to_string(),
        };
        who = who.push(selectable_text(detail).size(12).style(text::secondary));
        let mut line = row![who.width(Length::Fill)].align_y(Alignment::Center);
        if can_unban {
            line = line.push(
                button(text("Unban").size(12))
                    .padding([3, 10])
                    .style(style::secondary_button)
                    .on_press(app::Message::Info(Message::Change(Change::Unban(
                        banned.user.id.clone(),
                    )))),
            );
        }
        line.into()
    });
    labelled("Banned", column(rows).spacing(6))
}

/// A room's notifications, as its menu names them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Notify(NotifyMode);

impl fmt::Display for Notify {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0.title())
    }
}

/// Who can read a room's history, as its menu names them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Reading(History);

impl fmt::Display for Reading {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0.title())
    }
}

/// Who can edit a room, as its menu names them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Editing(Permissions);

impl fmt::Display for Editing {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0.title())
    }
}

/// Who can join a room, as its menu names them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Join(JoinRule);

impl fmt::Display for Join {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0.title())
    }
}

/// A card target for a room in the list: the person for a direct chat.
pub fn target_of(room: &RoomSummary) -> Target {
    match (&room.direct_user, room.kind) {
        (Some(user), RoomKind::Direct) => Target::User(user.clone()),
        _ => Target::Room(room.id.clone()),
    }
}
