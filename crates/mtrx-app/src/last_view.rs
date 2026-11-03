//! The space and room last shown, per account, so Mtrx opens where it was
//! left (`view.json` in the profile's directory).

use std::path::{Path, PathBuf};

use mtrx_core::{OwnedRoomId, Profile, RoomSummary};
use serde::{Deserialize, Serialize};
use tracing::warn;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LastView {
    pub space: Option<OwnedRoomId>,
    pub room: Option<OwnedRoomId>,
}

impl LastView {
    pub fn path(profile: &Profile) -> PathBuf {
        profile.dir().join("view.json")
    }

    /// The saved view; nothing if there is none or it cannot be read.
    pub fn load(path: &Path) -> Self {
        std::fs::read(path)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, path: &Path) {
        let saved = serde_json::to_vec_pretty(self)
            .map_err(std::io::Error::other)
            .and_then(|json| std::fs::write(path, json));
        if let Err(e) = saved {
            warn!("could not save the last view: {e}");
        }
    }

    /// The space and room to show again, once they are in `rooms`: a room
    /// still joined, and the space if it still is one.
    pub fn found(&self, rooms: &[RoomSummary]) -> Option<(Option<OwnedRoomId>, OwnedRoomId)> {
        let room = self.room.as_ref()?;
        rooms.iter().find(|r| &r.id == room && !r.invited)?;
        let space = self
            .space
            .clone()
            .filter(|space| rooms.iter().any(|r| &r.id == space));
        Some((space, room.clone()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saves_loads_and_finds() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("view.json");
        assert_eq!(LastView::load(&path), LastView::default());
        let room: OwnedRoomId = "!room:example.org".try_into().unwrap();
        let space: OwnedRoomId = "!space:example.org".try_into().unwrap();
        let view = LastView {
            space: Some(space.clone()),
            room: Some(room.clone()),
        };
        view.save(&path);
        assert_eq!(LastView::load(&path), view);

        let summary = |id: &OwnedRoomId| RoomSummary {
            id: id.clone(),
            name: String::new(),
            avatar: None,
            topic: None,
            kind: mtrx_core::RoomKind::Room,
            invited: false,
            encrypted: false,
            public: false,
            unread_messages: 0,
            unread_mentions: 0,
            unread_notifications: 0,
            last_activity: None,
            tag: mtrx_core::RoomTag::None,
            direct_user: None,
            alias: None,
            joined_members: 1,
            pinned: Vec::new(),
        };
        assert_eq!(view.found(&[]), None);
        assert_eq!(view.found(&[summary(&room)]), Some((None, room.clone())));
        assert_eq!(
            view.found(&[summary(&room), summary(&space)]),
            Some((Some(space), room))
        );
    }
}
