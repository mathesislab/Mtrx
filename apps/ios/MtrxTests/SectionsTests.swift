import MtrxFFI
import XCTest
@testable import Mtrx

/// Which of a space's sections show on this phone, folded or not, in which
/// order. The library arranges them by it.
final class SectionLayoutTests: XCTestCase {
    private let arranged = ["invites", "direct_messages", "favourites", "category:a", "rooms", "low_priority"]

    func testReordering() {
        var layout = SectionLayout()
        XCTAssertNil(layout.order)
        // Rooms (4) before Direct messages (1).
        layout.move(arranged, from: [4], to: 1)
        XCTAssertEqual(
            layout.order, ["invites", "rooms", "direct_messages", "favourites", "category:a", "low_priority"])
        // Invites (0) last.
        layout.move(layout.order!, from: [0], to: 6)
        XCTAssertEqual(layout.order?.last, "invites")
    }

    func testFoldingAndShowing() {
        var layout = SectionLayout()
        layout.toggleFold("rooms")
        XCTAssertTrue(layout.folds("rooms"))
        layout.toggleFold("rooms")
        XCTAssertFalse(layout.folds("rooms"))
        layout.setAllShown(arranged, false)
        XCTAssertFalse(arranged.contains(where: layout.shows))
        layout.setShown("rooms", true)
        XCTAssertTrue(layout.shows("rooms"))
        layout.setAllShown(arranged, true)
        XCTAssertTrue(arranged.allSatisfy(layout.shows))
    }

    func testAsTheLibraryTakesIt() {
        var layout = SectionLayout()
        XCTAssertEqual(layout.library, MtrxFFI.SectionLayout(order: [], hidden: [], folded: []))
        layout.move(arranged, from: [4], to: 0)
        layout.setShown("low_priority", false)
        layout.toggleFold("rooms")
        layout.toggleFold("invites")
        XCTAssertEqual(
            layout.library,
            MtrxFFI.SectionLayout(
                order: ["rooms", "invites", "direct_messages", "favourites", "category:a", "low_priority"],
                hidden: ["low_priority"], folded: ["invites", "rooms"]))
    }
}

/// A space's list as this phone shows it, and the Notes to self at the top.
@MainActor
final class SpaceSectionsTests: XCTestCase {
    /// Team with Ann's chat, Lab and Paper; the Notes to self.
    private func lists(notesAtTop: Bool = true) -> RoomLists {
        Make.lists(
            [
                Make.room("!notes", kind: .direct, name: "Notes to self", directUser: "@me:example.org"),
                Make.room("!ann", kind: .direct, name: "Ann", unread: 2, directUser: "@ann:example.org"),
                Make.room("!lab", name: "Lab", unread: 3, parents: ["!team"]),
                Make.room("!paper", name: "Paper", unread: 1, parents: ["!team"]),
                Make.room("!team", kind: .space, name: "Team"),
            ],
            notesAtTop: notesAtTop, members: ["!team": ["@ann:example.org", "@me:example.org"]])
    }

    /// Team's sections with a favourite, a category and a low priority
    /// room: in the account's order, named, the empty ones left out.
    func testSectionsInOrderNamedWithoutEmptyOnes() {
        let lists = Make.lists(
            [
                Make.room("!ann", kind: .direct, name: "Ann", directUser: "@ann:example.org"),
                Make.room("!ice", name: "Ice", parents: ["!team"]),
                Make.room("!lab", name: "Lab", parents: ["!team"]),
                Make.room("!paper", name: "Paper", parents: ["!team"]),
                Make.room("!old", name: "Old", parents: ["!team"]),
                Make.room("!team", kind: .space, name: "Team"),
            ],
            members: ["!team": ["@ann:example.org", "@me:example.org"]], favourites: ["!ice"],
            lowPriority: ["!old"],
            categories: [TestCategory(space: "!team", id: "a", name: "Papers", rooms: ["!paper"])])
        let sections = lists.spaceList(
            key: .space(id: "!team"), layout: SectionLayout().library, sort: .aToZ, filter: "")
        XCTAssertEqual(
            sections.map(\.title), ["Direct messages", "Favourites", "Papers", "Rooms", "Low priority"])
        XCTAssertEqual(
            sections.map(\.key), ["direct_messages", "favourites", "category:a", "rooms", "low_priority"])
        XCTAssertEqual(sections.map { $0.rooms.map(\.id) }, [["!ann"], ["!ice"], ["!paper"], ["!lab"], ["!old"]])
    }

    /// The phone's layout, as the library takes it: hidden and folded.
    func testTheLayoutArrangesTheList() {
        let lists = lists()
        var layout = SectionLayout()
        layout.toggleFold("rooms")
        layout.setShown("direct_messages", false)
        let sections = lists.spaceList(
            key: .space(id: "!team"), layout: layout.library, sort: .aToZ, filter: "")
        XCTAssertEqual(sections.map(\.title), ["Rooms"])
        XCTAssertTrue(sections[0].folded)
        XCTAssertEqual(sections[0].rooms.map(\.id), ["!lab", "!paper"])
        XCTAssertEqual(sections[0].unread, 4)
    }

    func testTheLayoutIsKeptPerSpaceOnThisPhone() {
        let defaults = UserDefaults(suiteName: "SpaceSectionsTests")!
        defaults.removePersistentDomain(forName: "SpaceSectionsTests")
        defer { defaults.removePersistentDomain(forName: "SpaceSectionsTests") }
        SectionLayouts(defaults: defaults).change(.space(id: "!team")) { $0.toggleFold("rooms") }
        let again = SectionLayouts(defaults: defaults)
        XCTAssertTrue(again[.space(id: "!team")].folds("rooms"))
        XCTAssertFalse(again[.space(id: "!other")].folds("rooms"))
        again.forget()
        XCTAssertFalse(SectionLayouts(defaults: defaults)[.space(id: "!team")].folds("rooms"))
    }

    func testNotesAtTopIsChangedForTheAccount() async {
        let session = FakeSession(noHandle: .init())
        let model = SessionModel(session: session)
        model.start()
        await eventually { session.roomListListener != nil }
        let on = lists()
        session.roomListListener?.onUpdate(lists: on)
        await eventually { model.lists === on }
        XCTAssertTrue(model.notesAtTop)
        XCTAssertEqual(model.lists.notes()?.id, "!notes")
        await model.setNotesAtTop(false).value
        XCTAssertEqual(session.changes, ["notes at top false"])
        // As the library lists it.
        let off = lists(notesAtTop: false)
        session.roomListListener?.onUpdate(lists: off)
        await eventually { !model.notesAtTop }
        XCTAssertNil(model.lists.notes())
        session.changeError = MtrxError.Other(reason: "No connection")
        await model.setNotesAtTop(true).value
        XCTAssertEqual(model.error, "No connection")
    }
}
