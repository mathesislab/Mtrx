//! Helpers for tests, most against the local Synapse in `testserver/`.

use std::{
    sync::atomic::{AtomicU32, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use hmac::{Hmac, KeyInit, Mac};
use sha1::Sha1;

use crate::{
    Profile, Session,
    profile::{StoredAuth, StoredSession},
};

/// Shared secret from `testserver/homeserver.yaml`.
const SHARED_SECRET: &str = "mtrx-test-shared-secret";
/// Password of every user created by [`register`].
pub const PASSWORD: &str = "test-password";

/// `MTRX_TEST_HOMESERVER`, default `http://localhost:8008`.
pub fn homeserver() -> String {
    std::env::var("MTRX_TEST_HOMESERVER").unwrap_or_else(|_| "http://localhost:8008".into())
}

/// A user name unique per call, across test runs.
pub fn unique_name(prefix: &str) -> String {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    format!(
        "{prefix}{nanos}x{}",
        COUNTER.fetch_add(1, Ordering::Relaxed)
    )
}

/// Register `name` with [`PASSWORD`] via Synapse's shared-secret admin API.
pub async fn register(name: &str) -> Result<(), String> {
    let http = reqwest::Client::new();
    let url = format!("{}/_synapse/admin/v1/register", homeserver());
    let err = |e: &dyn std::fmt::Display| e.to_string();
    let nonce = http
        .get(&url)
        .send()
        .await
        .map_err(|e| err(&e))?
        .text()
        .await
        .map_err(|e| err(&e))?;
    let nonce: serde_json::Value = serde_json::from_str(&nonce).map_err(|e| err(&e))?;
    let nonce = nonce["nonce"].as_str().ok_or("no nonce")?;

    let mut mac = Hmac::<Sha1>::new_from_slice(SHARED_SECRET.as_bytes()).map_err(|e| err(&e))?;
    mac.update(format!("{nonce}\0{name}\0{PASSWORD}\0notadmin").as_bytes());
    let mac: String = mac
        .finalize()
        .into_bytes()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();

    let body = serde_json::json!({
        "nonce": nonce, "username": name, "password": PASSWORD, "admin": false, "mac": mac,
    });
    let response = http
        .post(&url)
        .header("content-type", "application/json")
        .body(body.to_string())
        .send()
        .await
        .map_err(|e| err(&e))?;
    if !response.status().is_success() {
        return Err(format!(
            "registering {name} failed: {}",
            response.text().await.unwrap_or_default()
        ));
    }
    Ok(())
}

/// Log `name` out on all their devices, as if signed out elsewhere: their
/// sessions' tokens stop working.
pub async fn log_out_everywhere(name: &str) -> Result<(), String> {
    let http = reqwest::Client::new();
    let err = |e: &dyn std::fmt::Display| e.to_string();
    let login = serde_json::json!({
        "type": "m.login.password",
        "identifier": { "type": "m.id.user", "user": name },
        "password": PASSWORD,
    });
    let response: serde_json::Value = http
        .post(format!("{}/_matrix/client/v3/login", homeserver()))
        .header("content-type", "application/json")
        .body(login.to_string())
        .send()
        .await
        .map_err(|e| err(&e))?
        .json()
        .await
        .map_err(|e| err(&e))?;
    let token = response["access_token"]
        .as_str()
        .ok_or_else(|| format!("no access token: {response}"))?;
    let response = http
        .post(format!("{}/_matrix/client/v3/logout/all", homeserver()))
        .bearer_auth(token)
        .header("content-type", "application/json")
        .body("{}")
        .send()
        .await
        .map_err(|e| err(&e))?;
    if !response.status().is_success() {
        return Err(format!("logging out failed: {}", response.status()));
    }
    Ok(())
}

/// A session of `profile` as if signed in before, restored without a
/// server: its homeserver refuses connections. For tests that need a
/// [`Session`] but no server.
pub async fn offline_session(profile: Profile) -> crate::Result<Session> {
    use matrix_sdk::{
        SessionMeta, SessionTokens, authentication::matrix::MatrixSession, ruma::owned_device_id,
        ruma::owned_user_id,
    };

    profile.save_session(&StoredSession {
        // Nothing listens on the discard port.
        homeserver: "http://127.0.0.1:9".into(),
        auth: StoredAuth::Password(MatrixSession {
            meta: SessionMeta {
                user_id: owned_user_id!("@offline:localhost"),
                device_id: owned_device_id!("OFFLINE"),
            },
            tokens: SessionTokens {
                access_token: "offline".into(),
                refresh_token: None,
            },
        }),
        store_passphrase: "offline".into(),
    })?;
    Session::restore(profile).await
}
