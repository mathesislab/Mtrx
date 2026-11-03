//! Calls through links: meeting links in messages are recognised, and a
//! space can set the call service its rooms use to start calls, in its state,
//! so every member sees it. A room's own calls are in its topic (see
//! [`crate::Topic`]); its old standing call link moves there.

use matrix_sdk::ruma::{
    RoomId,
    api::{client::state::get_state_event_for_key, error::ErrorKind},
    events::{
        AnySyncTimelineEvent, EmptyStateKey, StateEventType, StaticEventContent,
        macros::EventContent,
    },
    serde::Raw,
};
use rand::distr::{Alphanumeric, SampleString};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use url::Url;

use crate::{
    Error, LinkKind, Result, RoomHandle, RoomLink, Session, Topic, timeline::send_message,
};

/// A call service, for starting calls.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CallService {
    /// Shown to people, e.g. "Jitsi Meet" or "Lab Jitsi".
    pub name: String,
    #[serde(flatten)]
    pub kind: ServiceKind,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServiceKind {
    /// A new meeting room for each call: `template` with `{room}` replaced
    /// by a random name, e.g. Jitsi's `https://meet.jit.si/{room}`.
    Rooms { template: String },
    /// Always the same link, e.g. a personal Zoom or Teams room.
    Fixed { url: String },
    /// A new Google Meet, created through the Google connector.
    GoogleMeet,
}

const ROOM_PLACEHOLDER: &str = "{room}";

impl CallService {
    /// Jitsi Meet on `server`, e.g. `https://meet.jit.si`.
    pub fn jitsi(name: &str, server: &str) -> Result<Self> {
        Self::rooms(
            name,
            &format!("{}/{ROOM_PLACEHOLDER}", server.trim_end_matches('/')),
        )
    }

    /// New rooms from `template`, which must contain `{room}`.
    pub fn rooms(name: &str, template: &str) -> Result<Self> {
        let service = Self {
            name: name.to_owned(),
            kind: ServiceKind::Rooms {
                template: template.to_owned(),
            },
        };
        if !template.contains(ROOM_PLACEHOLDER) || service.link_for("x").is_none() {
            return Err(Error::InvalidCallService(template.to_owned()));
        }
        Ok(service)
    }

    pub fn fixed(name: &str, url: &str) -> Result<Self> {
        Url::parse(url).map_err(|_| Error::InvalidCallService(url.to_owned()))?;
        Ok(Self {
            name: name.to_owned(),
            kind: ServiceKind::Fixed {
                url: url.to_owned(),
            },
        })
    }

    pub fn google_meet() -> Self {
        Self {
            name: "Google Meet".into(),
            kind: ServiceKind::GoogleMeet,
        }
    }

    /// A link for a new call; `None` for Google Meet, which needs the
    /// Google connector to create one.
    pub fn new_link(&self) -> Option<Url> {
        match &self.kind {
            ServiceKind::Rooms { .. } => {
                let name = Alphanumeric
                    .sample_string(&mut rand::rng(), 12)
                    .to_lowercase();
                self.link_for(&format!("mtrx-{name}"))
            }
            ServiceKind::Fixed { url } => Url::parse(url).ok(),
            ServiceKind::GoogleMeet => None,
        }
    }

    fn link_for(&self, room: &str) -> Option<Url> {
        match &self.kind {
            ServiceKind::Rooms { template } => {
                Url::parse(&template.replace(ROOM_PLACEHOLDER, room)).ok()
            }
            _ => None,
        }
    }

    /// Whether `url` is a call link of this service.
    fn matches(&self, url: &Url) -> bool {
        match &self.kind {
            ServiceKind::Rooms { template } => {
                let prefix = &template[..template.find(ROOM_PLACEHOLDER).unwrap_or(0)];
                url.as_str().starts_with(prefix) && url.as_str().len() > prefix.len()
            }
            ServiceKind::Fixed { url: fixed } => {
                url.as_str().trim_end_matches('/') == fixed.trim_end_matches('/')
            }
            ServiceKind::GoogleMeet => is_google_meet(url),
        }
    }
}

/// The meeting service a link belongs to, e.g. "Zoom", or `None` if it is
/// not a meeting link. `services` (e.g. a self-hosted Jitsi) are checked
/// before the well-known ones.
pub fn recognize_call_link(link: &str, services: &[CallService]) -> Option<String> {
    let url = Url::parse(link).ok()?;
    if !matches!(url.scheme(), "https" | "http") {
        return None;
    }
    if let Some(service) = services.iter().find(|s| s.matches(&url)) {
        return Some(service.name.clone());
    }
    let host = url.host_str()?;
    let path = url.path();
    let has_path = path.len() > 1;
    let under = |domain: &str| host == domain || host.ends_with(&format!(".{domain}"));
    let name = if is_google_meet(&url) {
        "Google Meet"
    } else if under("zoom.us") && ["/j/", "/my/", "/wc/"].iter().any(|p| path.starts_with(p)) {
        "Zoom"
    } else if (host == "teams.microsoft.com" && path.starts_with("/l/meetup-join"))
        || (host == "teams.live.com" && path.starts_with("/meet"))
    {
        "Microsoft Teams"
    } else if (host == "meet.jit.si" || host == "8x8.vc" || host.starts_with("jitsi.")) && has_path
    {
        "Jitsi Meet"
    } else if under("webex.com") && (path.contains("/meet/") || path.contains("/j.php")) {
        "Webex"
    } else if under("whereby.com") && has_path {
        "Whereby"
    } else if host == "call.element.io" {
        "Element Call"
    } else if host.contains("mirotalk") && path.starts_with("/join/") {
        "MiroTalk"
    } else if (host.contains("bigbluebutton") || host.starts_with("bbb.")) && has_path {
        "BigBlueButton"
    } else {
        return None;
    };
    Some(name.to_owned())
}

/// `https://meet.google.com/abc-defg-hij`.
fn is_google_meet(url: &Url) -> bool {
    url.host_str() == Some("meet.google.com") && url.path().len() > 1
}

/// A room's standing call link, before calls moved into the topic.
#[derive(Clone, Debug, Default, Serialize, Deserialize, EventContent)]
#[ruma_event(type = "org.mtrx.call_link", kind = State, state_key_type = EmptyStateKey)]
struct CallLinkEventContent {
    /// `None` once removed (state cannot be deleted).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    url: Option<String>,
}

/// The call link a timeline's state event of `event_type` sets: `None` if it
/// is not a call link event, `Some(None)` if it removes the link.
pub(crate) fn call_link_change(
    event_type: &StateEventType,
    raw: Option<&Raw<AnySyncTimelineEvent>>,
) -> Option<Option<String>> {
    if *event_type != StateEventType::from(CallLinkEventContent::TYPE) {
        return None;
    }
    let content = raw.and_then(|raw| raw.get_field::<CallLinkEventContent>("content").ok()?);
    Some(content.and_then(|c| c.url))
}

/// The call service of a space, used by the rooms in it.
#[derive(Clone, Debug, Default, Serialize, Deserialize, EventContent)]
#[ruma_event(type = "org.mtrx.call_service", kind = State, state_key_type = EmptyStateKey)]
struct CallServiceEventContent {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    service: Option<CallService>,
}

impl RoomHandle {
    /// Move the room's old standing call link (`org.mtrx.call_link`) into
    /// its topic as its call, then remove it; if we may change both.
    pub async fn move_call_link_to_topic(&self) -> Result<()> {
        let content = fetch_state::<CallLinkEventContent>(&self.session, self.id()).await?;
        let Some(url) = content.and_then(|c| Url::parse(c.url.as_deref()?).ok()) else {
            return Ok(());
        };
        if !(self.can_change_topic().await?
            && self.can_send_state(CallLinkEventContent::TYPE).await?)
        {
            return Ok(());
        }
        let mut topic = Topic::parse(&self.room.topic().unwrap_or_default());
        if !topic.links.iter().any(|link| link.url == url) {
            topic.links.insert(
                0,
                RoomLink {
                    kind: LinkKind::Call { passcode: None },
                    title: None,
                    url,
                },
            );
            self.set_topic(&topic.to_topic()).await?;
        }
        self.room
            .send_state_event(CallLinkEventContent { url: None })
            .await?;
        Ok(())
    }

    /// Set the room's old standing call link, as older versions did.
    #[cfg(feature = "test-support")]
    pub async fn set_old_call_link(&self, url: &Url) -> Result<()> {
        self.room
            .send_state_event(CallLinkEventContent {
                url: Some(url.to_string()),
            })
            .await?;
        Ok(())
    }

    /// Whether we may change the room's topic, and so its calls and links.
    pub async fn can_change_topic(&self) -> Result<bool> {
        self.can_send_state("m.room.topic").await
    }

    /// The call service set for this space.
    pub async fn call_service(&self) -> Result<Option<CallService>> {
        let content = fetch_state::<CallServiceEventContent>(&self.session, self.id()).await?;
        Ok(content.and_then(|c| c.service))
    }

    /// Set or remove the call service of this space.
    pub async fn set_call_service(&self, service: Option<CallService>) -> Result<()> {
        self.room
            .send_state_event(CallServiceEventContent { service })
            .await?;
        Ok(())
    }

    /// Whether we may change the call service of this space.
    pub async fn can_set_call_service(&self) -> Result<bool> {
        self.can_send_state(CallServiceEventContent::TYPE).await
    }

    /// Post a call link as a message.
    pub async fn post_call(&self, service: &str, url: &Url) -> Result<()> {
        send_message(
            &self.room,
            &self.session.send_lock,
            &format!("{service} call: {url}"),
            Vec::new(),
            None,
        )
        .await?;
        Ok(())
    }

    async fn can_send_state(&self, event_type: &str) -> Result<bool> {
        let levels = (self.room.power_levels().await).map_err(matrix_sdk::Error::from)?;
        Ok(levels.user_can_send_state(&self.session.user_id(), StateEventType::from(event_type)))
    }
}

/// The content of a state event with an empty state key, if set, from the
/// server: sliding sync only keeps the state types the room list asks for.
async fn fetch_state<C>(session: &Session, room_id: &RoomId) -> Result<Option<C>>
where
    C: StaticEventContent + DeserializeOwned,
{
    let request = get_state_event_for_key::v3::Request::new(
        room_id.to_owned(),
        StateEventType::from(C::TYPE),
        String::new(),
    );
    match session.client.send(request).await {
        Ok(response) => Ok(serde_json::from_str(response.event_or_content.get()).ok()),
        Err(e) if e.client_api_error_kind() == Some(&ErrorKind::NotFound) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_call_link_changes_from_the_timeline() {
        let call_link = StateEventType::from("org.mtrx.call_link");
        let event = |content: &str| {
            Raw::<AnySyncTimelineEvent>::from_json_string(format!(
                r#"{{"type": "org.mtrx.call_link", "state_key": "", "event_id": "$a",
                    "sender": "@alice:x", "origin_server_ts": 1, "content": {content}}}"#
            ))
            .unwrap()
        };
        let set = event(r#"{"url": "https://meet.jit.si/ice"}"#);
        assert_eq!(
            call_link_change(&call_link, Some(&set)),
            Some(Some("https://meet.jit.si/ice".into()))
        );
        assert_eq!(call_link_change(&call_link, Some(&event("{}"))), Some(None));
        assert_eq!(
            call_link_change(&StateEventType::RoomTopic, Some(&set)),
            None
        );
    }

    #[test]
    fn recognizes_meeting_links() {
        let cases = [
            ("https://meet.google.com/abc-defg-hij", Some("Google Meet")),
            ("https://us02web.zoom.us/j/123456789?pwd=x", Some("Zoom")),
            (
                "https://teams.microsoft.com/l/meetup-join/19%3ameeting",
                Some("Microsoft Teams"),
            ),
            ("https://meet.jit.si/OurWeeklyCall", Some("Jitsi Meet")),
            ("https://whereby.com/lab", Some("Whereby")),
            ("https://p2p.mirotalk.com/join/abc", Some("MiroTalk")),
            ("https://meet.google.com/", None),
            ("https://zoom.us/pricing", None),
            ("https://example.org/j/1", None),
            ("mailto:a@b.c", None),
        ];
        for (link, expected) in cases {
            assert_eq!(
                recognize_call_link(link, &[]).as_deref(),
                expected,
                "{link}"
            );
        }
    }

    #[test]
    fn configured_services_come_first() {
        let lab = CallService::jitsi("Lab Jitsi", "https://video.lab.example/").unwrap();
        let zoom = CallService::fixed("My Zoom", "https://zoom.us/j/42").unwrap();
        let services = [lab.clone(), zoom];
        let link = lab.new_link().unwrap();
        assert!(link.as_str().starts_with("https://video.lab.example/mtrx-"));
        assert_eq!(
            recognize_call_link(link.as_str(), &services).as_deref(),
            Some("Lab Jitsi")
        );
        assert_eq!(
            recognize_call_link("https://zoom.us/j/42/", &services).as_deref(),
            Some("My Zoom")
        );
        assert_ne!(lab.new_link(), lab.new_link(), "a new room each time");
    }

    #[test]
    fn rejects_invalid_services() {
        assert!(CallService::rooms("x", "https://meet.example/fixed").is_err());
        assert!(CallService::rooms("x", "not a url {room}").is_err());
        assert!(CallService::fixed("x", "zoom").is_err());
    }

    #[test]
    fn services_serialize_readably() {
        let service = CallService::jitsi("Jitsi Meet", "https://meet.jit.si").unwrap();
        let json = serde_json::to_value(&service).unwrap();
        assert_eq!(
            json,
            serde_json::json!({
                "name": "Jitsi Meet",
                "type": "rooms",
                "template": "https://meet.jit.si/{room}",
            })
        );
        assert_eq!(
            serde_json::from_value::<CallService>(json).unwrap(),
            service
        );
    }
}
