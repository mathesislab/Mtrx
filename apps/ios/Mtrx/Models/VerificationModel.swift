import Foundation
import MtrxFFI

/// An emoji verification of this device with another of ours, asked for
/// here (outgoing) or by the other device (incoming). Holds the
/// `Verification` and its subscription while the screen shows: the
/// protocol is followed only while the object lives. The encryption state
/// that follows is pushed to `EncryptionModel`.
@MainActor
final class VerificationModel: ObservableObject, Identifiable {
    let incoming: Bool
    @Published private(set) var state: VerificationState
    /// An incoming request not answered yet.
    @Published private(set) var asking: Bool
    /// An answer (accept, confirm, mismatch) is being sent.
    @Published private(set) var busy = false
    @Published private(set) var error: String?

    private let verification: Verification
    /// The subscription, and the answers sent.
    private let work = Work()

    init(verification: Verification, incoming: Bool) {
        self.verification = verification
        self.incoming = incoming
        asking = incoming
        state = verification.state()
        let feed = Feed<VerificationState>()
        work.follow(verification.subscribe(listener: feed), feed) { [weak self] in
            self?.state = $0
        }
    }

    /// Verified, or cancelled: nothing more to answer.
    var ended: Bool {
        switch state {
        case .done, .cancelled: return true
        default: return false
        }
    }

    func accept() {
        guard asking, !ended else { return }
        answer { verification in
            try await verification.accept()
            self.asking = false
        }
    }

    /// The emoji match.
    func confirm() {
        guard case .emojis = state else { return }
        answer { try await $0.confirm() }
    }

    /// The emoji differ: cancels, and tells the other device.
    func mismatch() {
        guard case .emojis = state else { return }
        answer { try await $0.mismatch() }
    }

    /// Stop (or decline a request), unless it has ended. Sent in the
    /// background: the screen may close at once.
    func cancel() {
        guard !ended else { return }
        let verification = self.verification
        Task {
            try? await verification.cancel()
        }
    }

    private func answer(_ send: @escaping @MainActor (Verification) async throws -> Void) {
        guard !busy else { return }
        busy = true
        error = nil
        let verification = self.verification
        work.request {
            defer { self.busy = false }
            try await send(verification)
        } failed: { message in
            self.error = message
        }
    }
}
