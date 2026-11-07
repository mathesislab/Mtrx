//! Links kept in a room's topic, one per line after its text, so every
//! client shows them:
//!
//! ```text
//! What the room is about.
//! [call] https://meet.google.com/abc-defg-hij
//! [call Weekly passcode 123] https://zoom.us/j/123456789
//! [link] https://drive.google.com/drive/folders/xyz
//! [link Paper draft] https://www.overleaf.com/project/abc
//! ```
//!
//! The calls come first, and the first is the room's: the header joins it.

use matrix_sdk::ruma::{
    events::{AnySyncTimelineEvent, StateEventType},
    serde::Raw,
};
use serde::Deserialize;
use url::Url;

use crate::{CallService, recognize_call_link};

/// A room's topic: its text, and the links on lines of their own.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Topic {
    /// The topic without its link lines.
    pub text: String,
    pub links: Vec<RoomLink>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RoomLink {
    pub kind: LinkKind,
    pub title: Option<String>,
    pub url: Url,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LinkKind {
    Call { passcode: Option<String> },
    Link,
}

impl Topic {
    /// Lines that are not links stay in the text, so nothing is lost.
    pub fn parse(topic: &str) -> Self {
        let mut text = Vec::new();
        let mut links = Vec::new();
        for line in topic.lines() {
            match RoomLink::parse(line) {
                Some(link) => links.push(link),
                None => text.push(line),
            }
        }
        Self {
            text: text.join("\n").trim().to_owned(),
            links,
        }
    }

    /// The text, then a line for each call, then for each other link, in
    /// their order.
    pub fn to_topic(&self) -> String {
        let text = Some(self.text.trim().to_owned()).filter(|t| !t.is_empty());
        let others = self.links.iter().filter(|l| !l.is_call());
        (text.into_iter())
            .chain(self.calls().chain(others).map(RoomLink::line))
            .collect::<Vec<_>>()
            .join("\n")
    }

    pub fn calls(&self) -> impl Iterator<Item = &RoomLink> {
        self.links.iter().filter(|l| l.is_call())
    }

    /// The links that aren't calls, in their order.
    pub fn others(&self) -> impl Iterator<Item = &RoomLink> {
        self.links.iter().filter(|l| !l.is_call())
    }

    /// The room's call, which the header joins.
    pub fn default_call(&self) -> Option<&RoomLink> {
        self.calls().next()
    }

    /// No text and no links: no topic.
    pub fn is_empty(&self) -> bool {
        self.text.trim().is_empty() && self.links.is_empty()
    }

    /// The text's first non-blank line: list rows and directory entries.
    pub fn first_line(&self) -> Option<&str> {
        self.text.lines().map(str::trim).find(|l| !l.is_empty())
    }

    /// The text's first line, else the names of the calls and then the
    /// other links (see [`RoomLink::name`]): under the room's name.
    pub fn preview(&self, services: &[CallService]) -> Option<String> {
        if let Some(first) = self.first_line() {
            return Some(first.to_owned());
        }
        let names: Vec<String> = (self.calls().chain(self.others()))
            .map(|l| l.name(services))
            .collect();
        (!names.is_empty()).then(|| names.join(" · "))
    }

    /// With the call (`calls`) or other link at `from` moved to `to` among
    /// its kind; the calls come first. Unchanged if either is out of range.
    pub fn moved(self, calls: bool, from: usize, to: usize) -> Topic {
        let (mut call_links, mut others): (Vec<_>, Vec<_>) =
            self.links.into_iter().partition(RoomLink::is_call);
        let moving = if calls { &mut call_links } else { &mut others };
        if from < moving.len() && to < moving.len() {
            let link = moving.remove(from);
            moving.insert(to, link);
        }
        Topic {
            text: self.text,
            links: call_links.into_iter().chain(others).collect(),
        }
    }

    /// The topic of `text` and `links` as typed in an editor: the calls
    /// first, each kind in its order; blank links left out, and those
    /// without a web address too, counted in the second value.
    pub fn from_drafts(text: &str, links: &[LinkDraft]) -> (Topic, usize) {
        let typed: Vec<RoomLink> = (links.iter())
            .filter(|d| !d.is_blank())
            .filter_map(LinkDraft::link)
            .collect();
        let dropped = links.iter().filter(|d| !d.is_blank()).count() - typed.len();
        let (calls, others): (Vec<_>, Vec<_>) = typed.into_iter().partition(RoomLink::is_call);
        let topic = Topic {
            text: text.trim().to_owned(),
            links: calls.into_iter().chain(others).collect(),
        };
        (topic, dropped)
    }
}

/// A call or link as typed in an editor.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LinkDraft {
    pub call: bool,
    pub title: String,
    pub url: String,
    /// A call's; ignored for a link.
    pub passcode: String,
}

impl LinkDraft {
    /// The link, if its address is a web address (see [`typed_link_url`]).
    pub fn link(&self) -> Option<RoomLink> {
        let some = |s: &str| Some(s.trim().to_owned()).filter(|s| !s.is_empty());
        Some(RoomLink {
            kind: if self.call {
                LinkKind::Call {
                    passcode: some(&self.passcode),
                }
            } else {
                LinkKind::Link
            },
            title: some(&self.title),
            url: typed_link_url(&self.url)?,
        })
    }

    /// Nothing typed in (a link's passcode doesn't count).
    pub fn is_blank(&self) -> bool {
        let passcode = if self.call {
            self.passcode.as_str()
        } else {
            ""
        };
        [self.title.as_str(), passcode, self.url.as_str()]
            .iter()
            .all(|s| s.trim().is_empty())
    }

    /// Its line in the topic; as typed if it isn't a link yet, which then
    /// reads as text.
    pub fn line(&self) -> String {
        match self.link() {
            Some(link) => link.line(),
            None => format!(
                "[{}] {}",
                if self.call { "call" } else { "link" },
                self.url.trim()
            ),
        }
    }
}

impl From<&RoomLink> for LinkDraft {
    fn from(link: &RoomLink) -> Self {
        Self {
            call: link.is_call(),
            title: link.title.clone().unwrap_or_default(),
            url: link.url.to_string(),
            passcode: link.passcode().unwrap_or_default().to_owned(),
        }
    }
}

/// A typed address as the topic keeps it: "https://" may be left out
/// ("zoom.us/j/1"); http(s) with a host only. None if it isn't a web
/// address.
pub fn typed_link_url(text: &str) -> Option<Url> {
    let text = text.trim();
    if text.is_empty() || text.contains(char::is_whitespace) {
        return None;
    }
    let with_https = || Url::parse(&format!("https://{text}")).ok();
    let url = match Url::parse(text) {
        // "zoom.us:443/j/1" reads as the scheme "zoom.us".
        Ok(url) if url.scheme().contains('.') => with_https()?,
        Ok(url) => url,
        Err(_) if !text.contains("://") => with_https()?,
        Err(_) => return None,
    };
    web_url(url)
}

/// `url` if it is a web address: http(s), with a host.
fn web_url(url: Url) -> Option<Url> {
    let host = url.host_str().is_some_and(|h| !h.is_empty());
    (matches!(url.scheme(), "https" | "http") && host).then_some(url)
}

/// What changed from topic `old` to `new`, e.g. "added the link Paper
/// draft and made Weekly the room's call"; none if nothing did.
pub fn describe_change(old: &str, new: &str) -> Option<String> {
    let (old, new) = (Topic::parse(old), Topic::parse(new));
    let find = |links: &[RoomLink], url: &Url| links.iter().position(|l| &l.url == url);
    let what = |link: &RoomLink| {
        let kind = if link.is_call() { "call" } else { "link" };
        format!("{kind} {}", link.name(&[]))
    };
    let mut parts = Vec::new();
    if old.text != new.text {
        parts.push("changed the topic".to_owned());
    }
    for link in &new.links {
        match find(&old.links, &link.url) {
            None => parts.push(format!("added the {}", what(link))),
            Some(at) if old.links[at] != *link => parts.push(format!("changed the {}", what(link))),
            Some(_) => {}
        }
    }
    for link in &old.links {
        if find(&new.links, &link.url).is_none() {
            parts.push(format!("removed the {}", what(link)));
        }
    }
    // A call that was there before, now first.
    let new_call = (new.default_call())
        .filter(|call| old.default_call().map(|c| &c.url) != Some(&call.url))
        .filter(|call| find(&old.links, &call.url).is_some());
    if let Some(call) = new_call {
        parts.push(format!("made {} the room's call", call.name(&[])));
    } else if parts.is_empty() {
        let order =
            |topic: &Topic| -> Vec<Url> { topic.links.iter().map(|l| l.url.clone()).collect() };
        if order(&old) != order(&new) {
            parts.push("reordered the calls and links".to_owned());
        }
    }
    let last = parts.pop()?;
    Some(match parts.is_empty() {
        true => last,
        false => format!("{} and {last}", parts.join(", ")),
    })
}

/// The old (if known) and new topic a timeline's state event of
/// `event_type` sets; none if it is not a topic.
pub(crate) fn topic_change(
    event_type: &StateEventType,
    raw: Option<&Raw<AnySyncTimelineEvent>>,
) -> Option<(Option<String>, String)> {
    #[derive(Deserialize)]
    struct Content {
        #[serde(default)]
        topic: String,
    }
    #[derive(Deserialize)]
    struct Unsigned {
        prev_content: Option<Content>,
    }
    if *event_type != StateEventType::RoomTopic {
        return None;
    }
    let raw = raw?;
    let new = raw.get_field::<Content>("content").ok()??.topic;
    let old = (raw.get_field::<Unsigned>("unsigned").ok().flatten())
        .and_then(|u| u.prev_content)
        .map(|c| c.topic);
    Some((old, new))
}

impl RoomLink {
    /// `[call] URL`, `[call Title passcode 123] URL`, `[link Title] URL`;
    /// the URL may be in angle brackets.
    fn parse(line: &str) -> Option<Self> {
        let (inside, url) = line.trim().strip_prefix('[')?.split_once(']')?;
        let url = url.trim();
        let url = (url.strip_prefix('<'))
            .and_then(|u| u.strip_suffix('>'))
            .unwrap_or(url);
        if url.contains(char::is_whitespace) {
            return None;
        }
        let url = web_url(Url::parse(url).ok()?)?;
        let mut words = inside.split_whitespace();
        let keyword = words.next()?;
        let words: Vec<&str> = words.collect();
        let (title, kind) = if keyword.eq_ignore_ascii_case("call") {
            // The last "passcode" starts the passcode.
            match words
                .iter()
                .rposition(|w| w.eq_ignore_ascii_case("passcode"))
            {
                Some(at) if at + 1 < words.len() => (
                    &words[..at],
                    LinkKind::Call {
                        passcode: Some(words[at + 1..].join(" ")),
                    },
                ),
                _ => (&words[..], LinkKind::Call { passcode: None }),
            }
        } else if keyword.eq_ignore_ascii_case("link") {
            (&words[..], LinkKind::Link)
        } else {
            return None;
        };
        Some(Self {
            kind,
            title: Some(title.join(" ")).filter(|t| !t.is_empty()),
            url,
        })
    }

    /// The link's line in the topic.
    pub fn line(&self) -> String {
        let mut words = vec![match self.kind {
            LinkKind::Call { .. } => "call".to_owned(),
            LinkKind::Link => "link".to_owned(),
        }];
        words.extend(self.title.as_deref().map(one_line));
        if let LinkKind::Call {
            passcode: Some(passcode),
        } = &self.kind
        {
            words.push(format!("passcode {}", one_line(passcode)));
        }
        words.retain(|w| !w.is_empty());
        format!("[{}] {}", words.join(" "), self.url)
    }

    pub fn is_call(&self) -> bool {
        matches!(self.kind, LinkKind::Call { .. })
    }

    pub fn passcode(&self) -> Option<&str> {
        match &self.kind {
            LinkKind::Call { passcode } => passcode.as_deref(),
            LinkKind::Link => None,
        }
    }

    /// What the link is called: its title, else a call's service (among
    /// `services` and the well-known ones), else its host.
    pub fn name(&self, services: &[CallService]) -> String {
        if let Some(title) = &self.title {
            return title.clone();
        }
        (self.is_call())
            .then(|| recognize_call_link(self.url.as_str(), services))
            .flatten()
            .unwrap_or_else(|| self.host())
    }

    /// The host without "www.", e.g. "overleaf.com".
    pub fn host(&self) -> String {
        match self.url.host_str() {
            Some(host) => host.trim_start_matches("www.").to_owned(),
            None => self.url.to_string(),
        }
    }
}

/// `text` fit for inside the brackets: one line, no brackets.
fn one_line(text: &str) -> String {
    text.replace(['[', ']'], " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn link(kind: LinkKind, title: Option<&str>, url: &str) -> RoomLink {
        RoomLink {
            kind,
            title: title.map(Into::into),
            url: Url::parse(url).unwrap(),
        }
    }

    const CALL: LinkKind = LinkKind::Call { passcode: None };

    #[test]
    fn reads_links_from_their_lines() {
        let topic = Topic::parse(
            "PalMA-ice project chat.\n\
             [call] https://meet.google.com/abc-defg-hij\n\
             Weekly on Tuesdays.\n\
             [Call Weekly passcode 123 456] <https://zoom.us/j/1>\n\
             [link] https://drive.google.com/drive/folders/xyz\n\
             [link Paper draft] https://www.overleaf.com/project/abc\n",
        );
        assert_eq!(topic.text, "PalMA-ice project chat.\nWeekly on Tuesdays.");
        assert_eq!(
            topic.links,
            [
                link(CALL, None, "https://meet.google.com/abc-defg-hij"),
                link(
                    LinkKind::Call {
                        passcode: Some("123 456".into())
                    },
                    Some("Weekly"),
                    "https://zoom.us/j/1"
                ),
                link(
                    LinkKind::Link,
                    None,
                    "https://drive.google.com/drive/folders/xyz"
                ),
                link(
                    LinkKind::Link,
                    Some("Paper draft"),
                    "https://www.overleaf.com/project/abc"
                ),
            ]
        );
        assert_eq!(
            topic.default_call().map(|c| c.url.as_str()),
            Some("https://meet.google.com/abc-defg-hij")
        );
    }

    #[test]
    fn other_lines_stay_text() {
        for line in [
            "[call]",
            "[call] not a url",
            "[call] https://zoom.us/j/1 and more",
            "[call] mailto:a@b.c",
            "[meeting] https://zoom.us/j/1",
            "[] https://zoom.us/j/1",
            "Join at https://zoom.us/j/1",
        ] {
            let topic = Topic::parse(line);
            assert_eq!(
                (topic.text.as_str(), topic.links.len()),
                (line, 0),
                "{line}"
            );
        }
    }

    #[test]
    fn writes_the_lines_back() {
        let topic = Topic {
            text: "About.\n".into(),
            links: vec![
                link(
                    LinkKind::Call {
                        passcode: Some("123".into()),
                    },
                    Some("Weekly [Zoom]"),
                    "https://zoom.us/j/1",
                ),
                link(CALL, None, "https://meet.google.com/abc-defg-hij"),
                link(
                    LinkKind::Link,
                    Some("Draft\nv2"),
                    "https://drive.google.com",
                ),
            ],
        };
        let written = topic.to_topic();
        assert_eq!(
            written,
            "About.\n\
             [call Weekly Zoom passcode 123] https://zoom.us/j/1\n\
             [call] https://meet.google.com/abc-defg-hij\n\
             [link Draft v2] https://drive.google.com/"
        );
        let read = Topic::parse(&written);
        assert_eq!(read.text, "About.");
        assert_eq!(read.links[0].title.as_deref(), Some("Weekly Zoom"));
        assert_eq!(read.links[1], topic.links[1]);
        assert_eq!(
            Topic::parse(&written).to_topic(),
            written,
            "the same once tidied"
        );
        assert_eq!(
            Topic::parse("[link] https://a.org/\n[call] https://b.org/").to_topic(),
            "[call] https://b.org/\n[link] https://a.org/",
            "the calls first"
        );
    }

    #[test]
    fn describes_changes() {
        let old = "About.\n\
            [call] https://meet.google.com/abc-defg-hij\n\
            [call Weekly] https://zoom.us/j/1\n\
            [link Draft] https://overleaf.com/a";
        let describe = |new: &str| describe_change(old, new);
        assert_eq!(describe(old), None);
        assert_eq!(
            describe(&old.replace("About.", "About us.")).as_deref(),
            Some("changed the topic")
        );
        assert_eq!(
            describe(
                "About.\n\
                 [call Weekly] https://zoom.us/j/1\n\
                 [call] https://meet.google.com/abc-defg-hij\n\
                 [link Draft] https://overleaf.com/a"
            )
            .as_deref(),
            Some("made Weekly the room's call")
        );
        assert_eq!(
            describe(
                "About.\n\
                 [call] https://meet.google.com/abc-defg-hij\n\
                 [call Weekly passcode 1] https://zoom.us/j/1\n\
                 [link] https://drive.google.com/x"
            )
            .as_deref(),
            Some(
                "changed the call Weekly, added the link drive.google.com and removed the link \
                 Draft"
            )
        );
        assert_eq!(
            describe_change("", "[link] https://a.org/\n[link] https://b.org/").as_deref(),
            Some("added the link a.org and added the link b.org")
        );
        assert_eq!(
            describe_change(
                "[link] https://a.org/\n[link] https://b.org/",
                "[link] https://b.org/\n[link] https://a.org/"
            )
            .as_deref(),
            Some("reordered the calls and links")
        );
    }

    #[test]
    fn reads_topic_changes_from_the_timeline() {
        let event = |content: &str, unsigned: &str| {
            Raw::<AnySyncTimelineEvent>::from_json_string(format!(
                r#"{{"type": "m.room.topic", "state_key": "", "event_id": "$a",
                    "sender": "@alice:x", "origin_server_ts": 1, "content": {content},
                    "unsigned": {unsigned}}}"#
            ))
            .unwrap()
        };
        let topic = StateEventType::RoomTopic;
        let changed = event(
            r#"{"topic": "new"}"#,
            r#"{"prev_content": {"topic": "old"}}"#,
        );
        assert_eq!(
            topic_change(&topic, Some(&changed)),
            Some((Some("old".into()), "new".into()))
        );
        let first = event(r#"{"topic": "new"}"#, "{}");
        assert_eq!(
            topic_change(&topic, Some(&first)),
            Some((None, "new".into()))
        );
        assert_eq!(topic_change(&StateEventType::RoomName, Some(&first)), None);
        assert_eq!(topic_change(&topic, None), None);
    }

    #[test]
    fn takes_typed_web_addresses_only() {
        let url = |text: &str| typed_link_url(text).map(String::from);
        assert_eq!(url(" zoom.us/j/1 ").as_deref(), Some("https://zoom.us/j/1"));
        assert_eq!(
            url("zoom.us:443/j/1").as_deref(),
            Some("https://zoom.us/j/1")
        );
        assert_eq!(
            url("http://lab.org/call").as_deref(),
            Some("http://lab.org/call")
        );
        for text in [
            "",
            "mailto:a@b.c",
            "mailto:x",
            "ftp://lab.org/x",
            "https://",
            "https:///",
            "lab org",
        ] {
            assert_eq!(url(text), None, "{text}");
        }
        assert!(Topic::parse("[link] https://").links.is_empty(), "no host");
    }

    #[test]
    fn makes_topics_from_drafts() {
        let draft = |call, title: &str, url: &str| LinkDraft {
            call,
            title: title.into(),
            url: url.into(),
            passcode: if call { "42".into() } else { String::new() },
        };
        let (topic, dropped) = Topic::from_drafts(
            " About. \n",
            &[
                draft(false, "Drive", "drive.google.com/x"),
                draft(true, "", "zoom.us/j/1"),
                LinkDraft::default(),
                draft(false, "Mail", "mailto:a@b.c"),
                draft(true, "Weekly", "https://meet.google.com/abc-defg-hij"),
            ],
        );
        assert_eq!(dropped, 1, "the mail; the blank one left out");
        assert_eq!(
            topic.to_topic(),
            "About.\n\
             [call passcode 42] https://zoom.us/j/1\n\
             [call Weekly passcode 42] https://meet.google.com/abc-defg-hij\n\
             [link Drive] https://drive.google.com/x"
        );
        assert_eq!(
            topic.links.iter().map(LinkDraft::from).collect::<Vec<_>>()[0],
            LinkDraft {
                url: "https://zoom.us/j/1".into(),
                ..draft(true, "", "")
            }
        );
        let mail = draft(false, "", "mailto:x");
        assert_eq!(mail.link(), None);
        assert_eq!(mail.line(), "[link] mailto:x", "as typed");
    }

    #[test]
    fn previews_and_moves() {
        let topic = Topic::parse(
            "\n  About.\nMore.\n\
             [link Draft] https://overleaf.com/a\n\
             [call Weekly] https://zoom.us/j/1\n\
             [call] https://meet.google.com/abc-defg-hij",
        );
        assert_eq!(topic.first_line(), Some("About."));
        assert_eq!(topic.preview(&[]).as_deref(), Some("About."));
        let links = Topic {
            text: String::new(),
            ..topic.clone()
        };
        assert_eq!(links.first_line(), None);
        assert_eq!(
            links.preview(&[]).as_deref(),
            Some("Weekly · Google Meet · Draft"),
            "calls first"
        );
        assert!(!links.is_empty() && Topic::parse(" \n").is_empty());
        assert_eq!(Topic::default().preview(&[]), None);

        let moved = topic.clone().moved(true, 1, 0);
        assert_eq!(
            moved.to_topic(),
            "About.\nMore.\n\
             [call] https://meet.google.com/abc-defg-hij\n\
             [call Weekly] https://zoom.us/j/1\n\
             [link Draft] https://overleaf.com/a"
        );
        assert_eq!(
            moved.links[0].url.as_str(),
            "https://meet.google.com/abc-defg-hij",
            "calls first in the links too"
        );
        assert_eq!(
            topic.clone().moved(false, 0, 3).to_topic(),
            topic.to_topic(),
            "out of range"
        );
    }

    #[test]
    fn names_links() {
        let services = [CallService::fixed("My Zoom", "https://zoom.us/j/42").unwrap()];
        let name = |kind, title, url| link(kind, title, url).name(&services);
        assert_eq!(name(CALL, None, "https://zoom.us/j/42"), "My Zoom");
        assert_eq!(
            name(CALL, None, "https://meet.google.com/abc-defg-hij"),
            "Google Meet"
        );
        assert_eq!(name(CALL, Some("Weekly"), "https://zoom.us/j/42"), "Weekly");
        assert_eq!(name(CALL, None, "https://www.lab.org/call"), "lab.org");
        assert_eq!(
            name(LinkKind::Link, None, "https://meet.google.com/abc-defg-hij"),
            "meet.google.com",
            "a link is not a call"
        );
    }
}
