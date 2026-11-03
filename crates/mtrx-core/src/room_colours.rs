//! The room list's colour for Home and each space, by name (the app's
//! palette). Personal, so stored as account data ([`crate::account_data`])
//! and synced to the user's devices.

use std::collections::BTreeMap;

use matrix_sdk::ruma::{OwnedRoomId, events::macros::EventContent};
use serde::{Deserialize, Serialize};

use crate::{Live, Result, Session, account_data::AccountData};

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoomListColours {
    /// At Home, and in spaces without their own; `None` for the default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub home: Option<String>,
    #[serde(default)]
    pub spaces: BTreeMap<OwnedRoomId, String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, EventContent)]
#[ruma_event(type = "org.mtrx.room_list_colours", kind = GlobalAccountData)]
pub(crate) struct RoomListColoursEventContent {
    #[serde(flatten)]
    colours: RoomListColours,
}

impl AccountData for RoomListColours {
    type Content = RoomListColoursEventContent;

    fn from_content(content: Self::Content) -> Self {
        content.colours
    }

    fn into_content(self) -> Self::Content {
        RoomListColoursEventContent { colours: self }
    }
}

impl Session {
    pub fn room_list_colours(&self) -> RoomListColours {
        self.colours.get()
    }

    /// The current colours, then every change, from this device or others.
    pub fn room_list_colours_updates(&self) -> Live<RoomListColours> {
        self.colours.updates()
    }

    /// Change the colours: shown at once, then saved on the server; if
    /// saving fails, the server's colours are restored.
    pub async fn change_room_list_colours(
        &self,
        change: impl FnOnce(&mut RoomListColours),
    ) -> Result<()> {
        self.colours.change(change).await
    }
}
