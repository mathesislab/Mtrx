import MtrxFFI
import XCTest
@testable import Mtrx

/// Pausing notifications and quiet hours, as the library gives them.
@MainActor
final class QuietModelTests: XCTestCase {
    private var session: FakeSession!
    private var model: QuietModel!

    override func setUp() async throws {
        session = FakeSession(noHandle: .init())
        model = QuietModel(session: session)
        model.start()
        await eventually { self.model.state != nil }
    }

    func testFollowsTheLibrary() async {
        XCTAssertFalse(model.quiet)
        XCTAssertEqual(model.state?.line, "Notifications on")
        XCTAssertEqual(model.pauseMenu().map(\.title).first, "For 1 hour")
    }

    func testPausesAndResumes() async {
        await model.pause(.hour).value
        await eventually { self.model.quiet }
        await model.resume().value
        await eventually { !self.model.quiet }
        XCTAssertEqual(session.changes, ["pause hour", "resume"])
    }

    func testSetsQuietHoursAndMentions() async {
        var hours = model.state!.hours
        hours.on = true
        hours.fromMinute = 23 * 60
        await model.setHours(hours).value
        await model.setMentionsThrough(true).value
        await eventually { self.model.state?.mentionsThrough == true }
        XCTAssertEqual(model.state?.hours.fromMinute, 23 * 60)
        XCTAssertEqual(session.changes, ["hours 1380-420", "mentions through true"])
    }

    func testAFailedChangeSaysWhy() async {
        session.changeError = MtrxError.Other(reason: "Offline")
        await model.resume().value
        XCTAssertEqual(model.error, "Offline")
    }

    func testMinutesAsTimesOfDay() {
        XCTAssertEqual(QuietHours.minute(QuietHours.date(22 * 60 + 30)), 22 * 60 + 30)
    }
}
