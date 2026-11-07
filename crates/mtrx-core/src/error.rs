use thiserror::Error;

pub type Result<T, E = Error> = std::result::Result<T, E>;

#[derive(Debug, Error)]
pub enum Error {
    #[error("invalid profile name `{0}` (use letters, digits, `-` and `_`)")]
    InvalidProfileName(String),
    #[error("could not determine a data directory for this platform")]
    NoDataDir,
    #[error("profile `{0}` is not logged in")]
    NotLoggedIn(String),
    #[error("profile `{0}` is already logged in; log out first")]
    AlreadyLoggedIn(String),
    #[error("login was cancelled")]
    LoginCancelled,
    #[error(transparent)]
    OAuth(#[from] matrix_sdk::authentication::oauth::OAuthError),
    #[error("no room matches `{0}`")]
    RoomNotFound(String),
    #[error("`{0}` matches several rooms: {1:?}")]
    AmbiguousRoom(String, Vec<matrix_sdk::ruma::OwnedRoomId>),
    #[error("sync: {0}")]
    Sync(String),
    #[error("room list: {0}")]
    RoomList(String),
    #[error("timeline: {0}")]
    Timeline(String),
    /// A message looked for is not in the timeline, nor in the older
    /// events loaded for it (see [`crate::RoomTimeline::seek`]).
    #[error("The original message was not found in the room's history")]
    EventNotFound,
    #[error("verification: {0}")]
    Verification(String),
    /// Not a recovery key (mistyped), or not the one of this account's
    /// backup.
    #[error("The recovery key is not valid")]
    InvalidRecoveryKey,
    #[error("no scheduled message `{0}`")]
    ScheduledNotFound(String),
    #[error("the send time is in the past")]
    SendTimeInPast,
    #[error("not a valid call service link: `{0}`")]
    InvalidCallService(String),
    #[error("permissions: {0}")]
    Permissions(String),
    #[error("invalid media type `{0}`")]
    InvalidMediaType(String),
    /// A download failed or was cancelled (why, in English).
    #[error("{0}")]
    Download(String),
    /// An export could not start (why, in English).
    #[error("{0}")]
    Export(String),
    #[error("notifications: {0}")]
    Notifications(String),
    /// The server does not share presence: it refuses ours and others'
    /// (its answer, for logs).
    #[error("This server does not share presence")]
    PresenceUnavailable(String),

    #[error(transparent)]
    Matrix(#[from] matrix_sdk::Error),
    #[error(transparent)]
    Http(#[from] matrix_sdk::HttpError),
    #[error(transparent)]
    Recovery(#[from] matrix_sdk::encryption::recovery::RecoveryError),
    #[error(transparent)]
    Store(#[from] matrix_sdk::StoreError),
    #[error(transparent)]
    Crypto(#[from] matrix_sdk::encryption::CryptoStoreError),
    #[error(transparent)]
    ClientBuild(#[from] matrix_sdk::ClientBuildError),
    #[cfg(not(any(target_os = "ios", target_os = "android")))]
    #[error("secret storage: {0}")]
    Keyring(#[from] keyring::Error),
    /// The OS credential store cannot be opened (Linux: no Secret Service).
    #[cfg(not(any(target_os = "ios", target_os = "android")))]
    #[error("{advice} ({0})", advice = crate::secrets::NO_SECRET_STORE)]
    NoSecretStore(String),
    /// The app's secret store failed ([`crate::SecretStore::Custom`]).
    #[error("secret storage: {0}")]
    SecretStorage(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}
