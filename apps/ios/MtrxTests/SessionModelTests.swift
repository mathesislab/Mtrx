import Combine
import MtrxFFI
import XCTest
@testable import Mtrx

@MainActor
final class SessionModelTests: XCTestCase {
    private var defaults: UserDefaults!
    private var session: FakeSession!
    private var model: SessionModel!
    /// Fed by the session's subscriptions.
    private var encryption: EncryptionModel { model.encryption }

    override func setUp() async throws {
        defaults = UserDefaults(suiteName: "SessionModelTests")
        defaults.removePersistentDomain(forName: "SessionModelTests")
        session = FakeSession(noHandle: .init())
        model = SessionModel(session: session, defaults: defaults)
        model.start()
        await eventually { self.session.roomListListener != nil }
    }

    override func tearDown() async throws {
        defaults.removePersistentDomain(forName: "SessionModelTests")
    }

    /// The account, its navigation and its encryption are models of their
    /// own: a room-list update changes only the room lists, so only the
    /// views that show rooms are drawn again.
    func testARoomListUpdateChangesOnlyTheRoomLists() async {
        // The encryption subscription's first state.
        await eventually { self.encryption.state != nil }
        var changed: Set<String> = []
        let watches: [AnyCancellable] = [
            model.objectWillChange.sink { _ in _ = changed.insert("session") },
            model.rooms.objectWillChange.sink { _ in _ = changed.insert("rooms") },
            model.home.objectWillChange.sink { _ in _ = changed.insert("home") },
            model.encryption.objectWillChange.sink { _ in _ = changed.insert("encryption") },
        ]
        let lists = Make.roomsBySpace()
        session.roomListListener?.onUpdate(lists: lists)
        await eventually { self.model.rooms.lists === lists }
        XCTAssertEqual(changed, ["rooms"])
        watches.forEach { $0.cancel() }
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
        await eventually { self.encryption.state == EncryptionState(deviceVerified: false, recovery: .unknown) }
        XCTAssertFalse(encryption.needsSetup)
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
            await eventually { self.encryption.state == state }
            XCTAssertEqual(encryption.needsSetup, banner, "\(state)")
        }
    }

    func testTheBannerGoesAwayByItselfAfterRecovery() async {
        session.encryptionListener?.onUpdate(state: EncryptionState(deviceVerified: false, recovery: .incomplete))
        await eventually { self.encryption.needsSetup }

        encryption.recovery.key = "EsTc abcd"
        encryption.recovery.recover()
        await eventually { self.encryption.recovery.recovered }
        // Parts of the state settle after `recover` returns: as pushed.
        session.encryptionListener?.onUpdate(
            state: EncryptionState(deviceVerified: true, recovery: .incomplete, keysArriving: true))
        await eventually { self.encryption.state?.keysArriving == true }
        XCTAssertTrue(encryption.needsSetup)
        session.encryptionListener?.onUpdate(
            state: EncryptionState(deviceVerified: true, recovery: .enabled, keysArriving: true))
        await eventually { !self.encryption.needsSetup }
    }

    func testSignOutEndsTheEncryptionSubscription() async throws {
        XCTAssertEqual(session.encryptionSubscription?.cancelled, false)
        try await model.logout()
        XCTAssertEqual(session.encryptionSubscription?.cancelled, true)
    }

    func testVerifiesWithAnotherDevice() async {
        encryption.verifyWithAnotherDevice()
        await eventually { self.encryption.verification != nil }
        XCTAssertFalse(encryption.requestingVerification)
        XCTAssertEqual(encryption.verification?.incoming, false)
        XCTAssertEqual(encryption.verification?.asking, false)

        session.selfVerification.listener?.onUpdate(state: .done)
        await eventually { self.encryption.verification?.state == .done }

        encryption.closeVerification()
        XCTAssertNil(encryption.verification)
        XCTAssertEqual(session.selfVerification.subscription?.cancelled, true)
    }

    func testKnowsWhetherAnotherDeviceCanVerify() async {
        session.deviceList = [DeviceInfo(id: "DEVICE", name: "iPhone", verified: false, current: true)]
        await encryption.loadDevices()
        XCTAssertEqual(encryption.hasOtherDevice, false)
        session.deviceList.append(DeviceInfo(id: "OTHER", name: "Mtrx desktop", verified: true, current: false))
        await encryption.loadDevices()
        XCTAssertEqual(encryption.hasOtherDevice, true)
    }

    func testAsksAboutARequestFromOurOtherDevice() async {
        let request = FakeVerification(noHandle: .init())
        session.verificationRequestListener?.onRequest(verification: request)
        await eventually { self.encryption.verification != nil }
        XCTAssertEqual(encryption.verification?.incoming, true)
        XCTAssertEqual(encryption.verification?.asking, true)
        XCTAssertEqual(request.cancelled, 0)
    }

    func testCancelsARequestFromAnotherPerson() async {
        let request = FakeVerification(noHandle: .init())
        request.user = "@bob:example.org"
        session.verificationRequestListener?.onRequest(verification: request)
        await eventually { request.cancelled == 1 }
        XCTAssertNil(encryption.verification)
    }

    func testANewRequestReplacesAnUnansweredOne() async {
        let first = FakeVerification(noHandle: .init())
        session.verificationRequestListener?.onRequest(verification: first)
        await eventually { self.encryption.verification != nil }
        let shown = encryption.verification

        let second = FakeVerification(noHandle: .init())
        session.verificationRequestListener?.onRequest(verification: second)
        await eventually { self.encryption.verification !== shown }
        await eventually { first.cancelled == 1 }
        XCTAssertEqual(second.cancelled, 0)
    }

    func testARequestDuringAVerificationIsCancelled() async {
        encryption.verifyWithAnotherDevice()
        await eventually { self.encryption.verification != nil }
        let shown = encryption.verification

        let request = FakeVerification(noHandle: .init())
        session.verificationRequestListener?.onRequest(verification: request)
        await eventually { request.cancelled == 1 }
        XCTAssertTrue(encryption.verification === shown)
    }

    /// Let tasks started on the main actor run.
    private func settle() async {
        try? await Task.sleep(nanoseconds: 50_000_000)
    }
}
