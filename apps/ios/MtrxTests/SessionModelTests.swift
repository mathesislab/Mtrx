import MtrxFFI
import XCTest
@testable import Mtrx

@MainActor
final class SessionModelTests: XCTestCase {
    private var session: FakeSession!
    private var model: SessionModel!

    override func setUp() async throws {
        session = FakeSession(noHandle: .init())
        model = SessionModel(session: session)
        model.start()
        await eventually { self.session.roomListListener != nil }
    }

    func testDownloadsFollowThePushedListAndFilesFindTheirs() async {
        await eventually { self.session.downloadsListener != nil }
        let download = Download(
            id: 7, fileName: "melt.pdf", kind: .file, size: 2000, source: "source", roomId: "!a",
            roomName: "Ice", eventId: "$e", state: .downloading(received: 1000, total: 2000),
            fraction: 0.5, status: "1.0 of 2.0 kB")
        session.downloadsListener?.onUpdate(downloads: [download])
        await eventually { self.model.downloads.downloads == [download] }
        let media = Media(
            kind: .file, filename: "melt.pdf", size: 2000, mimetype: nil, width: nil, height: nil,
            source: "source")
        XCTAssertEqual(model.downloads.download(of: media)?.id, 7)
        XCTAssertFalse(download.stopped)
        var other = media
        other.source = "other"
        XCTAssertNil(model.downloads.download(of: other))
    }

    func testBannerFollowsThePushedState() async {
        // The subscription's first state: no sync yet.
        await eventually { self.model.encryption == EncryptionState(deviceVerified: false, recovery: .unknown) }
        XCTAssertFalse(model.needsEncryptionSetup)
        let cases: [(EncryptionState, Bool)] = [
            (EncryptionState(deviceVerified: false, recovery: .incomplete), true),
            (EncryptionState(deviceVerified: false, recovery: .disabled), true),
            (EncryptionState(deviceVerified: true, recovery: .incomplete), true),
            (EncryptionState(deviceVerified: true, recovery: .enabled), false),
            (EncryptionState(deviceVerified: true, recovery: .disabled), false),
            // Not before sync has run.
            (EncryptionState(deviceVerified: false, recovery: .unknown), false),
        ]
        for (state, banner) in cases {
            session.encryptionListener?.onUpdate(state: state)
            await eventually { self.model.encryption == state }
            XCTAssertEqual(model.needsEncryptionSetup, banner, "\(state)")
        }
        XCTAssertEqual(session.encryptionRequests, 0)
    }

    func testTheBannerGoesAwayByItselfAfterRecovery() async {
        session.encryptionListener?.onUpdate(state: EncryptionState(deviceVerified: false, recovery: .incomplete))
        await eventually { self.model.needsEncryptionSetup }

        model.recovery.key = "EsTc abcd"
        model.recovery.recover()
        await eventually { self.model.recovery.recovered }
        // Parts of the state settle after `recover` returns: as pushed.
        session.encryptionListener?.onUpdate(
            state: EncryptionState(deviceVerified: true, recovery: .incomplete, keysArriving: true))
        await eventually { self.model.encryption?.keysArriving == true }
        XCTAssertTrue(model.needsEncryptionSetup)
        session.encryptionListener?.onUpdate(
            state: EncryptionState(deviceVerified: true, recovery: .enabled, keysArriving: true))
        await eventually { !self.model.needsEncryptionSetup }
        XCTAssertEqual(session.encryptionRequests, 0)
    }

    func testSyncAndTheRoomListDoNotAskForTheEncryptionState() async {
        model.setForeground(true)
        await eventually { self.session.syncStarts == 1 }
        session.roomListListener?.onUpdate(lists: Make.lists([Make.room("!a")]))
        await eventually { self.model.rooms.count == 1 }
        model.setForeground(false)
        model.setForeground(true)
        await eventually { self.session.syncStops == 1 && self.session.syncStarts == 2 }
        await settle()
        XCTAssertEqual(session.encryptionRequests, 0)
    }

    func testSignOutEndsTheEncryptionSubscription() async throws {
        XCTAssertEqual(session.encryptionSubscription?.cancelled, false)
        try await model.logout()
        XCTAssertEqual(session.encryptionSubscription?.cancelled, true)
    }

    func testVerifiesWithAnotherDevice() async {
        model.verifyWithAnotherDevice()
        await eventually { self.model.verification != nil }
        XCTAssertFalse(model.requestingVerification)
        XCTAssertEqual(model.verification?.incoming, false)
        XCTAssertEqual(model.verification?.asking, false)

        session.selfVerification.listener?.onUpdate(state: .done)
        await eventually { self.model.verification?.state == .done }
        // The encryption state that follows is pushed, not asked.
        XCTAssertEqual(session.encryptionRequests, 0)

        model.closeVerification()
        XCTAssertNil(model.verification)
        XCTAssertEqual(session.selfVerification.subscription?.cancelled, true)
    }

    func testKnowsWhetherAnotherDeviceCanVerify() async {
        session.deviceList = [DeviceInfo(id: "DEVICE", name: "iPhone", verified: false, current: true)]
        await model.loadDevices()
        XCTAssertEqual(model.hasOtherDevice, false)
        session.deviceList.append(DeviceInfo(id: "OTHER", name: "Mtrx desktop", verified: true, current: false))
        await model.loadDevices()
        XCTAssertEqual(model.hasOtherDevice, true)
    }

    func testAsksAboutARequestFromOurOtherDevice() async {
        let request = FakeVerification(noHandle: .init())
        session.verificationRequestListener?.onRequest(verification: request)
        await eventually { self.model.verification != nil }
        XCTAssertEqual(model.verification?.incoming, true)
        XCTAssertEqual(model.verification?.asking, true)
        XCTAssertEqual(request.cancelled, 0)
    }

    func testCancelsARequestFromAnotherPerson() async {
        let request = FakeVerification(noHandle: .init())
        request.user = "@bob:example.org"
        session.verificationRequestListener?.onRequest(verification: request)
        await eventually { request.cancelled == 1 }
        XCTAssertNil(model.verification)
    }

    func testANewRequestReplacesAnUnansweredOne() async {
        let first = FakeVerification(noHandle: .init())
        session.verificationRequestListener?.onRequest(verification: first)
        await eventually { self.model.verification != nil }
        let shown = model.verification

        let second = FakeVerification(noHandle: .init())
        session.verificationRequestListener?.onRequest(verification: second)
        await eventually { self.model.verification !== shown }
        await eventually { first.cancelled == 1 }
        XCTAssertEqual(second.cancelled, 0)
    }

    func testARequestDuringAVerificationIsCancelled() async {
        model.verifyWithAnotherDevice()
        await eventually { self.model.verification != nil }
        let shown = model.verification

        let request = FakeVerification(noHandle: .init())
        session.verificationRequestListener?.onRequest(verification: request)
        await eventually { request.cancelled == 1 }
        XCTAssertTrue(model.verification === shown)
    }

    /// Let tasks started on the main actor run.
    private func settle() async {
        try? await Task.sleep(nanoseconds: 50_000_000)
    }
}
