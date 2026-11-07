//! People: their presence, those we know, the emoji we use, and direct
//! chats with them.

use std::sync::Arc;

use iced::Task;
use mtrx_core::{Activity, Contact, OwnedRoomId, OwnedUserId, RoomTag, Session};

use super::{Cx, modal, pane};
use crate::{
    app, dialog,
    presence::{self, Presences},
    sidebar, task,
};

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

/// Others' presence, the people we know, and the emoji we use.
#[derive(Default)]
pub struct People {
    /// Others' presence, for the people shown.
    pub presences: Presences,
    /// People the user knows, closest first; refreshed when a dialog
    /// picking people opens.
    pub contacts: Vec<Contact>,
    /// The emoji the user reacts with, most used first; refreshed when a
    /// reaction picker opens.
    pub frequent_emoji: Vec<String>,
}

impl People {
    pub fn update(&mut self, message: Message, cx: &mut Cx<'_>) -> Task<app::Message> {
        match message {
            Message::Presence(presence::Message::Input) => {
                self.activity(cx.session, Activity::Input)
            }
            Message::Presence(message) => {
                self.presences.update(message);
                Task::none()
            }
            // Those known so far at once, then afresh.
            Message::LoadContacts => {
                let session = cx.session.clone();
                Task::batch([
                    contacts_for_dialog(self.contacts.clone()),
                    Task::perform(
                        async move { session.contacts().await.unwrap_or_default() },
                        |contacts| to(Message::ContactsLoaded(contacts)),
                    ),
                ])
            }
            Message::ContactsLoaded(contacts) => {
                self.contacts = contacts.clone();
                contacts_for_dialog(contacts)
            }
            // Fresher than the watcher's, and for people it does not follow.
            Message::LookUp(user) => {
                if self.presences.unavailable() {
                    return Task::none();
                }
                presence::look_up(cx.session, user).map(|message| to(Message::Presence(message)))
            }
            Message::FrequentEmoji(emoji) => {
                self.frequent_emoji = emoji;
                Task::none()
            }
            Message::ChatWith(user) => {
                cx.close_card();
                let session = cx.session.clone();
                task::request(async move { session.direct_chat(&user).await }, |result| {
                    to(Message::ChatStarted(result))
                })
            }
            Message::ChatStarted(Ok(room)) => Task::done(pane::to(pane::Message::Open(room))),
            Message::ChatStarted(Err(e)) => {
                cx.notices
                    .report(format!("Could not open the direct chat: {e}"), None);
                Task::none()
            }
            Message::StartNotes => self.update(Message::ChatWith(cx.own_user.clone()), cx),
            Message::FavouritePerson(user) => {
                let session = cx.session.clone();
                Task::future(async move {
                    let room = session.direct_chat(&user).await?;
                    session.room(&room)?.set_tag(RoomTag::Favourite).await
                })
                .then(|result| match result {
                    Ok(()) => Task::none(),
                    Err(e) => Task::done(app::Message::Sidebar(sidebar::Message::ArrangeFailed(
                        e.to_string(),
                    ))),
                })
            }
            // First among the most used here at once, and in account data
            // (shared with Element).
            Message::Used(used) => {
                for emoji in &used {
                    self.frequent_emoji.retain(|e| e != emoji);
                    self.frequent_emoji.insert(0, emoji.clone());
                }
                let session = cx.session.clone();
                task::background("record the emoji used", async move {
                    session.record_emoji(&used).await
                })
            }
        }
    }

    /// What the app is doing, which sets our presence, unless the server
    /// does not share it.
    pub fn activity(&self, session: &Arc<Session>, activity: Activity) -> Task<app::Message> {
        if self.presences.unavailable() {
            return Task::none();
        }
        presence::activity(session, activity).map(|message| to(Message::Presence(message)))
    }

    /// Follow the presence of `people`, those shown, once they settle.
    pub fn follow(&mut self, people: Vec<OwnedUserId>) -> Task<app::Message> {
        match self.presences.show(people) {
            Some(generation) => Task::perform(tokio::time::sleep(presence::SETTLE), move |()| {
                to(Message::Presence(presence::Message::Settle(generation)))
            }),
            None => Task::none(),
        }
    }
}

/// `contacts` for the dialog picking people, if one is open.
fn contacts_for_dialog(contacts: Vec<Contact>) -> Task<app::Message> {
    Task::done(modal::to(modal::Message::Dialog(
        dialog::Message::Contacts(contacts),
    )))
}

/// The user's most used emoji, from account data.
pub fn load_frequent_emoji(session: &Arc<Session>) -> Task<app::Message> {
    let session = session.clone();
    Task::perform(async move { session.frequent_emoji().await }, |emoji| {
        to(Message::FrequentEmoji(emoji))
    })
}

/// Count the uses of `used` (reacted with, sent), if any.
pub fn used(used: Vec<String>) -> Task<app::Message> {
    if used.is_empty() {
        return Task::none();
    }
    Task::done(to(Message::Used(used)))
}

/// `message` for people, as the app gets it.
pub fn to(message: Message) -> app::Message {
    app::Message::People(message)
}
