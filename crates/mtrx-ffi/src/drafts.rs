//! Drafts: what one wrote in a room and did not send, kept on this device
//! until sent, with the message it answers.

use std::sync::Arc;

use mtrx_core::{self as core, OwnedEventId};

use crate::{
    MtrxError, Session,
    runtime::{self, InRuntime, run},
};

/// A room's unsent message.
#[derive(Clone, Debug, Default, PartialEq, Eq, uniffi::Record)]
pub struct Draft {
    pub text: String,
    /// The event ID of the message it answers.
    #[uniffi(default)]
    pub reply_to: Option<String>,
}

impl From<core::Draft> for Draft {
    fn from(draft: core::Draft) -> Self {
        Self {
            text: draft.text,
            reply_to: draft.reply_to.map(|id| id.to_string()),
        }
    }
}

impl From<Draft> for core::Draft {
    /// A `reply_to` that is not an event ID is left out.
    fn from(draft: Draft) -> Self {
        Self {
            text: draft.text,
            reply_to: (draft.reply_to).and_then(|id| OwnedEventId::try_from(id).ok()),
        }
    }
}

/// Keeps a room's draft on this device as the composer changes. When the
/// room opens, `restore()` brings back the one left there: put it in the
/// composer only if nothing was written meanwhile. Say what is written
/// after every change, `changed(draft)`: it is saved once writing pauses
/// for 1.5 s, unless it is the one kept; during an edit, pass the draft
/// from before it. `flush()` saves it at once: leaving the room, or the app
/// going to the background. Sending clears it (an empty draft). Leaving the
/// room keeps it. `RoomLists.drafts()` follows.
#[derive(uniffi::Object)]
pub struct DraftKeeper {
    /// None for a room ID that is not valid: nothing is kept.
    inner: Option<InRuntime<Arc<core::DraftKeeper>>>,
}

#[uniffi::export]
impl Session {
    /// Keeps the draft of room `room_id`.
    pub fn draft_keeper(&self, room_id: String) -> Arc<DraftKeeper> {
        let inner = crate::error::room_id(&room_id)
            .ok()
            .map(|room| InRuntime::new(Arc::new(self.core().draft_keeper(&room))));
        Arc::new(DraftKeeper { inner })
    }

    /// The draft left in a room, if any.
    pub async fn draft(&self, room_id: String) -> Result<Option<Draft>, MtrxError> {
        let room = crate::error::room_id(&room_id)?;
        let session = self.core();
        run(async move { Ok(session.draft(&room).await?.map(Draft::from)) }).await
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

#[uniffi::export]
impl DraftKeeper {
    /// The draft left in the room, once: nil if there is none, if it cannot
    /// be read, or on later calls.
    pub async fn restore(&self) -> Option<Draft> {
        let keeper = Arc::clone(self.inner.as_deref()?);
        run(async move { keeper.restore().await.map(Draft::from) }).await
    }

    /// What is written now (text, and the message it answers), after
    /// every change.
    pub fn changed(&self, draft: Draft) {
        if let Some(keeper) = &self.inner {
            let _runtime = runtime::enter();
            keeper.changed(draft.into());
        }
    }

    /// Save the draft now, after the saves under way.
    pub async fn flush(&self) {
        if let Some(keeper) = self.inner.as_deref() {
            let keeper = Arc::clone(keeper);
            run(async move { keeper.flush().await }).await;
        }
    }
}
