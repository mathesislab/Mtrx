//! What shows over the whole window, above the card: Settings, a dialog,
//! joining, exporting, or an image or PDF shown large.

use std::{collections::HashMap, sync::Arc, time::Instant};

use iced::Task;
use mtrx_core::{MediaRef, OwnedRoomId, OwnedRoomOrAliasId, Session};

use super::{Cx, Popup, pane, people, start};
use crate::{
    app, arrange,
    dialog::{self, Dialog},
    directory, downloads, export, join,
    media::{self, Viewer},
    pdf_viewer, presence,
    settings::{self, Settings},
    sidebar::{self, Sidebar},
    spelling, task,
    update::{self, Updater},
};

#[derive(Debug, Clone)]
pub enum Message {
    ShowSettings(bool),
    Settings(settings::Message),
    OpenDialog(dialog::Kind),
    Dialog(dialog::Message),
    /// Export a room's or a space's chats, called `.1`.
    OpenExport(export::Target, String),
    Export(export::Message),
    /// Join a space or room from elsewhere.
    OpenJoin,
    Join(join::Message),
    /// Show an image large.
    OpenImage(downloads::File),
    CloseImage,
    /// Open the menu of the image in the viewer.
    OpenViewerMenu,
    /// The whole image for the viewer.
    ImageLoaded(MediaRef, Result<media::Picture, String>),
    /// Show a PDF large.
    OpenPdf(downloads::File),
    Pdf(pdf_viewer::Message),
    /// Fast ticks while something is in progress.
    Progress,
    /// Copying the image shown failed: the viewer says so (else a notice).
    Failed(String),
}

/// The panel over the window, above the card, and the requests of dialogs
/// closed while they run.
#[derive(Default)]
pub struct Modal {
    pub shown: Option<Shown>,
    /// The last dialog request made.
    pub last_request: dialog::RequestId,
    /// Dialogs closed while their requests run: notices for success and
    /// failure.
    background: HashMap<dialog::RequestId, (String, String)>,
}

/// What shows over the whole window: one at a time, the one opened last
/// replacing any other.
// One, in the boxed [`super::Main`].
#[allow(clippy::large_enum_variant)]
pub enum Shown {
    Settings(settings::Panel),
    Dialog(Dialog),
    /// Joining a space or room from elsewhere.
    Join(join::Join),
    /// Exporting chats.
    Export(export::Dialog),
    /// An image shown large.
    Image(Viewer),
    /// A PDF shown large.
    Pdf(pdf_viewer::Viewer),
}

impl Modal {
    pub fn is_some(&self) -> bool {
        self.shown.is_some()
    }

    /// The settings panel, while open.
    pub fn settings(&mut self) -> Option<&mut settings::Panel> {
        match &mut self.shown {
            Some(Shown::Settings(panel)) => Some(panel),
            _ => None,
        }
    }

    /// The PDF viewer, while it shows.
    pub fn pdf(&mut self) -> Option<&mut pdf_viewer::Viewer> {
        match &mut self.shown {
            Some(Shown::Pdf(pdf)) => Some(pdf),
            _ => None,
        }
    }

    /// Esc: a dialog cancels, joining closes, as their own Cancel and
    /// Close; anything else closes at once. False if nothing shows.
    pub fn escape(&mut self) -> Option<Task<app::Message>> {
        match self.shown.as_ref()? {
            Shown::Dialog(_) => Some(Task::done(to(Message::Dialog(dialog::Message::Cancel)))),
            Shown::Join(_) => Some(Task::done(to(Message::Join(join::Message::Close)))),
            Shown::Export(_) => Some(Task::done(to(Message::Export(export::Message::Cancel)))),
            _ => {
                self.shown = None;
                Some(Task::none())
            }
        }
    }

    /// An update shown in the open settings is seen: no dot on the gear
    /// until a newer one. True if that changed `settings`.
    pub fn see_update(&mut self, updater: &Updater, settings: &mut Settings) -> bool {
        if self.settings().is_none() || !updater.unseen(&settings.updates) {
            return false;
        }
        let version = (updater.available.as_ref()).map(|u| u.version.to_string());
        settings.updates.seen = version;
        true
    }

    pub fn update(
        &mut self,
        message: Message,
        cx: &mut Cx<'_>,
        updater: &mut Updater,
        sidebar: &Sidebar,
    ) -> Task<app::Message> {
        match message {
            Message::ShowSettings(true) => self.open_settings(cx, updater, sidebar),
            Message::ShowSettings(false) => {
                self.shown
                    .take_if(|shown| matches!(shown, Shown::Settings(_)));
                Task::none()
            }
            Message::Settings(message) => self.settings_update(message, cx, updater),
            Message::OpenDialog(kind) => {
                cx.cover();
                // A card stays below, e.g. a space's overview, to come
                // back to.
                let near = near_spaces(&kind, &cx.lists.graph, sidebar.space.as_ref());
                let stop = Task::done(app::Message::Sidebar(sidebar::Message::Arrange(
                    arrange::Message::Stop,
                )));
                let mut dialog = Dialog::new(kind);
                dialog.fill_category(&sidebar.arrange.categories);
                if !dialog.picks_people() {
                    self.shown = Some(Shown::Dialog(dialog));
                    return stop;
                }
                dialog.prefer_spaces(near);
                dialog.offer_self(cx.own_user.clone());
                let owners = if dialog.creates_room() {
                    let session = cx.session.clone();
                    dialog.offer_owners(Task::perform(
                        async move { session.offers_owners().await },
                        |offered| to(Message::Dialog(dialog::Message::OwnersOffered(offered))),
                    ))
                } else {
                    Task::none()
                };
                self.shown = Some(Shown::Dialog(dialog));
                let contacts = Task::done(app::Message::People(people::Message::LoadContacts));
                Task::batch([stop, contacts, owners])
            }
            Message::Dialog(message) => self.dialog(message, cx),
            Message::OpenExport(target, title) => {
                cx.cover();
                self.shown = Some(Shown::Export(export::Dialog::new(
                    target,
                    title,
                    cx.lists,
                    export::today(),
                )));
                Task::done(app::Message::Sidebar(sidebar::Message::Arrange(
                    arrange::Message::Stop,
                )))
            }
            Message::Export(message) => self.export(message, cx),
            Message::OpenJoin => {
                cx.cover();
                cx.close_card();
                let (join, action) = join::Join::new(
                    cx.session.user_id().server_name().to_owned(),
                    &cx.session.directory_servers().servers,
                );
                self.shown = Some(Shown::Join(join));
                Task::batch([
                    self.join_action(action, cx),
                    iced::widget::operation::focus(join::SEARCH),
                ])
            }
            Message::Join(message) => {
                let Some(Shown::Join(join)) = &mut self.shown else {
                    return Task::none();
                };
                // A server whose directory answered is remembered.
                let remember = match &message {
                    join::Message::Found(search, Ok(_)) => search.server.clone().map(|server| {
                        let session = cx.session.clone();
                        task::background("remember a directory server", async move {
                            session.remember_directory_server(&server).await
                        })
                    }),
                    _ => None,
                };
                let action = join.update(message);
                Task::batch([
                    remember.unwrap_or_else(Task::none),
                    self.join_action(action, cx),
                ])
            }
            Message::OpenImage(file) => {
                cx.cover();
                let mut viewer = Viewer::new(file);
                let load = viewer.load(cx.session.clone());
                self.shown = Some(Shown::Image(viewer));
                load
            }
            Message::OpenViewerMenu => {
                if let Some(Shown::Image(_)) = &self.shown {
                    *cx.popup = Some(Popup::Viewer);
                }
                Task::none()
            }
            Message::CloseImage => {
                self.shown
                    .take_if(|shown| matches!(shown, Shown::Image(_) | Shown::Pdf(_)));
                cx.popup.take_if(|popup| matches!(popup, Popup::Viewer));
                Task::none()
            }
            Message::ImageLoaded(media, result) => {
                if let Some(Shown::Image(viewer)) = &mut self.shown {
                    viewer.loaded(&media, result);
                }
                Task::none()
            }
            Message::OpenPdf(file) => {
                cx.cover();
                let (viewport, scale) = (
                    cx.window.viewport(cx.settings),
                    cx.window.pixel_scale(cx.settings),
                );
                let mut viewer = pdf_viewer::Viewer::new(file, viewport, scale);
                let open = viewer.open(cx.session.clone());
                self.shown = Some(Shown::Pdf(viewer));
                open
            }
            Message::Pdf(message) => match self.pdf() {
                Some(viewer) => viewer.update(message),
                None => Task::none(),
            },
            Message::Failed(e) => {
                match &mut self.shown {
                    Some(Shown::Image(viewer)) => viewer.error = Some(e),
                    Some(Shown::Pdf(pdf)) => pdf.error = Some(e),
                    _ => cx.notices.report(e, None),
                }
                Task::none()
            }
            Message::Progress => {
                if let Some(Shown::Dialog(dialog)) = &mut self.shown {
                    dialog.tick(Instant::now());
                }
                Task::none()
            }
        }
    }

    fn open_settings(
        &mut self,
        cx: &mut Cx<'_>,
        updater: &Updater,
        sidebar: &Sidebar,
    ) -> Task<app::Message> {
        cx.cover();
        let mut panel = settings::Panel::default();
        panel.google.account = cx.google.account().unwrap_or_else(|e| {
            panel.google.error = Some(e.to_string());
            None
        });
        panel.account = Some(cx.session.user_id().to_string());
        let mut load = Task::none();
        if let Some(space) = &sidebar.space {
            let listed = cx.lists.room(space);
            panel.space = Some(settings::SpaceCalls::new(
                space.clone(),
                listed.map_or_else(|| space.to_string(), |r| r.name.clone()),
                listed.map(|r| r.rights).unwrap_or_default(),
            ));
            let (session, space) = (cx.session.clone(), space.clone());
            load = panel.load_space(Task::perform(
                async move {
                    let service = match session.room(&space) {
                        Ok(room) => room.call_service().await.ok().flatten(),
                        Err(_) => None,
                    };
                    settings::Message::SpaceLoaded(space, service)
                },
                |message| to(Message::Settings(message)),
            ));
        }
        self.shown = Some(Shown::Settings(panel));
        if self.see_update(updater, cx.settings) {
            cx.save_settings();
        }
        load
    }

    fn settings_update(
        &mut self,
        message: settings::Message,
        cx: &mut Cx<'_>,
        updater: &mut Updater,
    ) -> Task<app::Message> {
        // Preferences change also from outside the panel, e.g. a card.
        let action = match (message, self.settings()) {
            (settings::Message::Preference(preference), _) => {
                settings::change(cx.settings, preference)
            }
            (message, Some(panel)) => panel.update(cx.settings, message),
            (_, None) => return Task::none(),
        };
        // All but nothing and the account's changes save the settings.
        if !matches!(
            action,
            settings::Action::None
                | settings::Action::GoogleSignIn(_)
                | settings::Action::GoogleSignOut
                | settings::Action::Colour(..)
                | settings::Action::SetSpaceService(..)
                | settings::Action::AtHome(_)
                | settings::Action::NotesAtTop(_)
        ) {
            cx.save_settings();
        }
        match action {
            settings::Action::None | settings::Action::Save | settings::Action::TextSize => {
                Task::none()
            }
            settings::Action::CheckUpdates => {
                updater.update(update::Message::Check, &cx.settings.updates)
            }
            settings::Action::Appearance => {
                #[cfg(target_os = "macos")]
                cx.settings.appearance.apply();
                Task::none()
            }
            settings::Action::Spelling => {
                spelling::set_enabled(cx.settings.check_spelling);
                Task::none()
            }
            // The switch is off while the server does not share presence.
            settings::Action::SharePresence => {
                presence::sharing(cx.session, cx.settings.share_presence)
                    .map(|message| app::Message::People(people::Message::Presence(message)))
            }
            settings::Action::GoogleSignIn(client) => {
                let google = cx.google.clone();
                let sign_in = Task::perform(
                    async move {
                        google
                            .sign_in(client, |url| {
                                let _ = open::that_detached(url.as_str());
                            })
                            .await
                    },
                    |result| {
                        to(Message::Settings(settings::Message::GoogleSignedIn(
                            result.map_err(|e| e.to_string()),
                        )))
                    },
                );
                // Asked for from the panel, which shows it signing in.
                match self.settings() {
                    Some(panel) => panel.google_sign_in(sign_in),
                    None => Task::none(),
                }
            }
            settings::Action::Colour(space, tint) => {
                let session = cx.session.clone();
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
                    |result| to(Message::Settings(settings::Message::ColourSaved(result))),
                )
            }
            settings::Action::AtHome(at_home) => Task::done(app::Message::Sidebar(
                sidebar::Message::Arrange(arrange::Message::AtHome(at_home)),
            )),
            settings::Action::NotesAtTop(on) => Task::done(app::Message::Sidebar(
                sidebar::Message::Arrange(arrange::Message::NotesAtTop(on)),
            )),
            settings::Action::GoogleSignOut => {
                let google = cx.google.clone();
                Task::perform(async move { google.sign_out().await }, |result| {
                    to(Message::Settings(settings::Message::GoogleSignedOut(
                        result.map_err(|e| e.to_string()),
                    )))
                })
            }
            settings::Action::SetSpaceService(space, service) => {
                let session = cx.session.clone();
                task::request(
                    async move { session.room(&space)?.set_call_service(service).await },
                    |result| to(Message::Settings(settings::Message::SpaceSaved(result))),
                )
            }
        }
    }

    fn dialog(&mut self, message: dialog::Message, cx: &mut Cx<'_>) -> Task<app::Message> {
        // The request of a dialog closed while it ran: what it changed, and
        // a notice, which opens what it made.
        let message = match message {
            dialog::Message::Done(id, result) if self.background.contains_key(&id) => {
                let (done, failed) = self.background.remove(&id).unwrap_or_default();
                return match result {
                    Ok(opened) => {
                        let room = match &opened {
                            dialog::Opened::Room(id) | dialog::Opened::Space(id) => {
                                Some(id.clone())
                            }
                            _ => None,
                        };
                        cx.notices.done(done, room);
                        request_done(opened)
                    }
                    Err(e) => {
                        cx.notices.report(format!("{failed}: {e}"), None);
                        Task::none()
                    }
                };
            }
            message => message,
        };
        let Some(Shown::Dialog(dialog)) = &mut self.shown else {
            return Task::none();
        };
        match dialog.update(message) {
            dialog::Action::None => Task::none(),
            dialog::Action::Close => {
                self.shown = None;
                Task::none()
            }
            dialog::Action::Background(id) => {
                self.background.insert(id, dialog.outcome());
                self.shown = None;
                Task::none()
            }
            dialog::Action::Search(query) => {
                let session = cx.session.clone();
                dialog.search_users(Task::perform(
                    {
                        let query = query.clone();
                        async move { session.search_users(&query, 10).await.unwrap_or_default() }
                    },
                    move |users| to(Message::Dialog(dialog::Message::Suggestions(query, users))),
                ))
            }
            dialog::Action::Arrange(message) => {
                self.shown = None;
                Task::done(app::Message::Sidebar(sidebar::Message::Arrange(message)))
            }
            dialog::Action::Submit(request) => {
                let id = self.last_request.next();
                self.last_request = id;
                dialog.sent(id);
                Task::perform(run_request(cx.session.clone(), request), move |result| {
                    to(Message::Dialog(dialog::Message::Done(id, result)))
                })
            }
            dialog::Action::Finished(opened) => {
                self.shown = None;
                let show = match &opened {
                    dialog::Opened::Room(id) => {
                        Task::done(app::Message::Pane(pane::Message::Open(id.clone())))
                    }
                    dialog::Opened::Space(id) => Task::done(app::Message::Sidebar(
                        sidebar::Message::SelectSpace(Some(id.clone())),
                    )),
                    _ => Task::none(),
                };
                Task::batch([request_done(opened), show])
            }
        }
    }

    fn export(&mut self, message: export::Message, cx: &mut Cx<'_>) -> Task<app::Message> {
        let Some(Shown::Export(dialog)) = &mut self.shown else {
            return Task::none();
        };
        match dialog.update(message, export::today()) {
            export::Action::None => Task::none(),
            export::Action::Close => {
                self.shown = None;
                Task::none()
            }
            export::Action::PickFolder => Task::perform(
                async {
                    let folder = rfd::AsyncFileDialog::new().pick_folder().await;
                    folder.map(|folder| folder.path().to_owned())
                },
                |folder| folder.map(|folder| to(Message::Export(export::Message::Folder(folder)))),
            )
            .and_then(Task::done),
            export::Action::Export(request) => {
                match cx.session.export(request) {
                    Ok(_) => self.shown = None,
                    Err(e) => dialog.failed(e.to_string()),
                }
                Task::none()
            }
        }
    }

    /// Run what the join dialog asks for.
    fn join_action(&mut self, action: join::Action, cx: &mut Cx<'_>) -> Task<app::Message> {
        let session = cx.session.clone();
        let done = |room: OwnedRoomOrAliasId| {
            move |result| to(Message::Join(join::Message::Done(room, result)))
        };
        let join = match &mut self.shown {
            Some(Shown::Join(join)) => Some(join),
            _ => None,
        };
        match action {
            join::Action::None => Task::none(),
            join::Action::Close => {
                self.shown = None;
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
                    move |result| to(Message::Join(join::Message::Found(search, result))),
                ))
            }),
            join::Action::Preview(address, via) => join.map_or_else(Task::none, |join| {
                join.browse(task::request(
                    {
                        let address = address.clone();
                        async move { session.preview(&address, via).await }
                    },
                    move |result| {
                        to(Message::Join(join::Message::Previewed(
                            address.to_string(),
                            result,
                        )))
                    },
                ))
            }),
            join::Action::Join(room, via, space) => task::request(
                {
                    let join = directory::join(session, room.clone(), via, false);
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
                    let knock = directory::join(session, room.clone(), via, true);
                    async move { knock.await.map(|_| join::Joined::Asked) }
                },
                done(room),
            ),
            join::Action::ShowSpace(space) => {
                self.shown = None;
                Task::done(app::Message::ShowJoined(space))
            }
        }
    }
}

/// What a dialog's request changed, shown, whether the dialog still shows
/// or not: the members changed, a room left closes, signing out ends the
/// session.
fn request_done(opened: dialog::Opened) -> Task<app::Message> {
    match opened {
        dialog::Opened::Nothing | dialog::Opened::Room(_) | dialog::Opened::Space(_) => {
            Task::none()
        }
        dialog::Opened::Members(room) => Task::done(app::Message::Pane(
            pane::Message::MembersChanged(room, Ok(())),
        )),
        dialog::Opened::Left(room) => Task::done(app::Message::Left(room)),
        dialog::Opened::SignedOut => Task::done(start::to(start::Message::SignedOut)),
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

/// `message` for what shows over the window, as the app gets it.
pub fn to(message: Message) -> app::Message {
    app::Message::Modal(message)
}
