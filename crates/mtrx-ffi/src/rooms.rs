//! Finding, joining, making and leaving rooms and spaces: a space's rooms,
//! a server's directory, an address or link; answering invites; a room's
//! topic and its links.

use mtrx_core::{self as core, OwnedServerName, OwnedUserId, RoomOrAliasId};

use crate::{MtrxError, Role, Session, error::room_id, runtime::run};

/// Who may join a listed room.
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum JoinAccess {
    Public,
    /// Members of a space it is in.
    SpaceMembers,
    /// Anyone may ask; members let them in.
    Knock,
    /// Only invited people (servers rarely list these).
    Invite,
}

/// Our membership of a listed room, if any.
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum Membership {
    Joined,
    Invited,
    /// We asked to join, and wait to be let in.
    Asked,
}

/// A room or space to join, as a space or a directory lists it.
#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct DirectoryRoom {
    pub id: String,
    pub alias: Option<String>,
    /// Its name, else its address, else its ID.
    pub name: String,
    pub topic: Option<String>,
    /// `mxc://` URI, for `Session.avatar`.
    pub avatar_url: Option<String>,
    pub members: u64,
    pub is_space: bool,
    /// For a space, how many rooms and spaces it lists.
    pub children: u64,
    pub access: JoinAccess,
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
            topic: room.topic,
            avatar_url: room.avatar.map(|a| a.to_string()),
            members: room.members,
            is_space: room.is_space,
            children: room.children,
            access: match room.access {
                core::JoinAccess::Public => JoinAccess::Public,
                core::JoinAccess::SpaceMembers => JoinAccess::SpaceMembers,
                core::JoinAccess::Knock => JoinAccess::Knock,
                core::JoinAccess::Invite => JoinAccess::Invite,
            },
            membership: room.membership.map(|m| match m {
                core::Membership::Joined => Membership::Joined,
                core::Membership::Invited => Membership::Invited,
                core::Membership::Asked => Membership::Asked,
            }),
            role: room.role.map(Role::from),
            via: room.via.iter().map(ToString::to_string).collect(),
            suggested: room.suggested,
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

/// A link on its own line in a topic: a call or another link.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct RoomLink {
    pub call: bool,
    pub title: Option<String>,
    pub url: String,
    /// A call's passcode, if any.
    pub passcode: Option<String>,
    /// What to call it: its title, else the call's service, else its host.
    pub name: String,
}

/// A room's topic: its text, and the links on lines of their own.
#[derive(Clone, Debug, Default, PartialEq, Eq, uniffi::Record)]
pub struct Topic {
    pub text: String,
    pub links: Vec<RoomLink>,
}

/// `topic` as its text and links (`[call] …` and `[link] …` lines).
#[uniffi::export]
pub fn parse_topic(topic: String) -> Topic {
    let parsed = core::Topic::parse(&topic);
    Topic {
        links: parsed
            .links
            .iter()
            .map(|link| RoomLink {
                call: link.is_call(),
                title: link.title.clone(),
                url: link.url.to_string(),
                passcode: link.passcode().map(ToOwned::to_owned),
                name: link.name(&[]),
            })
            .collect(),
        text: parsed.text,
    }
}

/// The topic to save for `topic`'s text and links; links whose URL is not
/// valid are left out.
#[uniffi::export]
pub fn compose_topic(topic: Topic) -> String {
    core::Topic {
        text: topic.text,
        links: topic
            .links
            .into_iter()
            .filter_map(|link| {
                Some(core::RoomLink {
                    kind: if link.call {
                        core::LinkKind::Call {
                            passcode: link.passcode.filter(|p| !p.trim().is_empty()),
                        }
                    } else {
                        core::LinkKind::Link
                    },
                    title: link.title.filter(|t| !t.trim().is_empty()),
                    url: core::Url::parse(link.url.trim()).ok()?,
                })
            })
            .collect(),
    }
    .to_topic()
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
    /// A space's rooms and subspaces as its server lists them; rooms only
    /// invited people may join are left out.
    pub async fn space_children(&self, space_id: String) -> Result<Vec<DirectoryRoom>, MtrxError> {
        let space = room_id(&space_id)?;
        let session = self.core();
        run(async move {
            let rooms = session.space_children(&space).await?;
            Ok(rooms.into_iter().map(DirectoryRoom::from).collect())
        })
        .await
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
    fn topic_links_round_trip() {
        let topic = parse_topic(
            "Ice group\n[call Weekly passcode 42] https://meet.example.org/ice\n\
             [link] https://example.org/drive"
                .into(),
        );
        assert_eq!(topic.text, "Ice group");
        assert_eq!(topic.links.len(), 2);
        assert!(topic.links[0].call);
        assert_eq!(topic.links[0].name, "Weekly");
        assert_eq!(topic.links[0].passcode.as_deref(), Some("42"));
        assert_eq!(topic.links[1].name, "example.org");
        assert_eq!(parse_topic(compose_topic(topic.clone())), topic);
    }

    #[test]
    fn addresses() {
        let address = parse_address("https://matrix.to/#/#ice:example.org".into()).unwrap();
        assert_eq!(address.room, "#ice:example.org");
        assert!(parse_address("not an address".into()).is_none());
    }
}
