import Foundation
import MtrxFFI
import UIKit

/// The other person of a direct chat, in their info sheet: their name and
/// picture (the chat's at once, then their profile's where set), copying
/// their ID, and inviting them to one of our spaces. The calls run in tasks
/// the model owns, so closing the sheet does not stop an invite.
@MainActor
final class PersonInfoModel: ObservableObject, Identifiable {
    /// How long "Copied" shows after copying the ID (nanoseconds).
    static let copiedTime: UInt64 = 2_000_000_000
    /// How long "Invited to …" shows after an invite (nanoseconds).
    static let confirmationTime: UInt64 = 3_000_000_000

    let userId: String
    @Published private(set) var name: String
    /// `mxc://` URI.
    @Published private(set) var avatarUrl: String?
    /// The ID was just copied.
    @Published private(set) var copied = false
    /// The list of spaces to invite to is open.
    @Published var choosingSpace = false
    /// The space an invite is being sent to.
    @Published private(set) var inviting: String?
    /// "Invited to <space>.", for a moment after an invite.
    @Published private(set) var confirmation: String?
    /// Why the last invite failed.
    @Published var inviteError: String?

    private let session: Session
    private let pasteboard: @MainActor (String) -> Void
    private var profile: Task<Void, Never>?
    private var copiedReset: Task<Void, Never>?
    private var confirmationReset: Task<Void, Never>?

    /// `room`: the direct chat, whose name and picture show until the
    /// profile's arrive.
    init(
        session: Session, userId: String, room: RoomSummary,
        pasteboard: @escaping @MainActor (String) -> Void = { UIPasteboard.general.string = $0 }
    ) {
        self.session = session
        self.userId = userId
        self.pasteboard = pasteboard
        name = room.name
        avatarUrl = room.avatarUrl
    }

    /// Fetch the profile: its name and picture replace the chat's where
    /// set. A failure keeps the chat's, without a message.
    func load() {
        guard profile == nil else { return }
        let (session, userId) = (self.session, self.userId)
        profile = Task {
            guard let profile = try? await session.userProfile(userId: userId) else { return }
            if let name = profile.name, !name.isEmpty {
                self.name = name
            }
            if let avatarUrl = profile.avatarUrl {
                self.avatarUrl = avatarUrl
            }
        }
    }

    func copyId() {
        pasteboard(userId)
        copied = true
        copiedReset?.cancel()
        copiedReset = Task {
            try? await Task.sleep(nanoseconds: Self.copiedTime)
            guard !Task.isCancelled else { return }
            copied = false
        }
    }

    /// Invite the person to `space` (one of the top-level spaces): on
    /// success the list closes and says so; on failure it stays, with the
    /// reason.
    func invite(to space: RoomSummary) {
        guard inviting == nil else { return }
        inviting = space.id
        inviteError = nil
        let (session, userId) = (self.session, self.userId)
        Task {
            do {
                try await session.room(roomId: space.id).invite(userId: userId)
                choosingSpace = false
                confirm("Invited to \(space.name).")
            } catch {
                inviteError = errorMessage(error)
            }
            inviting = nil
        }
    }

    private func confirm(_ text: String) {
        confirmation = text
        confirmationReset?.cancel()
        confirmationReset = Task {
            try? await Task.sleep(nanoseconds: Self.confirmationTime)
            guard !Task.isCancelled else { return }
            confirmation = nil
        }
    }
}
