//! Finding, joining, making and leaving rooms and spaces: a space's rooms,
//! a server's directory, an address or link; answering invites; a room's
//! topic and its links.

use mtrx_core::{self as core, OwnedServerName, OwnedUserId, RoomOrAliasId};

use crate::{JoinRule, Membership, MtrxError, Role, Session, error::room_id, runtime::run};

/// A room or space to join, as a space or a directory lists it.
#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct DirectoryRoom {
    pub id: String,
    pub alias: Option<String>,
    /// Its name, else its address, else its ID.
    pub name: String,
    pub topic: Topic,
    /// `mxc://` URI, for `Session.avatar`.
    pub avatar_url: Option<String>,
    pub members: u64,
    pub is_space: bool,
    /// For a space, how many rooms and spaces it lists.
    pub children: u64,
    pub join_rule: JoinRule,
    /// Who may join, in words: "Public: anyone can join"…
    pub join_rule_fact: String,
    /// As the server had it when listed; what to show is
    /// `RoomLists.joinButton`.
    pub membership: Option<Membership>,
    /// Ours, where we are a member.
    pub role: Option<Role>,
    /// Servers to join it through: pass to `Session.join` or `knock`.
    pub via: Vec<String>,
    /// The space's admins suggest it to newcomers.
    pub suggested: bool,
}

impl From<core::DirectoryRoom> for DirectoryRoom {
    fn from(room: core::DirectoryRoom) -> Self {
        Self {
            id: room.id.to_string(),
            alias: room.alias.map(|a| a.to_string()),
            name: room.name,
            topic: Topic::from(&room.topic),
            avatar_url: room.avatar.map(|a| a.to_string()),
            members: room.members,
            is_space: room.is_space,
            children: room.children,
            join_rule: room.join_rule.into(),
            join_rule_fact: room.join_rule.fact().to_owned(),
            membership: room.membership.map(Membership::from),
            role: room.role.map(Role::from),
            via: room.via.iter().map(ToString::to_string).collect(),
            suggested: room.suggested,
        }
    }
}

/// What a listed room's button does, as the room lets us in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum JoinAction {
    /// We are in: open it.
    Open,
    Join,
    AskToJoin,
    /// An invite, to answer.
    Invited,
    /// We asked, and wait: nothing to do.
    Asked,
    /// Only invited people may join: nothing to do.
    InviteOnly,
}

impl From<core::JoinAction> for JoinAction {
    fn from(action: core::JoinAction) -> Self {
        match action {
            core::JoinAction::Open => Self::Open,
            core::JoinAction::Join => Self::Join,
            core::JoinAction::AskToJoin => Self::AskToJoin,
            core::JoinAction::Invited => Self::Invited,
            core::JoinAction::Asked => Self::Asked,
            core::JoinAction::InviteOnly => Self::InviteOnly,
        }
    }
}

/// A listed room's button (or note), from `RoomLists.joinButton`.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct JoinButton {
    pub action: JoinAction,
    /// "Join", "Ask to join", "Open", "Invited", "Asked", "Invite only".
    pub title: String,
}

impl From<core::JoinAction> for JoinButton {
    fn from(action: core::JoinAction) -> Self {
        Self {
            action: action.into(),
            title: action.title().to_owned(),
        }
    }
}

/// Which directory entries to list.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, uniffi::Enum)]
pub enum DirectoryKind {
    #[default]
    All,
    Spaces,
    Rooms,
}

/// One page of a server's public directory.
#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct DirectoryPage {
    pub rooms: Vec<DirectoryRoom>,
    /// Pass to `Session.publicRooms` for the next page, if there is one.
    pub next: Option<String>,
}

/// A room or space as an address or link names it: what to preview or
/// join, and the servers to go through.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct Address {
    /// `!id:server` or `#alias:server`.
    pub room: String,
    pub via: Vec<String>,
}

/// What a new room or space is to be.
#[derive(Clone, Debug, Default, PartialEq, Eq, uniffi::Record)]
pub struct NewRoom {
    pub name: String,
    pub topic: Option<String>,
    pub space: bool,
    /// Not for a space.
    pub encrypted: bool,
    /// Only moderators may invite and edit it; else every member.
    pub moderated: bool,
    pub invite: Vec<String>,
    /// Owners like us, among those invited; only where
    /// `Session.offersOwners()`.
    pub co_owners: Vec<String>,
    /// The space to make it in, if any.
    pub parent: Option<String>,
}

/// A call or link on its own line in a topic. Output only: the editor
/// types `LinkDraft`s.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct RoomLink {
    pub title: Option<String>,
    pub url: String,
    /// A call's passcode, if any.
    pub passcode: Option<String>,
    /// What to call it: its title, else the call's service, else its host.
    pub name: String,
}

/// A room's topic, parsed by core: its text, and the calls and links on
/// lines of their own.
#[derive(Clone, Debug, Default, PartialEq, Eq, uniffi::Record)]
pub struct Topic {
    /// Without the link lines.
    pub text: String,
    /// In order; the first is the room's call, which the header joins.
    pub calls: Vec<RoomLink>,
    /// The other links, in order.
    pub links: Vec<RoomLink>,
    /// The text's first non-blank line: list rows and directory entries.
    pub first_line: Option<String>,
    /// The first line, else the links' names: under the room's name.
    pub preview: Option<String>,
}

impl From<&core::Topic> for Topic {
    /// The calls named after the well-known services: the phones have no
    /// call services of their own.
    fn from(topic: &core::Topic) -> Self {
        let link = |link: &core::RoomLink| RoomLink {
            title: link.title.clone(),
            url: link.url.to_string(),
            passcode: link.passcode().map(ToOwned::to_owned),
            name: link.name(&[]),
        };
        Self {
            text: topic.text.clone(),
            calls: topic.calls().map(link).collect(),
            links: topic.others().map(link).collect(),
            first_line: topic.first_line().map(ToOwned::to_owned),
            preview: topic.preview(&[]),
        }
    }
}

impl TryFrom<&Topic> for core::Topic {
    type Error = MtrxError;

    /// Back from the record (the apps' tests).
    fn try_from(topic: &Topic) -> Result<Self, MtrxError> {
        let link = |link: &RoomLink, call: bool| {
            Ok(core::RoomLink {
                kind: match call {
                    true => core::LinkKind::Call {
                        passcode: link.passcode.clone(),
                    },
                    false => core::LinkKind::Link,
                },
                title: link.title.clone(),
                url: core::Url::parse(&link.url).map_err(MtrxError::invalid)?,
            })
        };
        let calls = topic.calls.iter().map(|l| link(l, true));
        Ok(Self {
            text: topic.text.clone(),
            links: calls
                .chain(topic.links.iter().map(|l| link(l, false)))
                .collect::<Result<_, MtrxError>>()?,
        })
    }
}

/// A call or link as typed in the topic editor.
#[derive(Clone, Debug, Default, PartialEq, Eq, uniffi::Record)]
pub struct LinkDraft {
    pub call: bool,
    pub title: String,
    pub url: String,
    /// A call's; ignored for a link.
    pub passcode: String,
}

impl From<LinkDraft> for core::LinkDraft {
    fn from(draft: LinkDraft) -> Self {
        Self {
            call: draft.call,
            title: draft.title,
            url: draft.url,
            passcode: draft.passcode,
        }
    }
}

/// The address a typed URL is saved as ("example.org/x" →
/// "https://example.org/x"), or none if it isn't a web address: the editor
/// marks the field.
#[uniffi::export]
pub fn link_url(text: String) -> Option<String> {
    core::typed_link_url(&text).map(String::from)
}

/// The topic to save, for `Room.setTopic`, from typed text and links: the
/// calls first, each kind in its order; blank links and those without a web
/// address left out.
#[uniffi::export]
pub fn compose_topic(text: String, links: Vec<LinkDraft>) -> String {
    let links: Vec<core::LinkDraft> = links.into_iter().map(Into::into).collect();
    core::Topic::from_drafts(&text, &links).0.to_topic()
}

/// The room an address or link names (`#alias:server`, `!id:server`, a
/// matrix.to link, a `matrix:` URI), if any.
#[uniffi::export]
pub fn parse_address(text: String) -> Option<Address> {
    let (room, via) = core::parse_address(&text)?;
    Some(Address {
        room: room.to_string(),
        via: via.iter().map(ToString::to_string).collect(),
    })
}

fn address(room: &str) -> Result<core::OwnedRoomOrAliasId, MtrxError> {
    <&RoomOrAliasId>::try_from(room)
        .map(ToOwned::to_owned)
        .map_err(MtrxError::invalid)
}

fn servers(via: &[String]) -> Result<Vec<OwnedServerName>, MtrxError> {
    via.iter()
        .map(|s| OwnedServerName::try_from(s.as_str()).map_err(MtrxError::invalid))
        .collect()
}

fn users(ids: &[String]) -> Result<Vec<OwnedUserId>, MtrxError> {
    ids.iter()
        .map(|id| OwnedUserId::try_from(id.as_str()).map_err(MtrxError::invalid))
        .collect()
}

#[uniffi::export]
impl Session {
    /// A space's rooms and subspaces as its server lists them, with the
    /// ones we are in or invited to that it leaves out; rooms only invited
    /// people may join are left out. Its server is asked again after a
    /// minute.
    pub async fn space_children(&self, space_id: String) -> Result<Vec<DirectoryRoom>, MtrxError> {
        let space = room_id(&space_id)?;
        let session = self.core();
        run(async move {
            let rooms = session.space_children(&space).await?;
            Ok(rooms.into_iter().map(DirectoryRoom::from).collect())
        })
        .await
    }

    /// A space's rooms as `spaceChildren` last gave them, however long ago,
    /// to show at once; none before.
    pub fn listed_space_children(&self, space_id: String) -> Option<Vec<DirectoryRoom>> {
        let space = room_id(&space_id).ok()?;
        let rooms = self.core().listed_space_children(&space)?;
        Some(rooms.into_iter().map(DirectoryRoom::from).collect())
    }

    /// A page of `server`'s public directory (ours if `None`) matching
    /// `query`; `page` from the last page's `next`.
    pub async fn public_rooms(
        &self,
        server: Option<String>,
        query: String,
        kind: DirectoryKind,
        page: Option<String>,
    ) -> Result<DirectoryPage, MtrxError> {
        let server = server
            .map(|s| OwnedServerName::try_from(s.as_str()).map_err(MtrxError::invalid))
            .transpose()?;
        let kind = match kind {
            DirectoryKind::All => core::DirectoryKind::All,
            DirectoryKind::Spaces => core::DirectoryKind::Spaces,
            DirectoryKind::Rooms => core::DirectoryKind::Rooms,
        };
        let session = self.core();
        run(async move {
            let page = session
                .public_rooms(server.as_deref(), &query, kind, page)
                .await?;
            Ok(DirectoryPage {
                rooms: page.rooms.into_iter().map(DirectoryRoom::from).collect(),
                next: page.next,
            })
        })
        .await
    }

    /// What a room or space is, before joining it.
    pub async fn preview(&self, address: Address) -> Result<DirectoryRoom, MtrxError> {
        let (room, via) = (self::address(&address.room)?, servers(&address.via)?);
        let session = self.core();
        run(async move { Ok(session.preview(&room, via).await?.into()) }).await
    }

    /// Join a room or space (`!id`, `#alias`), or accept an invite to it,
    /// through `via`; its room ID.
    pub async fn join(&self, room: String, via: Vec<String>) -> Result<String, MtrxError> {
        let (room, via) = (address(&room)?, servers(&via)?);
        let session = self.core();
        run(async move { Ok(session.join(&room, &via).await?.to_string()) }).await
    }

    /// Ask to join a room that takes requests; its room ID.
    pub async fn knock(&self, room: String, via: Vec<String>) -> Result<String, MtrxError> {
        let (room, via) = (address(&room)?, servers(&via)?);
        let session = self.core();
        run(async move { Ok(session.knock(&room, via).await?.to_string()) }).await
    }

    /// Servers whose directories we browsed, most recent first.
    pub fn directory_servers(&self) -> Vec<String> {
        self.core().directory_servers().servers
    }

    /// Remember a server whose directory we browsed (synced to our other
    /// devices).
    pub async fn remember_directory_server(&self, server: String) -> Result<(), MtrxError> {
        let server = OwnedServerName::try_from(server.as_str()).map_err(MtrxError::invalid)?;
        let session = self.core();
        run(async move { Ok(session.remember_directory_server(&server).await?) }).await
    }

    /// IDs of the joined rooms and spaces a user is in too (for a direct
    /// chat's list of the rooms shared with the person), direct chats
    /// included.
    pub async fn shared_rooms(&self, user_id: String) -> Result<Vec<String>, MtrxError> {
        let user = crate::error::user_id(&user_id)?;
        let session = self.core();
        run(async move {
            Ok(session
                .shared_rooms(&user)
                .await
                .iter()
                .map(ToString::to_string)
                .collect())
        })
        .await
    }

    /// Whether new rooms may have co-owners (room version 12 offered).
    pub async fn offers_owners(&self) -> bool {
        let session = self.core();
        run(async move { Ok::<_, MtrxError>(session.offers_owners().await) })
            .await
            .unwrap_or(false)
    }

    /// Make a room or space; its room ID.
    pub async fn create_room(&self, new: NewRoom) -> Result<String, MtrxError> {
        let new = core::NewRoom {
            name: Some(new.name),
            topic: new.topic.filter(|t| !t.trim().is_empty()),
            space: new.space,
            encrypted: new.encrypted && !new.space,
            invite: users(&new.invite)?,
            direct: false,
            permissions: if new.moderated {
                core::Permissions::Moderators
            } else {
                core::Permissions::Everyone
            },
            parent: new.parent.as_deref().map(room_id).transpose()?,
            co_owners: users(&new.co_owners)?,
        };
        let session = self.core();
        run(async move { Ok(session.create_room(new).await?.to_string()) }).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn topics_split_calls_and_links() {
        let parsed = core::Topic::parse(
            "[link] https://example.org/drive\n\
             [call Weekly passcode 42] https://meet.example.org/ice\n\
             [call] https://meet.google.com/abc-defg-hij",
        );
        let topic = Topic::from(&parsed);
        assert_eq!(topic.text, "");
        let names = |links: &[RoomLink]| links.iter().map(|l| l.name.clone()).collect::<Vec<_>>();
        assert_eq!(names(&topic.calls), ["Weekly", "Google Meet"]);
        assert_eq!(names(&topic.links), ["example.org"]);
        assert_eq!(topic.calls[0].passcode.as_deref(), Some("42"));
        assert_eq!(topic.first_line, None);
        assert_eq!(
            topic.preview.as_deref(),
            Some("Weekly · Google Meet · example.org")
        );
        assert_eq!(
            core::Topic::try_from(&topic).unwrap(),
            parsed.clone().moved(true, 0, 0),
            "back, the calls first"
        );
        let text = Topic::from(&core::Topic::parse("\nIce group\nMore"));
        assert_eq!(
            (text.first_line.as_deref(), text.preview.as_deref()),
            (Some("Ice group"), Some("Ice group"))
        );
    }

    #[test]
    fn composes_typed_topics() {
        let draft = |call, title: &str, url: &str, passcode: &str| LinkDraft {
            call,
            title: title.into(),
            url: url.into(),
            passcode: passcode.into(),
        };
        let composed = compose_topic(
            " Ice group ".into(),
            vec![
                draft(false, "Drive", "example.org/drive", "stale"),
                // Dragged above the other: the room's call now.
                draft(true, "Weekly", "meet.example.org/ice", "42"),
                draft(false, "", "mailto:x", ""),
                draft(false, "", "", "stale"),
                draft(true, "", "https://meet.google.com/abc-defg-hij", ""),
            ],
        );
        assert_eq!(
            composed,
            "Ice group\n\
             [call Weekly passcode 42] https://meet.example.org/ice\n\
             [call] https://meet.google.com/abc-defg-hij\n\
             [link Drive] https://example.org/drive",
            "the calls first, in order; the mail and the blank link left out"
        );
        assert_eq!(
            link_url("example.org/x".into()).as_deref(),
            Some("https://example.org/x")
        );
        assert_eq!(link_url("mailto:x".into()), None);
    }

    #[test]
    fn addresses() {
        let address = parse_address("https://matrix.to/#/#ice:example.org".into()).unwrap();
        assert_eq!(address.room, "#ice:example.org");
        assert!(parse_address("not an address".into()).is_none());
    }
}
