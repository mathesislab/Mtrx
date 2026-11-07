import MtrxFFI
import XCTest
@testable import Mtrx

/// A model's work: loads and subscriptions end with it, requests run to
/// their end and apply their result to it.
@MainActor
final class WorkTests: XCTestCase {
    /// Waits until cancelled (or ten seconds); says which.
    private static func waitLong() async -> String {
        do {
            try await Task.sleep(nanoseconds: 10_000_000_000)
            return "finished"
        } catch {
            return "cancelled"
        }
    }

    func testALoadReplacesTheOneUnderItsKey() async {
        let work = Work()
        var ends: [String] = []
        let first = work.load("rows") {
            let end = await Self.waitLong()
            ends.append("first " + end)
        }
        let second = work.load("rows") { ends.append("second") }
        let other = work.load("members") { ends.append("other") }
        await first.value
        await second.value
        await other.value
        XCTAssertEqual(Set(ends), ["first cancelled", "second", "other"])
    }

    func testLoadsAndSubscriptionsEndWithTheWork() async {
        var work: Work? = Work()
        var ends: [String] = []
        let load = work?.load("rows") {
            let end = await Self.waitLong()
            ends.append(end)
        }
        let handle = FakeTaskHandle(noHandle: .init())
        work?.follow(handle, Feed<Int>()) { _ in }
        work = nil
        await load?.value
        XCTAssertEqual(ends, ["cancelled"])
        XCTAssertTrue(handle.cancelled)
    }

    func testCancelAllEndsLoadsAndSubscriptionsButNotRequests() async {
        let work = Work()
        var ends: [String] = []
        let load = work.load("rows") {
            let end = await Self.waitLong()
            ends.append("load " + end)
        }
        let request = work.request({
            try await Task.sleep(nanoseconds: 50_000_000)
            ends.append("request")
        }, failed: { XCTFail($0) })
        let handle = FakeTaskHandle(noHandle: .init())
        work.follow(handle, Feed<Int>()) { _ in }
        work.cancelAll()
        await load.value
        await request.value
        XCTAssertEqual(Set(ends), ["load cancelled", "request"])
        XCTAssertTrue(handle.cancelled)
    }

    /// The model is held until its request ends, which applies to it.
    func testARequestAppliesItsResultAfterItsOwnerLetGo() async {
        let applied = Applied()
        var owner: Owner? = Owner(applied)
        weak var held = owner
        let request = owner?.save()
        owner = nil
        XCTAssertNotNil(held, "gone before its request ended")
        await request?.value
        XCTAssertEqual(applied.results, ["saved"])
        await eventually { held == nil }
    }

    func testAFailedRequestSaysWhy() async {
        let work = Work()
        var failures: [String] = []
        await work.request({ throw MtrxError.Other(reason: "Not allowed") }, failed: { failures.append($0) }).value
        XCTAssertEqual(failures, ["Not allowed"])
        // Cancelled: nothing to say.
        await work.request({ throw CancellationError() }, failed: { failures.append($0) }).value
        XCTAssertEqual(failures, ["Not allowed"])
    }
}

/// What an owner applied, kept by the test.
@MainActor
private final class Applied {
    var results: [String] = []
}

/// A model with a request.
@MainActor
private final class Owner {
    private let work = Work()
    private let applied: Applied

    init(_ applied: Applied) {
        self.applied = applied
    }

    func save() -> Task<Void, Never> {
        work.request({
            try await Task.sleep(nanoseconds: 50_000_000)
            self.applied.results.append("saved")
        }, failed: { XCTFail($0) })
    }
}
