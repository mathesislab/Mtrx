import MtrxFFI
import SwiftUI
import XCTest
@testable import Mtrx

/// The key screens drawn with made-up data, as PNGs to look at: in the
/// folder `SCREENSHOTS_DIR` names (CI: `TEST_RUNNER_SCREENSHOTS_DIR`), else
/// in a temporary one. Each in a window the size of the simulator's screen,
/// light, and a few dark too. They check nothing but that drawing works.
@MainActor
final class ScreenshotTests: XCTestCase {
    private var defaults: UserDefaults!
    private var session: FakeSession!
    private var model: SessionModel!
    private var app: AppModel!

    override func setUp() async throws {
        defaults = UserDefaults(suiteName: "ScreenshotTests")
        defaults.removePersistentDomain(forName: "ScreenshotTests")
        session = FakeSession(noHandle: .init())
        session.encryption = EncryptionState(deviceVerified: true, recovery: .enabled)
        model = SessionModel(session: session, defaults: defaults)
        model.start()
        await eventually { self.session.roomListListener != nil }
        session.roomListListener?.onUpdate(state: Screens.rooms)
        await eventually { self.model.loaded }
        app = AppModel(core: FakeMtrx(noHandle: .init()), browser: FakeBrowser())
    }

    override func tearDown() async throws {
        defaults.removePersistentDomain(forName: "ScreenshotTests")
    }

    // MARK: Tabs

    /// With the Notes to self at the top, and under each person the spaces
    /// we share.
    func testChats() async throws {
        model.tab = .chats
        try await snap("01-chats", HomeView())
    }

    func testSpaces() async throws {
        model.tab = .spaces
        model.selectSpace(nil)
        try await snap("02-spaces", HomeView())
    }

    /// Every kind of section, Rooms folded with its unread badge, and the
    /// Notes to self at the top.
    func testASpacesRooms() async throws {
        model.tab = .spaces
        model.selectSpace(.space("!polar"))
        model.sectionLayouts.change(.space("!polar")) { $0.toggleFold("rooms") }
        try await snap("03-space-rooms", HomeView())
    }

    /// Low priority hidden, Rooms moved first.
    func testShowSheet() async throws {
        let polar = try XCTUnwrap(model.lists.space(.space("!polar")))
        model.sectionLayouts.change(.space("!polar")) {
            $0.setShown("low_priority", false)
            $0.move(polar.sections.map(\.key), from: [4], to: 0)
        }
        try await snap("03b-show-sheet", ShowSheet(item: polar))
    }

    func testSettings() async throws {
        model.tab = .settings
        try await snap("13-settings", HomeView())
    }

    /// Home, then each space, its rooms newest first; unfolded, the
    /// headings without unread counts.
    func testRooms() async throws {
        model.tab = .rooms
        model.roomsPath = []
        try await snap("04-rooms", HomeView())
        try await snap("04-rooms-dark", dark: true, HomeView())
    }

    /// Polar lab folded, with its unread count; Teaching unfolded.
    func testRoomsFolded() async throws {
        model.tab = .rooms
        model.roomsPath = []
        model.toggleGroupFold("!polar")
        try await snap("04b-rooms-folded", HomeView())
    }

    func testARoomWithItsTopicLine() async throws {
        session.fakeTimeline.initialRows = Screens.timeline
        model.tab = .rooms
        model.roomsPath = ["!ice"]
        try await snap("05-room", wait: 2, HomeView())
        try await snap("05-room-dark", dark: true, wait: 2, HomeView())
    }

    /// Under a direct chat's header: the rooms shared with Ann, by space.
    func testSharedRooms() async throws {
        session.shared = ["!ice", "!sea", "!models", "!ann", "!polar"]
        try await snap(
            "05c-shared-rooms",
            NavigationStack {
                SharedRoomsPanel(userId: "@ann:example.org", name: "Ann Lindqvist")
                    .navigationTitle("Ann Lindqvist")
                    .navigationBarTitleDisplayMode(.inline)
            })
        session.shared = ["!ann"]
        try await snap(
            "05d-no-shared-rooms",
            NavigationStack {
                SharedRoomsPanel(userId: "@ann:example.org", name: "Ann Lindqvist")
                    .navigationTitle("Ann Lindqvist")
                    .navigationBarTitleDisplayMode(.inline)
            })
    }

    /// The draft left in the room is back, with the message it answers.
    func testARoomWithItsDraft() async throws {
        session.fakeTimeline.initialRows = Screens.timeline
        session.drafts["!ice"] = Draft(text: "I'll add them to the paper's figures", replyTo: "$2")
        model.tab = .rooms
        model.roomsPath = ["!ice"]
        try await snap("05b-room-draft", wait: 2, HomeView())
    }

    /// The largest text size (Accessibility): the lists put a room's time
    /// and counts under its name; a room, and its info.
    func testLargeText() async throws {
        model.tab = .chats
        try await snap("14-large-chats", large: true, HomeView())
        model.tab = .rooms
        model.roomsPath = []
        try await snap("14b-large-rooms", large: true, HomeView())
        session.fakeTimeline.initialRows = Screens.timeline
        model.roomsPath = ["!ice"]
        try await snap("14c-large-room", large: true, wait: 2, HomeView())
        session.settings = Screens.settings
        session.memberList = Screens.members
        try await snap(
            "14d-large-room-info", large: true, RoomInfoView(room: Screens.ice, model: model.info(for: Screens.ice)))
    }

    // MARK: A room's topic and info

    func testTopicPanel() async throws {
        session.fakePinned.initialRows = Array(Screens.timeline.suffix(2))
        let info = model.info(for: Screens.ice)
        await info.loadSettings()
        let pinned = PinnedModel(session: session, roomId: "!ice")
        try await snap(
            "06-topic-panel",
            TopicPanel(topic: parseTopic(topic: Screens.topic), canEdit: true, info: info, pinned: pinned) { _ in })
    }

    func testTopicEditor() async throws {
        let info = model.info(for: Screens.ice)
        try await snap(
            "06b-topic-editor",
            NavigationStack { TopicEditor(draft: TopicDraft(parseTopic(topic: Screens.topic)), info: info) })
    }

    func testRoomInfo() async throws {
        session.settings = Screens.settings
        session.memberList = Screens.members
        try await snap("07-room-info", RoomInfoView(room: Screens.ice, model: model.info(for: Screens.ice)))
    }

    func testRoomSettingsAndMembers() async throws {
        session.settings = Screens.settings
        session.memberList = Screens.members
        let info = model.info(for: Screens.ice)
        await info.load()
        try await snap(
            "07b-room-settings",
            NavigationStack {
                List {
                    RoomSettingsSections(room: Screens.ice, model: info)
                }
                .navigationTitle("Room info")
                .navigationBarTitleDisplayMode(.inline)
            })
        try await snap(
            "07c-room-members",
            NavigationStack {
                List {
                    MembersSection(room: Screens.ice, model: info)
                }
                .navigationTitle("Room info")
                .navigationBarTitleDisplayMode(.inline)
            })
    }

    func testMemberMenu() async throws {
        session.memberList = Screens.members
        let info = model.info(for: Screens.ice)
        await info.load()
        try await snap("08-member-menu", NavigationStack { MemberMenu(member: Screens.members[2], model: info) })
    }

    // MARK: Spaces: overview, joining, making

    func testSpaceOverview() async throws {
        session.children = .success(Screens.children)
        session.settings = Make.settings(access: .public, encrypted: false, listed: true)
        session.memberList = Screens.members
        try await snap("09-space-overview", SpaceOverviewView(space: Screens.polar))
    }

    /// The last list, its server not answering.
    func testSpaceOverviewNotAnswering() async throws {
        model.spaceChildren["!polar"] = Screens.children
        session.children = .failure(MtrxError.Other(reason: "timed out"))
        try await snap("09b-space-overview-failed", SpaceOverviewView(space: Screens.polar))
    }

    func testJoin() async throws {
        session.directory = .success(DirectoryPage(rooms: Screens.directory, next: "2"))
        let join = JoinModel(session: session, ownServer: model.serverName)
        try await snap("10-join", JoinView(model: join))
    }

    func testNewRoomWithCoOwners() async throws {
        session.offersOwnersResult = true
        let new = NewRoomModel(session: session, space: false, parent: (id: "!polar", name: "Polar lab"))
        new.name = "Fieldwork"
        new.topic = "Planning the spring campaign"
        for person in ["@ann:example.org", "@bob:example.org"] {
            new.person = person
            new.addPerson()
        }
        new.setCoOwner("@ann:example.org", true)
        try await snap("11-new-room", NewRoomView(model: new))
    }

    func testInvite() async throws {
        let party = model.room("!party")!
        try await snap("12-invite", NavigationStack { InviteView(room: party) })
    }

    // MARK: Drawing

    /// Draw `view` in a window the size of the screen, after `wait` seconds
    /// for its tasks, and save it as `<name>.png`; with `large`, in the
    /// largest text size (Accessibility, from iOS 17).
    private func snap<V: View>(
        _ name: String, dark: Bool = false, large: Bool = false, wait: Double = 1, _ view: V
    ) async throws {
        let scene = try XCTUnwrap(UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }.first)
        let window = UIWindow(windowScene: scene)
        window.frame = scene.screen.bounds
        window.overrideUserInterfaceStyle = dark ? .dark : .light
        if large, #available(iOS 17, *) {
            window.traitOverrides.preferredContentSizeCategory = .accessibilityExtraExtraExtraLarge
        }
        window.rootViewController = UIHostingController(
            rootView: view
                .environmentObject(app)
                .environmentObject(model)
                .environmentObject(model.images)
                .environmentObject(model.presence))
        window.makeKeyAndVisible()
        defer { window.isHidden = true }
        try await Task.sleep(nanoseconds: UInt64(wait * 1_000_000_000))
        let format = UIGraphicsImageRendererFormat()
        format.scale = 2
        let image = UIGraphicsImageRenderer(bounds: window.bounds, format: format).image { _ in
            _ = window.drawHierarchy(in: window.bounds, afterScreenUpdates: true)
        }
        let data = try XCTUnwrap(image.pngData())
        try data.write(to: Self.folder.appendingPathComponent("\(name).png"))
    }

    private static var folder: URL {
        let path = ProcessInfo.processInfo.environment["SCREENSHOTS_DIR"]
        let folder = path.map { URL(fileURLWithPath: $0) }
            ?? FileManager.default.temporaryDirectory.appendingPathComponent("ios-screenshots")
        try? FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
        return folder
    }
}

/// Made-up rooms, people and messages for the screenshots.
private enum Screens {
    static let now = Date()

    static let topic = """
        Sea ice and ice sheets: models, data and papers.
        Weekly meeting on Thursdays.
        [call Weekly passcode 4711] https://meet.example.org/ice
        [link Drafts] https://overleaf.com/project/ice
        """

    static let ice = Make.room(
        "!ice", name: "Ice sheets", unread: 0, lastActivity: now, topic: topic, parents: ["!polar"],
        pinned: ["$p2"], alias: "#ice:example.org", members: 4)
    static let polar = Make.room(
        "!polar", kind: .space, name: "Polar lab", topic: "The polar group: rooms for projects and talk.",
        members: 12)

    static let rooms = RoomListState(
        rooms: [
            ice,
            Make.room(
                "!sea", name: "Sea ice", unread: 3, lastActivity: now.addingTimeInterval(-600), parents: ["!polar"]),
            Make.room(
                "!ann", kind: .direct, name: "Ann Lindqvist", unread: 2, directUser: "@ann:example.org",
                lastActivity: now.addingTimeInterval(-1200)),
            Make.room(
                "!reading", name: "Reading group", lastActivity: now.addingTimeInterval(-86_400), parents: ["!teach"]),
            Make.room(
                "!models", name: "Models", unread: 5, lastActivity: now.addingTimeInterval(-3_600),
                parents: ["!polar"], mentions: 1),
            Make.room(
                "!notes", kind: .direct, name: "Notes to self", directUser: "@me:example.org",
                lastActivity: now.addingTimeInterval(-3_000)),
            Make.room(
                "!paper", name: "Ice sheet paper", lastActivity: now.addingTimeInterval(-5_000), parents: ["!polar"]),
            Make.room(
                "!old", name: "Field season 2025", lastActivity: now.addingTimeInterval(-900_000),
                parents: ["!polar"]),
            Make.room("!join", invited: true, name: "Glacier seminar", parents: ["!polar"]),
            Make.room(
                "!bob", kind: .direct, name: "Bob Meyer", directUser: "@bob:example.org",
                lastActivity: now.addingTimeInterval(-7_200)),
            Make.room("!party", invited: true, name: "Summer party", topic: "Saturday at the lake."),
            Make.room(
                "!cy", kind: .direct, name: "Cy Park", directUser: "@cy:example.org",
                lastActivity: now.addingTimeInterval(-200_000)),
            Make.room("!teach", kind: .space, name: "Teaching"),
            polar,
        ],
        loaded: true,
        spaces: [
            SpaceRooms(
                spaceId: "!polar",
                roomIds: ["!ice", "!sea", "!ann", "!models", "!paper", "!old", "!join", "!bob"],
                sections: [
                    Make.section(.invites, ["!join"]),
                    Make.section(.directMessages, ["!ann", "!bob"]),
                    Make.section(.favourites, ["!ice"]),
                    Make.section(.category, ["!paper"], name: "Papers"),
                    Make.section(.rooms, ["!sea", "!models"]),
                    Make.section(.lowPriority, ["!old"]),
                ]),
            SpaceRooms(spaceId: "!teach", roomIds: ["!ann", "!reading"]),
        ],
        roomsOutsideSpaces: ["!party"], notesAtTop: true, notesRoomId: "!notes",
        drafts: ["!ann", "!reading", "!paper"])

    static var timeline: [TimelineRow] {
        let start = now.addingTimeInterval(-3_600)
        func at(_ minutes: Double) -> Double { start.timeIntervalSince(Make.start) / 60 + minutes }
        return [
            TimelineRow(id: "d", content: .day(date: start)),
            Make.row(
                "1", Make.event("ann", Make.text("Morning! The new ice sheet runs are in."), at: at(0), eventId: "$1")),
            Make.row(
                "2",
                Make.event(
                    "ann", Make.text("Plots at https://example.org/runs"), at: at(1), eventId: "$2",
                    readBy: [Make.reader("bob")]),
                firstInGroup: false),
            Make.row(
                "3",
                Make.event(
                    "me",
                    Make.text(
                        "Great, looking now.",
                        reactions: [Reaction(key: "👍", count: 2, byUs: false, senders: [])]),
                    at: at(5), own: true, eventId: "$3")),
            Make.row("x", Make.event("me", .redacted, at: at(6), own: true, eventId: "$x"), firstInGroup: false),
            Make.row("u", Make.event("me", .unableToDecrypt, at: at(7), own: true, eventId: "$u"), firstInGroup: false),
            Make.row(
                "q",
                Make.event(
                    "me",
                    Make.text(
                        "Thanks, the plots at https://example.org/runs look right.",
                        edited: true,
                        replyTo: ReplyQuote(
                            eventId: "$2", state: .loaded, senderId: "@ann:example.org", senderName: "Ann",
                            kind: .text, body: "Plots at https://example.org/runs")),
                    at: at(8), own: true, eventId: "$q"),
                firstInGroup: false),
            Make.row(
                "n",
                Make.event(
                    "me",
                    .message(
                        message: Message(
                            kind: .notice, body: "Runs archived.", edited: false, media: nil, reactions: [],
                            replyTo: nil, rawBody: "")),
                    at: at(9), own: true, eventId: "$n"),
                firstInGroup: false),
            Make.row(
                "o", Make.event("me", .other(description: "poll"), at: at(10), own: true, eventId: "$o"),
                firstInGroup: false),
            Make.row(
                "p1", Make.event("bob", Make.text("Meeting moved to 14:00 on Thursday."), at: at(20), eventId: "$p1")),
            Make.row(
                "p2",
                Make.event(
                    "me", Make.text("Agenda: runs, the paper, fieldwork."), at: at(22), own: true, eventId: "$p2")),
        ]
    }

    static let settings = Make.settings(
        access: .knock, encrypted: true, listed: false,
        spaces: [SpaceLink(spaceId: "!polar", suggested: true, canChange: true)], canAddTo: ["!teach"],
        banned: [Banned(userId: "@spam:example.org", name: "Spammer", reason: "Spam")], address: "#ice:example.org")

    static let members = [
        Make.member("me", level: 100, upTo: 99),
        Make.member("ann", level: nil),
        Make.member("bob", level: 50, upTo: 100, remove: true, ban: true),
        Make.member("cy", level: 0, upTo: 100, remove: true, ban: true),
        Make.member("dee", level: 0, upTo: 100, remove: true, invited: true),
    ]

    static let children = [
        Make.directoryRoom(
            "!ice", name: "Ice sheets", membership: .joined, members: 4, topic: "Models, data and papers",
            role: .admin),
        Make.directoryRoom("!sea", name: "Sea ice", membership: .joined, members: 9),
        Make.directoryRoom("!field", name: "Fieldwork", access: .spaceMembers, members: 6, topic: "Spring campaign"),
        Make.directoryRoom("!data", name: "Data", access: .knock, members: 3),
        Make.directoryRoom("!seminar", name: "Seminar", access: .knock, membership: .asked, members: 15),
        Make.directoryRoom("!glacio", name: "Glaciology", space: true, members: 20, children: 4),
    ]

    static let directory = [
        Make.directoryRoom("!matrix", name: "Matrix HQ", members: 12_000, topic: "The Matrix protocol"),
        Make.directoryRoom("!climate", name: "Climate science", space: true, members: 340, children: 12),
        Make.directoryRoom("!earth", name: "Earth system models", access: .knock, members: 85),
        Make.directoryRoom("!ice", name: "Ice sheets", membership: .joined, members: 4, role: .admin),
    ]
}
