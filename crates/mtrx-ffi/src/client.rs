//! The library's entry point: signing in, and restoring a saved session.

use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
};

use mtrx_core::{self as core, Profile, SecretStore, Url};

use crate::{
    MtrxError, Notification, SecretBackend, Session,
    runtime::{InRuntime, run},
    secrets::AppSecrets,
};

/// One account per app for now.
const PROFILE: &str = "default";

/// The library, set up with the app's directories and secret storage.
/// Create one at app start and keep it.
#[derive(uniffi::Object)]
pub struct Mtrx {
    data_dir: PathBuf,
    cache_dir: PathBuf,
    secrets: SecretStore,
}

#[uniffi::export]
impl Mtrx {
    /// `data_dir` keeps the encrypted store: private and persistent (iOS:
    /// Application Support; Android: `Context.filesDir`). `cache_dir` keeps
    /// caches and downloaded media, which the OS may delete (iOS: Caches;
    /// Android: `Context.cacheDir`). Both are created as needed.
    #[uniffi::constructor]
    pub fn new(data_dir: String, cache_dir: String, secrets: Arc<dyn SecretBackend>) -> Arc<Self> {
        Arc::new(Self {
            data_dir: data_dir.into(),
            cache_dir: cache_dir.into(),
            secrets: SecretStore::Custom(Arc::new(AppSecrets(secrets))),
        })
    }

    /// The saved session, or `None` if not signed in. Sync is not started.
    pub async fn restore(&self) -> Result<Option<Arc<Session>>, MtrxError> {
        let profile = self.profile()?;
        run(async move {
            match core::Session::restore(profile).await {
                Ok(session) => Ok(Some(Session::new(session))),
                Err(core::Error::NotLoggedIn(_)) => Ok(None),
                Err(error) => Err(error.into()),
            }
        })
        .await
    }

    /// Sign in with a password. `homeserver` is a server name
    /// (`matrix.org`) or a URL.
    pub async fn login(
        &self,
        homeserver: String,
        user: String,
        password: String,
    ) -> Result<Arc<Session>, MtrxError> {
        let profile = self.profile()?;
        run(async move {
            let session = core::Session::login(profile, &homeserver, &user, &password).await?;
            Ok(Session::new(session))
        })
        .await
    }

    /// Whether the server offers browser sign-in ([`Mtrx::start_oauth_login`]).
    pub async fn supports_oauth(&self, homeserver: String) -> Result<bool, MtrxError> {
        run(async move { Ok(core::Session::supports_oauth(&homeserver).await?) }).await
    }

    /// What the push for `event_id` in `room_id` is about, read with the
    /// saved session without syncing: the event, decrypted, as the push
    /// rules judge it. For the process that receives the push, which may
    /// run next to the app's session (syncing in this process or another)
    /// or alone; the store's locks keep them apart. Calls in a process run
    /// one at a time. `None` if there is nothing to show (our own message,
    /// a muted room, an ignored sender, a deleted event); otherwise post a
    /// notification. Fails with `NotLoggedIn` without a saved session,
    /// `Other` when the event cannot be fetched.
    pub async fn notification(
        &self,
        room_id: String,
        event_id: String,
    ) -> Result<Option<Notification>, MtrxError> {
        crate::push::notification(self.profile()?, room_id, event_id).await
    }

    /// Start a browser sign-in (OAuth 2.0): open [`OAuthLogin::url`] in a
    /// browser session that returns to [`crate::oauth_redirect_uri`], then
    /// call [`OAuthLogin::finish`] with the URL it returned to.
    pub async fn start_oauth_login(
        &self,
        homeserver: String,
    ) -> Result<Arc<OAuthLogin>, MtrxError> {
        let profile = self.profile()?;
        let redirect = Url::parse(core::APP_OAUTH_REDIRECT_URI).expect("valid redirect URI");
        run(async move {
            let login = core::Session::start_oauth_login(profile, &homeserver, redirect).await?;
            Ok(Arc::new(OAuthLogin {
                url: login.url().to_string(),
                login: Mutex::new(Some(InRuntime::new(login))),
            }))
        })
        .await
    }
}

impl Mtrx {
    fn profile(&self) -> Result<Profile, MtrxError> {
        Ok(Profile::new(PROFILE, &self.data_dir, self.secrets.clone())?
            .with_cache_root(&self.cache_dir))
    }
}

/// A browser sign-in waiting for the browser to return. Dropping it
/// cancels the sign-in.
#[derive(uniffi::Object)]
pub struct OAuthLogin {
    url: String,
    /// Taken by [`OAuthLogin::finish`].
    login: Mutex<Option<InRuntime<core::OAuthLogin>>>,
}

#[uniffi::export]
impl OAuthLogin {
    /// The server's sign-in page.
    pub fn url(&self) -> String {
        self.url.clone()
    }

    /// Complete the sign-in with the URL the browser returned to (it starts
    /// with [`crate::oauth_redirect_uri`]). Only the first call proceeds.
    pub async fn finish(&self, redirect_url: String) -> Result<Arc<Session>, MtrxError> {
        let redirect = Url::parse(&redirect_url).map_err(MtrxError::invalid)?;
        let login = self
            .login
            .lock()
            .expect("not poisoned")
            .take()
            .ok_or_else(|| MtrxError::invalid("this sign-in was already finished"))?;
        run(async move { Ok(Session::new(login.into_inner().finish(&redirect).await?)) }).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::secrets::tests::Memory;

    fn block_on<T>(future: impl Future<Output = T>) -> T {
        futures_executor::block_on(future)
    }

    #[test]
    fn restore_without_a_session_is_none() {
        let (data, cache) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let mtrx = Mtrx::new(
            data.path().to_string_lossy().into(),
            cache.path().to_string_lossy().into(),
            Arc::new(Memory::default()),
        );
        assert!(block_on(mtrx.restore()).unwrap().is_none());
    }

    #[test]
    fn a_push_without_a_session_is_not_logged_in() {
        let (data, cache) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let mtrx = Mtrx::new(
            data.path().to_string_lossy().into(),
            cache.path().to_string_lossy().into(),
            Arc::new(Memory::default()),
        );
        let push = |room: &str, event: &str| block_on(mtrx.notification(room.into(), event.into()));
        assert!(matches!(
            push("!r:example.org", "$e:example.org"),
            Err(MtrxError::NotLoggedIn)
        ));
        assert!(matches!(
            push("r", "$e:example.org"),
            Err(MtrxError::InvalidInput { .. })
        ));
        assert!(matches!(
            push("!r:example.org", "e"),
            Err(MtrxError::InvalidInput { .. })
        ));
    }

    #[test]
    fn profile_uses_both_directories() {
        let mtrx = Mtrx::new("/data".into(), "/cache".into(), Arc::new(Memory::default()));
        let profile = mtrx.profile().unwrap();
        assert_eq!(
            profile.dir(),
            std::path::Path::new("/data/profiles/default")
        );
        assert_eq!(
            profile.cache_dir(),
            std::path::Path::new("/cache/profiles/default")
        );
    }
}
