//! People: their presence, those we know, the emoji we use, and direct
//! chats with them.

use mtrx_core::{Contact, OwnedRoomId, OwnedUserId};

use crate::presence;

#[derive(Debug, Clone)]
pub enum Message {
    /// Others' presence, and ours.
    Presence(presence::Message),
    /// People the user knows, for suggestions.
    ContactsLoaded(Vec<Contact>),
    /// The emoji the user reacts with, most used first.
    FrequentEmoji(Vec<String>),
    /// Start or open a direct chat with a member.
    ChatWith(OwnedUserId),
    /// The direct chat started or found, or why not.
    ChatStarted(Result<OwnedRoomId, String>),
    /// Start the Notes to self and open them.
    StartNotes,
    /// Star the direct chat with someone, starting it if there is none.
    FavouritePerson(OwnedUserId),
}
