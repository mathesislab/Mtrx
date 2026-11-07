//! Dialogs for managing rooms: create a room, space, direct chat or
//! category, choose a category's rooms, invite, add rooms to a space,
//! leave. Joining has its own dialog ([`crate::join`]).

use std::time::{Duration, Instant};

use iced::{
    Alignment, Element, Length, Task,
    widget::{
        TextInput, button, checkbox, column, container, row, space, text, text_input, toggler,
    },
};
use mtrx_core::{
    Confirmation, Contact, Grouping, Member, Membership, NewRoom, OwnedRoomId, OwnedUserId,
    Permissions, RoomCategories, RoomKind, RoomLists, SpaceKey, User, UserId, a_role, typed_level,
};

use crate::{
    app,
    arrange::{self, Arrange, CategoryEdit},
    fonts, frame,
    hint::{Position, hint},
    spelling, style,
    task::Owned,
};

/// Which dialog is open.
#[derive(Clone, Debug)]
pub enum Kind {
    /// In `space` if given.
    NewRoom {
        space: Option<(OwnedRoomId, String)>,
    },
    NewSpace,
    DirectChat,
    Invite {
        room: OwnedRoomId,
        name: String,
    },
    AddExisting {
        space: OwnedRoomId,
        name: String,
    },
    Leave {
        room: OwnedRoomId,
        name: String,
    },
    /// Remove someone (`name`) from a room, or ban them.
    RemoveMember {
        room: OwnedRoomId,
        user: OwnedUserId,
        name: String,
        ban: bool,
    },
    /// A new power level for someone, or ourselves.
    Level(LevelChange),
    /// Encrypt a room's messages: not to be undone.
    Encrypt {
        room: OwnedRoomId,
        name: String,
    },
    SignOut,
    /// A new category of `space` (Home for `None`), with `room` in it if
    /// given, or the rooms of an existing one.
    Category {
        space: Option<OwnedRoomId>,
        category: Option<String>,
        room: Option<OwnedRoomId>,
    },
}

/// A power level to give someone, or ourselves: one chosen, asked first
/// as it can't be undone, or one entered.
#[derive(Clone, Debug)]
pub struct LevelChange {
    pub room: OwnedRoomId,
    pub member: Member,
    /// The level chosen and what to ask; `None`: entered in the dialog.
    pub asked: Option<(i64, Confirmation)>,
}

#[derive(Debug, Clone)]
pub enum Message {
    Name(String),
    Topic(String),
    /// Why someone is removed, for them to see.
    Reason(String),
    /// A power level, typed.
    Level(String),
    Encrypted(bool),
    Moderated(bool),
    /// The people search field changed.
    Query(String),
    /// People the user shares spaces, rooms or chats with.
    Contacts(Vec<Contact>),
    /// The server's user directory's answer to a query.
    Suggestions(String, Vec<User>),
    Pick(User),
    Unpick(usize),
    ToggleRoom(OwnedRoomId),
    /// Tick all of these rooms.
    TickAll(Vec<OwnedRoomId>),
    /// Whether the server gives new rooms co-owners.
    OwnersOffered(bool),
    /// An invited person co-owns the new room, or not.
    CoOwner(OwnedUserId, bool),
    Submit,
    Cancel,
    /// A request finished; on success, possibly with a room to show. Only
    /// the dialog that made it takes it.
    Done(RequestId, Result<Opened, String>),
}

/// A dialog's request, as the app numbers them.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct RequestId(u64);

impl RequestId {
    /// The one after this.
    pub fn next(self) -> Self {
        Self(self.0 + 1)
    }
}

/// What to show once a dialog's request succeeded.
#[derive(Debug, Clone)]
pub enum Opened {
    Nothing,
    Room(OwnedRoomId),
    Space(OwnedRoomId),
    Left(OwnedRoomId),
    /// A room's members changed.
    Members(OwnedRoomId),
    SignedOut,
}

/// What the app should do after a [`Message`].
pub enum Action {
    None,
    Close,
    /// Close, letting the running request finish in the background.
    Background(RequestId),
    Search(String),
    /// Send the request, and say so with [`Dialog::sent`].
    Submit(Request),
    Finished(Opened),
    /// Close, and change the room list's arrangement.
    Arrange(arrange::Message),
}

/// A request for the server, made by the app.
pub enum Request {
    CreateRoom(NewRoom),
    CreateSpace(NewRoom),
    DirectChat(OwnedUserId),
    Invite(OwnedRoomId, Vec<OwnedUserId>),
    AddToSpace(OwnedRoomId, Vec<OwnedRoomId>),
    Leave(OwnedRoomId),
    RemoveMember(OwnedRoomId, OwnedUserId, Option<String>, bool),
    SetLevel(OwnedRoomId, OwnedUserId, i64),
    Encrypt(OwnedRoomId),
    SignOut,
}

pub struct Dialog {
    kind: Kind,
    name: String,
    topic: String,
    reason: String,
    level: String,
    encrypted: bool,
    moderated: bool,
    query: String,
    /// People the user knows, closest first.
    contacts: Vec<Contact>,
    /// Spaces whose members are suggested first.
    near: Vec<OwnedRoomId>,
    /// Ourselves, offered first for a direct chat: notes to self.
    own: Option<User>,
    /// The server's user directory's results for the query.
    directory: Vec<User>,
    people: Vec<User>,
    rooms: Vec<OwnedRoomId>,
    /// Invited people who co-own the new room, where the server offers it.
    co_owners: Vec<OwnedUserId>,
    owners_offered: bool,
    /// The user search, replaced by each new query, and whether the
    /// server gives new rooms co-owners.
    tasks: Owned,
    /// The request running, and when it was sent.
    busy: Option<(RequestId, Instant)>,
    /// Refreshed by the app's tick, for the elapsed time.
    now: Instant,
    error: Option<String>,
}

/// After this long a request says it may continue in the background.
const SLOW: Duration = Duration::from_secs(15);
const WIDTH: f32 = 480.0;

impl Dialog {
    pub fn new(kind: Kind) -> Self {
        Self {
            kind,
            name: String::new(),
            topic: String::new(),
            reason: String::new(),
            level: String::new(),
            encrypted: true,
            moderated: false,
            query: String::new(),
            contacts: Vec::new(),
            near: Vec::new(),
            own: None,
            directory: Vec::new(),
            people: Vec::new(),
            rooms: Vec::new(),
            co_owners: Vec::new(),
            owners_offered: false,
            tasks: Owned::default(),
            busy: None,
            now: Instant::now(),
            error: None,
        }
    }

    /// Search the user directory with `search`.
    pub fn search_users<M: Send + 'static>(&mut self, search: Task<M>) -> Task<M> {
        self.tasks.run("search", search)
    }

    /// Ask whether the server gives new rooms co-owners, with `ask`.
    pub fn offer_owners<M: Send + 'static>(&mut self, ask: Task<M>) -> Task<M> {
        self.tasks.run("owners", ask)
    }

    /// It creates a room or space, which may get co-owners.
    pub fn creates_room(&self) -> bool {
        matches!(self.kind, Kind::NewRoom { .. } | Kind::NewSpace)
    }

    pub fn picks_people(&self) -> bool {
        matches!(
            self.kind,
            Kind::NewRoom { .. } | Kind::NewSpace | Kind::DirectChat | Kind::Invite { .. }
        )
    }

    /// Offer a chat with ourselves (notes to self) in a direct chat dialog.
    pub fn offer_self(&mut self, own: OwnedUserId) {
        if matches!(self.kind, Kind::DirectChat) {
            self.own = Some(User {
                id: own,
                name: Some("Notes to self".into()),
                avatar: None,
            });
        }
    }

    /// The name and rooms of the category being changed; the room to put
    /// into a new one.
    pub fn fill_category(&mut self, categories: &RoomCategories) {
        let Kind::Category {
            space,
            category,
            room,
        } = &self.kind
        else {
            return;
        };
        match category {
            Some(id) => {
                if let Some(category) = categories.layout(space.as_deref()).category(id) {
                    self.name = category.name.clone();
                    self.rooms = category.rooms.clone();
                }
            }
            None => self.rooms = room.iter().cloned().collect(),
        }
    }

    /// Suggest members of these spaces first.
    pub fn prefer_spaces(&mut self, spaces: Vec<OwnedRoomId>) {
        self.near = spaces;
    }

    /// People to suggest: those the user knows that match the query (the
    /// closest, and those in nearby spaces, first), then the server's
    /// directory results. Picked people are left out.
    fn suggestions(&self) -> (Vec<&User>, Vec<&User>) {
        const KNOWN: usize = 8;
        let query = self.query.trim().trim_start_matches('@').to_lowercase();
        let picked = |user: &User| self.people.iter().any(|p| p.id == user.id);
        let mut known: Vec<(bool, bool, &Contact)> = self
            .contacts
            .iter()
            .filter(|c| !picked(&c.user))
            .filter_map(|c| {
                let name = c.user.name.as_deref().unwrap_or_default().to_lowercase();
                let id = c.user.id.as_str()[1..].to_lowercase();
                let starts = name.starts_with(&query)
                    || name.split_whitespace().any(|w| w.starts_with(&query))
                    || id.starts_with(&query);
                (starts || name.contains(&query) || id.contains(&query)).then(|| {
                    let near = c.spaces.iter().any(|s| self.near.contains(s));
                    (!starts, !near, c)
                })
            })
            .collect();
        // Stable: within equal keys, the closest first.
        known.sort_by_key(|(later, far, _)| (*later, *far));
        let own = self.own.as_ref().filter(|own| {
            !picked(own)
                && ("notes to self".contains(&query)
                    || own.id.as_str()[1..].to_lowercase().contains(&query))
        });
        let known: Vec<&User> = own
            .into_iter()
            .chain(known.into_iter().take(KNOWN).map(|(_, _, c)| &c.user))
            .collect();
        let others = self
            .directory
            .iter()
            .filter(|u| !picked(u) && !known.iter().any(|k| k.id == u.id))
            .collect();
        (known, others)
    }

    /// A request is running.
    pub fn busy(&self) -> bool {
        self.busy.is_some()
    }

    /// Its request was sent, as `id`.
    pub fn sent(&mut self, id: RequestId) {
        self.busy = Some((id, Instant::now()));
        self.error = None;
    }

    pub fn tick(&mut self, now: Instant) {
        self.now = now;
    }

    /// Notices for when the request finishes in the background: on success,
    /// and the start of the message on failure.
    pub fn outcome(&self) -> (String, String) {
        let name = self.name.trim();
        let (done, failed) = match &self.kind {
            Kind::NewRoom { .. } => (
                format!("The room “{name}” is ready."),
                format!("Could not create the room “{name}”"),
            ),
            Kind::NewSpace => (
                format!("The space “{name}” is ready."),
                format!("Could not create the space “{name}”"),
            ),
            Kind::DirectChat => (
                "The direct chat is ready.".into(),
                "Could not start the direct chat".into(),
            ),
            Kind::Invite { name, .. } => (
                format!("Invites to {name} sent."),
                format!("Could not invite everyone to {name}"),
            ),
            Kind::AddExisting { name, .. } => (
                format!("Rooms added to {name}."),
                format!("Could not add all rooms to {name}"),
            ),
            Kind::RemoveMember { name, ban, .. } => (
                format!("{name} was {}.", if *ban { "banned" } else { "removed" }),
                format!("Could not {} {name}", if *ban { "ban" } else { "remove" }),
            ),
            Kind::Level(LevelChange { member, .. }) => {
                let role = self
                    .level()
                    .map(|l| a_role(l, member.manage.default_level))
                    .unwrap_or_default();
                let name = member.user.display_name();
                if member.own {
                    (
                        format!("You are {role} now."),
                        "Could not change your level".into(),
                    )
                } else {
                    (
                        format!("{name} is {role} now."),
                        format!("Could not change {name}'s level"),
                    )
                }
            }
            Kind::Leave { name, .. } => {
                (format!("Left {name}."), format!("Could not leave {name}"))
            }
            Kind::Encrypt { name, .. } => (
                format!("{name} is encrypted now."),
                format!("Could not encrypt {name}"),
            ),
            Kind::SignOut => ("Signed out.".into(), "Could not sign out".into()),
            // Saved without waiting, like other changes to the room list.
            Kind::Category { .. } => (String::new(), String::new()),
        };
        (done, failed)
    }

    /// What is happening while the request runs.
    fn progress(&self, since: Instant) -> Element<'_, app::Message> {
        let doing = match &self.kind {
            Kind::NewRoom { .. } => "Creating the room on the server…",
            Kind::NewSpace => "Creating the space on the server…",
            Kind::DirectChat => "Starting the chat…",
            Kind::Invite { .. } => "Sending the invites…",
            Kind::AddExisting { .. } => "Adding the rooms…",
            Kind::Leave { .. } => "Leaving…",
            Kind::RemoveMember { ban: true, .. } => "Banning…",
            Kind::RemoveMember { .. } => "Removing…",
            Kind::Level(_) => "Changing the level…",
            Kind::Encrypt { .. } => "Turning encryption on…",
            Kind::SignOut => "Signing out…",
            Kind::Category { .. } => "Saving…",
        };
        let elapsed = self.now.saturating_duration_since(since);
        let mut status = column![
            text(if elapsed.as_secs() >= 3 {
                format!("{doing} {} s", elapsed.as_secs())
            } else {
                doing.to_owned()
            })
            .size(13)
        ]
        .spacing(4);
        if elapsed >= SLOW {
            status = status.push(
                text(
                    "The server is taking long. You can close this; Mtrx tells you \
                     when it is done.",
                )
                .size(12)
                .style(text::secondary),
            );
        }
        status.into()
    }

    pub fn update(&mut self, message: Message) -> Action {
        match message {
            Message::Name(v) => self.name = v,
            Message::Topic(v) => self.topic = v,
            Message::Reason(v) => self.reason = v,
            Message::Level(v) => self.level = v,
            Message::Encrypted(v) => self.encrypted = v,
            Message::Moderated(v) => self.moderated = v,
            Message::Query(query) => {
                self.query = query.clone();
                self.tasks.cancel(&"search");
                self.directory.clear();
                if self.picks_people() && query.trim().trim_start_matches('@').len() >= 2 {
                    return Action::Search(query.trim().to_owned());
                }
            }
            Message::Contacts(contacts) => self.contacts = contacts,
            Message::Suggestions(query, users) => {
                if query == self.query.trim() {
                    self.directory = users;
                }
            }
            Message::Pick(user) => {
                if matches!(self.kind, Kind::DirectChat) {
                    self.people.clear();
                }
                if !self.people.iter().any(|p| p.id == user.id) {
                    self.people.push(user);
                }
                self.query.clear();
                self.directory.clear();
            }
            Message::Unpick(index) if index < self.people.len() => {
                let person = self.people.remove(index);
                self.co_owners.retain(|id| *id != person.id);
            }
            Message::OwnersOffered(offered) => self.owners_offered = offered,
            Message::CoOwner(user, on) => {
                self.co_owners.retain(|id| *id != user);
                if on {
                    self.co_owners.push(user);
                }
            }
            Message::Unpick(_) => {}
            Message::ToggleRoom(room) => match self.rooms.iter().position(|r| *r == room) {
                Some(index) => {
                    self.rooms.remove(index);
                }
                None => self.rooms.push(room),
            },
            Message::TickAll(rooms) => {
                for room in rooms {
                    if !self.rooms.contains(&room) {
                        self.rooms.push(room);
                    }
                }
            }
            Message::Submit if matches!(self.kind, Kind::Category { .. }) => {
                if let Some(edit) = self.category_edit() {
                    return Action::Arrange(arrange::Message::SaveCategory(edit));
                }
            }
            Message::Submit => {
                if let Some(request) = self.request().filter(|_| !self.busy()) {
                    return Action::Submit(request);
                }
            }
            Message::Cancel => match self.busy {
                Some((id, _)) => return Action::Background(id),
                None => return Action::Close,
            },
            // Another dialog's.
            Message::Done(id, _) if self.busy.map(|(running, _)| running) != Some(id) => {}
            Message::Done(_, Ok(opened)) => return Action::Finished(opened),
            Message::Done(_, Err(e)) => {
                self.busy = None;
                self.error = Some(e);
            }
        }
        Action::None
    }

    /// The request the dialog describes, if complete.
    fn request(&self) -> Option<Request> {
        let name = self.name.trim();
        let topic = self.topic.trim();
        let optional = |s: &str| (!s.is_empty()).then(|| s.to_owned());
        let named = optional(name);
        let people = || self.people.iter().map(|p| p.id.clone()).collect::<Vec<_>>();
        let permissions = if self.moderated {
            Permissions::Moderators
        } else {
            Permissions::Everyone
        };
        Some(match &self.kind {
            Kind::NewRoom { space } => Request::CreateRoom(NewRoom {
                name: Some(named.clone()?),
                topic: optional(topic),
                encrypted: self.encrypted,
                invite: people(),
                co_owners: self.co_owners.clone(),
                permissions,
                parent: space.as_ref().map(|(id, _)| id.clone()),
                ..Default::default()
            }),
            Kind::NewSpace => Request::CreateSpace(NewRoom {
                name: Some(named.clone()?),
                topic: optional(topic),
                space: true,
                invite: people(),
                co_owners: self.co_owners.clone(),
                permissions,
                ..Default::default()
            }),
            Kind::DirectChat => Request::DirectChat(self.person()?),
            Kind::Invite { room, .. } => {
                let mut people = people();
                if people.is_empty() {
                    people.push(self.person()?);
                }
                Request::Invite(room.clone(), people)
            }
            Kind::AddExisting { space, .. } if !self.rooms.is_empty() => {
                Request::AddToSpace(space.clone(), self.rooms.clone())
            }
            Kind::AddExisting { .. } => return None,
            Kind::Leave { room, .. } => Request::Leave(room.clone()),
            Kind::RemoveMember {
                room, user, ban, ..
            } => Request::RemoveMember(
                room.clone(),
                user.clone(),
                optional(self.reason.trim()),
                *ban,
            ),
            Kind::Encrypt { room, .. } => Request::Encrypt(room.clone()),
            Kind::Level(change) => Request::SetLevel(
                change.room.clone(),
                change.member.user.id.clone(),
                self.level()?,
            ),
            Kind::SignOut => Request::SignOut,
            // Not for the server: see `category_edit`.
            Kind::Category { .. } => return None,
        })
    }

    /// The level to give: the one asked about, or the one entered, if we
    /// may give it.
    fn level(&self) -> Option<i64> {
        let Kind::Level(change) = &self.kind else {
            return None;
        };
        match &change.asked {
            Some((level, _)) => Some(*level),
            None => typed_level(&self.level, change.member.manage.up_to?),
        }
    }

    /// The category the dialog describes, if named; rooms are optional.
    fn category_edit(&self) -> Option<CategoryEdit> {
        let Kind::Category {
            space, category, ..
        } = &self.kind
        else {
            return None;
        };
        let name = self.name.trim();
        (!name.is_empty()).then(|| CategoryEdit {
            space: space.clone(),
            category: category.clone(),
            name: name.to_owned(),
            rooms: self.rooms.clone(),
        })
    }

    /// The single person chosen, or a full user ID typed in.
    fn person(&self) -> Option<OwnedUserId> {
        self.people
            .first()
            .map(|p| p.id.clone())
            .or_else(|| OwnedUserId::try_from(self.query.trim()).ok())
    }

    /// `rooms` are the joined rooms, for adding existing ones to a space or
    /// putting them into a category (`arrange`; of those the room list lists,
    /// by `grouping`).
    pub fn view<'a>(
        &'a self,
        lists: &'a RoomLists,
        arrange: &'a Arrange,
        grouping: &Grouping,
    ) -> Element<'a, app::Message> {
        let (title, submit) = match &self.kind {
            Kind::NewRoom {
                space: Some((_, space)),
            } => (format!("New room in {space}"), "Create"),
            Kind::NewRoom { space: None } => ("New room".into(), "Create"),
            Kind::NewSpace => ("New space".into(), "Create"),
            Kind::DirectChat => ("New direct chat".into(), "Start chat"),
            Kind::Invite { name, .. } => (format!("Invite to {name}"), "Invite"),
            Kind::AddExisting { name, .. } => (format!("Add your rooms to {name}"), "Add"),
            Kind::Leave { name, .. } => (format!("Leave {name}?"), "Leave"),
            Kind::Encrypt { name, .. } => (format!("Encrypt {name}?"), "Encrypt"),
            Kind::RemoveMember {
                name, ban: false, ..
            } => (format!("Remove {name}?"), "Remove"),
            Kind::RemoveMember { name, .. } => (format!("Ban {name}?"), "Ban"),
            Kind::Level(change) => match &change.asked {
                Some((_, asked)) => (asked.title.clone(), asked.button.as_str()),
                None if change.member.own => ("Your power level".into(), "Set"),
                None => (
                    format!("Power level for {}", change.member.user.display_name()),
                    "Set",
                ),
            },
            Kind::SignOut => ("Sign out?".into(), "Sign out"),
            Kind::Category { category: None, .. } => ("New category".into(), "Create"),
            Kind::Category { .. } => ("Edit category".into(), "Save"),
        };
        let mut body = column![].spacing(12);
        body = match &self.kind {
            Kind::NewRoom { .. } | Kind::NewSpace => {
                let space = matches!(self.kind, Kind::NewSpace);
                let mut fields = column![
                    self.prose_field("Name", &self.name, Message::Name),
                    self.prose_field("Topic (optional)", &self.topic, Message::Topic),
                ]
                .spacing(8);
                if !space {
                    fields = fields.push(option(
                        "Encrypted",
                        "Only members can read messages. Cannot be turned off later.",
                        toggler(self.encrypted).on_toggle(|v| message(Message::Encrypted(v))),
                    ));
                }
                fields = fields.push(option(
                    "Only moderators can edit",
                    "Otherwise everyone can invite, rename and change the topic.",
                    toggler(self.moderated).on_toggle(|v| message(Message::Moderated(v))),
                ));
                body.push(fields)
                    .push(self.people_view("Invite people (optional)"))
            }
            Kind::DirectChat => body.push(self.people_view("Name or @user:server")),
            Kind::Invite { .. } => body.push(self.people_view("Name or @user:server")),
            Kind::AddExisting { space, .. } => body.push(self.rooms_view(space, lists)),
            Kind::Category { space, .. } => body
                .push(self.prose_field("Name", &self.name, Message::Name))
                .push(self.category_rooms_view(space.as_ref(), lists, arrange, grouping)),
            Kind::Encrypt { .. } => body.push(
                text(
                    "From now on, only members' devices can read new messages; servers and \
                     bridges can't. Encryption can't be turned off again.",
                )
                .size(13),
            ),
            Kind::Leave { .. } => body.push(
                text("You can only come back if the room is public or someone invites you.")
                    .size(13),
            ),
            Kind::RemoveMember { ban, .. } => body
                .push(
                    text(if *ban {
                        "They can't come back until someone lifts the ban."
                    } else {
                        "They can come back if the room lets them join or someone invites them."
                    })
                    .size(13),
                )
                .push(self.prose_field(
                    "Reason (optional, they see it)",
                    &self.reason,
                    Message::Reason,
                )),
            Kind::Level(change) => {
                let manage = &change.member.manage;
                if change.asked.is_none() {
                    body = body.push(
                        column![
                            text_input("Level", &self.level)
                                .on_input(|v| message(Message::Level(v)))
                                .on_submit(message(Message::Submit))
                                .padding(8)
                                .style(style::input),
                            text(format!(
                                "Admin 100 · Moderator 50 · Member {}{}",
                                manage.default_level,
                                match manage.up_to {
                                    Some(up_to) if up_to < 100 => format!(" · at most {up_to}"),
                                    _ => String::new(),
                                }
                            ))
                            .size(12)
                            .style(text::secondary),
                        ]
                        .spacing(6),
                    );
                }
                // What can't be undone about it, if anything.
                let warning = self
                    .level()
                    .and_then(|level| change.member.confirmation(level))
                    .map(|asked| asked.message)
                    .unwrap_or_default();
                body.push(text(warning).size(13))
            }
            Kind::SignOut => body.push(
                text(
                    "This device's data, including its message keys, is deleted. To read \
                     encrypted messages after signing in again, you need your recovery key \
                     or another signed-in device.",
                )
                .size(13),
            ),
        };
        if let Some((_, since)) = self.busy {
            body = body.push(self.progress(since));
        }
        if let Some(error) = &self.error {
            body = body.push(text(error).size(12).style(text::danger));
        }
        let ready = (self.request().is_some() || self.category_edit().is_some()) && !self.busy();
        let submit_message = ready.then(|| message(Message::Submit));
        // A confirmation is short: its answer below the question, as in
        // system alerts. A form's action stays in the header.
        let action = if matches!(
            self.kind,
            Kind::Leave { .. }
                | Kind::Encrypt { .. }
                | Kind::SignOut
                | Kind::RemoveMember { .. }
                | Kind::Level(LevelChange { asked: Some(_), .. })
        ) {
            body = body.push(
                row![
                    space::horizontal(),
                    button(text("Cancel"))
                        .style(style::text_button)
                        .on_press(message(Message::Cancel)),
                    button(text(submit))
                        .style(style::danger_button)
                        .on_press_maybe(submit_message),
                ]
                .spacing(8),
            );
            None
        } else {
            Some(frame::primary(submit, submit_message))
        };
        frame::frame(
            frame::title(title),
            action,
            message(Message::Cancel),
            body,
            WIDTH,
        )
    }

    fn field<'a>(
        &self,
        placeholder: &'a str,
        value: &'a str,
        on_input: fn(String) -> Message,
    ) -> TextInput<'a, app::Message> {
        text_input(placeholder, value)
            .on_input(move |v| message(on_input(v)))
            .on_submit(message(Message::Submit))
            .style(style::input)
            .padding(8)
    }

    /// A [`Self::field`] for prose, its misspelled words marked.
    fn prose_field<'a>(
        &self,
        placeholder: &'a str,
        value: &'a str,
        on_input: fn(String) -> Message,
    ) -> TextInput<'a, app::Message> {
        spelling::checked(self.field(placeholder, value, on_input), value)
    }

    /// Chosen people, the search field and its suggestions.
    fn people_view<'a>(&'a self, placeholder: &'a str) -> Element<'a, app::Message> {
        let mut section = column![].spacing(6);
        if !self.people.is_empty() {
            // A new room's invited people may co-own it.
            let owners = self.owners_offered && self.creates_room();
            let chips = self.people.iter().enumerate().map(|(i, person)| {
                let name = button(text(format!("{} ×", person.display_name())).size(13))
                    .padding([3, 8])
                    .style(style::secondary_button)
                    .on_press(message(Message::Unpick(i)));
                if !owners {
                    return name.into();
                }
                let id = person.id.clone();
                let owner = hint(
                    checkbox(self.co_owners.contains(&person.id))
                        .label("Co-owner")
                        .text_size(12)
                        .size(14)
                        .on_toggle(move |on| message(Message::CoOwner(id.clone(), on))),
                    "Owns the room as you do: nobody can demote them, and they can't step down.",
                    Position::Bottom,
                );
                row![name, owner]
                    .spacing(6)
                    .align_y(Alignment::Center)
                    .into()
            });
            section = section.push(row(chips).spacing(12).wrap());
        }
        let single = matches!(self.kind, Kind::DirectChat) && !self.people.is_empty();
        if !single {
            section = section.push(self.field(placeholder, &self.query, Message::Query));
        }
        if single {
            return section.into();
        }
        let (known, others) = self.suggestions();
        let mut list = column![].spacing(2);
        if !known.is_empty() {
            list = list.push(list_heading("People you know"));
            for user in known {
                list = list.push(person(user));
            }
        }
        if !others.is_empty() {
            list = list.push(list_heading("Others on the server"));
            for user in others {
                list = list.push(person(user));
            }
        }
        // A full user ID can be picked even if no one by it was found.
        let query = self.query.trim();
        let listed = |id: &UserId| {
            self.people.iter().any(|p| p.id == id)
                || self.contacts.iter().any(|c| c.user.id == id)
                || self.directory.iter().any(|u| u.id == id)
                || self.own.as_ref().is_some_and(|o| o.id == id)
        };
        if let Ok(id) = UserId::parse(query)
            && !listed(&id)
        {
            list = list.push(person(&User::from(id)));
        }
        section = section.push(frame::scroll(list).height(Length::Shrink));
        section.into()
    }

    /// Joined rooms (not direct chats) not yet in `space` that the search
    /// finds, to tick.
    fn rooms_view<'a>(
        &'a self,
        space: &OwnedRoomId,
        lists: &'a RoomLists,
    ) -> Element<'a, app::Message> {
        let candidates: Vec<_> = (lists.rooms.iter())
            .filter(|r| {
                r.membership == Membership::Joined
                    && r.kind != RoomKind::Direct
                    && r.id != *space
                    && !lists.graph.parents(&r.id).contains(space)
            })
            .collect();
        if candidates.is_empty() {
            return text("All your rooms are already in this space.")
                .size(13)
                .into();
        }
        let query = self.query.trim().to_lowercase();
        let found: Vec<_> = candidates
            .into_iter()
            .filter(|r| r.name.to_lowercase().contains(&query))
            .collect();
        let chosen = match self.rooms.len() {
            0 => "Rooms you're in that aren't in this space yet.".to_owned(),
            1 => "1 room chosen".to_owned(),
            n => format!("{n} rooms chosen"),
        };
        let list = found.into_iter().map(|room| {
            let toggle = message(Message::ToggleRoom(room.id.clone()));
            let mut line = row![
                checkbox(self.rooms.contains(&room.id)).on_toggle({
                    let toggle = toggle.clone();
                    move |_| toggle.clone()
                }),
                text(&room.name).width(Length::Fill),
            ]
            .spacing(8)
            .align_y(Alignment::Center);
            if room.kind == RoomKind::Space {
                line = line.push(text("space").size(12).style(text::secondary));
            }
            button(line)
                .width(Length::Fill)
                .padding([4, 8])
                .style(style::list_item(false))
                .on_press(toggle)
                .into()
        });
        column![
            self.field("Search your rooms", &self.query, Message::Query),
            text(chosen).size(12).style(text::secondary),
            frame::scroll(column(list).spacing(2)).height(300),
        ]
        .spacing(8)
        .into()
    }
}

impl Dialog {
    /// The rooms and chats `space` lists that the search finds, to tick,
    /// with the category each is in now.
    fn category_rooms_view<'a>(
        &'a self,
        space: Option<&OwnedRoomId>,
        lists: &'a RoomLists,
        arrange: &'a Arrange,
        grouping: &Grouping,
    ) -> Element<'a, app::Message> {
        let layout = arrange.categories.layout(space.map(|s| s.as_ref()));
        let editing = match &self.kind {
            Kind::Category { category, .. } => category.as_deref(),
            _ => None,
        };
        let key = space.map_or(SpaceKey::Home, |space| SpaceKey::Space(space.clone()));
        let found: Vec<_> = (lists.search(Some(&key), grouping, &self.query).into_iter())
            .filter(|r| r.membership == Membership::Joined)
            .collect();
        let unticked: Vec<OwnedRoomId> = (found.iter())
            .filter(|r| !self.rooms.contains(&r.id))
            .map(|r| r.id.clone())
            .collect();
        let chosen = match self.rooms.len() {
            0 => "No rooms chosen; you can also drag them in later.".to_owned(),
            1 => "1 room chosen".to_owned(),
            n => format!("{n} rooms chosen"),
        };
        let tick_all = button(text("Choose all shown").size(13))
            .padding([3, 8])
            .style(style::secondary_button)
            .on_press_maybe((!unticked.is_empty()).then(|| message(Message::TickAll(unticked))));
        let list = found.into_iter().map(|room| {
            let ticked = self.rooms.contains(&room.id);
            let toggle = message(Message::ToggleRoom(room.id.clone()));
            let mut line = row![
                checkbox(ticked).on_toggle({
                    let toggle = toggle.clone();
                    move |_| toggle.clone()
                }),
                text(&room.name).width(Length::Fill),
            ]
            .spacing(8)
            .align_y(Alignment::Center);
            // Ticking moves a room out of the category it is in.
            if let Some(other) = layout
                .category_of(&room.id)
                .filter(|c| Some(c.id.as_str()) != editing)
            {
                line = line.push(
                    text(format!("in {}", other.name))
                        .size(12)
                        .style(text::secondary),
                );
            }
            button(line)
                .width(Length::Fill)
                .padding([4, 8])
                .style(style::list_item(false))
                .on_press(toggle)
                .into()
        });
        column![
            self.field("Search rooms and chats", &self.query, Message::Query),
            row![
                text(chosen)
                    .size(12)
                    .style(text::secondary)
                    .width(Length::Fill),
                tick_all,
            ]
            .spacing(8)
            .align_y(Alignment::Center),
            frame::scroll(column(list).spacing(2)).height(260),
        ]
        .spacing(8)
        .into()
    }
}

fn list_heading<'a>(heading: &'a str) -> Element<'a, app::Message> {
    container(
        text(heading)
            .size(11)
            .font(fonts::SEMIBOLD)
            .style(text::secondary),
    )
    .padding([4, 8])
    .into()
}

/// A suggested person; picks them.
fn person<'a>(user: &User) -> Element<'a, app::Message> {
    button(
        column![
            text(user.display_name()),
            text(user.id.to_string()).size(12).style(text::secondary),
        ]
        .spacing(1),
    )
    .width(Length::Fill)
    .padding([4, 8])
    .style(style::list_item(false))
    .on_press(message(Message::Pick(user.clone())))
    .into()
}

fn option<'a>(
    label: &'a str,
    description: &'a str,
    toggle: impl Into<Element<'a, app::Message>>,
) -> Element<'a, app::Message> {
    row![
        column![
            text(label),
            text(description).size(12).style(text::secondary)
        ]
        .spacing(2)
        .width(Length::Fill),
        toggle.into(),
    ]
    .spacing(16)
    .align_y(Alignment::Center)
    .into()
}

fn message(message: Message) -> app::Message {
    app::Message::Dialog(message)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn room(id: &str) -> OwnedRoomId {
        format!("!{id}:x").try_into().unwrap()
    }

    fn saved(action: Action) -> CategoryEdit {
        match action {
            Action::Arrange(arrange::Message::SaveCategory(edit)) => edit,
            _ => panic!("not saved"),
        }
    }

    #[test]
    fn a_category_needs_a_name_only() {
        let mut dialog = Dialog::new(Kind::Category {
            space: None,
            category: None,
            room: Some(room("seminar")),
        });
        dialog.fill_category(&RoomCategories::default());
        // The search is local: no user directory.
        assert!(matches!(
            dialog.update(Message::Query("pap".into())),
            Action::None
        ));
        assert!(matches!(dialog.update(Message::Submit), Action::None));
        dialog.update(Message::Name(" Papers ".into()));
        dialog.update(Message::TickAll(vec![room("paper-a"), room("seminar")]));
        dialog.update(Message::ToggleRoom(room("seminar")));
        let edit = saved(dialog.update(Message::Submit));
        assert_eq!(edit.name, "Papers");
        assert_eq!(edit.category, None);
        assert_eq!(edit.rooms, [room("paper-a")]);
    }

    #[test]
    fn editing_starts_from_the_category() {
        let mut categories = RoomCategories::default();
        let id = categories.home.add_category("Teaching");
        categories.home.place(&room("seminar"), Some(&id));
        let mut dialog = Dialog::new(Kind::Category {
            space: None,
            category: Some(id.clone()),
            room: None,
        });
        dialog.fill_category(&categories);
        // No rooms is fine too.
        dialog.update(Message::ToggleRoom(room("seminar")));
        let edit = saved(dialog.update(Message::Submit));
        assert_eq!(edit.name, "Teaching");
        assert_eq!(edit.category, Some(id));
        assert!(edit.rooms.is_empty());
    }
}
