//! Room list rendering.

use std::{collections::HashMap, fmt::Write};

use mtrx_core::{OwnedRoomId, RoomKind, RoomListState, RoomSummary};

/// Print spaces as a tree with their rooms, then rooms outside any space,
/// then direct messages and invites.
pub fn print_tree(state: &RoomListState, ids: bool) {
    let (rooms, graph) = (&state.rooms, &state.graph);
    let by_id: HashMap<&OwnedRoomId, &RoomSummary> = rooms.iter().map(|r| (&r.id, r)).collect();
    let in_list = |r: &RoomSummary| graph.parents(&r.id).iter().any(|p| by_id.contains_key(p));

    for space in graph.top_level(rooms) {
        let Some(space) = by_id.get(&space) else {
            continue;
        };
        println!("{}", line(space, ids));
        for (depth, room) in graph.tree(&space.id) {
            if let Some(room) = by_id.get(&room) {
                println!("{}{}", "  ".repeat(depth + 1), line(room, ids));
            }
        }
    }

    let loose = |r: &&RoomSummary, kind| !r.invited && r.kind == kind && !in_list(r);
    print_section(
        "Rooms",
        rooms.iter().filter(|r| loose(r, RoomKind::Room)),
        ids,
    );
    print_section(
        "Direct messages",
        rooms.iter().filter(|r| loose(r, RoomKind::Direct)),
        ids,
    );
    print_section("Invites", rooms.iter().filter(|r| r.invited), ids);
}

fn print_section<'a>(title: &str, rooms: impl Iterator<Item = &'a RoomSummary>, ids: bool) {
    let mut rooms = rooms.peekable();
    if rooms.peek().is_some() {
        println!("{title}");
        for room in rooms {
            println!("  {}", line(room, ids));
        }
    }
}

fn line(room: &RoomSummary, ids: bool) -> String {
    let marker = match room.kind {
        RoomKind::Space => "+",
        RoomKind::Room => "#",
        RoomKind::Direct => "@",
    };
    let mut out = format!("{marker} {}", room.name);
    if room.unread_mentions > 0 {
        let _ = write!(
            out,
            "  [{} unread, {} mentions]",
            room.unread_messages, room.unread_mentions
        );
    } else if room.unread_messages > 0 {
        let _ = write!(out, "  [{} unread]", room.unread_messages);
    }
    if room.encrypted {
        out += "  (e2ee)";
    }
    if ids {
        let _ = write!(out, "  {}", room.id);
    }
    out
}
