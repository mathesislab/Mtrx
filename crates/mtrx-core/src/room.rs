//! A room or space we are in or invited to, for the calls on it: its
//! settings, people, state and calls. Its timeline, pinned messages and
//! draft are on [`crate::RoomTimeline`] and [`crate::DraftKeeper`].

use std::{fmt, sync::Arc};

use matrix_sdk::ruma::RoomId;

use crate::{Error, Result, Session};

/// A known room or space ([`Session::room`]). Clones are cheap; each keeps
/// the session.
#[derive(Clone)]
pub struct RoomHandle {
    pub(crate) session: Arc<Session>,
    pub(crate) room: matrix_sdk::Room,
}

impl fmt::Debug for RoomHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("RoomHandle").field(&self.id()).finish()
    }
}

impl Session {
    /// A room or space we know: joined, invited, or left but still stored.
    pub fn room(self: &Arc<Self>, id: &RoomId) -> Result<RoomHandle> {
        Ok(RoomHandle {
            session: Arc::clone(self),
            room: self.sdk_room(id)?,
        })
    }

    pub(crate) fn sdk_room(&self, id: &RoomId) -> Result<matrix_sdk::Room> {
        self.client
            .get_room(id)
            .ok_or_else(|| Error::RoomNotFound(id.to_string()))
    }
}

impl RoomHandle {
    pub fn id(&self) -> &RoomId {
        self.room.room_id()
    }
}
