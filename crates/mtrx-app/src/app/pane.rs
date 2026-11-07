//! The room pane, right of the room list: a room opening, open or invited
//! to, and its member list.

use mtrx_core::{OwnedRoomId, OwnedUserId, RoomTimeline};

use super::Shared;
use crate::members;

#[derive(Debug, Clone)]
pub enum Message {
    Open(OwnedRoomId),
    /// Its timeline, or why it did not open.
    Opened(OwnedRoomId, Result<Shared<RoomTimeline>, String>),
    /// Accept (`true`) or decline an invite.
    Answer(OwnedRoomId, bool),
    Answered(OwnedRoomId, bool, Result<(), String>),
    ToggleMembers,
    /// Open the room with its members shown (where Invite is).
    ShowMembers(OwnedRoomId),
    MemberMenu(members::MenuKey),
    /// Give a member a power level, without asking.
    SetLevel(OwnedRoomId, OwnedUserId, i64),
    /// A room's members were changed by us, or not.
    MembersChanged(OwnedRoomId, Result<(), String>),
    /// Reload the open room's timeline (⌘R / Ctrl+R); while offline, try
    /// the server now.
    Reload,
}
