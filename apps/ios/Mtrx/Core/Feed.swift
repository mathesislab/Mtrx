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

extension Feed: RoomListListener where Value == RoomListState {
    func onUpdate(state: RoomListState) { send(state) }
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

/// A model's subscriptions and the tasks consuming them, cancelled together
/// (also when the model goes away).
@MainActor
final class Subscriptions {
    private var handles: [TaskHandle] = []
    private var tasks: [Task<Void, Never>] = []

    /// Keep `handle` running and apply each of `feed`'s states with `apply`.
    func add<Value>(_ handle: TaskHandle, _ feed: Feed<Value>, _ apply: @escaping @MainActor (Value) -> Void) {
        handles.append(handle)
        let stream = feed.stream
        tasks.append(Task { @MainActor in
            for await value in stream {
                apply(value)
            }
        })
    }

    func add(_ task: Task<Void, Never>) {
        tasks.append(task)
    }

    func cancelAll() {
        handles.forEach { $0.cancel() }
        tasks.forEach { $0.cancel() }
        handles = []
        tasks = []
    }

    deinit {
        // Not isolated: only the handles' and tasks' own (thread-safe)
        // cancellation is used.
        for handle in handles { handle.cancel() }
        for task in tasks { task.cancel() }
    }
}
