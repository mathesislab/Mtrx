import MtrxFFI
import XCTest
@testable import Mtrx

/// The forms kept in the preferences; the lists are the library's.
final class RoomListsTests: XCTestCase {
    func testTheSavedSpaceRoundTrips() {
        for key in [SpaceKey.space(id: "!team:example.org"), .outside] {
            XCTAssertEqual(SpaceKey(saved: key.saved), key)
        }
        // From when Home was called Other rooms.
        XCTAssertEqual(SpaceKey(saved: "other-rooms"), .outside)
    }

    func testTheSavedSortRoundTrips() {
        for sort in RoomSort.allCases {
            XCTAssertEqual(RoomSort(saved: sort.saved), sort)
        }
        XCTAssertEqual(RoomSort(saved: "default"), .listOrder)
    }
}
