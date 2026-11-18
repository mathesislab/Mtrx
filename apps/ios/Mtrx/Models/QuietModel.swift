import Foundation
import MtrxFFI

/// Pausing notifications and quiet hours, on all our devices with Mtrx, as
/// the library pushes them. This iPhone shows no notifications yet: here
/// they are set for the other devices. Its own object, so a change redraws
/// only the views that show it.
@MainActor
final class QuietModel: ObservableObject {
    /// Nil until the library said.
    @Published private(set) var state: QuietState?
    /// Why the last change failed.
    @Published var error: String?

    private let session: Session
    /// The subscription, and the changes.
    private let work = Work()

    init(session: Session) {
        self.session = session
    }

    func start() {
        let feed = Feed<QuietState>()
        work.follow(session.subscribeQuiet(listener: feed), feed) { [weak self] in
            self?.state = $0
        }
    }

    /// Paused or in quiet hours.
    var quiet: Bool { state?.quiet ?? false }

    /// The pause menu, its titles as of now.
    func pauseMenu() -> [PauseMenuItem] {
        session.pauseMenu()
    }

    @discardableResult
    func pause(_ pause: PauseFor) -> Task<Void, Never> {
        change { try await $0.pauseNotifications(pause: pause) }
    }

    @discardableResult
    func resume() -> Task<Void, Never> {
        change { try await $0.resumeNotifications() }
    }

    @discardableResult
    func setHours(_ hours: QuietHours) -> Task<Void, Never> {
        change { try await $0.setQuietHours(hours: hours) }
    }

    @discardableResult
    func setMentionsThrough(_ on: Bool) -> Task<Void, Never> {
        change { try await $0.setMentionsThrough(on: on) }
    }

    /// A change, shown at once by the library; a failure says why.
    private func change(_ call: @escaping (Session) async throws -> Void) -> Task<Void, Never> {
        error = nil
        let session = self.session
        return work.request {
            try await call(session)
        } failed: { [weak self] message in
            self?.error = message
        }
    }
}

extension QuietHours {
    /// `minute` after midnight, as a time today, for a time picker.
    static func date(_ minute: UInt16) -> Date {
        let calendar = Calendar.current
        let start = calendar.startOfDay(for: Date())
        return calendar.date(byAdding: .minute, value: Int(minute), to: start) ?? start
    }

    /// The minutes after midnight of `date`'s time.
    static func minute(_ date: Date) -> UInt16 {
        let parts = Calendar.current.dateComponents([.hour, .minute], from: date)
        return UInt16((parts.hour ?? 0) * 60 + (parts.minute ?? 0))
    }
}
