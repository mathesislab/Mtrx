//! A logged-in account: login, restore and logout.

use std::sync::Arc;

use matrix_sdk::{
    Client,
    authentication::oauth::{
        ClientId, OAuthSession,
        error::{OAuthAuthorizationCodeError, OAuthError},
        registration::{ApplicationType, ClientMetadata, Localized, OAuthGrantType},
    },
    cross_process_lock::CrossProcessLockConfig,
    encryption::{BackupDownloadStrategy, EncryptionSettings, verification::VerificationRequest},
    event_handler::EventHandlerDropGuard,
    ruma::{OwnedDeviceId, OwnedUserId, serde::Raw},
    store::RoomLoadSettings,
    utils::{
        UrlOrQuery,
        local_server::{LocalServerBuilder, LocalServerResponse},
    },
};
use matrix_sdk_ui::sync_service::SyncService;
use tokio::sync::broadcast;
use tracing::warn;
use url::Url;

use crate::{
    AbortOnDrop, Error, Profile, Result,
    profile::{StoredAuth, StoredSession},
    scheduled::Schedule,
};

const DEVICE_DISPLAY_NAME: &str = "Mtrx";
/// The latest events synced for every room (the SDK's default is 1): a
/// room's messages since the previous sync, so that each can notify, and
/// its latest ones before it is opened.
const ROOM_LIST_TIMELINE_LIMIT: u32 = 10;
/// Shown by the server's login page as the app's home page.
const CLIENT_URI: &str = "https://alex-robinson.github.io/";
/// Where the server sends the browser back to the mobile apps after a
/// browser login ([`Session::start_oauth_login`]): a private-use URI scheme
/// (RFC 8252), which servers require to be [`CLIENT_URI`]'s host reversed.
/// The apps register the scheme with the OS.
pub const APP_OAUTH_REDIRECT_URI: &str = "io.github.alex-robinson:/oauth";
const BROWSER_DONE_PAGE: &str = "<!doctype html><title>Mtrx</title>\
    <p style=\"font-family: sans-serif\">Signed in. You can close this tab and return to Mtrx.</p>";

/// A logged-in Matrix account bound to a [`Profile`].
pub struct Session {
    pub(crate) client: Client,
    pub(crate) sync: Arc<SyncService>,
    pub(crate) verification_tx: broadcast::Sender<VerificationRequest>,
    pub(crate) room_keys: Arc<crate::encryption::RoomKeys>,
    /// Enqueues messages, see [`crate::timeline::send_message`].
    pub(crate) outbox: Arc<crate::timeline::Outbox>,
    pub(crate) schedule: Arc<Schedule>,
    pub(crate) downloads: crate::downloads::Downloads,
    /// Set once [`Session::start_scheduler`] ran.
    pub(crate) scheduler: std::sync::Mutex<Option<AbortOnDrop>>,
    pub(crate) notifications: broadcast::Sender<crate::Notification>,
    _notifier: EventHandlerDropGuard,
    pub(crate) categories: crate::account_data::Synced<crate::RoomCategories>,
    pub(crate) colours: crate::account_data::Synced<crate::RoomListColours>,
    pub(crate) directory_servers: crate::account_data::Synced<crate::DirectoryServers>,
    pub(crate) drafts: crate::drafts::Drafts,
    /// Changes made here, until the room list, timelines and settings have
    /// them.
    pub(crate) echoes: Arc<crate::echo::Echoes>,
    /// The joined spaces and what they list, as last synced.
    pub(crate) spaces: crate::Live<Arc<crate::SpaceGraph>>,
    /// Each space's rooms as its server last listed them.
    pub(crate) listings: crate::directory::Listings,
    pub(crate) connection: tokio::sync::watch::Receiver<crate::Connection>,
    /// Kept for the session: it follows push rule changes, so it knows a
    /// room's mode right after setting it (sync echoes it later).
    pub(crate) notification_settings: matrix_sdk::notification_settings::NotificationSettings,
    /// Changes to rooms' modes, one after another in each room.
    pub(crate) notify_turns: crate::room_settings::NotifyTurns,
    /// Sets our presence and keeps it, see [`Session::set_presence`].
    pub(crate) presence_keeper: crate::presence::Keeper,
    pub(crate) own_presence: crate::OwnPresence,
    profile: Profile,
    _connection: AbortOnDrop,
}

impl Session {
    /// Log in with a password and persist the session in `profile`.
    ///
    /// `homeserver` may be a server name (`matrix.org`) or a URL.
    pub async fn login(
        profile: Profile,
        homeserver: &str,
        user: &str,
        password: &str,
    ) -> Result<Self> {
        let (login, ()) = NewLogin::start(profile, homeserver)
            .await?
            .step(async |login| {
                login
                    .client
                    .matrix_auth()
                    .login_username(user, password)
                    .initial_device_display_name(DEVICE_DISPLAY_NAME)
                    .await?;
                Ok(())
            })
            .await?;
        login.finish().await
    }

    /// Log in through the server's web page (OAuth 2.0, e.g. matrix.org).
    /// `open_url` must show the given URL to the user, normally in their
    /// browser; the login completes when the browser is redirected back to a
    /// temporary local web server. Dropping the future cancels the login.
    pub async fn login_oauth(
        profile: Profile,
        homeserver: &str,
        open_url: impl FnOnce(Url),
    ) -> Result<Self> {
        let (redirect_uri, redirect) = LocalServerBuilder::new()
            .response(LocalServerResponse::Html(BROWSER_DONE_PAGE.into()))
            .spawn()
            .await?;
        let login = Self::start_oauth_login(profile, homeserver, redirect_uri).await?;
        open_url(login.url().clone());
        match redirect.await {
            Some(query) => login.finish_with(UrlOrQuery::Query(query.0)).await,
            None => {
                login.login.abandon()?;
                Err(Error::LoginCancelled)
            }
        }
    }

    /// Start a browser login (OAuth 2.0) that the caller completes, for apps
    /// that receive the redirect themselves (the mobile apps, with
    /// [`APP_OAUTH_REDIRECT_URI`]): show [`OAuthLogin::url`] in a browser,
    /// then pass the URL it was redirected to to [`OAuthLogin::finish`].
    pub async fn start_oauth_login(
        profile: Profile,
        homeserver: &str,
        redirect_uri: Url,
    ) -> Result<OAuthLogin> {
        let (login, url) = NewLogin::start(profile, homeserver)
            .await?
            .step(async |login| {
                let authorization = login
                    .client
                    .oauth()
                    .login(
                        redirect_uri.clone(),
                        None,
                        Some(client_metadata(redirect_uri)?.into()),
                        None,
                    )
                    .build()
                    .await?;
                Ok(authorization.url)
            })
            .await?;
        Ok(OAuthLogin { login, url })
    }

    /// Whether the server offers [`Session::login_oauth`].
    pub async fn supports_oauth(homeserver: &str) -> Result<bool> {
        let client = client_builder(homeserver).build().await?;
        Ok(client.oauth().server_metadata().await.is_ok())
    }

    /// Reopen the session previously saved in `profile`.
    pub async fn restore(profile: Profile) -> Result<Self> {
        let client = restore_client(&profile, RoomLoadSettings::All, app_encryption()).await?;
        client
            .encryption()
            .wait_for_e2ee_initialization_tasks()
            .await;
        Self::new(client, profile).await
    }

    async fn new(client: Client, profile: Profile) -> Result<Self> {
        let sync = Arc::new(
            SyncService::builder(client.clone())
                .with_offline_mode()
                .with_room_list_timeline_limit(ROOM_LIST_TIMELINE_LIMIT)
                .build()
                .await
                .map_err(|e| Error::Sync(e.to_string()))?,
        );
        let echoes = Arc::new(crate::echo::Echoes::default());
        let (connection, watch_connection) =
            crate::connection::watch(client.clone(), sync.clone(), echoes.clone());
        let verification_tx = crate::encryption::register_handlers(&client);
        let notification_settings = client.notification_settings().await;
        let (notifications, notifier) =
            crate::notifications::register(&client, sync.clone(), notification_settings.clone());
        let outbox = Arc::new(crate::timeline::Outbox::default());
        let schedule = Arc::new(Schedule::load(client.clone(), outbox.clone()).await?);
        let categories = crate::account_data::Synced::load(client.clone()).await;
        let colours = crate::account_data::Synced::load(client.clone()).await;
        let directory_servers = crate::account_data::Synced::load(client.clone()).await;
        let drafts = crate::drafts::Drafts::load(&client).await;
        let downloads = crate::downloads::Downloads::load(
            client.clone(),
            crate::downloads::Places::new(profile.cache_dir()),
        )
        .await;
        let presence_keeper = crate::presence::Keeper::new(client.clone());
        let spaces = crate::space_graph::follow(client.clone()).await;
        Ok(Self {
            client,
            sync,
            verification_tx,
            room_keys: Default::default(),
            outbox,
            schedule,
            downloads,
            scheduler: Default::default(),
            notifications,
            _notifier: notifier,
            categories,
            colours,
            directory_servers,
            drafts,
            echoes,
            spaces,
            listings: Default::default(),
            connection,
            notification_settings,
            notify_turns: Default::default(),
            presence_keeper: presence_keeper.clone(),
            own_presence: crate::presence::own_presence(presence_keeper),
            profile,
            _connection: watch_connection,
        })
    }

    /// Start syncing in the background. Live values ([`Session::room_list`],
    /// ...) only update while sync runs.
    pub async fn start_sync(&self) {
        self.sync.start().await;
    }

    /// Stop syncing and persist the sync position.
    pub async fn stop_sync(&self) {
        self.sync.stop().await;
    }

    /// Log out on the server and delete all local data of the profile.
    ///
    /// Local data is deleted even if the server is unreachable or the token
    /// was already revoked. The session must not be used afterwards.
    pub async fn logout(&self) -> Result<()> {
        if let Err(e) = self.client.logout().await {
            warn!("server-side logout failed, removing local data anyway: {e}");
        }
        self.forget().await
    }

    /// Delete all local data of the profile, for a session the server no
    /// longer accepts (see [`crate::Connection::SignedOut`]), so it can be
    /// logged in again. The session must not be used afterwards.
    pub async fn forget(&self) -> Result<()> {
        self.sync.stop().await;
        self.profile.forget_session()?;
        // Store files may still be open (e.g. on Windows they then cannot be
        // deleted); the next login removes anything left behind.
        if let Err(e) = self.profile.remove_data() {
            warn!("could not delete all local data: {e}");
        }
        Ok(())
    }

    pub fn profile(&self) -> &Profile {
        &self.profile
    }

    pub fn user_id(&self) -> OwnedUserId {
        self.client
            .user_id()
            .expect("session is logged in")
            .to_owned()
    }

    pub fn device_id(&self) -> OwnedDeviceId {
        self.client
            .device_id()
            .expect("session is logged in")
            .to_owned()
    }

    pub fn homeserver(&self) -> String {
        self.client.homeserver().to_string()
    }
}

/// A browser login waiting for the redirect, see [`Session::start_oauth_login`].
/// Dropping it cancels the login; the next login deletes what it stored.
pub struct OAuthLogin {
    login: NewLogin,
    url: Url,
}

impl OAuthLogin {
    /// The server's login page, to show in a browser.
    pub fn url(&self) -> &Url {
        &self.url
    }

    /// Complete the login with the URL the browser was redirected to, and
    /// persist the session in the profile.
    pub async fn finish(self, redirect: &Url) -> Result<Session> {
        self.finish_with(UrlOrQuery::Url(redirect.clone())).await
    }

    async fn finish_with(self, redirect: UrlOrQuery) -> Result<Session> {
        let (login, ()) = self
            .login
            .step(async move |login| {
                match login.client.oauth().finish_login(redirect).await {
                    // The user declined on the server's page.
                    Err(matrix_sdk::Error::OAuth(error))
                        if matches!(
                            *error,
                            OAuthError::AuthorizationCode(OAuthAuthorizationCodeError::Cancelled)
                        ) =>
                    {
                        Err(Error::LoginCancelled)
                    }
                    result => Ok(result?),
                }
            })
            .await?;
        login.finish().await
    }
}

/// A login in progress: a client with a fresh store, not yet saved in the
/// profile.
struct NewLogin {
    profile: Profile,
    client: Client,
    store_passphrase: String,
}

impl NewLogin {
    async fn start(profile: Profile, homeserver: &str) -> Result<Self> {
        if profile.load_session()?.is_some() {
            return Err(Error::AlreadyLoggedIn(profile.name().to_owned()));
        }
        // Leftovers from an earlier failed login would not open with a new passphrase.
        profile.wipe()?;
        let store_passphrase = random_passphrase();
        match build_client(&profile, homeserver, &store_passphrase, app_encryption()).await {
            Ok(client) => Ok(Self {
                profile,
                client,
                store_passphrase,
            }),
            Err(e) => {
                profile.wipe()?;
                Err(e)
            }
        }
    }

    /// Run a step of the login; if it fails, delete what the login stored.
    async fn step<T>(self, step: impl AsyncFnOnce(&Self) -> Result<T>) -> Result<(Self, T)> {
        match step(&self).await {
            Ok(value) => Ok((self, value)),
            Err(e) => {
                self.abandon()?;
                Err(e)
            }
        }
    }

    fn abandon(self) -> Result<()> {
        let Self {
            profile, client, ..
        } = self;
        drop(client);
        profile.wipe()
    }

    /// Persist the session in the profile.
    async fn finish(self) -> Result<Session> {
        let (login, ()) = self
            .step(async |login| {
                // Cross-signing and backup setup run in the background; finish
                // them before the caller can exit or start syncing.
                login
                    .client
                    .encryption()
                    .wait_for_e2ee_initialization_tasks()
                    .await;
                login.profile.save_session(&StoredSession {
                    homeserver: login.client.homeserver().to_string(),
                    auth: stored_auth(&login.client),
                    store_passphrase: login.store_passphrase.clone(),
                })
            })
            .await?;
        Session::new(login.client, login.profile).await
    }
}

fn client_builder(homeserver: &str) -> matrix_sdk::ClientBuilder {
    let builder = Client::builder();
    // A full URL is used as-is; a bare server name goes through .well-known discovery.
    if homeserver.contains("://") {
        builder.homeserver_url(homeserver)
    } else {
        builder.server_name_or_homeserver_url(homeserver)
    }
}

/// A client of the session saved in `profile`, signed in, not syncing.
/// `rooms` limits the rooms loaded from the store (a push needs one).
pub(crate) async fn restore_client(
    profile: &Profile,
    rooms: RoomLoadSettings,
    encryption: EncryptionSettings,
) -> Result<Client> {
    let stored = profile
        .load_session()?
        .ok_or_else(|| Error::NotLoggedIn(profile.name().to_owned()))?;
    let client = build_client(
        profile,
        &stored.homeserver,
        &stored.store_passphrase,
        encryption,
    )
    .await?;
    match stored.auth {
        StoredAuth::Password(session) => {
            client.matrix_auth().restore_session(session, rooms).await?
        }
        StoredAuth::OAuth { client_id, session } => {
            let session = OAuthSession {
                client_id: ClientId::new(client_id),
                user: session,
            };
            client.oauth().restore_session(session, rooms).await?
        }
    }
    Ok(client)
}

/// The app's encryption: cross-signing and the key backup set up, keys
/// fetched from the backup when a message cannot be decrypted.
fn app_encryption() -> EncryptionSettings {
    EncryptionSettings {
        auto_enable_cross_signing: true,
        backup_download_strategy: BackupDownloadStrategy::AfterDecryptionFailure,
        auto_enable_backups: true,
    }
}

async fn build_client(
    profile: &Profile,
    homeserver: &str,
    passphrase: &str,
    encryption: EncryptionSettings,
) -> Result<Client> {
    let holder = lock_holder();
    let client = client_builder(homeserver)
        .sqlite_store_with_cache_path(
            profile.store_dir(),
            profile.cache_store_dir(),
            Some(passphrase),
        )
        .cross_process_store_config(CrossProcessLockConfig::multi_process(holder.clone()))
        .handle_refresh_tokens()
        .with_encryption_settings(encryption)
        .build()
        .await?;
    share_tokens(&client, profile, holder).await?;
    Ok(client)
}

/// The name under which a client takes the locks in the store it shares
/// with other clients of the profile: in other processes (the app and the
/// CLI; a mobile app and its push handler) or in this one (a push handled
/// next to the app's session). Each needs its own, or the locks would let
/// them in together.
fn lock_holder() -> String {
    static COUNT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let program = std::env::current_exe()
        .ok()
        .and_then(|path| Some(path.file_stem()?.to_string_lossy().into_owned()))
        .unwrap_or_else(|| "mtrx".to_owned());
    let n = COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    format!("{program}-{}-{n}", std::process::id())
}

/// Every refresh replaces the refresh token, so the clients using a
/// profile must not refresh at once, nor lose a new token: refreshes take
/// a lock in the store they share, new tokens are saved before a refresh
/// completes, and the other clients reload them.
async fn share_tokens(client: &Client, profile: &Profile, holder: String) -> Result<()> {
    client
        .oauth()
        .enable_cross_process_refresh_lock(holder)
        .await?;
    let (saved, to_save) = (profile.clone(), profile.clone());
    client.set_session_callbacks(
        Box::new(move |_| {
            let stored = saved
                .load_session()?
                .ok_or(Error::NotLoggedIn(saved.name().to_owned()))?;
            Ok(stored.auth.tokens())
        }),
        Box::new(move |client| {
            // While logging in nothing is saved yet; the login saves it all.
            if let Some(mut stored) = to_save.load_session()? {
                stored.auth = stored_auth(&client);
                to_save.save_session(&stored)?;
            }
            Ok(())
        }),
    )?;
    Ok(())
}

fn stored_auth(client: &Client) -> StoredAuth {
    match client.oauth().full_session() {
        Some(session) => StoredAuth::OAuth {
            client_id: session.client_id.as_str().to_owned(),
            session: session.user,
        },
        None => StoredAuth::Password(client.matrix_auth().session().expect("logged in")),
    }
}

/// How we present ourselves to the server's login page.
fn client_metadata(redirect_uri: Url) -> Result<Raw<ClientMetadata>> {
    let client_uri = Url::parse(CLIENT_URI).expect("valid URL");
    let mut metadata = ClientMetadata::new(
        ApplicationType::Native,
        vec![OAuthGrantType::AuthorizationCode {
            redirect_uris: vec![redirect_uri],
        }],
        Localized::new(client_uri, None),
    );
    metadata.client_name = Some(Localized::new(DEVICE_DISPLAY_NAME.to_owned(), None));
    Ok(Raw::new(&metadata)?)
}

fn random_passphrase() -> String {
    let bytes: [u8; 32] = rand::random();
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_client_takes_the_store_locks_under_its_own_name() {
        let (one, two) = (lock_holder(), lock_holder());
        assert_ne!(one, two);
        let pid = format!("-{}-", std::process::id());
        assert!(one.contains(&pid), "{one}");
    }

    #[test]
    fn app_redirect_scheme_is_the_client_uri_host_reversed() {
        let redirect = Url::parse(APP_OAUTH_REDIRECT_URI).unwrap();
        let host = Url::parse(CLIENT_URI)
            .unwrap()
            .host_str()
            .unwrap()
            .to_owned();
        let reversed: Vec<_> = redirect.scheme().split('.').rev().collect();
        assert_eq!(reversed.join("."), host);
        assert!(
            redirect.host().is_none(),
            "private-use scheme URIs have no authority"
        );
        assert!(client_metadata(redirect).is_ok());
    }
}
