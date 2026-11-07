//! The account's state: this device verified, the connection, scheduled
//! messages, and notifications.

use std::time::SystemTime;

use futures_util::SinkExt;
use iced::{
    Element, Length, Task,
    widget::{button, column, container, row, text, text_input},
};
use mtrx_core::{
    Connection, Notification, OwnedRoomId, RecoveryStep, RoomLists, ScheduledMessage, Session,
};

use super::{Cx, Popup, pane, start};
use crate::{
    app, fonts,
    hint::{Position, hint},
    room, style,
};

#[derive(Debug, Clone)]
pub enum Message {
    RecoveryKey(String),
    Recover,
    RecoveryProgress(RecoveryStep),
    Recovered(Result<bool, String>),
    /// The server became reachable or not.
    Connection(Connection),
    /// Try the server now.
    Reconnect,
    Scheduled(Vec<ScheduledMessage>),
    /// Refreshes times shown relative to now.
    Tick,
    Notified(Notification),
    /// A desktop notification about the room was clicked.
    NotificationClicked(OwnedRoomId),
}

/// This device's verification, the connection, and scheduled messages.
pub struct Account {
    /// `None` until known.
    pub verified: Option<bool>,
    pub recovery_key: String,
    /// Set while recovery runs.
    pub recovery_step: Option<RecoveryStep>,
    pub recovery_error: Option<String>,
    pub connection: Connection,
    /// Scheduled messages of all rooms, earliest first.
    pub scheduled: Vec<ScheduledMessage>,
    /// When scheduled times were last compared to the clock.
    pub now: SystemTime,
}

impl Account {
    pub fn new(session: &Session) -> Self {
        Self {
            verified: None,
            recovery_key: String::new(),
            recovery_step: None,
            recovery_error: None,
            connection: session.connection(),
            scheduled: session.scheduled(),
            now: SystemTime::now(),
        }
    }

    /// `message`; `open` is the room open, about which no notification
    /// shows while the window has the focus.
    pub fn update(
        &mut self,
        message: Message,
        cx: &mut Cx<'_>,
        open: Option<&OwnedRoomId>,
    ) -> Task<app::Message> {
        match message {
            Message::RecoveryKey(key) => {
                self.recovery_key = key;
                Task::none()
            }
            Message::Recover => {
                if self.recovery_step.is_some() || self.recovery_key.trim().is_empty() {
                    return Task::none();
                }
                self.recovery_step = Some(RecoveryStep::CheckingKey);
                self.recovery_error = None;
                let (session, key) = (cx.session.clone(), self.recovery_key.trim().to_owned());
                Task::run(
                    iced::stream::channel(16, async move |mut tx| {
                        let progress = |step| {
                            let _ = tx.clone().try_send(to(Message::RecoveryProgress(step)));
                        };
                        let result = async {
                            session.recover(&key, progress).await?;
                            session.is_device_verified().await
                        }
                        .await;
                        let recovered = Message::Recovered(result.map_err(|e| e.to_string()));
                        let _ = tx.send(to(recovered)).await;
                    }),
                    |message| message,
                )
            }
            Message::RecoveryProgress(step) => {
                if self.recovery_step.is_some() {
                    self.recovery_step = Some(step);
                }
                Task::none()
            }
            Message::Recovered(result) => {
                self.recovery_step = None;
                match result {
                    Ok(verified) => {
                        self.verified = Some(verified);
                        self.recovery_key.clear();
                    }
                    Err(e) => self.recovery_error = Some(e),
                }
                Task::none()
            }
            Message::Connection(connection) => {
                let back =
                    self.connection == Connection::Offline && connection == Connection::Online;
                self.connection = connection;
                // History that could not load while offline.
                if back {
                    Task::done(app::Message::Shown(room::Message::Online))
                } else {
                    Task::none()
                }
            }
            Message::Reconnect => {
                let session = cx.session.clone();
                Task::future(async move { session.reconnect().await }).discard()
            }
            Message::Scheduled(scheduled) => {
                // A message given a new time may have been sent (it became
                // due) or cancelled elsewhere.
                let gone = |id: &str| !scheduled.iter().any(|m| m.id == id);
                cx.popup.take_if(|popup| {
                    matches!(popup, Popup::SendLater(later)
                        if later.rescheduling.as_ref().is_some_and(|m| gone(&m.id)))
                });
                // One edited in the room open may be among them.
                let room = Task::done(app::Message::Shown(room::Message::Scheduled(
                    scheduled.clone(),
                )));
                self.scheduled = scheduled;
                self.now = SystemTime::now();
                room
            }
            Message::Tick => {
                self.now = SystemTime::now();
                Task::none()
            }
            Message::Notified(notification) => {
                let looking = cx.window.focused && open == Some(&notification.room_id);
                if !cx.settings.desktop_notifications || looking {
                    return Task::none();
                }
                let room = notification.room_id.clone();
                Task::future(notify(notification)).then(move |clicked| {
                    if clicked {
                        Task::done(to(Message::NotificationClicked(room.clone())))
                    } else {
                        Task::none()
                    }
                })
            }
            Message::NotificationClicked(room) => {
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
                Task::batch([raise, Task::done(pane::to(pane::Message::Open(room)))])
            }
        }
    }

    /// Asks for the recovery key while this device is not verified.
    pub fn recovery_banner(&self) -> Option<Element<'_, app::Message>> {
        if self.verified != Some(false) {
            return None;
        }
        let busy = self.recovery_step.is_some();
        let mut input = text_input("Recovery key", &self.recovery_key)
            .style(style::input)
            .secure(true)
            .padding(8)
            .width(320);
        if !busy {
            input = input
                .on_input(|key| to(Message::RecoveryKey(key)))
                .on_submit(to(Message::Recover));
        }
        let unlock = button(text(if busy { "Unlocking…" } else { "Unlock" }))
            .style(style::primary_button)
            .on_press_maybe((!busy).then_some(to(Message::Recover)));
        let mut banner = column![
            text("This device is not verified, so older encrypted messages cannot be read."),
            row![input, unlock].spacing(8),
        ]
        .spacing(8);
        if let Some(step) = self.recovery_step {
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
        if let Some(error) = &self.recovery_error {
            banner = banner.push(text(error).style(text::danger));
        }
        Some(
            container(
                container(banner)
                    .padding(12)
                    .width(Length::Fill)
                    .style(style::banner),
            )
            .padding([8, 24])
            .into(),
        )
    }

    /// Points to rooms (other than the open one) with overdue scheduled
    /// messages; their composer bar offers sending or cancelling them.
    pub fn overdue_banner<'a>(
        &'a self,
        lists: &'a RoomLists,
        open: Option<&OwnedRoomId>,
    ) -> Option<Element<'a, app::Message>> {
        let mut rooms: Vec<(&OwnedRoomId, usize)> = Vec::new();
        for message in self.scheduled.iter().filter(|m| m.overdue) {
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
            let name = (lists.room(id)).map_or(id.as_str(), |r| r.name.as_str());
            let label = if count > 1 {
                format!("{name} ({count})")
            } else {
                name.to_owned()
            };
            banner = banner.push(
                button(text(label).size(13))
                    .padding([2, 8])
                    .style(style::secondary_button)
                    .on_press(pane::to(pane::Message::Open(id.clone()))),
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

    /// Along the bottom while the server is unreachable or no longer
    /// accepts the session.
    pub fn connection_bar<'a>(&self) -> Option<Element<'a, app::Message>> {
        match self.connection {
            Connection::Online => None,
            Connection::Offline => Some(offline_bar()),
            Connection::SignedOut => Some(signed_out_bar()),
        }
    }
}

/// Along the bottom, near the composer, while the server is unreachable.
fn offline_bar<'a>() -> Element<'a, app::Message> {
    container(
        row![
            text("Offline – reconnecting…")
                .size(13)
                .font(fonts::SEMIBOLD),
            text("Messages are sent once the server is back.")
                .size(13)
                .width(Length::Fill),
            button(text("Retry now").size(13))
                .padding([3, 10])
                .style(style::problem_button)
                .on_press(to(Message::Reconnect)),
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
fn signed_out_bar<'a>() -> Element<'a, app::Message> {
    container(
        row![
            text("Your session has ended")
                .size(13)
                .font(fonts::SEMIBOLD),
            text("The server no longer accepts this login: it expired or was signed out elsewhere.")
                .size(13)
                .width(Length::Fill),
            hint(
                button(text("Sign in again").size(13))
                    .padding([3, 10])
                    .style(style::problem_button)
                    .on_press(start::to(start::Message::SignInAgain)),
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

/// macOS asks once whether Mtrx may show notifications; only for the app
/// bundle (`Mtrx.app`), as macOS sends them for bundles only.
pub fn ask_to_notify() -> Task<app::Message> {
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

/// `message` for the account, as the app gets it.
pub fn to(message: Message) -> app::Message {
    app::Message::Account(message)
}
