use std::time::{Duration, SystemTime};

use matrix_sdk::{
    deserialized_responses::{TimelineEvent, UnableToDecryptInfo, UnableToDecryptReason},
    ruma::{owned_event_id, owned_room_id, owned_user_id, serde::Raw},
};
use mtrx_format::{Block, Document, Inline};
use serde_json::json;
use tempfile::TempDir;

use super::{
    event::{Body, Content, Item, Related, Style, convert},
    html::{HtmlWriter, PAGE, POLICY, href, page_file},
    job::{Recent, Relations, preview},
    json::JsonWriter,
    spool::Spool,
    text::event_text,
    write::{
        About, Attached, Index, IndexRoom, IndexState, Quote, Rendered, RoomEnd, RoomHead,
        RoomProgress, Shown, Writer,
    },
    *,
};
use crate::MediaKind;

fn running(rooms_done: usize, rooms: usize, room_fraction: Option<f32>) -> Export {
    Export::example(
        1,
        "Ice sheets",
        ExportState::Running {
            room: "Lab chat".into(),
            rooms_done,
            rooms,
            messages: 4210,
            files: 0,
            room_fraction,
        },
    )
}

#[test]
fn numbers_are_grouped_and_counted() {
    assert_eq!(grouped(7), "7");
    assert_eq!(grouped(4210), "4\u{a0}210");
    assert_eq!(grouped(1_234_567), "1\u{a0}234\u{a0}567");
    assert_eq!(count(1, "message"), "1 message");
    assert_eq!(count(12_340, "message"), "12\u{a0}340 messages");
}

#[test]
fn status_lines_and_fractions() {
    let export = running(2, 7, Some(0.5));
    assert_eq!(
        export.status(),
        "Lab chat · 3 of 7 rooms · 4\u{a0}210 messages"
    );
    assert_eq!(export.fraction(), Some(2.5 / 7.0));
    // A single room's progress is known only with a limit.
    assert_eq!(running(0, 1, None).fraction(), None);
    assert_eq!(running(0, 1, Some(0.25)).fraction(), Some(0.25));
    assert_eq!(running(0, 1, None).status(), "4\u{a0}210 messages");
    assert_eq!(running(3, 7, None).fraction(), Some(3.0 / 7.0));

    let summary = ExportSummary {
        rooms: 7,
        messages: 12_340,
        files: 85,
        files_left_out: 2,
        undecryptable: 1,
        rooms_failed: vec![("Old".into(), "Forbidden".into())],
    };
    let mut done = running(7, 7, None);
    done.state = ExportState::Done(summary.clone());
    assert_eq!(
        done.status(),
        "7 rooms, 12\u{a0}340 messages, 85 files, 2 files left out, 1 not decrypted, 1 room failed"
    );
    assert_eq!(done.fraction(), None);
    let mut cancelled = running(3, 7, None);
    cancelled.state = ExportState::Cancelled(ExportSummary {
        rooms: 3,
        ..Default::default()
    });
    assert_eq!(cancelled.status(), "Cancelled after 3 of 7 rooms");
    let mut failed = running(0, 1, None);
    failed.state = ExportState::Failed {
        message: "Disk full".into(),
        summary,
    };
    assert_eq!(failed.status(), "Failed: Disk full");
}

fn text_item(id: &str, text: &str) -> Item {
    Item {
        id: Some(id.try_into().unwrap()),
        sender: owned_user_id!("@alice:example.org"),
        ts: 1_700_000_000_000,
        content: Content::Message(Body {
            style: Style::Text,
            text: text.into(),
            html: None,
            file: None,
        }),
        reply_to: None,
        thread: None,
        raw: None,
    }
}

#[test]
fn spooled_pages_come_back_oldest_first() {
    let folder = TempDir::new().unwrap();
    let path = folder.path().join(".spool").join("0.jsonl");
    let mut spool = Spool::create(&path).unwrap();
    // As the server sends them: newest first.
    spool
        .write_page(&[text_item("$5", "five"), text_item("$4", "four")])
        .unwrap();
    spool.write_page(&[]).unwrap();
    spool
        .write_page(&[
            text_item("$3", "three"),
            text_item("$2", "two"),
            text_item("$1", "one"),
        ])
        .unwrap();
    let mut pages = spool.read().unwrap();
    let mut seen = Vec::new();
    while let Some(page) = pages.next_page().unwrap() {
        seen.push(
            page.into_iter()
                .map(|i| i.id.unwrap().to_string())
                .collect::<Vec<_>>(),
        );
    }
    assert_eq!(seen, [vec!["$1", "$2", "$3"], vec!["$4", "$5"]]);
    drop(pages);
    assert!(!path.exists(), "the spool goes once read");
}

fn plain(event: serde_json::Value) -> TimelineEvent {
    TimelineEvent::from_plaintext(Raw::from_json_string(event.to_string()).unwrap())
}

fn message_event(id: &str, sender: &str, content: serde_json::Value) -> TimelineEvent {
    plain(json!({
        "type": "m.room.message",
        "event_id": id,
        "sender": sender,
        "origin_server_ts": 1_700_000_000_000u64,
        "content": content,
    }))
}

#[test]
fn events_are_read_as_they_show() {
    let text = convert(
        &message_event(
            "$a",
            "@alice:example.org",
            json!({"msgtype": "m.text", "body": "**hi**", "format": "org.matrix.custom.html",
                   "formatted_body": "<strong>hi</strong>",
                   "m.relates_to": {"m.in_reply_to": {"event_id": "$q"}}}),
        ),
        false,
    );
    let item = text.item;
    assert_eq!(item.reply_to, Some(owned_event_id!("$q")));
    assert!(item.raw.is_none());
    let Content::Message(body) = &item.content else {
        panic!("{:?}", item.content)
    };
    assert_eq!(body.html.as_deref(), Some("<strong>hi</strong>"));
    assert!(item.is_message());

    let edit = convert(
        &message_event(
            "$e",
            "@alice:example.org",
            json!({"msgtype": "m.text", "body": "* new", "m.new_content": {"msgtype": "m.text", "body": "new"},
                   "m.relates_to": {"rel_type": "m.replace", "event_id": "$a"}}),
        ),
        true,
    );
    assert!(matches!(edit.item.content, Content::Hidden));
    assert!(edit.item.raw.is_some(), "kept for JSON");
    assert!(matches!(
        edit.related,
        Some(Related::Edit { target, body, .. }) if target == "$a" && body.text == "new"
    ));

    let reaction = convert(
        &plain(json!({
            "type": "m.reaction", "event_id": "$r", "sender": "@bob:example.org",
            "origin_server_ts": 1, "content": {"m.relates_to": {"rel_type": "m.annotation", "event_id": "$a", "key": "👍"}},
        })),
        false,
    );
    assert!(matches!(
        reaction.related,
        Some(Related::Reaction { target, key, .. }) if target == "$a" && key == "👍"
    ));

    let deleted = convert(
        &plain(json!({
            "type": "m.room.message", "event_id": "$d", "sender": "@bob:example.org",
            "origin_server_ts": 1, "content": {},
            "unsigned": {"redacted_because": {"type": "m.room.redaction", "event_id": "$x",
                "sender": "@bob:example.org", "origin_server_ts": 2, "content": {}, "redacts": "$d"}},
        })),
        false,
    );
    assert!(matches!(deleted.item.content, Content::Deleted));

    let joined = convert(
        &plain(json!({
            "type": "m.room.member", "event_id": "$j", "sender": "@bob:example.org",
            "state_key": "@bob:example.org", "origin_server_ts": 1,
            "content": {"membership": "join", "displayname": "Bob"},
        })),
        false,
    );
    assert!(matches!(&joined.item.content, Content::Change(c) if c == "joined"));
    let renamed = convert(
        &plain(json!({
            "type": "m.room.name", "event_id": "$n", "sender": "@bob:example.org",
            "state_key": "", "origin_server_ts": 1, "content": {"name": "Lab"},
        })),
        false,
    );
    assert!(
        matches!(&renamed.item.content, Content::Change(c) if c == "renamed the room to “Lab”")
    );
    let power = convert(
        &plain(json!({
            "type": "m.room.power_levels", "event_id": "$p", "sender": "@bob:example.org",
            "state_key": "", "origin_server_ts": 1, "content": {},
        })),
        false,
    );
    assert!(matches!(power.item.content, Content::Hidden));

    let encrypted = Raw::from_json_string(
        json!({
            "type": "m.room.encrypted", "event_id": "$u", "sender": "@bob:example.org",
            "origin_server_ts": 1, "content": {"algorithm": "m.megolm.v1.aes-sha2",
                "ciphertext": "x", "sender_key": "k", "device_id": "D", "session_id": "s"},
        })
        .to_string(),
    )
    .unwrap();
    let utd = convert(
        &TimelineEvent::from_utd(
            encrypted,
            UnableToDecryptInfo {
                session_id: Some("s".into()),
                reason: UnableToDecryptReason::MissingMegolmSession {
                    withheld_code: None,
                },
            },
        ),
        false,
    );
    assert!(
        matches!(&utd.item.content, Content::Undecryptable(Some(why)) if why == "the key is missing")
    );
    assert!(utd.item.is_message());
}

#[test]
fn edits_by_the_sender_and_reactions_apply() {
    let target = owned_event_id!("$a");
    let alice = owned_user_id!("@alice:example.org");
    let bob = owned_user_id!("@bob:example.org");
    let body = |text: &str| Body {
        style: Style::Text,
        text: text.into(),
        html: None,
        file: None,
    };
    let mut relations = Relations::default();
    // Newest first, as fetched.
    for (sender, text) in [(&alice, "newest"), (&bob, "not theirs"), (&alice, "older")] {
        relations.add(Related::Edit {
            target: target.clone(),
            sender: sender.clone(),
            body: body(text),
        });
    }
    assert_eq!(
        (relations.edits.get(&(target.clone(), alice.clone()))).map(|b| b.text.as_str()),
        Some("newest")
    );
    for (sender, key) in [(&alice, "👍"), (&bob, "👍"), (&bob, "🎉"), (&bob, "👍")] {
        relations.add(Related::Reaction {
            target: target.clone(),
            sender: sender.clone(),
            key: key.into(),
        });
    }
    assert_eq!(
        relations.take_reactions(&target),
        [("👍".to_owned(), 2), ("🎉".to_owned(), 1)]
    );
    assert!(relations.take_reactions(&target).is_empty());

    let mut recent = Recent::new();
    for n in 0..2001 {
        recent.insert(
            format!("$m{n}").try_into().unwrap(),
            ("Alice".into(), n.to_string()),
        );
    }
    assert!(!recent.quotes.contains_key(&owned_event_id!("$m0")));
    assert!(recent.quotes.contains_key(&owned_event_id!("$m2000")));
    assert_eq!(preview("a\n  b"), "a b");
    assert!(preview(&"x".repeat(500)).ends_with('…'));
}

fn about(format: ExportFormat) -> About {
    About {
        title: "Ice <sheets>".into(),
        exported_by: "@alice:example.org".into(),
        exported_at: SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000),
        format,
        range: ExportRange::All,
        files: Some(Some(100_000_000)),
    }
}

fn head(format: ExportFormat) -> RoomHead {
    RoomHead {
        id: owned_room_id!("!lab:example.org"),
        name: "Lab & chat".into(),
        topic: Some("Runs <b>".into()),
        folder: "lab".into(),
        about: about(format),
    }
}

fn rendered(n: u64, shown: Shown) -> Rendered {
    Rendered {
        id: Some(format!("$e{n}").try_into().unwrap()),
        sender: owned_user_id!("@alice:example.org"),
        sender_name: "Alice <script>".into(),
        time: SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000 + n),
        shown,
        reply: None,
        thread: None,
        reactions: Vec::new(),
        raw: Some(serde_json::value::to_raw_value(&json!({"event_id": format!("$e{n}")})).unwrap()),
    }
}

fn message(text: &str) -> Shown {
    Shown::Message {
        style: Style::Text,
        document: Document::from_plain(text),
        edited: false,
        file: None,
    }
}

#[test]
fn html_is_escaped_safe_and_paged() {
    let folder = TempDir::new().unwrap();
    let mut writer = HtmlWriter::new(folder.path()).unwrap();
    writer.begin_room(&head(ExportFormat::Html)).unwrap();
    let link = Document {
        blocks: vec![Block::Paragraph(vec![
            Inline::Link {
                url: "javascript:alert(1)".into(),
                content: vec![Inline::Text("click".into())],
            },
            Inline::Text(" <img src=x onerror=alert(1)>".into()),
        ])],
    };
    let mut first = rendered(
        0,
        Shown::Message {
            style: Style::Text,
            document: link,
            edited: true,
            file: Some(Attached {
                kind: MediaKind::Image,
                name: "a \"b\".png".into(),
                size: Some(1000),
                saved: Ok("files/2026-11-07 a #b.png".into()),
            }),
        },
    );
    first.reactions = vec![("👍".into(), 2)];
    writer.event(&first).unwrap();
    let mut reply = rendered(1, message("answer"));
    reply.reply = Some(Quote {
        id: owned_event_id!("$e0"),
        original: Some(("Alice <script>".into(), "click".into())),
    });
    writer.event(&reply).unwrap();
    for n in 2..=PAGE as u64 {
        writer.event(&rendered(n, message("more"))).unwrap();
    }
    writer.end_room(&RoomEnd::Stopped).unwrap();

    let room = folder.path().join("lab");
    let page = std::fs::read_to_string(room.join(page_file(1))).unwrap();
    assert!(
        page.contains(&format!("content=\"{POLICY}\"")),
        "the policy"
    );
    assert!(!page.contains("<script"), "names escaped");
    assert!(page.contains("Alice &lt;script&gt;"));
    assert!(!page.contains("javascript:"), "unsafe links are text");
    assert!(page.contains("click"));
    assert!(page.contains("&lt;img src=x onerror=alert(1)&gt;"));
    assert!(page.contains("src=\"files/2026-11-07%20a%20%23b.png\""));
    assert!(page.contains("alt=\"a &quot;b&quot;.png\""));
    assert!(page.contains("👍 2"));
    assert!(page.contains("(edited)"));
    assert!(
        page.contains("<a href=\"#e_e0\">Alice &lt;script&gt;</a>"),
        "{page}"
    );
    assert!(page.contains("Lab &amp; chat") && page.contains("Runs &lt;b&gt;"));
    assert!(page.contains("href=\"messages-2.html\">Later"));
    assert!(!page.contains("Export stopped here"));
    let second = std::fs::read_to_string(room.join(page_file(2))).unwrap();
    assert!(second.contains("href=\"messages.html\">← Earlier"));
    assert!(second.contains("Export stopped here"));
    assert!(second.ends_with("</html>\n"));
    assert!(!room.join(page_file(3)).exists());
    assert!(folder.path().join("style.css").exists());
    assert_eq!(href("lab/messages.html"), "lab/messages.html");
}

fn index_rooms() -> Vec<IndexRoom> {
    vec![
        IndexRoom {
            id: owned_room_id!("!lab:example.org"),
            name: "Lab & chat".into(),
            folder: Some("lab".into()),
            progress: RoomProgress::Complete,
            messages: 3,
            files: 1,
        },
        IndexRoom {
            id: owned_room_id!("!out:example.org"),
            name: "Outside".into(),
            folder: None,
            progress: RoomProgress::NotJoined,
            messages: 0,
            files: 0,
        },
    ]
}

#[test]
fn the_index_lists_the_tree() {
    let folder = TempDir::new().unwrap();
    let about = about(ExportFormat::Html);
    let rooms = index_rooms();
    let tree = vec![
        Line::Room { depth: 0, room: 0 },
        Line::Space {
            depth: 0,
            name: "Field".into(),
        },
        Line::Room { depth: 1, room: 1 },
        Line::Room { depth: 1, room: 0 },
    ];
    let summary = ExportSummary {
        rooms: 1,
        messages: 3,
        files: 1,
        ..Default::default()
    };
    let index = Index {
        about: &about,
        topic: Some("Ice"),
        space: true,
        tree: &tree,
        rooms: &rooms,
        state: IndexState::Cancelled,
        summary: &summary,
    };
    let mut html = HtmlWriter::new(folder.path()).unwrap();
    html.index(&index).unwrap();
    let page = std::fs::read_to_string(folder.path().join("index.html")).unwrap();
    assert!(page.contains("Ice &lt;sheets&gt;"));
    assert_eq!(page.matches("<a href=\"lab/messages.html\">").count(), 2);
    assert!(page.contains("Outside <span class=\"count\">Not joined</span>"));
    assert!(page.contains("class=\"space d0\">Field"));
    assert!(page.contains("Cancelled: 1 room, 3 messages, 1 file."));

    let mut json = JsonWriter::new(folder.path());
    json.index(&index).unwrap();
    let value: serde_json::Value =
        serde_json::from_slice(&std::fs::read(folder.path().join("index.json")).unwrap()).unwrap();
    assert_eq!(value["state"], "cancelled");
    assert_eq!(value["rooms"][1]["space"], "Field");
    assert_eq!(value["rooms"][2]["status"], "not_joined");
    assert_eq!(value["rooms"][3]["file"], "lab/messages.json");
}

#[test]
fn json_stays_valid_however_a_room_ends() {
    for end in [
        RoomEnd::Complete,
        RoomEnd::Stopped,
        RoomEnd::Failed("Forbidden".into()),
    ] {
        let folder = TempDir::new().unwrap();
        let mut writer = JsonWriter::new(folder.path());
        writer.begin_room(&head(ExportFormat::Json)).unwrap();
        let mut file = rendered(0, Shown::Hidden);
        file.shown = Shown::Message {
            style: Style::Text,
            document: Document::default(),
            edited: false,
            file: Some(Attached {
                kind: MediaKind::File,
                name: "r.pdf".into(),
                size: None,
                saved: Ok("files/r.pdf".into()),
            }),
        };
        writer.event(&file).unwrap();
        writer.event(&rendered(1, Shown::Hidden)).unwrap();
        writer.end_room(&end).unwrap();
        let value: serde_json::Value = serde_json::from_slice(
            &std::fs::read(folder.path().join("lab/messages.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(value["room_id"], "!lab:example.org");
        assert_eq!(value["room_name"], "Lab & chat");
        assert_eq!(value["messages"][1]["event_id"], "$e1");
        assert_eq!(value["files"]["$e0"], "files/r.pdf");
        let status = match end {
            RoomEnd::Complete => "complete",
            RoomEnd::Stopped => "stopped",
            RoomEnd::Failed(_) => "failed",
        };
        assert_eq!(value["status"], status);
    }
    // Nothing written in the room: still valid.
    let folder = TempDir::new().unwrap();
    let mut writer = JsonWriter::new(folder.path());
    writer.begin_room(&head(ExportFormat::Json)).unwrap();
    writer.end_room(&RoomEnd::Stopped).unwrap();
    let value: serde_json::Value =
        serde_json::from_slice(&std::fs::read(folder.path().join("lab/messages.json")).unwrap())
            .unwrap();
    assert_eq!(value["messages"], json!([]));
}

#[test]
fn text_lines_have_the_time_sender_and_text() {
    let mut event = rendered(0, message("first line\nsecond"));
    event.sender_name = "Alice".into();
    event.reactions = vec![("👍".into(), 2)];
    event.reply = Some(Quote {
        id: owned_event_id!("$q"),
        original: None,
    });
    let text = event_text(&event).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    assert!(lines[0].ends_with("  Alice: first line"), "{text}");
    assert_eq!(lines[0].len(), "2023-11-14 22:13  Alice: first line".len());
    assert_eq!(lines[1], "    second");
    assert_eq!(lines[2], "    (in reply to a message from earlier)");
    assert_eq!(lines[3], "    [👍 2]");
    let mut left_out = rendered(1, Shown::Hidden);
    left_out.sender_name = "Bob".into();
    left_out.shown = Shown::Message {
        style: Style::Emote,
        document: Document::default(),
        edited: false,
        file: Some(Attached {
            kind: MediaKind::Video,
            name: "big.mp4".into(),
            size: Some(200_000_000),
            saved: Err("left out, larger than 100 MB".into()),
        }),
    };
    assert!(
        event_text(&left_out)
            .unwrap()
            .contains("* Bob [file left out: big.mp4, 200 MB: left out, larger than 100 MB]")
    );
    assert!(event_text(&rendered(2, Shown::Hidden)).is_none());
}

fn planned(name: &str, joined: bool) -> PlannedRoom {
    PlannedRoom {
        id: format!("!{}:example.org", name.len()).try_into().unwrap(),
        name: name.into(),
        folder: joined.then(String::new),
    }
}

#[test]
fn rooms_get_folders_of_their_own_and_empty_subspaces_go() {
    let mut plan = Plan {
        title: "Ice".into(),
        topic: None,
        tree: vec![
            Line::Space {
                depth: 0,
                name: "Empty".into(),
            },
            Line::Space {
                depth: 1,
                name: "Also empty".into(),
            },
            Line::Space {
                depth: 0,
                name: "Field".into(),
            },
            Line::Room { depth: 1, room: 0 },
            Line::Room { depth: 0, room: 1 },
        ],
        rooms: vec![
            planned("Lab", true),
            planned("lab", true),
            planned("index.html", true),
            planned("../x", true),
            planned("Gone", false),
        ],
        space: true,
    };
    plan.prune();
    assert_eq!(plan.tree.len(), 3);
    assert!(matches!(&plan.tree[0], Line::Space { name, .. } if name == "Field"));
    plan.name_folders();
    let folders: Vec<Option<&str>> = plan.rooms.iter().map(|r| r.folder.as_deref()).collect();
    assert_eq!(
        folders,
        [
            Some("Lab"),
            Some("lab (2)"),
            Some("index.html (2)"),
            Some("_x"),
            None
        ]
    );
    assert_eq!(plan.exported(), 4);
}

#[test]
fn folder_names_say_what_and_when() {
    let at = SystemTime::now();
    let day = chrono::DateTime::<chrono::Local>::from(at).format("%Y-%m-%d");
    assert_eq!(
        folder_name("Ice sheets v1.2 / runs", at),
        format!("Mtrx – Ice sheets v1 2 _ runs – {day}")
    );
}
