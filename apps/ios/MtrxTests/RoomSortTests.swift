import MtrxFFI
import XCTest
@testable import Mtrx

/// The order chosen is remembered across launches, for every space.
@MainActor
final class RememberedSortTests: XCTestCase {
    private var defaults: UserDefaults!

    override func setUp() async throws {
        defaults = UserDefaults(suiteName: "RememberedSortTests")
        defaults.removePersistentDomain(forName: "RememberedSortTests")
    }

    override func tearDown() async throws {
        defaults.removePersistentDomain(forName: "RememberedSortTests")
    }

    private func launch() -> SessionModel {
        SessionModel(session: FakeSession(noHandle: .init()), defaults: defaults)
    }

    func testDefaultUntilChosen() {
        XCTAssertEqual(launch().home.roomSort, .listOrder)
    }

    func testTheChoiceIsRemembered() {
        let first = launch()
        first.home.setRoomSort(.mostRecent)
        XCTAssertEqual(first.home.roomSort, .mostRecent)
        XCTAssertEqual(launch().home.roomSort, .mostRecent)
        first.home.setRoomSort(.zToA)
        XCTAssertEqual(launch().home.roomSort, .zToA)
    }

    func testAnUnknownSavedValueIsTheDefault() {
        defaults.set("by-size", forKey: "roomSort")
        XCTAssertEqual(launch().home.roomSort, .listOrder)
    }
}
