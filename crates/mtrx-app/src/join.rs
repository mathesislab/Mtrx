//! Join a space or room from elsewhere: browse a server's public directory
//! (ours, one browsed before, or another), search it, or paste an address
//! or link to see that one room.

use std::{collections::HashSet, fmt};

use iced::{
    Alignment, Element, Length,
    task::Handle,
    widget::{button, column, container, pick_list, row, text, text_input},
};
use mtrx_core::{
    DirectoryKind, DirectoryPage, DirectoryRoom, OwnedRoomId, OwnedRoomOrAliasId, OwnedServerName,
    RoomLists, parse_address,
};

use crate::{app, avatar::Avatars, directory, frame, style};

const WIDTH: f32 = 560.0;

/// A server to browse, as its menu lists them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Server {
    Named(OwnedServerName),
    /// Type another server's name.
    Other,
}

impl fmt::Display for Server {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Named(name) => f.write_str(name.as_str()),
            Self::Other => f.write_str("Other server…"),
        }
    }
}

#[derive(Debug, Clone)]
pub enum Message {
    Server(Server),
    /// The other server's name, as typed.
    OtherName(String),
    /// Browse the typed server.
    Browse,
    Query(String),
    Kind(DirectoryKind),
    /// A page of the directory, for the search it answers.
    Found(Search, Result<DirectoryPage, String>),
    More,
    /// The room an address or link names, for that address.
    Previewed(String, Result<DirectoryRoom, String>),
    /// Join a room, or a space (`true`).
    Join(OwnedRoomOrAliasId, Vec<OwnedServerName>, bool),
    Knock(OwnedRoomOrAliasId, Vec<OwnedServerName>),
    Done(OwnedRoomOrAliasId, Result<Joined, String>),
    Close,
}

/// What joining did.
#[derive(Debug, Clone)]
pub enum Joined {
    Room,
    Space(OwnedRoomId),
    Asked,
}

/// A directory search: on `server` (ours if `None`), from `page`.
#[derive(Clone, Debug, PartialEq)]
pub struct Search {
    pub server: Option<OwnedServerName>,
    pub query: String,
    pub kind: DirectoryKind,
    pub page: Option<String>,
}

/// What the app should do after a [`Message`].
pub enum Action {
    None,
    Close,
    Search(Search),
    Preview(OwnedRoomOrAliasId, Vec<OwnedServerName>),
    Join(OwnedRoomOrAliasId, Vec<OwnedServerName>, bool),
    Knock(OwnedRoomOrAliasId, Vec<OwnedServerName>),
    /// Joined a space: show it.
    ShowSpace(OwnedRoomId),
}

pub struct Join {
    /// Ours first, then those browsed before.
    servers: Vec<OwnedServerName>,
    server: Server,
    other: String,
    query: String,
    kind: DirectoryKind,
    /// The search shown, and its results so far.
    search: Option<Search>,
    rooms: Vec<DirectoryRoom>,
    next: Option<String>,
    loading: bool,
    error: Option<String>,
    preview: Option<(String, Option<Result<DirectoryRoom, String>>)>,
    /// Joining or asking to join, by the ID or address used.
    busy: HashSet<String>,
    /// The running search or preview; replaced by each new one.
    pub request: Option<Handle>,
}

impl Join {
    /// `own` is our server; `browsed` the servers browsed before, most
    /// recent first.
    pub fn new(own: OwnedServerName, browsed: &[String]) -> (Self, Action) {
        let mut servers = vec![own.clone()];
        servers.extend(
            browsed
                .iter()
                .filter_map(|s| OwnedServerName::try_from(s.as_str()).ok())
                .filter(|s| *s != own),
        );
        let mut join = Self {
            servers,
            server: Server::Named(own),
            other: String::new(),
            query: String::new(),
            kind: DirectoryKind::All,
            search: None,
            rooms: Vec::new(),
            next: None,
            loading: false,
            error: None,
            preview: None,
            busy: HashSet::new(),
            request: None,
        };
        let action = join.find();
        (join, action)
    }

    /// The server browsed: `None` for ours.
    fn browsed(&self) -> Option<OwnedServerName> {
        match &self.server {
            Server::Named(name) if *name == self.servers[0] => None,
            Server::Named(name) => Some(name.clone()),
            Server::Other => None,
        }
    }

    /// Look up what the query names: the room of an address or link, else
    /// the directory's matches.
    fn find(&mut self) -> Action {
        self.error = None;
        if let Some((address, via)) = parse_address(&self.query) {
            self.preview = Some((self.query.trim().to_owned(), None));
            return Action::Preview(address, via);
        }
        self.preview = None;
        if self.server == Server::Other {
            return Action::None;
        }
        let search = Search {
            server: self.browsed(),
            query: self.query.trim().to_owned(),
            kind: self.kind,
            page: None,
        };
        self.search = Some(search.clone());
        self.rooms.clear();
        self.next = None;
        self.loading = true;
        Action::Search(search)
    }

    pub fn update(&mut self, message: Message) -> Action {
        match message {
            Message::Server(server) => {
                self.server = server;
                if self.server == Server::Other {
                    self.rooms.clear();
                    self.next = None;
                    self.search = None;
                    self.loading = false;
                    self.error = None;
                    return Action::None;
                }
                return self.find();
            }
            Message::OtherName(name) => self.other = name,
            Message::Browse => {
                let name = self.other.trim().trim_start_matches("https://");
                match OwnedServerName::try_from(name) {
                    Ok(name) => {
                        if !self.servers.contains(&name) {
                            self.servers.push(name.clone());
                        }
                        self.server = Server::Named(name);
                        self.other.clear();
                        return self.find();
                    }
                    Err(_) => self.error = Some(format!("“{name}” is not a server name.")),
                }
            }
            Message::Query(query) => {
                self.query = query;
                return self.find();
            }
            Message::Kind(kind) => {
                self.kind = kind;
                return self.find();
            }
            Message::Found(search, result) => {
                if self.search.as_ref().is_some_and(|s| {
                    s.server == search.server && s.query == search.query && s.kind == search.kind
                }) {
                    self.loading = false;
                    match result {
                        Ok(page) => {
                            self.rooms.extend(page.rooms);
                            self.next = page.next;
                        }
                        Err(e) => self.error = Some(self.refused(&search, e)),
                    }
                }
            }
            Message::More => {
                if let (Some(search), Some(next)) = (&self.search, &self.next)
                    && !self.loading
                {
                    self.loading = true;
                    return Action::Search(Search {
                        page: Some(next.clone()),
                        ..search.clone()
                    });
                }
            }
            Message::Previewed(address, result) => {
                if let Some((shown, preview)) = &mut self.preview
                    && *shown == address
                {
                    *preview = Some(result);
                }
            }
            Message::Join(room, via, space) => {
                self.error = None;
                self.busy.insert(room.to_string());
                return Action::Join(room, via, space);
            }
            Message::Knock(room, via) => {
                self.error = None;
                self.busy.insert(room.to_string());
                return Action::Knock(room, via);
            }
            Message::Done(room, result) => {
                self.busy.remove(room.as_str());
                match result {
                    Ok(Joined::Space(id)) => return Action::ShowSpace(id),
                    Ok(Joined::Room | Joined::Asked) => {}
                    Err(e) => self.error = Some(e),
                }
            }
            Message::Close => return Action::Close,
        }
        Action::None
    }

    /// Why a search failed, saying so plainly when another server keeps
    /// its directory to itself.
    fn refused(&self, search: &Search, error: String) -> String {
        match &search.server {
            Some(server) => format!(
                "{server} doesn't share its room list ({error}). Type an address you know, \
                 such as #room:{server}."
            ),
            None => error,
        }
    }

    /// `pictures`: rooms show their pictures, as the room list does.
    pub fn view<'a>(
        &'a self,
        lists: &'a RoomLists,
        avatars: &'a Avatars,
        pictures: bool,
    ) -> Element<'a, app::Message> {
        let mut servers: Vec<Server> = self.servers.iter().cloned().map(Server::Named).collect();
        servers.push(Server::Other);
        let mut body = column![
            row![
                pick_list(servers, Some(self.server.clone()), |s| message(
                    Message::Server(s)
                ))
                .text_size(13)
                .padding([6, 8]),
                text_input("Search, or paste #room:server or a link", &self.query)
                    .id(SEARCH)
                    .on_input(|q| message(Message::Query(q)))
                    .style(style::input)
                    .padding([6, 8])
                    .size(13)
                    .width(Length::Fill),
            ]
            .spacing(8)
            .align_y(Alignment::Center)
        ]
        .spacing(12);
        if self.server == Server::Other {
            body = body.push(
                row![
                    text_input("Server, such as uni-hamburg.de", &self.other)
                        .on_input(|n| message(Message::OtherName(n)))
                        .on_submit(message(Message::Browse))
                        .style(style::input)
                        .padding([6, 8])
                        .size(13)
                        .width(Length::Fill),
                    button(text("Browse").size(13))
                        .padding([5, 12])
                        .style(style::secondary_button)
                        .on_press_maybe(
                            (!self.other.trim().is_empty()).then(|| message(Message::Browse))
                        ),
                ]
                .spacing(8)
                .align_y(Alignment::Center),
            );
        }
        if let Some((_, preview)) = &self.preview {
            body = body.push(match preview {
                None => quiet("Looking it up…"),
                Some(Err(e)) => text(format!("Could not find it: {e}"))
                    .size(12)
                    .style(text::danger)
                    .into(),
                Some(Ok(room)) => self.entry(room, lists, avatars, pictures),
            });
        } else if self.server != Server::Other {
            body = body.push(directory::tabs(
                &[
                    ("All", DirectoryKind::All),
                    ("Spaces", DirectoryKind::Spaces),
                    ("Rooms", DirectoryKind::Rooms),
                ],
                self.kind,
                |kind| message(Message::Kind(kind)),
            ));
            let mut list = column![].spacing(2);
            for room in &self.rooms {
                list = list.push(self.entry(room, lists, avatars, pictures));
            }
            if self.loading {
                list = list.push(quiet("Looking…"));
            } else if self.rooms.is_empty() && self.error.is_none() {
                list = list.push(quiet("Nothing found."));
            } else if self.next.is_some() {
                list = list.push(
                    button(text("More").size(13))
                        .padding([4, 10])
                        .style(style::text_button)
                        .on_press(message(Message::More)),
                );
            }
            body = body.push(frame::scroll(list).height(Length::Shrink));
        }
        if let Some(error) = &self.error {
            body = body.push(text(error).size(12).style(text::danger));
        }
        frame::frame(
            frame::title("Join a space or room"),
            [],
            message(Message::Close),
            body,
            WIDTH,
        )
    }

    fn entry<'a>(
        &'a self,
        room: &'a DirectoryRoom,
        lists: &'a RoomLists,
        avatars: &'a Avatars,
        pictures: bool,
    ) -> Element<'a, app::Message> {
        let address: OwnedRoomOrAliasId = room.id.clone().into();
        directory::entry(
            room,
            lists.membership(&room.id),
            self.busy.contains(address.as_str()),
            container(text("")).into(),
            directory::Shown { avatars, pictures },
            message(Message::Join(
                address.clone(),
                room.via.clone(),
                room.is_space,
            )),
            message(Message::Knock(address, room.via.clone())),
        )
    }
}

/// The search field, focused when the dialog opens.
pub const SEARCH: &str = "join-search";

fn quiet<'a>(label: &'a str) -> Element<'a, app::Message> {
    container(text(label).size(12).style(text::secondary))
        .padding([4, 4])
        .into()
}

fn message(message: Message) -> app::Message {
    app::Message::Join(message)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn server(name: &str) -> OwnedServerName {
        name.try_into().unwrap()
    }

    fn opened() -> Join {
        let (join, action) = Join::new(server("ours.org"), &["ours.org".into(), "awi.de".into()]);
        assert!(matches!(
            action,
            Action::Search(Search { server: None, .. })
        ));
        assert_eq!(join.servers, [server("ours.org"), server("awi.de")]);
        join
    }

    #[test]
    fn an_address_is_looked_up_not_searched() {
        let mut join = opened();
        let action = join.update(Message::Query("https://matrix.to/#/#ice:awi.de".into()));
        assert!(matches!(action, Action::Preview(address, _) if address.as_str() == "#ice:awi.de"));
        assert!(matches!(
            join.update(Message::Query("ice".into())),
            Action::Search(Search { query, .. }) if query == "ice"
        ));
    }

    #[test]
    fn another_server_is_browsed_once_named() {
        let mut join = opened();
        assert!(matches!(
            join.update(Message::Server(Server::Other)),
            Action::None
        ));
        join.update(Message::OtherName("not a server".into()));
        assert!(matches!(join.update(Message::Browse), Action::None));
        assert!(join.error.is_some());
        join.update(Message::OtherName("uni-hamburg.de".into()));
        assert!(matches!(
            join.update(Message::Browse),
            Action::Search(Search { server: Some(s), .. }) if s == "uni-hamburg.de"
        ));
        assert!(join.servers.contains(&server("uni-hamburg.de")));
    }

    #[test]
    fn answers_to_older_searches_are_dropped() {
        let mut join = opened();
        let Action::Search(old) = join.update(Message::Query("ic".into())) else {
            panic!("no search");
        };
        join.update(Message::Query("ice".into()));
        join.update(Message::Found(old, Err("slow".into())));
        assert!(join.loading);
        assert!(join.error.is_none());
    }
}
