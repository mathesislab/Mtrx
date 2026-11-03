import MtrxFFI
import XCTest
@testable import Mtrx

/// Reading others' presence and telling the library what the app does,
/// which sets ours, through the session model.
@MainActor
final class PresenceModelTests: XCTestCase {
    private var session: FakeSession!
    private var defaults: UserDefaults!
    private var model: SessionModel!
    private var presence: PresenceModel { model.presence }

    override func setUp() async throws {
        defaults = UserDefaults(suiteName: "PresenceModelTests")
        defaults.removePersistentDomain(forName: "PresenceModelTests")
        session = FakeSession(noHandle: .init())
    }

    override func tearDown() async throws {
        defaults.removePersistentDomain(forName: "PresenceModelTests")
    }

    /// `sharing` nil: as never set.
    private func start(sharing: Bool? = true) async {
        if let sharing {
            defaults.set(sharing, forKey: "sharePresence")
        }
        model = SessionModel(session: session, defaults: defaults)
        model.start()
        await eventually { self.session.roomListListener != nil }
    }

    private static let bob = "@bob:example.org"
    private static let carol = "@carol:example.org"
    private static let dan = "@dan:example.org"

    /// The integration test's rooms, with the Notes to self.
    private func rooms(loaded: Bool = true, extra: [RoomSummary] = []) -> RoomLists {
        let notes = Make.room("!notes", kind: .direct, name: "Notes", directUser: "@me:example.org")
        return Make.roomsBySpace(loaded: loaded, extra: [notes] + extra)
    }

    private func push(_ lists: RoomLists) async {
        session.roomListListener?.onUpdate(lists: lists)
        await eventually { self.model.lists === lists }
    }

    // MARK: Others

    func testSubscribesToTheDirectChatsPeopleOnceLoaded() async {
        await start()
        model.setForeground(true)
        await push(rooms(loaded: false))
        XCTAssertTrue(session.presenceSubscriptions.isEmpty)

        await push(rooms())
        // Bob and Carol (an invite), not the Notes to self.
        XCTAssertEqual(session.presenceSubscriptions.map(\.users), [[Self.bob, Self.carol]])

        // Other changes keep the subscription.
        var changed = Make.roomsBySpaceRooms + [Make.room("!notes", kind: .direct, directUser: "@me:example.org")]
        changed[3].unreadMessages = 5
        changed.removeAll { $0.id == "!lab" }
        await push(Make.lists(changed, children: Make.roomsBySpaceChildren, members: ["!team": [Self.bob]]))
        XCTAssertEqual(session.presenceSubscriptions.count, 1)

        // A new direct chat: subscribed anew, the old one cancelled.
        await push(rooms(extra: [Make.room("!dan", kind: .direct, name: "Dan", directUser: Self.dan)]))
        XCTAssertEqual(session.presenceSubscriptions.map(\.users).last, [Self.bob, Self.carol, Self.dan])
        XCTAssertEqual(session.presenceSubscriptions.map(\.handle.cancelled), [true, false])
    }

    func testDotsFollowTheSubscription() async {
        await start()
        model.setForeground(true)
        await push(rooms())
        let bobRoom = model.room("!bob")!
        XCTAssertNil(presence.presence(of: bobRoom))

        session.presenceSubscriptions[0].listener.onUpdate(presences: [Self.bob: Make.presence(.online)])
        await eventually { self.presence.presence(of: bobRoom)?.state == .online }
        session.presenceSubscriptions[0].listener.onUpdate(presences: [
            Self.bob: Make.presence(.unavailable), Self.carol: Make.presence(.online),
        ])
        await eventually { self.presence.presence(of: bobRoom)?.state == .unavailable }
        XCTAssertEqual(presence.presence(of: Self.carol)?.state, .online)
        // Rooms that are not someone else's direct chat show none.
        XCTAssertNil(presence.presence(of: model.room("!notes")!))
        XCTAssertNil(presence.presence(of: model.room("!lab")!))
    }

    func testReadsOnlyInTheForeground() async {
        await start()
        await push(rooms())
        // Not in the foreground yet.
        XCTAssertTrue(session.presenceSubscriptions.isEmpty)
        model.setForeground(true)
        XCTAssertEqual(session.presenceSubscriptions.count, 1)
        model.setForeground(false)
        XCTAssertEqual(session.presenceSubscriptions[0].handle.cancelled, true)
        model.setForeground(true)
        XCTAssertEqual(session.presenceSubscriptions.count, 2)
        XCTAssertEqual(session.presenceSubscriptions[1].users, [Self.bob, Self.carol])
    }

    func testSharingSyncDoesNotWaitForOurPresence() async {
        session.ownPresenceHangs = true
        await start()
        model.setForeground(true)
        await eventually { self.session.calls.count == 2 }
        // In either order.
        XCTAssertEqual(Set(session.calls), ["sharing true", "startSync"])
        session.ownPresenceHangs = false
        await eventually { self.session.calls.last == "presence foreground" }
    }

    func testNotSharingSyncWaitsUntilWeAreOffline() async throws {
        session.ownPresenceHangs = true
        await start(sharing: false)
        model.setForeground(true)
        try await Task.sleep(nanoseconds: 200_000_000)
        XCTAssertEqual(session.calls, ["sharing false"])
        session.ownPresenceHangs = false
        await eventually { self.session.calls.contains("startSync") }
        XCTAssertEqual(session.calls, ["sharing false", "presence foreground", "startSync"])
    }

    func testSignOutEndsTheSubscriptionAndGoesOfflineFirst() async throws {
        await start()
        model.setForeground(true)
        await push(rooms())
        await eventually { self.session.calls.contains("startSync") }
        try await model.logout()
        XCTAssertEqual(session.presenceSubscriptions[0].handle.cancelled, true)
        XCTAssertEqual(session.calls.suffix(2), ["presence signOut", "logout"])
        // The preference stays.
        XCTAssertEqual(defaults.object(forKey: "sharePresence") as? Bool, true)
    }

    func testThePersonSheetReadsAFreshPresence() async {
        await start()
        model.setForeground(true)
        await push(rooms())
        session.presenceSubscriptions[0].listener.onUpdate(presences: [Self.bob: Make.presence(.online)])
        await eventually { self.presence.presence(of: Self.bob) != nil }

        session.presenceResult = .success(Make.presence(.unavailable, status: "Lunch"))
        await presence.refresh(Self.bob)
        XCTAssertEqual(session.presenceRequests, [Self.bob])
        XCTAssertEqual(presence.presence(of: Self.bob)?.statusMessage, "Lunch")

        // Another failure keeps what is known.
        session.presenceResult = .failure(MtrxError.Other(reason: "Forbidden"))
        await presence.refresh(Self.bob)
        XCTAssertEqual(presence.presence(of: Self.bob)?.statusMessage, "Lunch")
        XCTAssertTrue(presence.available)
    }

    func testHiddenAfterOnUnavailable() async {
        await start()
        model.setForeground(true)
        await push(rooms())
        let subscription = session.presenceSubscriptions[0]
        subscription.listener.onUpdate(presences: [Self.bob: Make.presence(.online)])
        await eventually { self.presence.presence(of: Self.bob) != nil }

        subscription.listener.onUnavailable()
        await eventually { !self.presence.available }
        await assertHidden()
    }

    func testHiddenAfterTheError() async {
        await start()
        model.setForeground(true)
        await push(rooms())
        session.presenceSubscriptions[0].listener.onUpdate(presences: [Self.bob: Make.presence(.online)])
        await eventually { self.presence.presence(of: Self.bob) != nil }

        session.presenceResult = .failure(MtrxError.PresenceUnavailable)
        await presence.refresh(Self.bob)
        await assertHidden()
    }

    /// No presence shows, none is read or set any more.
    private func assertHidden(file: StaticString = #filePath, line: UInt = #line) async {
        XCTAssertFalse(presence.available, file: file, line: line)
        XCTAssertTrue(presence.presences.isEmpty, file: file, line: line)
        XCTAssertNil(presence.presence(of: model.room("!bob")!), file: file, line: line)
        XCTAssertEqual(session.presenceSubscriptions.map(\.handle.cancelled), [true], file: file, line: line)

        await settle()
        let told = session.presenceCalls
        await push(rooms(extra: [Make.room("!dan", kind: .direct, name: "Dan", directUser: Self.dan)]))
        model.setForeground(false)
        model.setForeground(true)
        presence.input()
        presence.setSharing(false)
        await settle()
        XCTAssertEqual(session.presenceSubscriptions.count, 1, file: file, line: line)
        XCTAssertEqual(session.presenceCalls, told, file: file, line: line)
    }

    // MARK: Ours

    func testForegroundAndBackgroundBeforeSyncStops() async {
        await start()
        model.setForeground(true)
        await eventually { self.session.calls.count == 3 }
        XCTAssertEqual(session.presenceCalls, ["sharing true", "presence foreground"])

        model.setForeground(false)
        await eventually { self.session.calls.count == 5 }
        XCTAssertEqual(session.calls.suffix(2), ["presence background", "stopSync"])

        // "Share my presence" goes to the library once.
        model.setForeground(true)
        await eventually { self.session.calls.count == 7 }
        XCTAssertEqual(session.presenceCalls.suffix(2), ["presence background", "presence foreground"])
    }

    func testNotSharingIsPassedBeforeSyncFirstStarts() async {
        await start(sharing: false)
        model.setForeground(true)
        await eventually { self.session.calls.count == 3 }
        XCTAssertEqual(session.calls, ["sharing false", "presence foreground", "startSync"])
    }

    func testSharingIsOnByDefault() async {
        await start(sharing: nil)
        XCTAssertTrue(presence.sharing)
        model.setForeground(true)
        await eventually { self.session.presenceCalls == ["sharing true", "presence foreground"] }
    }

    func testTheSwitch() async {
        await start()
        model.setForeground(true)
        await eventually { self.session.syncStarts == 1 }
        presence.setSharing(false)
        await eventually { self.session.presenceCalls.last == "sharing false" }
        XCTAssertEqual(defaults.object(forKey: "sharePresence") as? Bool, false)
        presence.setSharing(true)
        await eventually { self.session.presenceCalls.last == "sharing true" }
        XCTAssertEqual(defaults.object(forKey: "sharePresence") as? Bool, true)
        // Unchanged: nothing to say.
        presence.setSharing(true)
        await settle()
        XCTAssertEqual(session.presenceCalls.count, 4)
    }

    func testInputInTheForegroundIsPassedOnInOrder() async {
        await start()
        presence.input()
        model.setForeground(true)
        presence.input()
        presence.input()
        model.setForeground(false)
        presence.input()
        await eventually { self.session.syncStops == 1 }
        await settle()
        XCTAssertEqual(session.presenceCalls, [
            "sharing true", "presence foreground", "presence input", "presence input", "presence background",
        ])
    }

    func testFailuresToSetOursAreIgnored() async {
        await start()
        session.ownPresenceError = MtrxError.Other(reason: "No connection")
        model.setForeground(true)
        await eventually { self.session.syncStarts == 1 }
        await settle()
        XCTAssertTrue(presence.available)
        XCTAssertNil(model.error)
    }

    func testHiddenWhenSettingOursIsRefused() async {
        await start()
        session.ownPresenceError = MtrxError.PresenceUnavailable
        model.setForeground(true)
        await push(rooms())
        await eventually { !self.presence.available }
        // Sync starts all the same.
        await eventually { self.session.syncStarts == 1 }
        XCTAssertTrue(session.presenceSubscriptions.allSatisfy(\.handle.cancelled))
    }

    /// Let tasks started on the main actor run.
    private func settle() async {
        try? await Task.sleep(nanoseconds: 50_000_000)
    }
}
