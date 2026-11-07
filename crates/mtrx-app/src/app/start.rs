//! Starting: the saved session or signing in, until the rooms show; and
//! signing in again once the session ended.

use mtrx_core::Session;

use super::{LiveLists, Shared};
use crate::login;

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
