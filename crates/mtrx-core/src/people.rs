//! The people the user knows: whom they share spaces, rooms and direct
//! chats with. For suggesting people and showing who someone is.

use std::collections::HashMap;

use matrix_sdk::{
    Room, RoomMemberships,
    ruma::{
        OwnedMxcUri, OwnedRoomId, OwnedUserId, UserId,
        api::client::profile::{AvatarUrl, DisplayName},
    },
};
use tracing::warn;

use crate::{Result, Session};

/// Someone: their user ID, with their name and picture as far as known.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct User {
    pub id: OwnedUserId,
    /// Display name, if they set one.
    pub name: Option<String>,
    pub avatar: Option<OwnedMxcUri>,
}

impl User {
    /// Their name, else their user ID's localpart.
    pub fn display_name(&self) -> String {
        match self.name.as_deref().map(str::trim) {
            Some(name) if !name.is_empty() => name.to_owned(),
            _ => self.localpart().to_owned(),
        }
    }

    /// `alice` of `@alice:example.org`.
    pub fn localpart(&self) -> &str {
        self.id.localpart()
    }
}

/// Someone whose name and picture are not known.
impl From<OwnedUserId> for User {
    fn from(id: OwnedUserId) -> Self {
        Self {
            id,
            name: None,
            avatar: None,
        }
    }
}

/// Someone the user shares spaces, rooms or a direct chat with.
#[derive(Clone, Debug, PartialEq)]
pub struct Contact {
    pub user: User,
    /// Joined spaces both are in.
    pub spaces: Vec<OwnedRoomId>,
    /// Joined rooms (not spaces) both are in, direct chats included.
    pub rooms: Vec<OwnedRoomId>,
    /// We have a direct chat with them.
    pub direct: bool,
}

impl Contact {
    /// Higher for people the user has more to do with.
    fn closeness(&self) -> usize {
        self.spaces.len() * 4 + self.rooms.len() + usize::from(self.direct) * 6
    }
}

/// Who someone is, for their info card.
#[derive(Clone, Debug, PartialEq)]
pub struct UserInfo {
    pub user: User,
    /// Spaces and rooms shared with them, as far as known.
    pub shared: Vec<OwnedRoomId>,
}

impl Session {
    /// The people the user shares spaces, rooms or direct chats with (joined
    /// or invited),
    /// closest first. Loads the member lists of spaces from the server once;
    /// rooms count with the members known locally.
    pub async fn contacts(&self) -> Result<Vec<Contact>> {
        let own = self.user_id();
        let mut contacts: HashMap<OwnedUserId, Contact> = HashMap::new();
        for room in self.client.joined_rooms() {
            let space = room.is_space();
            let direct = room.is_direct().await.unwrap_or(false);
            // Invited people count: the user chose them.
            let memberships = RoomMemberships::JOIN | RoomMemberships::INVITE;
            let members = if space {
                room.members(memberships).await
            } else {
                room.members_no_sync(memberships).await
            };
            let members = match members {
                Ok(members) => members,
                Err(e) => {
                    warn!(room = %room.room_id(), "could not read members: {e}");
                    continue;
                }
            };
            for member in members.into_iter().filter(|m| m.user_id() != own) {
                let contact = contacts
                    .entry(member.user_id().to_owned())
                    .or_insert_with(|| Contact {
                        user: User {
                            id: member.user_id().to_owned(),
                            name: member.display_name().map(ToOwned::to_owned),
                            avatar: member.avatar_url().map(ToOwned::to_owned),
                        },
                        spaces: Vec::new(),
                        rooms: Vec::new(),
                        direct: false,
                    });
                if space {
                    contact.spaces.push(room.room_id().to_owned());
                } else {
                    contact.rooms.push(room.room_id().to_owned());
                }
                contact.direct |= direct;
            }
        }
        let mut contacts: Vec<_> = contacts.into_values().collect();
        contacts.sort_by_cached_key(|c| {
            let name = c.user.name.clone().unwrap_or_default().to_lowercase();
            (std::cmp::Reverse(c.closeness()), name)
        });
        Ok(contacts)
    }

    /// A user's name and picture from their profile on the server.
    pub async fn user_profile(&self, user: &UserId) -> Result<User> {
        let profile = self.client.account().fetch_user_profile_of(user).await?;
        Ok(User {
            id: user.to_owned(),
            name: profile.get_static::<DisplayName>().ok().flatten(),
            avatar: profile.get_static::<AvatarUrl>().ok().flatten(),
        })
    }

    /// A user's name and picture from their profile, and what we share.
    pub async fn user_info(&self, user: &UserId) -> Result<UserInfo> {
        let user = self.user_profile(user).await.unwrap_or_else(|e| {
            warn!("could not load the profile of {user}: {e}");
            User::from(user.to_owned())
        });
        let shared = self.shared_rooms(&user.id).await;
        Ok(UserInfo { user, shared })
    }

    /// The joined rooms and spaces `user` is in too, direct chats included.
    pub async fn shared_rooms(&self, user: &UserId) -> Vec<OwnedRoomId> {
        let mut shared = Vec::new();
        for room in self.client.joined_rooms() {
            if is_member(&room, user).await {
                shared.push(room.room_id().to_owned());
            }
        }
        shared
    }
}

/// Joined member of `room`, by the members known locally (spaces load
/// theirs from the server once).
async fn is_member(room: &Room, user: &UserId) -> bool {
    let member = if room.is_space() {
        room.get_member(user).await
    } else {
        room.get_member_no_sync(user).await
    };
    member.ok().flatten().is_some_and(|m| {
        *m.membership() == matrix_sdk::ruma::events::room::member::MembershipState::Join
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_name_else_the_localpart() {
        let mut ann = User::from(OwnedUserId::try_from("@ann:example.org").unwrap());
        assert_eq!(ann.display_name(), "ann");
        ann.name = Some(" Ann ".into());
        assert_eq!(ann.display_name(), "Ann");
        ann.name = Some(" ".into());
        assert_eq!(ann.display_name(), "ann");
        assert_eq!(ann.localpart(), "ann");
    }
}
