//! The errors the apps see.

/// Why a call failed. Show `reason`s (and [`MtrxError::LoginCancelled`]
/// not at all); match the others to react.
#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum MtrxError {
    /// No saved session.
    #[error("not signed in")]
    NotLoggedIn,
    #[error("already signed in; sign out first")]
    AlreadyLoggedIn,
    #[error("sign-in was cancelled")]
    LoginCancelled,
    #[error("no room `{room_id}`")]
    RoomNotFound { room_id: String },
    /// An argument that is not a valid ID, URL or recovery key, or a call
    /// made twice.
    #[error("{reason}")]
    InvalidInput { reason: String },
    /// The app's [`crate::SecretBackend`] failed.
    #[error("secret storage: {reason}")]
    SecretStorage { reason: String },
    /// The server does not share presence: hide it for the session.
    #[error("This server does not share presence")]
    PresenceUnavailable,
    /// A message looked for is not in the timeline, nor in the older
    /// events loaded for it ([`crate::Timeline::seek`]).
    #[error("The original message was not found in the room's history")]
    EventNotFound,
    /// Anything else, e.g. the server is unreachable or refused the password.
    #[error("{reason}")]
    Other { reason: String },
}

impl MtrxError {
    pub(crate) fn invalid(reason: impl std::fmt::Display) -> Self {
        Self::InvalidInput {
            reason: reason.to_string(),
        }
    }
}

impl From<mtrx_core::Error> for MtrxError {
    fn from(error: mtrx_core::Error) -> Self {
        use mtrx_core::Error;
        match error {
            Error::NotLoggedIn(_) => Self::NotLoggedIn,
            Error::AlreadyLoggedIn(_) => Self::AlreadyLoggedIn,
            Error::LoginCancelled => Self::LoginCancelled,
            Error::RoomNotFound(room_id) => Self::RoomNotFound { room_id },
            Error::SecretStorage(reason) => Self::SecretStorage { reason },
            error @ Error::InvalidRecoveryKey => Self::invalid(error),
            Error::PresenceUnavailable(_) => Self::PresenceUnavailable,
            Error::EventNotFound => Self::EventNotFound,
            error => Self::Other {
                reason: error.to_string(),
            },
        }
    }
}

impl From<std::io::Error> for MtrxError {
    fn from(error: std::io::Error) -> Self {
        Self::Other {
            reason: error.to_string(),
        }
    }
}

/// A room ID from the apps.
pub(crate) fn room_id(id: &str) -> Result<mtrx_core::OwnedRoomId, MtrxError> {
    mtrx_core::OwnedRoomId::try_from(id).map_err(MtrxError::invalid)
}

/// A user ID from the apps.
pub(crate) fn user_id(id: &str) -> Result<mtrx_core::OwnedUserId, MtrxError> {
    mtrx_core::OwnedUserId::try_from(id).map_err(MtrxError::invalid)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn core_errors_keep_what_the_apps_match_on() {
        let convert = |e: mtrx_core::Error| MtrxError::from(e);
        assert!(matches!(
            convert(mtrx_core::Error::NotLoggedIn("default".into())),
            MtrxError::NotLoggedIn
        ));
        assert!(matches!(
            convert(mtrx_core::Error::RoomNotFound("!r:example.org".into())),
            MtrxError::RoomNotFound { room_id } if room_id == "!r:example.org"
        ));
        assert!(matches!(
            convert(mtrx_core::Error::SecretStorage("locked".into())),
            MtrxError::SecretStorage { reason } if reason == "locked"
        ));
        assert!(matches!(
            convert(mtrx_core::Error::InvalidRecoveryKey),
            MtrxError::InvalidInput { reason } if reason == "The recovery key is not valid"
        ));
        assert!(matches!(
            convert(mtrx_core::Error::PresenceUnavailable(
                "[403 / M_FORBIDDEN] off".into()
            )),
            MtrxError::PresenceUnavailable
        ));
        assert!(matches!(
            convert(mtrx_core::Error::EventNotFound),
            MtrxError::EventNotFound
        ));
        assert!(matches!(
            convert(mtrx_core::Error::Timeline("nothing to send".into())),
            MtrxError::Other { reason } if reason == "timeline: nothing to send"
        ));
    }
}
