//! Push notifications: registering a pusher, and reading what a push is
//! about from the process that receives it (docs/mobile.md, section 10).

use std::time::SystemTime;

use mtrx_core::{self as core, OwnedRoomId, Url};

use crate::{MtrxError, Session, User, runtime::run};

/// A pusher of the account, set by this device or another.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct PusherInfo {
    pub pushkey: String,
    pub app_id: String,
    pub app_name: String,
    pub device_name: String,
    pub lang: String,
    /// The push gateway's notify URL; nil for a pusher that is not HTTP.
    #[uniffi(default)]
    pub gateway_url: Option<String>,
    /// The server sends only the event and room IDs.
    #[uniffi(default)]
    pub event_id_only: bool,
}

impl From<core::PusherInfo> for PusherInfo {
    fn from(pusher: core::PusherInfo) -> Self {
        Self {
            pushkey: pusher.pushkey,
            app_id: pusher.app_id,
            app_name: pusher.app_name,
            device_name: pusher.device_name,
            lang: pusher.lang,
            gateway_url: pusher.gateway_url,
            event_id_only: pusher.event_id_only,
        }
    }
}

/// A message worth telling the user about: what a push is about, for a
/// local notification.
#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct Notification {
    pub room_id: String,
    pub event_id: String,
    pub room_name: String,
    /// `mxc://` URI, for `Session.avatar`.
    #[uniffi(default)]
    pub room_avatar_url: Option<String>,
    /// A direct chat.
    pub direct: bool,
    pub sender: User,
    /// A message's text, or what it is: "Image", "Video", "Audio", "File",
    /// "Location", "Sticker", "Call", "Encrypted message" (no key yet),
    /// "Invited you", "New message", "New event".
    pub body: String,
    /// Mentions us or matches a keyword.
    pub highlight: bool,
    /// The push rules ask for a sound.
    pub noisy: bool,
    pub timestamp: SystemTime,
}

impl From<core::Notification> for Notification {
    fn from(n: core::Notification) -> Self {
        Self {
            room_id: n.room_id.to_string(),
            event_id: n.event_id.to_string(),
            room_name: n.room_name,
            room_avatar_url: n.room_avatar.map(|uri| uri.to_string()),
            direct: n.direct,
            sender: User::from(&n.sender),
            body: n.body,
            highlight: n.highlight,
            noisy: n.noisy,
            timestamp: n.timestamp,
        }
    }
}

#[uniffi::export]
impl Session {
    /// Have the server push this account's notifications to a push gateway
    /// (`gateway_url`, ending in `/_matrix/push/v1/notify`) for `pushkey`
    /// (UnifiedPush: the endpoint URL), sending only event and room IDs.
    /// `app_id` e.g. `org.mtrx.app`; `lang` e.g. `en`. Calling it again
    /// replaces the pusher of the same `pushkey` and `app_id`.
    pub async fn set_pusher(
        &self,
        gateway_url: String,
        pushkey: String,
        app_id: String,
        app_name: String,
        device_name: String,
        lang: String,
    ) -> Result<(), MtrxError> {
        let gateway_url = Url::parse(&gateway_url).map_err(MtrxError::invalid)?;
        let pusher = core::HttpPusher {
            gateway_url,
            pushkey,
            app_id,
            app_name,
            device_name,
            lang,
        };
        let session = self.core();
        run(async move { Ok(session.set_pusher(pusher).await?) }).await
    }

    /// Stop the pushes to `pushkey`; succeeds if there are none.
    pub async fn delete_pusher(&self, pushkey: String) -> Result<(), MtrxError> {
        let session = self.core();
        run(async move { Ok(session.delete_pusher(&pushkey).await?) }).await
    }

    /// The account's pushers, of all its devices.
    pub async fn pushers(&self) -> Result<Vec<PusherInfo>, MtrxError> {
        let session = self.core();
        run(async move {
            Ok(session
                .pushers()
                .await?
                .into_iter()
                .map(PusherInfo::from)
                .collect())
        })
        .await
    }
}

/// See [`crate::Mtrx::notification`].
pub(crate) async fn notification(
    profile: core::Profile,
    room_id: String,
    event_id: String,
) -> Result<Option<Notification>, MtrxError> {
    let room_id = OwnedRoomId::try_from(room_id.as_str()).map_err(MtrxError::invalid)?;
    let event_id = core::OwnedEventId::try_from(event_id.as_str()).map_err(MtrxError::invalid)?;
    run(async move {
        let notification = core::Notification::fetch(&profile, &room_id, &event_id).await?;
        Ok(notification.map(Notification::from))
    })
    .await
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    #[test]
    fn a_notification_names_its_sender() {
        let notification = Notification::from(core::Notification {
            room_id: "!r:example.org".try_into().unwrap(),
            event_id: "$e:example.org".try_into().unwrap(),
            room_name: "Ice sheets".into(),
            room_avatar: Some("mxc://example.org/ice".into()),
            direct: false,
            sender: core::User {
                id: "@alice:example.org".try_into().unwrap(),
                name: None,
                avatar: Some("mxc://example.org/alice".into()),
            },
            body: "Image".into(),
            highlight: true,
            noisy: false,
            timestamp: SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000),
        });
        assert_eq!(
            notification.room_avatar_url.as_deref(),
            Some("mxc://example.org/ice")
        );
        assert_eq!(notification.sender.user_id, "@alice:example.org");
        assert_eq!(notification.sender.display_name, "alice");
        assert_eq!(
            notification.sender.avatar_url.as_deref(),
            Some("mxc://example.org/alice")
        );
        assert!(notification.highlight && !notification.noisy);
    }
}
