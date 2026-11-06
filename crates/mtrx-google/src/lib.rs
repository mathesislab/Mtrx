//! Google Meet links: Google sign-in (OAuth 2.0 with PKCE) and the Meet REST
//! API.
//!
//! Uses the user's own OAuth client of type "Desktop app"; Mtrx ships no
//! Google credentials. The connection is app-wide, not per profile.

use std::{
    path::Path,
    time::{Duration, Instant},
};

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use matrix_sdk::utils::local_server::{
    LocalServerBuilder, LocalServerIpAddress, LocalServerResponse,
};
use mtrx_core::{Secret, SecretStore};
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::sync::Mutex;
use tracing::warn;
use url::Url;

const SCOPES: &str = "https://www.googleapis.com/auth/meetings.space.created openid email";
const SECRETS_FILE: &str = "google.json";
/// Access tokens are refreshed this long before they expire.
const EXPIRY_MARGIN: Duration = Duration::from_secs(60);
const BROWSER_DONE_PAGE: &str = "<!doctype html><title>Mtrx</title>\
    <p style=\"font-family: sans-serif\">Signed in to Google. You can close this tab and return to Mtrx.</p>";

pub type Result<T, E = Error> = std::result::Result<T, E>;

/// What failed: signing in, or making a Meet link.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("not signed in to Google")]
    NotSignedIn,
    #[error("Google sign-in expired or was revoked; sign in to Google again")]
    SignInExpired,
    /// Google's answer, or our side of it (the local server, the stored secret).
    #[error("Google: {0}")]
    Other(String),
}

impl From<mtrx_core::Error> for Error {
    fn from(e: mtrx_core::Error) -> Self {
        Self::Other(e.to_string())
    }
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Self::Other(e.to_string())
    }
}

impl From<serde_json::Error> for Error {
    fn from(e: serde_json::Error) -> Self {
        Self::Other(e.to_string())
    }
}

/// An OAuth client the user created in their Google Cloud project
/// (type "Desktop app", with the Google Meet REST API enabled).
#[derive(Clone, Debug)]
pub struct GoogleClient {
    pub client_id: String,
    pub client_secret: String,
}

/// Connection to a Google account for creating Meet links.
pub struct Google {
    secret: Secret,
    endpoints: Endpoints,
    http: reqwest::Client,
    /// Cached access token; also serializes refreshes.
    access: Mutex<Option<AccessToken>>,
}

/// Google's endpoints, replaced by a local mock server in tests.
#[derive(Clone, Debug)]
struct Endpoints {
    auth: Url,
    token: Url,
    revoke: Url,
    spaces: Url,
}

impl Default for Endpoints {
    fn default() -> Self {
        let url = |s: &str| Url::parse(s).expect("valid URL");
        Self {
            auth: url("https://accounts.google.com/o/oauth2/v2/auth"),
            token: url("https://oauth2.googleapis.com/token"),
            revoke: url("https://oauth2.googleapis.com/revoke"),
            spaces: url("https://meet.googleapis.com/v2/spaces"),
        }
    }
}

/// Everything needed to get access tokens, stored as one secret.
#[derive(Serialize, Deserialize)]
struct Stored {
    client_id: String,
    client_secret: String,
    refresh_token: String,
    email: String,
}

struct AccessToken {
    token: String,
    expires: Instant,
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    expires_in: u64,
    refresh_token: Option<String>,
    id_token: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Space {
    meeting_uri: String,
}

/// Error body of the OAuth endpoints or the Meet API.
#[derive(Deserialize)]
#[serde(untagged)]
enum ErrorBody {
    OAuth {
        error: String,
        error_description: Option<String>,
    },
    Api {
        error: ApiError,
    },
}

#[derive(Deserialize)]
struct ApiError {
    message: String,
}

/// PKCE (RFC 7636) code verifier and its S256 challenge.
struct Pkce {
    verifier: String,
    challenge: String,
}

impl Google {
    /// The Google connection stored under `data_root`.
    pub fn new(data_root: &Path, secrets: SecretStore) -> Self {
        Self::with_endpoints(data_root, secrets, Endpoints::default())
    }

    fn with_endpoints(data_root: &Path, secrets: SecretStore, endpoints: Endpoints) -> Self {
        let secret = Secret::new(
            secrets,
            format!("google:{}", data_root.display()),
            "google".into(),
            data_root.join(SECRETS_FILE),
        );
        Self {
            secret,
            endpoints,
            http: reqwest::Client::new(),
            access: Mutex::new(None),
        }
    }

    /// Signed-in account email, if any.
    pub fn account(&self) -> Result<Option<String>> {
        Ok(self.load()?.map(|stored| stored.email))
    }

    /// Sign in through Google's web page, replacing any earlier sign-in.
    /// `open_url` must show the given URL to the user, normally in their
    /// browser; sign-in completes when the browser is redirected back to a
    /// temporary local web server. Dropping the future cancels. Returns the
    /// account email.
    pub async fn sign_in(
        &self,
        client: GoogleClient,
        open_url: impl FnOnce(Url),
    ) -> Result<String> {
        let (redirect_uri, redirect) = LocalServerBuilder::new()
            .ip_address(LocalServerIpAddress::Localhostv4)
            .response(LocalServerResponse::Html(BROWSER_DONE_PAGE.into()))
            .spawn()
            .await?;
        let pkce = Pkce::new();
        let state = random_token();
        open_url(self.auth_url(&client.client_id, &redirect_uri, &pkce.challenge, &state));
        let query = redirect
            .await
            .ok_or_else(|| Error::Other("sign-in was cancelled".into()))?;
        let code = authorization_code(&query.0, &state)?;

        let tokens = self
            .request_token(&[
                ("grant_type", "authorization_code"),
                ("code", &code),
                ("code_verifier", &pkce.verifier),
                ("client_id", &client.client_id),
                ("client_secret", &client.client_secret),
                ("redirect_uri", redirect_uri.as_str()),
            ])
            .await?;
        let refresh_token = tokens
            .refresh_token
            .ok_or_else(|| Error::Other("no refresh token in the sign-in response".into()))?;
        let id_token = tokens
            .id_token
            .ok_or_else(|| Error::Other("no ID token in the sign-in response".into()))?;
        let email = email_from_id_token(&id_token)?;
        let previous = self.load()?;
        self.save(&Stored {
            client_id: client.client_id,
            client_secret: client.client_secret,
            refresh_token,
            email: email.clone(),
        })?;
        *self.access.lock().await = Some(AccessToken::new(tokens.access_token, tokens.expires_in));
        // A replaced sign-in should not stay valid at Google.
        if let Some(previous) = previous
            && let Err(e) = self.revoke(&previous.refresh_token).await
        {
            warn!("revoking the previous Google token failed: {e}");
        }
        Ok(email)
    }

    /// Revoke the tokens at Google (best effort) and forget them.
    pub async fn sign_out(&self) -> Result<()> {
        if let Some(stored) = self.load()?
            && let Err(e) = self.revoke(&stored.refresh_token).await
        {
            warn!("revoking the Google token failed, forgetting it anyway: {e}");
        }
        self.access.lock().await.take();
        Ok(self.secret.delete()?)
    }

    /// A new Meet link.
    pub async fn create_meeting(&self) -> Result<Url> {
        let response = match self.post_space().await? {
            // The cached token was revoked; a refresh tells whether the
            // sign-in still holds.
            response if response.status() == StatusCode::UNAUTHORIZED => {
                self.access.lock().await.take();
                self.post_space().await?
            }
            response => response,
        };
        let space: Space = checked(response).await?.json().await.map_err(http_error)?;
        Url::parse(&space.meeting_uri)
            .map_err(|e| Error::Other(format!("invalid meeting link `{}`: {e}", space.meeting_uri)))
    }

    async fn post_space(&self) -> Result<reqwest::Response> {
        let token = self.access_token().await?;
        self.http
            .post(self.endpoints.spaces.clone())
            .bearer_auth(token)
            .json(&serde_json::json!({}))
            .send()
            .await
            .map_err(http_error)
    }

    /// A valid access token, refreshed if needed. A revoked or expired
    /// sign-in is forgotten.
    async fn access_token(&self) -> Result<String> {
        let mut access = self.access.lock().await;
        if let Some(cached) = &*access
            && Instant::now() + EXPIRY_MARGIN < cached.expires
        {
            return Ok(cached.token.clone());
        }
        let mut stored = self.load()?.ok_or(Error::NotSignedIn)?;
        let tokens = match self
            .request_token(&[
                ("grant_type", "refresh_token"),
                ("refresh_token", &stored.refresh_token),
                ("client_id", &stored.client_id),
                ("client_secret", &stored.client_secret),
            ])
            .await
        {
            Err(Error::SignInExpired) => {
                *access = None;
                self.secret.delete()?;
                return Err(Error::SignInExpired);
            }
            result => result?,
        };
        if let Some(refresh_token) = tokens.refresh_token {
            stored.refresh_token = refresh_token;
            self.save(&stored)?;
        }
        let token = tokens.access_token.clone();
        *access = Some(AccessToken::new(tokens.access_token, tokens.expires_in));
        Ok(token)
    }

    /// POST `form` to the token endpoint. `invalid_grant` (revoked or
    /// expired grant) gives [`Error::SignInExpired`].
    async fn request_token(&self, form: &[(&str, &str)]) -> Result<TokenResponse> {
        let response = self
            .http
            .post(self.endpoints.token.clone())
            .form(form)
            .send()
            .await
            .map_err(http_error)?;
        let status = response.status();
        if status.is_success() {
            return response.json().await.map_err(http_error);
        }
        let body = response.text().await.unwrap_or_default();
        match serde_json::from_str(&body) {
            Ok(ErrorBody::OAuth { error, .. }) if error == "invalid_grant" => {
                Err(Error::SignInExpired)
            }
            _ => Err(Error::Other(error_message(status, &body))),
        }
    }

    async fn revoke(&self, token: &str) -> Result<()> {
        let response = self
            .http
            .post(self.endpoints.revoke.clone())
            .form(&[("token", token)])
            .send()
            .await
            .map_err(http_error)?;
        checked(response).await.map(drop)
    }

    fn auth_url(&self, client_id: &str, redirect_uri: &Url, challenge: &str, state: &str) -> Url {
        let mut url = self.endpoints.auth.clone();
        url.query_pairs_mut()
            .append_pair("client_id", client_id)
            .append_pair("redirect_uri", redirect_uri.as_str())
            .append_pair("response_type", "code")
            .append_pair("scope", SCOPES)
            .append_pair("code_challenge", challenge)
            .append_pair("code_challenge_method", "S256")
            .append_pair("state", state)
            .append_pair("access_type", "offline")
            .append_pair("prompt", "consent");
        url
    }

    fn load(&self) -> Result<Option<Stored>> {
        match self.secret.load()? {
            Some(json) => Ok(Some(serde_json::from_str(&json)?)),
            None => Ok(None),
        }
    }

    fn save(&self, stored: &Stored) -> Result<()> {
        Ok(self.secret.save(&serde_json::to_string(stored)?)?)
    }
}

impl AccessToken {
    fn new(token: String, expires_in: u64) -> Self {
        Self {
            token,
            expires: Instant::now() + Duration::from_secs(expires_in),
        }
    }
}

impl Pkce {
    fn new() -> Self {
        let verifier = URL_SAFE_NO_PAD.encode(rand::random::<[u8; 32]>());
        Self {
            challenge: s256(&verifier),
            verifier,
        }
    }
}

fn s256(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

fn random_token() -> String {
    URL_SAFE_NO_PAD.encode(rand::random::<[u8; 16]>())
}

/// The `code` of the redirect's query, after checking `state`.
fn authorization_code(query: &str, state: &str) -> Result<String> {
    let param = |name: &str| {
        url::form_urlencoded::parse(query.as_bytes())
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.into_owned())
    };
    if let Some(error) = param("error") {
        return Err(Error::Other(format!("sign-in failed: {error}")));
    }
    if param("state").as_deref() != Some(state) {
        return Err(Error::Other(
            "sign-in response does not match the request".into(),
        ));
    }
    param("code")
        .ok_or_else(|| Error::Other("no authorization code in the sign-in response".into()))
}

/// The `email` claim. The token comes straight from Google's token endpoint
/// over TLS, so its signature is not checked.
fn email_from_id_token(id_token: &str) -> Result<String> {
    #[derive(Deserialize)]
    struct Claims {
        email: String,
    }
    let invalid = || Error::Other("invalid ID token in the sign-in response".into());
    let payload = id_token.split('.').nth(1).ok_or_else(invalid)?;
    let json = URL_SAFE_NO_PAD
        .decode(payload.trim_end_matches('='))
        .map_err(|_| invalid())?;
    let claims: Claims = serde_json::from_slice(&json).map_err(|_| invalid())?;
    Ok(claims.email)
}

/// `response` if successful, else an error with Google's message.
async fn checked(response: reqwest::Response) -> Result<reqwest::Response> {
    let status = response.status();
    if status.is_success() {
        return Ok(response);
    }
    let body = response.text().await.unwrap_or_default();
    Err(Error::Other(error_message(status, &body)))
}

fn error_message(status: StatusCode, body: &str) -> String {
    match serde_json::from_str(body) {
        Ok(ErrorBody::OAuth {
            error,
            error_description: Some(description),
        }) => format!("{error}: {description}"),
        Ok(ErrorBody::OAuth { error, .. }) => error,
        Ok(ErrorBody::Api { error }) => error.message,
        Err(_) => status.to_string(),
    }
}

fn http_error(e: reqwest::Error) -> Error {
    Error::Other(e.to_string())
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, sync::Arc};

    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::{TcpListener, TcpStream},
    };

    use super::*;

    const MEET_LINK: &str = "https://meet.google.com/abc-defg-hij";

    /// A request received by [`MockServer`].
    struct Request {
        path: String,
        head: String,
        body: String,
    }

    impl Request {
        fn form(&self) -> HashMap<String, String> {
            url::form_urlencoded::parse(self.body.as_bytes())
                .into_owned()
                .collect()
        }
    }

    /// Local HTTP server answering each path with a fixed status and body.
    struct MockServer {
        url: Url,
        requests: Arc<std::sync::Mutex<Vec<Request>>>,
    }

    impl MockServer {
        async fn start(routes: &[(&str, u16, &str)]) -> Self {
            let routes: Arc<Vec<(String, u16, String)>> = Arc::new(
                routes
                    .iter()
                    .map(|(path, status, body)| (path.to_string(), *status, body.to_string()))
                    .collect(),
            );
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let url = Url::parse(&format!("http://{}/", listener.local_addr().unwrap())).unwrap();
            let requests = Arc::new(std::sync::Mutex::new(Vec::new()));
            let log = requests.clone();
            tokio::spawn(async move {
                loop {
                    let (stream, _) = listener.accept().await.unwrap();
                    tokio::spawn(serve(stream, routes.clone(), log.clone()));
                }
            });
            Self { url, requests }
        }

        fn endpoints(&self) -> Endpoints {
            let url = |path: &str| self.url.join(path).unwrap();
            Endpoints {
                auth: url("auth"),
                token: url("token"),
                revoke: url("revoke"),
                spaces: url("v2/spaces"),
            }
        }

        /// Requests to `path`, in order.
        fn requests(&self, path: &str) -> Vec<Request> {
            let requests = self.requests.lock().unwrap();
            requests
                .iter()
                .filter(|r| r.path == path)
                .map(|r| Request {
                    path: r.path.clone(),
                    head: r.head.clone(),
                    body: r.body.clone(),
                })
                .collect()
        }
    }

    async fn serve(
        mut stream: TcpStream,
        routes: Arc<Vec<(String, u16, String)>>,
        requests: Arc<std::sync::Mutex<Vec<Request>>>,
    ) {
        let mut data = Vec::new();
        let mut buf = [0; 4096];
        let head_len = loop {
            let n = stream.read(&mut buf).await.unwrap();
            if n == 0 {
                return;
            }
            data.extend_from_slice(&buf[..n]);
            if let Some(i) = data.windows(4).position(|w| w == b"\r\n\r\n") {
                break i + 4;
            }
        };
        let head = String::from_utf8_lossy(&data[..head_len]).into_owned();
        let body_len = head
            .lines()
            .find_map(|line| {
                let line = line.to_ascii_lowercase();
                line.strip_prefix("content-length:")
                    .map(|n| n.trim().parse::<usize>().unwrap())
            })
            .unwrap_or(0);
        while data.len() < head_len + body_len {
            let n = stream.read(&mut buf).await.unwrap();
            data.extend_from_slice(&buf[..n]);
        }
        let path = head.split_whitespace().nth(1).unwrap().to_owned();
        let (status, reply) = routes
            .iter()
            .find(|(p, ..)| p.trim_start_matches('/') == path.trim_start_matches('/'))
            .map_or((404, ""), |(_, status, body)| (*status, body.as_str()));
        requests.lock().unwrap().push(Request {
            path,
            head,
            body: String::from_utf8_lossy(&data[head_len..]).into_owned(),
        });
        let response = format!(
            "HTTP/1.1 {status} Mock\r\ncontent-type: application/json\r\n\
             content-length: {}\r\nconnection: close\r\n\r\n{reply}",
            reply.len()
        );
        stream.write_all(response.as_bytes()).await.unwrap();
    }

    fn signed_in(dir: &Path, server: &MockServer) -> Google {
        let google = Google::with_endpoints(dir, SecretStore::File, server.endpoints());
        google
            .save(&Stored {
                client_id: "client".into(),
                client_secret: "secret".into(),
                refresh_token: "refresh1".into(),
                email: "alice@example.com".into(),
            })
            .unwrap();
        google
    }

    fn id_token(claims: &str) -> String {
        format!("e30.{}.sig", URL_SAFE_NO_PAD.encode(claims))
    }

    #[test]
    fn google_and_its_futures_are_send() {
        fn check<T: Send + Sync>() {}
        fn check_future<T: Send>(_: T) {}
        check::<Google>();
        let dir = tempfile::tempdir().unwrap();
        let google = Google::new(dir.path(), SecretStore::File);
        let client = GoogleClient {
            client_id: String::new(),
            client_secret: String::new(),
        };
        check_future(google.sign_in(client, |_| {}));
        check_future(google.sign_out());
        check_future(google.create_meeting());
    }

    #[tokio::test]
    async fn refreshes_token_and_creates_meeting() {
        let server = MockServer::start(&[
            (
                "/token",
                200,
                r#"{"access_token":"access1","expires_in":3600,"token_type":"Bearer"}"#,
            ),
            (
                "/v2/spaces",
                200,
                r#"{"name":"spaces/abc","meetingUri":"https://meet.google.com/abc-defg-hij"}"#,
            ),
        ])
        .await;
        let dir = tempfile::tempdir().unwrap();
        let google = signed_in(dir.path(), &server);

        assert_eq!(google.create_meeting().await.unwrap().as_str(), MEET_LINK);
        assert_eq!(google.create_meeting().await.unwrap().as_str(), MEET_LINK);

        let token_requests = server.requests("/token");
        assert_eq!(token_requests.len(), 1, "access token is cached");
        let form = token_requests[0].form();
        assert_eq!(form["grant_type"], "refresh_token");
        assert_eq!(form["refresh_token"], "refresh1");
        assert_eq!(form["client_id"], "client");
        assert_eq!(form["client_secret"], "secret");
        let spaces = server.requests("/v2/spaces");
        assert_eq!(spaces.len(), 2);
        assert!(
            spaces[0]
                .head
                .to_ascii_lowercase()
                .contains("authorization: bearer access1")
        );
        assert_eq!(spaces[0].body, "{}");
    }

    #[tokio::test]
    async fn invalid_grant_forgets_tokens() {
        let server = MockServer::start(&[(
            "/token",
            400,
            r#"{"error":"invalid_grant","error_description":"Token has been expired or revoked."}"#,
        )])
        .await;
        let dir = tempfile::tempdir().unwrap();
        let google = signed_in(dir.path(), &server);

        let error = google.create_meeting().await.unwrap_err();
        assert!(matches!(error, Error::SignInExpired), "{error}");
        assert_eq!(google.account().unwrap(), None);
        let error = google.create_meeting().await.unwrap_err();
        assert!(matches!(error, Error::NotSignedIn), "{error}");
    }

    #[tokio::test]
    async fn api_errors_carry_google_message() {
        let server = MockServer::start(&[
            ("/token", 200, r#"{"access_token":"access1","expires_in":3600}"#),
            (
                "/v2/spaces",
                403,
                r#"{"error":{"code":403,"message":"Meet API disabled","status":"PERMISSION_DENIED"}}"#,
            ),
        ])
        .await;
        let dir = tempfile::tempdir().unwrap();
        let google = signed_in(dir.path(), &server);

        let error = google.create_meeting().await.unwrap_err();
        assert!(
            matches!(&error, Error::Other(m) if m == "Meet API disabled"),
            "{error}"
        );
    }

    #[tokio::test]
    async fn sign_in_exchanges_code_and_stores_account() {
        let token_reply = format!(
            r#"{{"access_token":"access1","expires_in":3600,"refresh_token":"refresh1","id_token":"{}"}}"#,
            id_token(r#"{"iss":"https://accounts.google.com","email":"alice@example.com"}"#)
        );
        let server = MockServer::start(&[
            ("/token", 200, &token_reply),
            (
                "/v2/spaces",
                200,
                &format!(r#"{{"meetingUri":"{MEET_LINK}"}}"#),
            ),
        ])
        .await;
        let dir = tempfile::tempdir().unwrap();
        let google = Google::with_endpoints(dir.path(), SecretStore::File, server.endpoints());

        let mut auth_params = HashMap::new();
        let client = GoogleClient {
            client_id: "client".into(),
            client_secret: "secret".into(),
        };
        let email = google
            .sign_in(client, |url| {
                auth_params = url.query_pairs().into_owned().collect();
                // Play the browser: follow Google's redirect back to Mtrx.
                let mut redirect = Url::parse(&auth_params["redirect_uri"]).unwrap();
                redirect
                    .query_pairs_mut()
                    .append_pair("code", "code1")
                    .append_pair("state", &auth_params["state"]);
                tokio::spawn(reqwest::get(redirect));
            })
            .await
            .unwrap();

        assert_eq!(email, "alice@example.com");
        assert_eq!(
            google.account().unwrap().as_deref(),
            Some("alice@example.com")
        );
        let form = server.requests("/token")[0].form();
        assert_eq!(form["grant_type"], "authorization_code");
        assert_eq!(form["code"], "code1");
        assert_eq!(form["redirect_uri"], auth_params["redirect_uri"]);
        assert_eq!(s256(&form["code_verifier"]), auth_params["code_challenge"]);
        // The access token from sign-in is used without a refresh.
        google.create_meeting().await.unwrap();
        assert_eq!(server.requests("/token").len(), 1);
    }

    #[test]
    fn pkce_challenge_is_s256_of_verifier() {
        // RFC 7636, appendix B.
        assert_eq!(
            s256("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
        let pkce = Pkce::new();
        assert_eq!(pkce.verifier.len(), 43);
        assert_eq!(pkce.challenge, s256(&pkce.verifier));
    }

    #[test]
    fn auth_url_has_required_parameters() {
        let dir = tempfile::tempdir().unwrap();
        let google = Google::new(dir.path(), SecretStore::File);
        let redirect = Url::parse("http://127.0.0.1:23456/").unwrap();
        let url = google.auth_url("client", &redirect, "challenge", "state1");

        assert_eq!(
            url.origin().ascii_serialization(),
            "https://accounts.google.com"
        );
        assert_eq!(url.path(), "/o/oauth2/v2/auth");
        let params: HashMap<_, _> = url.query_pairs().into_owned().collect();
        let expected = [
            ("client_id", "client"),
            ("redirect_uri", "http://127.0.0.1:23456/"),
            ("response_type", "code"),
            ("scope", SCOPES),
            ("code_challenge", "challenge"),
            ("code_challenge_method", "S256"),
            ("state", "state1"),
            ("access_type", "offline"),
            ("prompt", "consent"),
        ];
        for (key, value) in expected {
            assert_eq!(params.get(key).map(String::as_str), Some(value), "{key}");
        }
    }

    #[test]
    fn authorization_code_checks_state_and_errors() {
        assert_eq!(authorization_code("code=c1&state=s1", "s1").unwrap(), "c1");
        assert!(authorization_code("code=c1&state=other", "s1").is_err());
        let error = authorization_code("error=access_denied&state=s1", "s1").unwrap_err();
        assert!(error.to_string().contains("access_denied"), "{error}");
    }

    #[tokio::test]
    async fn storage_round_trip_and_sign_out() {
        let server = MockServer::start(&[("/revoke", 200, "{}")]).await;
        let dir = tempfile::tempdir().unwrap();
        signed_in(dir.path(), &server);
        assert!(dir.path().join(SECRETS_FILE).exists());

        let google = Google::with_endpoints(dir.path(), SecretStore::File, server.endpoints());
        assert_eq!(
            google.account().unwrap().as_deref(),
            Some("alice@example.com")
        );
        google.sign_out().await.unwrap();

        assert_eq!(server.requests("/revoke")[0].form()["token"], "refresh1");
        assert_eq!(google.account().unwrap(), None);
        assert!(!dir.path().join(SECRETS_FILE).exists());
    }

    #[tokio::test]
    async fn sign_out_forgets_tokens_when_revoke_fails() {
        let server = MockServer::start(&[("/revoke", 400, r#"{"error":"invalid_token"}"#)]).await;
        let dir = tempfile::tempdir().unwrap();
        let google = signed_in(dir.path(), &server);

        google.sign_out().await.unwrap();
        assert_eq!(google.account().unwrap(), None);
    }
}
