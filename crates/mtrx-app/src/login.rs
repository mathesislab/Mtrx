//! Login screen: browser sign-in (OAuth, e.g. matrix.org) or, rarely
//! needed and so tucked away behind a link, a password.

use iced::{
    Alignment, Element, Length, Task,
    widget::{button, column, container, rule, text, text_input},
};

use crate::{fonts, logo, style, task::Owned};

#[derive(Debug, Clone)]
pub enum Message {
    Homeserver(String),
    User(String),
    Password(String),
    Browser,
    /// Show the password fields.
    ShowPassword,
    SubmitPassword,
    Cancel,
}

/// What the app should do after a [`Message`].
pub enum Action {
    None,
    Browser,
    Password,
}

/// The sign-ins, as keys of `Login::tasks`.
const PASSWORD: &str = "password";
const BROWSER: &str = "browser";

pub struct Login {
    pub homeserver: String,
    pub user: String,
    pub password: String,
    /// The password fields are shown.
    with_password: bool,
    /// The sign-in running: [`PASSWORD`] or [`BROWSER`].
    tasks: Owned,
    pub error: Option<String>,
}

impl Default for Login {
    fn default() -> Self {
        Self {
            homeserver: "matrix.org".into(),
            user: String::new(),
            password: String::new(),
            with_password: false,
            tasks: Owned::default(),
            error: None,
        }
    }
}

impl Login {
    pub fn update(&mut self, message: Message) -> Action {
        match message {
            Message::Homeserver(v) => self.homeserver = v,
            Message::User(v) => self.user = v,
            Message::Password(v) => self.password = v,
            Message::ShowPassword => self.with_password = true,
            Message::Browser if self.idle() && !self.homeserver.is_empty() => {
                self.error = None;
                return Action::Browser;
            }
            Message::SubmitPassword if self.can_submit_password() => {
                self.error = None;
                return Action::Password;
            }
            Message::Cancel => self.tasks.cancel(&BROWSER),
            Message::Browser | Message::SubmitPassword => {}
        }
        Action::None
    }

    /// Sign in with the password, by `sign_in`.
    pub fn password_sign_in<M: Send + 'static>(&mut self, sign_in: Task<M>) -> Task<M> {
        self.tasks.run(PASSWORD, sign_in)
    }

    /// Sign in in the browser, by `sign_in`; Cancel stops it.
    pub fn browser_sign_in<M: Send + 'static>(&mut self, sign_in: Task<M>) -> Task<M> {
        self.tasks.run(BROWSER, sign_in)
    }

    pub fn failed(&mut self, error: String) {
        self.error = Some(error);
    }

    fn idle(&self) -> bool {
        !self.tasks.running(&PASSWORD) && !self.tasks.running(&BROWSER)
    }

    fn can_submit_password(&self) -> bool {
        self.idle()
            && !self.homeserver.is_empty()
            && !self.user.is_empty()
            && !self.password.is_empty()
    }

    fn input<'a>(
        &self,
        placeholder: &'a str,
        value: &'a str,
        on_input: fn(String) -> Message,
        submit: Message,
    ) -> text_input::TextInput<'a, Message> {
        let field = text_input(placeholder, value)
            .padding(10)
            .style(style::input);
        if self.idle() {
            field.on_input(on_input).on_submit(submit)
        } else {
            field
        }
    }

    fn password_form(&self) -> Element<'_, Message> {
        column![
            self.input(
                "Username",
                &self.user,
                Message::User,
                Message::SubmitPassword,
            ),
            self.input(
                "Password",
                &self.password,
                Message::Password,
                Message::SubmitPassword,
            )
            .secure(true),
            button(text(if self.tasks.running(&PASSWORD) {
                "Signing in…"
            } else {
                "Sign in with password"
            }))
            .style(style::secondary_button)
            .padding([8, 16])
            .on_press_maybe(
                self.can_submit_password()
                    .then_some(Message::SubmitPassword)
            ),
        ]
        .spacing(14)
        .align_x(Alignment::Center)
        .width(Length::Fill)
        .into()
    }

    pub fn view(&self) -> Element<'_, Message> {
        let mut form = column![
            logo::view(96.0),
            text("Mtrx").size(32).font(fonts::SEMIBOLD),
            text("Sign in to your Matrix account").size(14),
            column![
                text("Homeserver: where your account is")
                    .size(12)
                    .style(text::secondary),
                self.input(
                    "Homeserver, e.g. matrix.org",
                    &self.homeserver,
                    Message::Homeserver,
                    Message::Browser
                ),
            ]
            .spacing(4),
        ]
        .spacing(14)
        .align_x(Alignment::Center)
        .width(360);

        form = if self.tasks.running(&BROWSER) {
            form.push(text("Continue in your browser…")).push(
                button(text("Cancel"))
                    .style(style::secondary_button)
                    .on_press(Message::Cancel),
            )
        } else {
            form.push(
                button(text("Sign in with browser").width(Length::Fill).center())
                    .style(style::primary_button)
                    .width(Length::Fill)
                    .padding(10)
                    .on_press_maybe(self.idle().then_some(Message::Browser)),
            )
            .push(
                text("Recommended for matrix.org and most servers")
                    .size(12)
                    .style(text::secondary),
            )
            .push(rule::horizontal(1))
            .push(if self.with_password {
                self.password_form()
            } else {
                button(text("Sign in with a password instead").size(12))
                    .style(style::text_button)
                    .on_press(Message::ShowPassword)
                    .into()
            })
        };
        if let Some(error) = &self.error {
            form = form.push(text(error).style(text::danger));
        }
        container(form).center(Length::Fill).into()
    }
}
