//! Drafts: what one wrote in a room and did not send, kept on this device
//! until sent, with the message it answers.

use mtrx_core::{self as core, OwnedEventId};

use crate::{MtrxError, Session, runtime::run};

/// A room's unsent message.
#[derive(Clone, Debug, Default, PartialEq, Eq, uniffi::Record)]
pub struct Draft {
    pub text: String,
    /// The event ID of the message it answers.
    #[uniffi(default)]
    pub reply_to: Option<String>,
}

#[uniffi::export]
impl Session {
    /// The draft left in a room, if any.
    pub async fn draft(&self, room_id: String) -> Result<Option<Draft>, MtrxError> {
        let room = crate::error::room_id(&room_id)?;
        let session = self.core();
        run(async move {
            Ok(session.draft(&room).await?.map(|draft| Draft {
                text: draft.text,
                reply_to: draft.reply_to.map(|id| id.to_string()),
            }))
        })
        .await
    }

    /// Keep a room's draft (when leaving it, or the app going to the
    /// background); an empty one (sent) clears it. `RoomLists.drafts()`
    /// follows.
    pub async fn save_draft(&self, room_id: String, draft: Draft) -> Result<(), MtrxError> {
        let room = crate::error::room_id(&room_id)?;
        let reply_to = draft
            .reply_to
            .map(|id| OwnedEventId::try_from(id.as_str()).map_err(MtrxError::invalid))
            .transpose()?;
        let draft = core::Draft {
            text: draft.text,
            reply_to,
        };
        let session = self.core();
        run(async move { Ok(session.save_draft(&room, &draft).await?) }).await
    }
}
