import MtrxFFI
import XCTest
@testable import Mtrx

/// The dot and the line under the name, per state and age.
final class PresenceTextTests: XCTestCase {
    private let now = Make.start

    func testDots() {
        XCTAssertEqual(PresenceText.dot(Make.presence(.online)), .online)
        XCTAssertEqual(PresenceText.dot(Make.presence(.unavailable)), .idle)
        XCTAssertNil(PresenceText.dot(Make.presence(.offline, lastActive: now)))
        XCTAssertNil(PresenceText.dot(nil))
        XCTAssertEqual(PresenceText.Dot.online.label, "Online")
        XCTAssertEqual(PresenceText.Dot.idle.label, "Idle")
    }

    func testLines() {
        let line = { (presence: Presence?) in PresenceText.line(presence, now: self.now, locale: Locale(identifier: "en_US")) }
        XCTAssertEqual(line(Make.presence(.online)), "Active now")
        XCTAssertEqual(line(Make.presence(.unavailable)), "Idle")
        // The status message first, whatever the state.
        XCTAssertEqual(line(Make.presence(.online, status: "Lunch")), "Lunch")
        XCTAssertEqual(line(Make.presence(.offline, lastActive: now, status: "On holiday")), "On holiday")
        XCTAssertEqual(line(Make.presence(.unavailable, status: "")), "Idle")
        // Offline: when last active.
        XCTAssertEqual(line(Make.presence(.offline, lastActive: now.addingTimeInterval(-30))), "Last active just now")
        XCTAssertEqual(line(Make.presence(.offline, lastActive: now.addingTimeInterval(5))), "Last active just now")
        let formatter = RelativeDateTimeFormatter()
        formatter.unitsStyle = .abbreviated
        formatter.locale = Locale(identifier: "en_US")
        let twoHours = now.addingTimeInterval(-2 * 3600)
        let ago = line(Make.presence(.offline, lastActive: twoHours))
        XCTAssertEqual(ago, "Last active \(formatter.localizedString(for: twoHours, relativeTo: now))")
        XCTAssertTrue(ago?.contains("2") == true, ago ?? "nil")
        // Nothing to say.
        XCTAssertNil(line(Make.presence(.offline)))
        XCTAssertNil(line(nil))
    }
}

/// Reading others' presence and setting ours, through the session model.
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
    private func start(sharing: Bool? = true, idleTime: TimeInterval = PresenceModel.idleTime) async {
        if let sharing {
            defaults.set(sharing, forKey: "sharePresence")
        }
        model = SessionModel(session: session, defaults: defaults, idleTime: idleTime)
        model.start()
        await eventually { self.session.roomListListener != nil }
    }

    private static let bob = "@bob:example.org"
    private static let carol = "@carol:example.org"
    private static let dan = "@dan:example.org"

    /// The integration test's rooms, with the Notes to self.
    private func rooms(loaded: Bool = true, extra: [RoomSummary] = []) -> RoomListState {
        var state = Make.roomsBySpace(loaded: loaded)
        state.rooms += [Make.room("!notes", kind: .direct, name: "Notes", directUser: "@me:example.org")] + extra
        return state
    }

    private func push(_ state: RoomListState) async {
        session.roomListListener?.onUpdate(state: state)
        await eventually { self.model.rooms == state.rooms && self.model.loaded == state.loaded }
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
        var changed = rooms()
        changed.rooms[3].unreadMessages = 5
        changed.rooms.removeAll { $0.id == "!lab" }
        await push(changed)
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

    func testSignOutEndsTheSubscriptionAndGoesOfflineFirst() async throws {
        await start()
        model.setForeground(true)
        await push(rooms())
        await eventually { self.session.calls.contains("startSync") }
        try await model.logout()
        XCTAssertEqual(session.presenceSubscriptions[0].handle.cancelled, true)
        XCTAssertEqual(session.calls.suffix(2), ["presence offline", "logout"])
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
        let set = session.presencesSet
        await push(rooms(extra: [Make.room("!dan", kind: .direct, name: "Dan", directUser: Self.dan)]))
        model.setForeground(false)
        model.setForeground(true)
        presence.setSharing(false)
        await settle()
        XCTAssertEqual(session.presenceSubscriptions.count, 1, file: file, line: line)
        XCTAssertEqual(session.presencesSet, set, file: file, line: line)
    }

    // MARK: Ours

    func testForegroundOnlineAndBackgroundOfflineBeforeSyncStops() async {
        await start()
        model.setForeground(true)
        await eventually { self.session.calls.count == 2 }
        XCTAssertEqual(Set(session.calls), ["presence online", "startSync"])

        model.setForeground(false)
        await eventually { self.session.calls.count == 4 }
        XCTAssertEqual(session.calls.suffix(2), ["presence offline", "stopSync"])

        model.setForeground(true)
        await eventually { self.session.calls.count == 6 }
        XCTAssertEqual(Set(session.calls.suffix(2)), ["presence online", "startSync"])
    }

    func testNotSharingGoesOfflineOnceBeforeSyncFirstStarts() async {
        await start(sharing: false)
        model.setForeground(true)
        await eventually { self.session.calls.count == 2 }
        XCTAssertEqual(session.calls, ["presence offline", "startSync"])

        model.setForeground(false)
        model.setForeground(true)
        await eventually { self.session.syncStarts == 2 }
        await settle()
        XCTAssertEqual(session.calls, ["presence offline", "startSync", "stopSync", "startSync"])
    }

    func testSharingIsOnByDefault() async {
        await start(sharing: nil)
        XCTAssertTrue(presence.sharing)
        model.setForeground(true)
        await eventually { self.session.presencesSet == ["online"] }
    }

    func testTheSwitch() async {
        await start()
        model.setForeground(true)
        await eventually { self.session.presencesSet == ["online"] }
        presence.setSharing(false)
        await eventually { self.session.presencesSet == ["online", "offline"] }
        XCTAssertEqual(defaults.object(forKey: "sharePresence") as? Bool, false)
        // Off: no lifecycle calls.
        model.setForeground(false)
        model.setForeground(true)
        await eventually { self.session.syncStarts == 2 }
        presence.input()
        await settle()
        XCTAssertEqual(session.presencesSet, ["online", "offline"])

        presence.setSharing(true)
        await eventually { self.session.presencesSet == ["online", "offline", "online"] }
        XCTAssertEqual(defaults.object(forKey: "sharePresence") as? Bool, true)

        // In the background, turning it on sets nothing until the
        // foreground.
        presence.setSharing(false)
        model.setForeground(false)
        presence.setSharing(true)
        await eventually { self.session.syncStops == 2 }
        await settle()
        XCTAssertEqual(session.presencesSet, ["online", "offline", "online", "offline"])
        model.setForeground(true)
        await eventually { self.session.presencesSet.last == "online" && self.session.presencesSet.count == 5 }
    }

    func testIdleAfterNoInputThenOnlineAgain() async {
        await start(idleTime: 0.2)
        model.setForeground(true)
        await eventually { self.session.presencesSet == ["online"] }
        // Input keeps us online, without calls.
        for _ in 0..<3 {
            try? await Task.sleep(nanoseconds: 100_000_000)
            presence.input()
        }
        XCTAssertEqual(session.presencesSet, ["online"])
        await eventually { self.session.presencesSet == ["online", "unavailable"] }
        XCTAssertEqual(presence.own, .unavailable)

        presence.input()
        presence.input()
        await eventually { self.session.presencesSet == ["online", "unavailable", "online"] }
        await eventually { self.session.presencesSet == ["online", "unavailable", "online", "unavailable"] }

        // Not in the background.
        model.setForeground(false)
        presence.input()
        await eventually { self.session.syncStops == 1 }
        try? await Task.sleep(nanoseconds: 300_000_000)
        XCTAssertEqual(session.presencesSet, ["online", "unavailable", "online", "unavailable", "offline"])
    }

    func testFailuresToSetOursAreIgnored() async {
        await start()
        session.setPresenceError = MtrxError.Other(reason: "No connection")
        model.setForeground(true)
        await eventually { self.session.syncStarts == 1 }
        await settle()
        XCTAssertTrue(presence.available)
        XCTAssertNil(model.error)
    }

    /// Let tasks started on the main actor run.
    private func settle() async {
        try? await Task.sleep(nanoseconds: 50_000_000)
    }
}
