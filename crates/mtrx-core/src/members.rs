//! The people in a room.

use std::collections::HashSet;

use matrix_sdk::{
    RoomMemberships,
    ruma::{
        Int, OwnedMxcUri, OwnedUserId, RoomId, UserId,
        events::room::member::MembershipState,
        events::room::power_levels::{RoomPowerLevels, UserPowerLevel},
        int,
    },
};

use crate::{Result, Session};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Member {
    pub id: OwnedUserId,
    pub name: Option<String>,
    pub avatar: Option<OwnedMxcUri>,
    pub role: Role,
    /// Their power level; `None` for a creator of the room, above all levels.
    pub level: Option<i64>,
    /// Invited, not joined yet.
    pub invited: bool,
    /// Ourselves.
    pub own: bool,
    /// What we may do to them.
    pub manage: Manage,
}

/// What we may do to a member, by the room's power levels.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Manage {
    pub remove: bool,
    pub ban: bool,
    /// The highest power level we may give them, if we may change theirs:
    /// ours, or below it for ourselves.
    pub up_to: Option<i64>,
    /// A plain member's level in the room.
    pub default_level: i64,
    /// Ourselves, and nobody else joined has our level or more: stepping
    /// down leaves nobody able to do all we can.
    pub alone: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Role {
    Admin,
    Moderator,
    Member,
}

impl Role {
    /// The power level the role stands for; `default` for a member.
    pub fn level(self, default: i64) -> i64 {
        match self {
            Role::Admin => 100,
            Role::Moderator => 50,
            Role::Member => default,
        }
    }
}

impl Session {
    /// Joined and invited members, admins first, then by name. Loads the
    /// member list from the server if it is not complete yet.
    pub async fn members(&self, room_id: &RoomId) -> Result<Vec<Member>> {
        let room = self.room(room_id)?;
        let levels = crate::permissions::power_levels(&room).await.ok();
        let own = self.user_id();
        let mut members: Vec<Member> = room
            .members(RoomMemberships::JOIN | RoomMemberships::INVITE)
            .await?
            .into_iter()
            .map(|member| {
                let id = member.user_id().to_owned();
                let level = member.power_level();
                let manage = match &levels {
                    Some(levels) => manage(levels, &own, &id),
                    None => Manage::default(),
                };
                Member {
                    name: member.display_name().map(ToOwned::to_owned),
                    avatar: member.avatar_url().map(ToOwned::to_owned),
                    role: role(level),
                    level: match level {
                        UserPowerLevel::Int(level) => Some(level.into()),
                        _ => None,
                    },
                    invited: *member.membership() == MembershipState::Invite,
                    own: id == own,
                    manage,
                    id,
                }
            })
            .collect();
        let alone = alone(&members);
        if let Some(us) = members.iter_mut().find(|m| m.own) {
            us.manage.alone = alone;
        }
        members.sort_by_cached_key(|m| {
            let name = m
                .name
                .clone()
                .unwrap_or_else(|| m.id.localpart().to_owned());
            (m.invited, m.role, name.to_lowercase())
        });
        Ok(members)
    }

    /// The IDs of the joined members. Loads the member list from the server
    /// if it is not complete yet.
    pub async fn joined_member_ids(&self, room_id: &RoomId) -> Result<HashSet<OwnedUserId>> {
        let room = self.room(room_id)?;
        Ok(room
            .members(RoomMemberships::JOIN)
            .await?
            .iter()
            .map(|member| member.user_id().to_owned())
            .collect())
    }

    /// Remove someone from a room (kick), or also keep them out (ban).
    pub async fn remove_member(
        &self,
        room_id: &RoomId,
        user: &UserId,
        reason: Option<&str>,
        ban: bool,
    ) -> Result<()> {
        let room = self.room(room_id)?;
        if ban {
            room.ban_user(user, reason).await?;
        } else {
            room.kick_user(user, reason).await?;
        }
        Ok(())
    }

    /// Give someone, or ourselves, a power level in a room.
    pub async fn set_level(&self, room_id: &RoomId, user: &UserId, level: i64) -> Result<()> {
        let room = self.room(room_id)?;
        room.update_power_levels(vec![(user, Int::new_saturating(level))])
            .await?;
        Ok(())
    }
}

/// What `own` may do to `target` (perhaps themselves).
fn manage(levels: &RoomPowerLevels, own: &UserId, target: &UserId) -> Manage {
    let default_level = levels.users_default.into();
    let ours = match levels.for_user(own) {
        UserPowerLevel::Int(level) => i64::from(level),
        // A creator gives any level.
        _ => i64::from(Int::MAX),
    };
    // At most our own; lowering ourselves, below it.
    let up_to = levels
        .user_can_change_user_power_level(own, target)
        .then(|| if target == own { ours - 1 } else { ours });
    if target == own {
        return Manage {
            up_to,
            default_level,
            ..Manage::default()
        };
    }
    Manage {
        remove: levels.user_can_kick_user(own, target),
        ban: levels.user_can_ban_user(own, target),
        up_to,
        default_level,
        alone: false,
    }
}

/// A level in a member's menu.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LevelChoice {
    pub title: String,
    pub level: i64,
    /// Theirs: ticked.
    pub current: bool,
    /// What to ask before giving it, if it can't be undone.
    pub confirmation: Option<Confirmation>,
}

/// A question before a change of level that can't be undone.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Confirmation {
    pub title: String,
    pub message: String,
    pub button: String,
}

impl Member {
    /// Their name, else their user ID.
    pub fn shown_name(&self) -> String {
        shown_name(self.name.as_deref(), self.id.as_str())
    }

    /// "Owner" (a creator, above every level), "Admin", "Mod", "Level 20",
    /// or nothing for a plain member.
    pub fn label(&self) -> Option<String> {
        match self.level {
            None => Some("Owner".into()),
            Some(100) => Some("Admin".into()),
            Some(50) => Some("Mod".into()),
            Some(level) if level == self.manage.default_level => None,
            Some(level) => Some(format!("Level {level}")),
        }
    }

    /// The levels we may give them, theirs ticked: Admin 100, Moderator 50,
    /// Member (the room's default), each if theirs or at most
    /// [`Manage::up_to`]; theirs first if none of those. Empty if we may not
    /// change theirs.
    pub fn level_choices(&self) -> Vec<LevelChoice> {
        let Some(up_to) = self.manage.up_to else {
            return Vec::new();
        };
        let mut levels = vec![("Admin", 100), ("Moderator", 50)];
        if levels
            .iter()
            .all(|(_, at)| *at != self.manage.default_level)
        {
            levels.push(("Member", self.manage.default_level));
        }
        let choice = |title: String, level: i64| LevelChoice {
            title,
            level,
            current: self.level == Some(level),
            confirmation: self.confirmation(level),
        };
        let custom = self
            .level
            .filter(|level| levels.iter().all(|(_, at)| at != level))
            .map(|level| choice(format!("Level {level}"), level));
        custom
            .into_iter()
            .chain(
                levels
                    .into_iter()
                    .filter(|(_, level)| self.level == Some(*level) || *level <= up_to)
                    .map(|(title, level)| choice(title.into(), level)),
            )
            .collect()
    }

    /// What to ask before giving them `level`, or nothing to just give it:
    /// any change to our own, or a level as high as ours.
    pub fn confirmation(&self, level: i64) -> Option<Confirmation> {
        level_confirmation(&self.shown_name(), self.own, &self.manage, level)
    }

    /// We may do something to them: change their level, remove or ban them.
    pub fn manageable(&self) -> bool {
        self.manage.up_to.is_some() || self.manage.remove || self.manage.ban
    }
}

/// A member's name, else their user ID.
pub fn shown_name(name: Option<&str>, id: &str) -> String {
    match name.map(str::trim) {
        Some(name) if !name.is_empty() => name.to_owned(),
        _ => id.to_owned(),
    }
}

/// What to ask before giving `level` to someone named `name` (`own`:
/// ourselves), or nothing to just give it: any change to our own, or a
/// level as high as ours.
pub fn level_confirmation(
    name: &str,
    own: bool,
    manage: &Manage,
    level: i64,
) -> Option<Confirmation> {
    let up_to = manage.up_to?;
    let default_level = manage.default_level;
    if own {
        let mut message =
            "You can't undo this: only someone with a higher level can raise you again.".to_owned();
        if manage.alone {
            message.push_str(
                " Nobody else here has your level, so nobody will be able to do all you can now.",
            );
        }
        return Some(Confirmation {
            title: format!("Step down to {}?", role_name(level, default_level)),
            message,
            button: "Step down".into(),
        });
    }
    if level < up_to {
        return None;
    }
    let message = if level == 100 {
        "Admins can do everything you can, including changing the room and its people. \
         You can't take it back: an admin can't demote another admin."
    } else {
        "They'll have the same level as you. You can't take it back: nobody can lower \
         someone with their own level."
    };
    Some(Confirmation {
        title: format!("Make {name} {}?", a_role(level, default_level)),
        message: message.into(),
        button: "Change".into(),
    })
}

/// What `level` makes someone: "admin", "moderator", "member" or "level 20".
pub fn role_name(level: i64, default_level: i64) -> String {
    match level {
        100 => "admin".into(),
        50 => "moderator".into(),
        level if level == default_level => "member".into(),
        level => format!("level {level}"),
    }
}

/// The role with its article: "an admin", "a member", "level 20".
pub fn a_role(level: i64, default_level: i64) -> String {
    match role_name(level, default_level) {
        role if role.starts_with("level") => role,
        role if role.starts_with('a') => format!("an {role}"),
        role => format!("a {role}"),
    }
}

/// A level typed for a custom level: a number, at most `up_to`.
pub fn typed_level(text: &str, up_to: i64) -> Option<i64> {
    text.trim().parse().ok().filter(|level| *level <= up_to)
}

/// Nobody joined but us has our level or more (a creator's is above all).
fn alone(members: &[Member]) -> bool {
    let level = |m: &Member| m.level.unwrap_or(i64::MAX);
    let joined = || members.iter().filter(|m| !m.invited);
    joined()
        .find(|m| m.own)
        .is_some_and(|us| !joined().any(|m| !m.own && level(m) >= level(us)))
}

pub(crate) fn role(level: UserPowerLevel) -> Role {
    match level {
        UserPowerLevel::Infinite => Role::Admin,
        UserPowerLevel::Int(level) if level >= int!(100) => Role::Admin,
        UserPowerLevel::Int(level) if level >= int!(50) => Role::Moderator,
        _ => Role::Member,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A member at `level` (`None`: an owner); `up_to`: the highest level
    /// we may give them (`None`: theirs is not ours to change).
    fn member(name: &str, level: Option<i64>, up_to: Option<i64>) -> Member {
        Member {
            id: format!("@{name}:example.org").try_into().unwrap(),
            name: Some(name[..1].to_uppercase() + &name[1..]),
            avatar: None,
            role: Role::Member,
            level,
            invited: false,
            own: name == "me",
            manage: Manage {
                up_to,
                ..Manage::default()
            },
        }
    }

    #[test]
    fn labels() {
        let label = |level| member("ann", level, None).label();
        assert_eq!(label(None).as_deref(), Some("Owner"));
        assert_eq!(label(Some(100)).as_deref(), Some("Admin"));
        assert_eq!(label(Some(50)).as_deref(), Some("Mod"));
        assert_eq!(label(Some(20)).as_deref(), Some("Level 20"));
        assert_eq!(label(Some(0)), None);
    }

    #[test]
    fn choices_up_to_our_level_theirs_ticked() {
        let titles = |m: &Member| {
            m.level_choices()
                .into_iter()
                .map(|c| (c.title, c.current))
                .collect::<Vec<_>>()
        };
        let bob = member("bob", Some(50), Some(50));
        assert_eq!(
            titles(&bob),
            [("Moderator".into(), true), ("Member".into(), false)]
        );
        let custom = member("bob", Some(20), Some(100));
        assert_eq!(
            titles(&custom),
            [
                ("Level 20".into(), true),
                ("Admin".into(), false),
                ("Moderator".into(), false),
                ("Member".into(), false),
            ]
        );
        assert!(member("bob", Some(0), None).level_choices().is_empty());
        // No "Member" where a plain member is a moderator.
        let mut moderated = member("bob", Some(50), Some(100));
        moderated.manage.default_level = 50;
        assert_eq!(
            titles(&moderated),
            [("Admin".into(), false), ("Moderator".into(), true)]
        );
    }

    #[test]
    fn asks_first_where_it_cannot_be_undone() {
        let bob = member("bob", Some(0), Some(100));
        assert_eq!(bob.confirmation(50), None);
        let admin = bob.confirmation(100).unwrap();
        assert_eq!(admin.title, "Make Bob an admin?");
        assert_eq!(admin.button, "Change");
        assert!(
            admin
                .message
                .contains("an admin can't demote another admin")
        );
        let as_high_as_us = member("bob", Some(0), Some(50)).confirmation(50).unwrap();
        assert_eq!(as_high_as_us.title, "Make Bob a moderator?");
        assert!(
            as_high_as_us
                .message
                .starts_with("They'll have the same level as you.")
        );
        // Asked in the menu too.
        let choices = bob.level_choices();
        assert_eq!(choices[0].confirmation, Some(admin));
        assert_eq!(choices[1].confirmation, None);
    }

    #[test]
    fn stepping_down_asks_first() {
        let mut me = member("me", Some(100), Some(99));
        let down = me.confirmation(50).unwrap();
        assert_eq!(down.title, "Step down to moderator?");
        assert_eq!(down.button, "Step down");
        assert_eq!(
            down.message,
            "You can't undo this: only someone with a higher level can raise you again."
        );
        me.manage.alone = true;
        let alone = me.confirmation(0).unwrap();
        assert_eq!(alone.title, "Step down to member?");
        assert!(
            alone
                .message
                .ends_with("so nobody will be able to do all you can now.")
        );
    }

    #[test]
    fn alone_at_our_level() {
        let me = member("me", Some(100), None);
        let bob = |level| member("bob", level, None);
        assert!(alone(&[me.clone(), bob(Some(50))]));
        assert!(!alone(&[me.clone(), bob(Some(100))]));
        assert!(!alone(&[me.clone(), bob(None)]));
        // Invited people do not count.
        let mut invited = bob(Some(100));
        invited.invited = true;
        assert!(alone(&[me, invited]));
        assert!(!alone(&[bob(Some(0))]));
    }

    #[test]
    fn custom_levels() {
        assert_eq!(typed_level(" 20 ", 50), Some(20));
        assert_eq!(typed_level("60", 50), None);
        assert_eq!(typed_level("lots", 50), None);
    }

    #[test]
    fn shown_name_else_the_user_id() {
        let mut ann = member("ann", Some(0), None);
        assert_eq!(ann.shown_name(), "Ann");
        ann.name = Some(" ".into());
        assert_eq!(ann.shown_name(), "@ann:example.org");
    }
}
