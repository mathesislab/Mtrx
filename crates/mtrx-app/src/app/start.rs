//! Starting: the saved session or signing in, until the rooms show; and
//! signing in again once the session ended.

use std::sync::Arc;

use iced::Task;
use mtrx_core::{RoomListColours, Session};

use super::{App, LiveLists, Main, Screen, Shared, people};
use crate::{
    app,
    last_view::LastView,
    login::{self, Login},
    presence, sidebar, task,
};

#[derive(Debug, Clone)]
pub enum Message {
    /// The saved session, or why there is none to open.
    Restored(Result<Shared<Session>, Shared<mtrx_core::Error>>),
    Login(login::Message),
    LoggedIn(Result<Shared<Session>, String>),
    /// The room lists, and whether this device is verified.
    Ready(Result<(Shared<LiveLists>, bool), String>),
    /// The server no longer accepts the session: forget it and log in again.
    SignInAgain,
    /// The ended session's local data is gone.
    Forgotten(Result<(), String>),
}

impl App {
    /// The saved session, or signing in.
    pub(super) fn restore(&self) -> Task<app::Message> {
        let profile = self.profile.clone();
        Task::perform(
            async move { Session::restore(profile).await.map(|s| Shared(Arc::new(s))) },
            |result| to(Message::Restored(result.map_err(|e| Shared(Arc::new(e))))),
        )
    }

    /// The screen changes: the session opens, or signing in.
    pub(super) fn start(&mut self, message: Message) -> Task<app::Message> {
        match message {
            Message::Restored(Ok(Shared(session))) | Message::LoggedIn(Ok(Shared(session))) => {
                self.open(session)
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
                        login.password_sign_in(task::request(
                            async move {
                                Session::login(profile, &homeserver, &user, &password)
                                    .await
                                    .map(|s| Shared(Arc::new(s)))
                            },
                            |result| to(Message::LoggedIn(result)),
                        ))
                    }
                    login::Action::Browser => login.browser_sign_in(task::request(
                        async move {
                            Session::login_oauth(profile, &homeserver, |url| {
                                let _ = open::that_detached(url.as_str());
                            })
                            .await
                            .map(|s| Shared(Arc::new(s)))
                        },
                        |result| to(Message::LoggedIn(result)),
                    )),
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
                    Ok((Shared(lists), verified)) => {
                        main.room_lists = Some(lists);
                        main.verified = Some(verified);
                    }
                    Err(e) => main.report(format!("Could not load the rooms: {e}"), None),
                }
                self.restore_view()
            }
            Message::SignInAgain => {
                let Screen::Main(main) = &self.screen else {
                    return Task::none();
                };
                let session = main.session.clone();
                task::request(async move { session.forget().await }, |result| {
                    to(Message::Forgotten(result))
                })
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
        }
    }

    /// Show the session: it syncs, and the rooms show once listed.
    fn open(&mut self, session: Arc<Session>) -> Task<app::Message> {
        let mut main = Main::new(session.clone());
        main.restore = self.view_file.as_deref().map(LastView::load);
        self.screen = Screen::Main(Box::new(main));
        let migrate = self.move_colours_to_account(&session);
        let share = self.settings.share_presence;
        let presence = presence::start(&session, share)
            .map(|message| app::Message::People(people::Message::Presence(message)));
        let start = task::request(
            async move {
                session.start_sync().await;
                session.start_scheduler();
                let list = session.room_list().await?;
                let lists = session.room_lists(&list);
                let verified = session.is_device_verified().await?;
                Ok((Shared(Arc::new(lists)), verified))
            },
            |result| to(Message::Ready(result)),
        );
        // Not sharing, offline before syncing, whose requests would
        // say online; sharing, sync need not wait.
        let start = if share {
            Task::batch([presence, start])
        } else {
            presence.chain(start)
        };
        Task::batch([start, migrate])
    }

    /// Room list colours chosen before the account kept them, moved there
    /// unless the account has colours already.
    fn move_colours_to_account(&self, session: &Arc<Session>) -> Task<app::Message> {
        let Some(legacy) = self.settings.legacy_colours() else {
            return Task::none();
        };
        let session = session.clone();
        task::request(
            async move {
                if session.room_list_colours() != RoomListColours::default() {
                    return Ok(());
                }
                session
                    .change_room_list_colours(|colours| *colours = legacy)
                    .await
            },
            |result| app::Message::Sidebar(sidebar::Message::ColoursMoved(result)),
        )
    }
}

/// `message` for starting, as the app gets it.
pub fn to(message: Message) -> app::Message {
    app::Message::Start(message)
}
