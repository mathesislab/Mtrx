//! Matrix client logic and state for Mtrx.
//!
//! Frontends (the `mtrx` CLI, the GUI) are built on this crate. It never
//! prints or draws: it exposes plain data types and live update streams.

// matrix-sdk futures are deeply nested.
#![recursion_limit = "256"]

mod account_data;
mod calls;
mod categories;
mod connection;
mod directory;
mod drafts;
mod echo;
mod emoji;
mod encryption;
mod error;
mod lists;
mod live;
mod media;
mod members;
mod notes;
mod notifications;
mod people;
mod permissions;
mod pins;
mod presence;
mod profile;
mod push;
mod room;
mod room_colours;
mod room_links;
mod room_settings;
mod rooms;
mod rows;
mod scheduled;
mod secrets;
mod send_time;
mod session;
mod space_graph;
mod spaces;
mod synced;
mod timeline;

#[cfg(feature = "test-support")]
pub mod test_support;

pub use calls::{CallService, ServiceKind, recognize_call_link};
pub use categories::{AtHome, Category, Grouping, Layout, RoomCategories, RoomTag, Section, Sort};
pub use connection::Connection;
pub use directory::{DirectoryKind, DirectoryPage, DirectoryRoom, DirectoryServers, parse_address};
pub use drafts::{DRAFT_PAUSE, Draft, DraftKeeper};
pub use encryption::{
    DeviceInfo, EncryptionState, RecoveryStatus, RecoveryStep, Verification, VerificationEmoji,
    VerificationState,
};
pub use error::{Error, Result};
pub use lists::{
    Badges, ListSection, RoomLists, RoomSort, SectionChoice, SectionLayout, SpaceEntry, SpaceKey,
};
pub use live::{AbortOnDrop, Live};
pub use media::MediaRef;
pub use members::{
    Confirmation, LevelChoice, Manage, Member, Role, a_role, level_confirmation, role_name,
    typed_level,
};
pub use notes::notes_markdown;
pub use notifications::Notification;
pub use people::{Contact, User, UserInfo};
pub use permissions::{Permissions, SpaceUpdate};
pub use pins::PinsChange;
pub use presence::{
    Activity, IDLE_AFTER, OwnPresence, Presence, PresenceDot, PresenceState, PresenceWatcher,
    describe_last_active,
};
pub use profile::Profile;
pub use push::{HttpPusher, PusherInfo};
pub use room::RoomHandle;
pub use room_colours::RoomListColours;
pub use room_links::{LinkKind, RoomLink, Topic, describe_change};
pub use room_settings::{Banned, History, NotifyMode, RoomSettings, SpaceLink};
pub use rooms::{JoinAction, JoinRule, Membership, NewRoom, RoomKind, RoomListState, RoomSummary};
pub use rows::{GROUP_GAP, MessageAction, MessageRow, Row, RowContext, rows};
pub use scheduled::{ScheduledFile, ScheduledMessage};
pub use secrets::{Secret, SecretBackend, SecretStore};
pub use send_time::{describe_send_time, parse_send_time};
pub use session::{APP_OAUTH_REDIRECT_URI, OAuthLogin, Session};
pub use space_graph::{Child, SpaceGraph};
pub use timeline::{
    Attachment, Event, EventContent, EventKey, Media, MediaKind, MembershipChange, Message,
    MessageKind, PendingSend, Reaction, Reply, RoomTimeline, SEEK_PAGE, SEEK_PAGES, SendState,
    TimelineEntry, Upload, UtdCause, mention_targets,
};

pub use url::Url;

/// Matrix identifier types used in this crate's API.
pub use matrix_sdk::ruma::{
    MxcUri, OwnedDeviceId, OwnedEventId, OwnedMxcUri, OwnedRoomAliasId, OwnedRoomId,
    OwnedRoomOrAliasId, OwnedServerName, OwnedUserId, RoomOrAliasId, ServerName, UserId,
};

pub(crate) fn system_time(
    ts: matrix_sdk::ruma::MilliSecondsSinceUnixEpoch,
) -> std::time::SystemTime {
    std::time::UNIX_EPOCH + std::time::Duration::from_millis(ts.get().into())
}
