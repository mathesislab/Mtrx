import Combine
import MtrxFFI

/// Whether this device can read encrypted messages, and the ways to make it
/// so: the recovery key, or verifying with another of our devices (asked
/// for here or by the other device).
@MainActor
final class EncryptionModel: ObservableObject {
    /// Whether this device is verified, the account's recovery, and
    /// whether message keys are arriving, as the library pushes it; nil
    /// until its first update.
    @Published private(set) var state: EncryptionState?
    /// Whether another of our devices is signed in (to verify with); nil
    /// until known.
    @Published private(set) var hasOtherDevice: Bool?
    /// The verification on screen, asked for here or by another device.
    @Published private(set) var verification: VerificationModel?
    @Published private(set) var requestingVerification = false
    /// Why asking another device to verify failed.
    @Published var verificationError: String?
    let recovery: RecoveryModel

    private let session: Session
    private let userId: String
    private var verificationRequest: Task<Void, Never>?

    init(session: Session, userId: String) {
        self.session = session
        self.userId = userId
        recovery = RecoveryModel(session: session)
    }

    /// The library's state, as its subscription pushes it.
    func apply(_ state: EncryptionState) {
        self.state = state
    }

    /// The banner shows: encrypted messages cannot all be read until the
    /// device is verified or recovery unlocked. Not before the first sync
    /// (recovery unknown).
    var needsSetup: Bool {
        guard let state, state.recovery != .unknown else { return false }
        return !state.deviceVerified || state.recovery == .incomplete
    }

    /// Learn whether another device can verify this one.
    func loadDevices() async {
        guard let devices = try? await session.devices() else { return }
        hasOtherDevice = devices.contains { !$0.current }
    }

    /// Ask our other devices to verify this one, and show the verification.
    func verifyWithAnotherDevice() {
        guard !requestingVerification, verification == nil else { return }
        requestingVerification = true
        verificationError = nil
        let session = self.session
        verificationRequest = Task {
            do {
                let verification = try await session.requestSelfVerification()
                show(verification, incoming: false)
            } catch {
                verificationError = errorMessage(error)
            }
            requestingVerification = false
            verificationRequest = nil
        }
    }

    /// The verification screen closes (or the account signs out): the
    /// verification is dropped.
    func closeVerification() {
        verification = nil
    }

    /// Another device asks to verify. Only our own devices are verified
    /// here: requests from other people are cancelled. One that comes while
    /// an earlier request is unanswered replaces it; while a verification
    /// runs, it is cancelled.
    func received(_ request: Verification) {
        let busy = verification.map { !$0.ended && !$0.asking } ?? false
        guard request.otherUser() == userId, !busy else {
            Task { try? await request.cancel() }
            return
        }
        verification?.cancel()
        show(request, incoming: true)
    }

    private func show(_ verification: Verification, incoming: Bool) {
        self.verification = VerificationModel(verification: verification, incoming: incoming)
    }
}
