import Foundation
import MtrxFFI

/// A model's work, of two kinds (as on Android). Loads and subscriptions
/// end with the model: they are cancelled when it goes away, or by
/// `cancelAll`. Requests (sending, reacting, changing a setting) run to
/// their end even if the model goes, and apply their result to it.
///
/// A load's body should hold the model weakly, so that the model can go
/// away while it runs; a request's holds it until it ends.
@MainActor
final class Work {
    private var handles: [TaskHandle] = []
    private var following: [Task<Void, Never>] = []
    private var loads: [String: Task<Void, Never>] = [:]

    /// Keep `handle` running and apply each of `feed`'s states.
    func follow<Value>(_ handle: TaskHandle, _ feed: Feed<Value>, _ apply: @escaping @MainActor (Value) -> Void) {
        handles.append(handle)
        let stream = feed.stream
        following.append(Task { @MainActor in
            for await value in stream {
                apply(value)
            }
        })
    }

    /// A load that ends with the model; replaces one under `key`. Returns
    /// its task, to wait for.
    @discardableResult
    func load(_ key: String, _ body: @escaping @MainActor () async -> Void) -> Task<Void, Never> {
        loads[key]?.cancel()
        let task = Task { @MainActor in await body() }
        loads[key] = task
        return task
    }

    /// A request that finishes even if the model goes; a failure goes to
    /// `failed`, as the text to show (none if the user cancelled).
    @discardableResult
    func request(
        _ body: @escaping @MainActor () async throws -> Void,
        failed: @escaping @MainActor (String) -> Void
    ) -> Task<Void, Never> {
        Task { @MainActor in
            do {
                try await body()
            } catch {
                if let message = errorMessage(error) {
                    failed(message)
                }
            }
        }
    }

    /// End the subscriptions and loads; requests go on.
    func cancelAll() {
        handles.forEach { $0.cancel() }
        following.forEach { $0.cancel() }
        loads.values.forEach { $0.cancel() }
        handles = []
        following = []
        loads = [:]
    }

    deinit {
        // Not isolated: only the handles' and tasks' own (thread-safe)
        // cancellation is used.
        for handle in handles { handle.cancel() }
        for task in following { task.cancel() }
        for task in loads.values { task.cancel() }
    }
}
