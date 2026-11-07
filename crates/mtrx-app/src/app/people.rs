//! People: their presence, those we know, the emoji we use, and direct
//! chats with them.

use std::sync::Arc;

use iced::Task;
use mtrx_core::{Contact, OwnedRoomId, OwnedUserId, Session};

use crate::{app, presence};

#[derive(Debug, Clone)]
pub enum Message {
    /// Others' presence, and ours.
    Presence(presence::Message),
    /// Load the people the user knows, for a dialog's suggestions.
    LoadContacts,
    /// People the user knows, for suggestions.
    ContactsLoaded(Vec<Contact>),
    /// Look up someone's presence now, for their card.
    LookUp(OwnedUserId),
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
    /// The emoji just used (reacted with, sent): first among the most used.
    Used(Vec<String>),
}

/// The user's most used emoji, from account data.
pub fn load_frequent_emoji(session: &Arc<Session>) -> Task<app::Message> {
    let session = session.clone();
    Task::perform(async move { session.frequent_emoji().await }, |emoji| {
        app::Message::People(Message::FrequentEmoji(emoji))
    })
}

/// Count the uses of `used` (reacted with, sent), if any.
pub fn used(used: Vec<String>) -> Task<app::Message> {
    if used.is_empty() {
        return Task::none();
    }
    Task::done(app::Message::People(Message::Used(used)))
}
