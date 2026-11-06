//! Push notifications: the pusher through which the server reaches a
//! device, and what a push is about ([`Notification::fetch`]), read by the
//! process that receives it.
//!
//! The server pushes only a room and an event ID (`event_id_only`), so the
//! device fetches and decrypts the event itself. That happens in a client
//! of its own next to the app's session, which may be syncing in this
//! process or another: the SDK's notification client, which takes the
//! store's cross-process locks while it writes (see
//! [`crate::session`]'s `lock_holder`).

use std::time::SystemTime;

use matrix_sdk::{
    encryption::EncryptionSettings,
    ruma::{
        EventId, RoomId, UserId,
        api::client::push::{self as api, PusherIds, PusherInit, PusherKind, get_pushers},
        push::{HttpPusherData, PushFormat},
    },
    store::RoomLoadSettings,
};
use matrix_sdk_ui::notification_client::{
    NotificationClient, NotificationEvent, NotificationItem, NotificationProcessSetup,
    NotificationStatus,
};
use tokio::sync::Mutex;
use url::Url;

use crate::{
    Error, Notification, Profile, Result, Session, User,
    notifications::{self, should_notify},
    session::restore_client,
};

/// A pusher posting to an HTTP push gateway, which forwards each push to
/// the device (UnifiedPush: through the distributor; iOS: through APNs).
/// The server sends only the event and room IDs (`event_id_only`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HttpPusher {
    /// The gateway's notify URL, `<gateway>/_matrix/push/v1/notify`.
    pub gateway_url: Url,
    /// What the gateway delivers to (UnifiedPush: the endpoint URL).
    pub pushkey: String,
    /// The app, e.g. `org.mtrx.app`.
    pub app_id: String,
    /// Shown in the account's list of pushers.
    pub app_name: String,
    pub device_name: String,
    /// Of the notifications, e.g. `en`.
    pub lang: String,
}

impl From<HttpPusher> for api::Pusher {
    fn from(pusher: HttpPusher) -> Self {
        let mut data = HttpPusherData::new(pusher.gateway_url.into());
        data.format = Some(PushFormat::EventIdOnly);
        PusherInit {
            ids: PusherIds::new(pusher.pushkey, pusher.app_id),
            kind: PusherKind::Http(data),
            app_display_name: pusher.app_name,
            device_display_name: pusher.device_name,
            profile_tag: None,
            lang: pusher.lang,
        }
        .into()
    }
}

/// A pusher of the account, set by this device or another.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PusherInfo {
    pub pushkey: String,
    pub app_id: String,
    pub app_name: String,
    pub device_name: String,
    pub lang: String,
    /// The gateway of an HTTP pusher; `None` for others (email).
    pub gateway_url: Option<String>,
    /// The server sends only the event and room IDs.
    pub event_id_only: bool,
}

impl From<api::Pusher> for PusherInfo {
    fn from(pusher: api::Pusher) -> Self {
        let (gateway_url, event_id_only) = match pusher.kind {
            PusherKind::Http(data) => {
                (Some(data.url), data.format == Some(PushFormat::EventIdOnly))
            }
            _ => (None, false),
        };
        Self {
            pushkey: pusher.ids.pushkey,
            app_id: pusher.ids.app_id,
            app_name: pusher.app_display_name,
            device_name: pusher.device_display_name,
            lang: pusher.lang,
            gateway_url,
            event_id_only,
        }
    }
}

impl Session {
    /// Have the server push this account's notifications through `pusher`.
    /// Setting it again (a new gateway or name) replaces it.
    pub async fn set_pusher(&self, pusher: HttpPusher) -> Result<()> {
        Ok(self.client.pusher().set(pusher.into(), false).await?)
    }

    /// Stop the pushes to `pushkey` (of any app); succeeds if there are none.
    pub async fn delete_pusher(&self, pushkey: &str) -> Result<()> {
        for pusher in self.raw_pushers().await? {
            if pusher.ids.pushkey == pushkey {
                self.client.pusher().delete(pusher.ids).await?;
            }
        }
        Ok(())
    }

    /// The account's pushers, of all its devices.
    pub async fn pushers(&self) -> Result<Vec<PusherInfo>> {
        Ok(self
            .raw_pushers()
            .await?
            .into_iter()
            .map(PusherInfo::from)
            .collect())
    }

    async fn raw_pushers(&self) -> Result<Vec<api::Pusher>> {
        let request = get_pushers::v3::Request::new();
        Ok(self.client.send(request).await?.pushers)
    }
}

/// One push at a time in a process: the SDK's notification client takes
/// the store's locks under a fixed name, so two at once would hold them
/// together.
static ONE_AT_A_TIME: Mutex<()> = Mutex::const_new(());

impl Notification {
    /// What a push is about: fetch and decrypt `event_id` of `room_id` with
    /// the session saved in `profile`, without syncing; from any process,
    /// also while the app's session syncs (in this process or another).
    /// `None` if there is nothing to show: the push rules say no (a muted
    /// room), it is our own event (also when the rules cannot be read), the
    /// sender is ignored, or the event was deleted.
    pub async fn fetch(
        profile: &Profile,
        room_id: &RoomId,
        event_id: &EventId,
    ) -> Result<Option<Self>> {
        let _one = ONE_AT_A_TIME.lock().await;
        // Cross-signing and the backup are the app's to set up.
        let client = restore_client(
            profile,
            RoomLoadSettings::One(room_id.to_owned()),
            EncryptionSettings::default(),
        )
        .await?;
        let own = client.user_id().expect("restored").to_owned();
        let notifications =
            NotificationClient::new(client, NotificationProcessSetup::MultipleProcesses)
                .await
                .map_err(notification_error)?;
        match notifications
            .get_notification(room_id, event_id)
            .await
            .map_err(notification_error)?
        {
            NotificationStatus::Event(item) => Ok(Self::pushed(room_id, event_id, &item, &own)),
            NotificationStatus::EventFilteredOut | NotificationStatus::EventRedacted => Ok(None),
            NotificationStatus::EventNotFound => Err(Error::Notifications(format!(
                "event {event_id} not found in {room_id}"
            ))),
        }
    }

    /// The notification of `item`, if someone else's event the push rules
    /// notify for.
    fn pushed(
        room_id: &RoomId,
        event_id: &EventId,
        item: &NotificationItem,
        own: &UserId,
    ) -> Option<Self> {
        let sender = item.event.sender();
        let notifies = item
            .actions
            .as_deref()
            // Unknown: the server pushed it, so its rules said to.
            .is_none_or(should_notify);
        if !notifies || sender == own {
            return None;
        }
        Some(Self {
            room_id: room_id.to_owned(),
            event_id: event_id.to_owned(),
            room_name: item.room_computed_display_name.clone(),
            room_avatar: item.room_avatar_url.clone().map(Into::into),
            direct: item.is_direct_message_room,
            sender: User {
                id: sender.to_owned(),
                name: item.sender_display_name.clone(),
                avatar: item.sender_avatar_url.clone().map(Into::into),
            },
            body: body(&item.event),
            highlight: item.has_mention.unwrap_or(false),
            noisy: item.is_noisy.unwrap_or(false),
            timestamp: match &item.event {
                NotificationEvent::Timeline(event) => crate::system_time(event.origin_server_ts()),
                NotificationEvent::Invite(_) => SystemTime::now(),
            },
        })
    }
}

fn notification_error(error: matrix_sdk_ui::notification_client::Error) -> Error {
    Error::Notifications(error.to_string())
}

fn body(event: &NotificationEvent) -> String {
    match event {
        NotificationEvent::Timeline(event) => notifications::body(event),
        NotificationEvent::Invite(_) => "Invited you".into(),
    }
}

#[cfg(test)]
mod tests {
    use matrix_sdk::ruma::{
        event_id,
        events::{AnySyncTimelineEvent, room::member::StrippedRoomMemberEvent},
        push::{Action, SoundTweakValue, Tweak},
        room_id,
        serde::Raw,
        user_id,
    };
    use matrix_sdk_ui::notification_client::RawNotificationEvent;
    use serde_json::{Value, json};

    use super::*;

    fn event(sender: &str, content: Value) -> Value {
        json!({
            "type": "m.room.message",
            "event_id": "$e:example.org",
            "sender": sender,
            "origin_server_ts": 1_700_000_000_000u64,
            "content": content,
        })
    }

    fn item(json: Value, actions: Option<Vec<Action>>) -> NotificationItem {
        let raw: Raw<AnySyncTimelineEvent> = Raw::new(&json).unwrap().cast_unchecked();
        NotificationItem {
            event: NotificationEvent::Timeline(Box::new(raw.deserialize().unwrap())),
            raw_event: RawNotificationEvent::Timeline(raw),
            sender_display_name: Some("Alice".into()),
            sender_avatar_url: Some("mxc://example.org/alice".into()),
            is_sender_name_ambiguous: false,
            room_computed_display_name: "Ice sheets".into(),
            room_avatar_url: None,
            room_canonical_alias: None,
            room_topic: None,
            room_join_rule: None,
            is_room_encrypted: Some(true),
            is_direct_message_room: true,
            joined_members_count: 2,
            service_members: Vec::new(),
            active_service_members_count: 0,
            is_space: false,
            is_noisy: actions
                .as_ref()
                .map(|a| a.iter().any(|a| a.sound().is_some())),
            has_mention: actions.as_ref().map(|a| a.iter().any(|a| a.is_highlight())),
            thread_id: None,
            actions,
            room_is_dm: true,
        }
    }

    fn text(body: &str) -> Value {
        json!({ "msgtype": "m.text", "body": body })
    }

    fn notification(item: &NotificationItem) -> Option<Notification> {
        Notification::pushed(
            room_id!("!r:example.org"),
            event_id!("$e:example.org"),
            item,
            user_id!("@bob:example.org"),
        )
    }

    #[test]
    fn a_message_notifies_as_the_push_rules_say() {
        let actions = vec![
            Action::Notify,
            Action::SetTweak(Tweak::Sound(SoundTweakValue::Default)),
            Action::SetTweak(Tweak::Highlight(true.into())),
        ];
        let n = notification(&item(
            event("@alice:example.org", text("Bob: the run finished")),
            Some(actions),
        ));
        assert_eq!(
            n,
            Some(Notification {
                room_id: room_id!("!r:example.org").to_owned(),
                event_id: event_id!("$e:example.org").to_owned(),
                room_name: "Ice sheets".into(),
                room_avatar: None,
                direct: true,
                sender: User {
                    id: user_id!("@alice:example.org").to_owned(),
                    name: Some("Alice".into()),
                    avatar: Some("mxc://example.org/alice".into()),
                },
                body: "Bob: the run finished".into(),
                highlight: true,
                noisy: true,
                timestamp: SystemTime::UNIX_EPOCH
                    + std::time::Duration::from_millis(1_700_000_000_000),
            })
        );

        // Push rules that do not notify; and unknown ones (the server
        // pushed it, so they did).
        let quiet = notification(&item(event("@alice:example.org", text("hi")), Some(vec![])));
        assert_eq!(quiet, None);
        let unknown = notification(&item(event("@alice:example.org", text("hi")), None));
        assert!(unknown.is_some_and(|n| !n.highlight && !n.noisy));
    }

    #[test]
    fn our_own_message_does_not_notify() {
        let own = |actions| {
            notification(&item(
                event("@bob:example.org", text("sent from the desktop")),
                actions,
            ))
        };
        assert_eq!(own(Some(vec![Action::Notify])), None);
        // Also when the rules cannot be read.
        assert_eq!(own(None), None);
    }

    #[test]
    fn bodies_name_what_is_not_text() {
        let body_of = |json: Value| notification(&item(json, None)).unwrap().body;
        let msg = |msgtype: &str, extra: Value| {
            let mut content = json!({ "msgtype": msgtype, "body": "cat.png" });
            content
                .as_object_mut()
                .unwrap()
                .extend(extra.as_object().unwrap().clone());
            event("@alice:example.org", content)
        };
        let url = json!({ "url": "mxc://example.org/cat" });
        assert_eq!(body_of(msg("m.image", url.clone())), "Image");
        assert_eq!(body_of(msg("m.video", url.clone())), "Video");
        assert_eq!(body_of(msg("m.audio", url.clone())), "Audio");
        assert_eq!(body_of(msg("m.file", url)), "File");
        assert_eq!(
            body_of(msg("m.location", json!({ "geo_uri": "geo:51.5,0.1" }))),
            "Location"
        );
        assert_eq!(body_of(msg("m.notice", json!({}))), "cat.png");
        let encrypted = json!({
            "type": "m.room.encrypted",
            "event_id": "$e:example.org",
            "sender": "@alice:example.org",
            "origin_server_ts": 1,
            "content": {
                "algorithm": "m.megolm.v1.aes-sha2",
                "ciphertext": "AwgA",
                "device_id": "DEV",
                "sender_key": "key",
                "session_id": "session",
            },
        });
        assert_eq!(body_of(encrypted), "Encrypted message");
        let topic = json!({
            "type": "m.room.topic",
            "event_id": "$e:example.org",
            "sender": "@alice:example.org",
            "origin_server_ts": 1,
            "state_key": "",
            "content": { "topic": "Ice" },
        });
        assert_eq!(body_of(topic), "New event");

        let invite: StrippedRoomMemberEvent = serde_json::from_value(json!({
            "type": "m.room.member",
            "sender": "@alice:example.org",
            "state_key": "@bob:example.org",
            "content": { "membership": "invite" },
        }))
        .unwrap();
        assert_eq!(
            body(&NotificationEvent::Invite(Box::new(invite))),
            "Invited you"
        );
    }

    #[test]
    fn pushers_send_only_event_ids_to_the_gateway() {
        let pusher = api::Pusher::from(HttpPusher {
            gateway_url: Url::parse("https://ntfy.sh/_matrix/push/v1/notify").unwrap(),
            pushkey: "https://ntfy.sh/upAbc?up=1".into(),
            app_id: "org.mtrx.app".into(),
            app_name: "Mtrx".into(),
            device_name: "Pixel".into(),
            lang: "en".into(),
        });
        let json = serde_json::to_value(&pusher).unwrap();
        assert_eq!(json["kind"], "http");
        assert_eq!(json["data"]["format"], "event_id_only");
        assert_eq!(
            json["data"]["url"],
            "https://ntfy.sh/_matrix/push/v1/notify"
        );
        assert_eq!(json["pushkey"], "https://ntfy.sh/upAbc?up=1");
        assert_eq!(json["app_id"], "org.mtrx.app");
        assert_eq!(json["lang"], "en");

        assert_eq!(
            PusherInfo::from(pusher),
            PusherInfo {
                pushkey: "https://ntfy.sh/upAbc?up=1".into(),
                app_id: "org.mtrx.app".into(),
                app_name: "Mtrx".into(),
                device_name: "Pixel".into(),
                lang: "en".into(),
                gateway_url: Some("https://ntfy.sh/_matrix/push/v1/notify".into()),
                event_id_only: true,
            }
        );
    }
}
