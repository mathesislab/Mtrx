import Network

/// Says when the device's network may be back, while started: each time a
/// usable path is reported (also the network as it is at the start).
@MainActor
final class NetworkMonitor {
    private let back: @MainActor () -> Void
    private var monitor: NWPathMonitor?

    init(back: @escaping @MainActor () -> Void) {
        self.back = back
    }

    func start() {
        guard monitor == nil else { return }
        let monitor = NWPathMonitor()
        monitor.pathUpdateHandler = { [weak self, weak monitor] path in
            MainActor.assumeIsolated {
                // Not one queued before a stop.
                guard let self, let monitor, self.monitor === monitor else { return }
                self.pathChanged(usable: path.status == .satisfied)
            }
        }
        self.monitor = monitor
        monitor.start(queue: .main)
    }

    func stop() {
        monitor?.cancel()
        monitor = nil
    }

    /// A path was reported, `usable` or not.
    func pathChanged(usable: Bool) {
        if usable {
            back()
        }
    }

    deinit {
        monitor?.cancel()
    }
}
