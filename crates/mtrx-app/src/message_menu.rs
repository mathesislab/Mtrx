//! A message's menu (right-click, or "⋯" in its toolbar): copying the
//! selection or the message, pinning it, math as a picture, and a file or
//! image; or, right-clicking a link in it, the link's.

use iced::Element;
use mtrx_core::{EventContent, MediaKind, MessageAction, MessageRow, OwnedRoomId};

use crate::{
    actions::{self, act},
    app, context_menu,
    downloads::{self, File},
    media::Origin,
    message, pins,
    selection::Pos,
};

/// Where a message's menu floats from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MenuAt {
    /// Where the message was right-clicked.
    Cursor,
    /// The "⋯" of the hover toolbar.
    Toolbar,
    /// Where a link in it was right-clicked: the link's menu instead.
    Link(String),
}

/// What the menu offers for the message of `row` in `room`: label and
/// action, in order. First its actions not in the toolbar (Copy, Pin or
/// Unpin; its text is selected in place), then its file's and its math's.
/// `in_selection`: the message is (partly) in the timeline's selection,
/// which Copy then copies; else Copy copies the message, as a selection of
/// all of it would.
pub fn items(
    room: &OwnedRoomId,
    row: &MessageRow,
    math_shown: bool,
    in_selection: bool,
) -> Vec<(String, app::Message)> {
    let EventContent::Message(message) = &row.event.content else {
        return Vec::new();
    };
    let document = &message.document;
    let mut items = Vec::new();
    for action in &row.actions {
        match action {
            MessageAction::Copy if in_selection => items.push((
                "Copy".to_owned(),
                act(room, actions::Message::CopySelection),
            )),
            MessageAction::Copy => {
                let all = message::copied(document, Pos::default(), Pos::END);
                items.push(("Copy".to_owned(), act(room, actions::Message::Copy(all))));
            }
            MessageAction::Pin | MessageAction::Unpin => {
                let pinned = *action == MessageAction::Unpin;
                items.extend(
                    (row.event.event_id.as_ref()).map(|id| pins::menu_item(room, id, pinned)),
                );
            }
            _ => {}
        }
    }
    if let Some(media) = &message.media {
        let file = File {
            media: media.clone(),
            room: room.clone(),
            event: row.event.event_id.clone(),
        };
        items.extend(media_items(&file, Origin::Room(room.clone())));
    }
    if math_shown {
        items.push((
            "Copy image".to_owned(),
            act(room, actions::Message::CopyImage(document.clone())),
        ));
        items.push((
            "Save image…".to_owned(),
            act(room, actions::Message::SaveImage(document.clone())),
        ));
    }
    items
}

/// What the menu offers for a message's file, also where an image is shown
/// large (`from`): copying an image, and saving the file somewhere asked.
pub fn media_items(file: &File, from: Origin) -> Vec<(String, app::Message)> {
    let mut items = Vec::new();
    if file.media.kind == MediaKind::Image {
        items.push((
            "Copy image".to_owned(),
            app::Message::CopyImage(file.media.clone(), from),
        ));
    }
    items.push((
        "Save as…".to_owned(),
        app::Message::Downloads(downloads::Message::SaveAs(file.clone())),
    ));
    items
}

/// What the menu offers for a link in `room`: a mention's opens the
/// person's card.
pub fn link_items(room: &OwnedRoomId, url: &str) -> Vec<(String, app::Message)> {
    let open = match message::mentioned_user(url) {
        Some(_) => "Info",
        None => "Open",
    };
    vec![
        (
            open.to_owned(),
            act(room, actions::Message::OpenLink(url.to_owned())),
        ),
        (
            "Copy link".to_owned(),
            act(room, actions::Message::CopyLink(url.to_owned())),
        ),
    ]
}

/// The menu showing `items`.
pub fn view<'a>(items: Vec<(String, app::Message)>) -> Element<'a, app::Message> {
    context_menu::menu(items)
}

#[cfg(test)]
mod tests {
    use mtrx_core::{Event, Message, MessageKind, Row, RowContext, TimelineEntry};
    use mtrx_format::Document;

    use super::*;

    fn message(kind: MessageKind, html: &str) -> Message {
        Message {
            kind,
            body: String::new(),
            document: Document::from_html(html),
            edited: false,
            in_reply_to: None,
            reactions: Vec::new(),
            media: None,
        }
    }

    /// The row of `message`, sent by someone else, where we may pin.
    fn row(message: Message) -> MessageRow {
        let event = Event {
            event_id: Some("$a:example.org".try_into().unwrap()),
            content: EventContent::Message(message),
            ..Event::test_message("a", "@bob:example.org".try_into().unwrap(), "")
        };
        let context = RowContext {
            can_pin: true,
            ..RowContext::default()
        };
        match mtrx_core::rows(&[TimelineEntry::Event(event)], &context).remove(0) {
            Row::Message(row) => row,
            row => panic!("not a message: {row:?}"),
        }
    }

    fn media(filename: &str, kind: MediaKind) -> mtrx_core::Media {
        mtrx_core::Media {
            kind,
            filename: filename.into(),
            size: None,
            mimetype: None,
            width: None,
            height: None,
            source: mtrx_core::MediaRef::plain("mxc://example.org/a"),
        }
    }

    fn labels(items: &[(String, app::Message)]) -> Vec<&str> {
        items.iter().map(|(label, _)| label.as_str()).collect()
    }

    /// The action an item takes in the room.
    fn act_of(message: &app::Message) -> Option<&actions::Message> {
        match message {
            app::Message::Room(_, crate::room::Message::Act(act)) => Some(act),
            _ => None,
        }
    }

    fn room() -> OwnedRoomId {
        "!room:example.org".try_into().unwrap()
    }

    #[test]
    fn offers_copy_and_the_picture() {
        let html = r#"Flux <span data-mx-maths="q=-k\nabla T"><code>q</code></span>, see <a href="https://example.org/a">data</a>"#;
        let text = row(message(MessageKind::Text, html));
        let items = items(&room(), &text, true, false);
        assert_eq!(labels(&items), ["Copy", "Pin", "Copy image", "Save image…"]);
        assert!(
            matches!(act_of(&items[0].1), Some(actions::Message::Copy(all)) if all.text == r"Flux $q=-k\nabla T$, see data"),
            "as shown, math as LaTeX"
        );
        assert!(
            matches!(act_of(&items[0].1), Some(actions::Message::Copy(all)) if all.html == r#"<p>Flux $q=-k\nabla T$, see <a href="https://example.org/a">data</a></p>"#),
            "and as HTML, with the link"
        );
        // Until the math is typeset, there is no picture.
        assert_eq!(
            labels(&super::items(&room(), &text, false, false)),
            ["Copy", "Pin"]
        );
        // In the selection, Copy copies the selection.
        assert!(matches!(
            act_of(&super::items(&room(), &text, false, true)[0].1),
            Some(actions::Message::CopySelection)
        ));
        // A file can be saved somewhere asked; an image also copied. Their
        // names are their text.
        let mut file = message(MessageKind::File, "report.pdf");
        file.media = Some(media("report.pdf", MediaKind::File));
        let items = super::items(&room(), &row(file.clone()), false, false);
        assert_eq!(labels(&items), ["Copy", "Pin", "Save as…"]);
        assert!(matches!(
            &items[2].1,
            app::Message::Downloads(downloads::Message::SaveAs(saved))
                if saved.media.filename == "report.pdf" && saved.room == room()
        ));
        file.media = Some(media("ice.png", MediaKind::Image));
        assert_eq!(
            labels(&super::items(&room(), &row(file), false, false)),
            ["Copy", "Pin", "Copy image", "Save as…"]
        );
    }

    #[test]
    fn a_link_is_opened_or_copied() {
        let url = "https://example.org/a";
        let items = link_items(&room(), url);
        assert_eq!(labels(&items), ["Open", "Copy link"]);
        assert!(
            matches!(act_of(&items[0].1), Some(actions::Message::OpenLink(open)) if open == url)
        );
        assert!(
            matches!(act_of(&items[1].1), Some(actions::Message::CopyLink(copied)) if copied == url)
        );
        // A mention's shows the person's card.
        let mention = "https://matrix.to/#/@nils:server.org";
        assert_eq!(labels(&link_items(&room(), mention)), ["Info", "Copy link"]);
    }
}
