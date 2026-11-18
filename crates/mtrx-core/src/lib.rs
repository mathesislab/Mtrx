//! Matrix client logic and state for Mtrx.
//!
//! Frontends (the `mtrx` CLI, the GUI) are built on this crate. It never
//! prints or draws: it exposes plain data types and live update streams.

// matrix-sdk futures are deeply nested.
#![recursion_limit = "256"]

mod account_data;
mod audio;
mod calls;
mod categories;
mod connection;
mod directory;
mod disappearing;
mod downloads;
mod drafts;
mod echo;
mod emoji;
mod encryption;
mod error;
mod export;
mod fetch;
mod lists;
mod live;
#[cfg(feature = "math")]
mod math;
mod media;
mod members;
mod mute;
mod notes;
mod notifications;
mod people;
mod permissions;
mod pins;
mod presence;
mod profile;
mod push;
mod quiet;
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
mod timeline;

#[cfg(feature = "test-support")]
pub mod test_support;

pub use audio::{WAVEFORM_BARS, playback_time};
pub use calls::{CallService, ServiceKind, recognize_call_link};
pub use categories::{AtHome, Category, Grouping, Layout, RoomCategories, RoomTag, Section, Sort};
pub use connection::Connection;
pub use directory::{DirectoryKind, DirectoryPage, DirectoryRoom, DirectoryServers, parse_address};
pub use disappearing::{
    AFTER_READ_CHOICES, AT_MOST_CHOICES, DEFAULT_AT_MOST, Disappear, Disappearing,
    DisappearingLimit, MARKER as DISAPPEARING_MARKER, Sent, disappear_words,
    disappearing_mode_words, duration_words,
};
pub use downloads::{Download, DownloadId, DownloadState, SaveTo, human_size};
pub use drafts::{DRAFT_PAUSE, Draft, DraftKeeper};
pub use encryption::{
    DeviceInfo, EncryptionState, RecoveryStatus, RecoveryStep, Verification, VerificationEmoji,
    VerificationState,
};
pub use error::{Error, Result};
pub use export::{
    Export, ExportFiles, ExportFormat, ExportId, ExportOf, ExportRange, ExportRequest, ExportState,
    ExportSummary,
};
pub use lists::{
    Badges, ListSection, RoomLists, RoomSort, SectionChoice, SectionLayout, SpaceEntry, SpaceKey,
};
pub use live::{AbortOnDrop, Live};
#[cfg(feature = "math")]
pub use math::{Typeset, typeset};
pub use media::{
    MediaQuality, MediaRef, PHOTO_MAX_BYTES, PHOTO_MAX_EDGE, VIDEO_THUMBNAIL_EDGE, VideoInfo,
    VideoPlan, reduce_photo, video_plan,
};
pub use members::{
    Confirmation, LevelChoice, Manage, Member, Role, a_role, level_confirmation, role_name,
    typed_level,
};
pub use mute::{MuteFor, TimedMute};
pub use notes::notes_markdown;
pub use notifications::Notification;
pub use people::{Contact, User, UserInfo};
pub use permissions::{Permissions, Rights, SpaceUpdate};
pub use pins::PinsChange;
pub use presence::{
    Activity, IDLE_AFTER, OwnPresence, Presence, PresenceDot, PresenceState, PresenceWatcher,
    describe_last_active,
};
pub use profile::Profile;
pub use push::{HttpPusher, PusherInfo};
pub use quiet::{
    Pause, PauseEntry, PauseFor, Quiet, QuietHours, QuietState, QuietStatus, pause_menu,
};
pub use room::RoomHandle;
pub use room_colours::RoomListColours;
pub use room_links::{LinkDraft, LinkKind, RoomLink, Topic, describe_change, typed_link_url};
pub use room_settings::{
    Banned, History, NotifyAction, NotifyChoice, NotifyMode, RoomSettings, SpaceLink,
};

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

/// Times of day and weekdays, as [`QuietHours`] has them.
pub use chrono::{NaiveTime, Weekday, WeekdaySet};

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
