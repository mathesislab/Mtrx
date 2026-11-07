import Foundation
import MtrxFFI

/// A listener for the library's subscriptions, which it calls on its own
/// threads with the whole state, now and after every change. `Feed` passes
/// the states on as an `AsyncStream` that a main-actor task consumes, in
/// order; only the newest state not consumed yet is kept, since each one
/// replaces the last. Values that do not replace each other (recovery
/// steps, verification requests) are all kept (`keepingAll`).
///
/// The library holds the feed while the subscription runs; when it ends
/// (its `TaskHandle` is cancelled or dropped) the feed is released and the
/// stream finishes.
final class Feed<Value: Sendable>: @unchecked Sendable {
    let stream: AsyncStream<Value>
    private let continuation: AsyncStream<Value>.Continuation

    init(keepingAll: Bool = false) {
        (stream, continuation) = AsyncStream.makeStream(
            of: Value.self, bufferingPolicy: keepingAll ? .unbounded : .bufferingNewest(1))
    }

    deinit {
        continuation.finish()
    }

    func send(_ value: Value) {
        continuation.yield(value)
    }

    /// End the stream after the values sent so far, e.g. when the call that
    /// reports to the feed has returned.
    func finish() {
        continuation.finish()
    }
}

extension Feed: RoomListsListener where Value == RoomLists {
    func onUpdate(lists: RoomLists) { send(lists) }
}

extension Feed: ConnectionListener where Value == ConnectionState {
    func onUpdate(state: ConnectionState) { send(state) }
}

extension Feed: TimelineRowsListener where Value == [TimelineRow] {
    func onUpdate(rows: [TimelineRow]) { send(rows) }
}

extension Feed: TypingListener where Value == [String] {
    func onUpdate(names: [String]) { send(names) }
}

extension Feed: RoomSettingsListener where Value == RoomSettings {
    func onUpdate(settings: RoomSettings) { send(settings) }
}

extension Feed: EncryptionStateListener where Value == EncryptionState {
    func onUpdate(state: EncryptionState) { send(state) }
}

extension Feed: RecoveryListener where Value == RecoveryStep {
    func onUpdate(step: RecoveryStep) { send(step) }
}

extension Feed: VerificationListener where Value == VerificationState {
    func onUpdate(state: VerificationState) { send(state) }
}

extension Feed: VerificationRequestListener where Value == Verification {
    func onRequest(verification: Verification) { send(verification) }
}

extension Feed: DownloadsListener where Value == [Download] {
    func onUpdate(downloads: [Download]) { send(downloads) }
}
