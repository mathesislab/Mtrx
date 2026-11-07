import MtrxFFI
import SwiftUI

/// An invite to a room, space or direct chat: Accept joins it and opens it,
/// Decline leaves it.
struct InviteView: View {
    @EnvironmentObject private var session: SessionModel
    @EnvironmentObject private var home: HomeNavigation
    let room: RoomSummary
    @State private var answering: Answer?
    @State private var error: String?

    private enum Answer {
        case accept
        case decline
    }

    private var what: String {
        switch room.kind {
        case .space: return "You're invited to the space \(room.name)."
        case .direct: return "\(room.name) wants to chat with you."
        case .room: return "You're invited to \(room.name)."
        }
    }

    var body: some View {
        // Scrolls where it does not fit (large text).
        ViewThatFits(in: .vertical) {
            content
            ScrollView { content }
        }
        .navigationBarTitleDisplayMode(.inline)
    }

    private var content: some View {
        VStack(spacing: 16) {
            Spacer()
            AvatarView(name: room.name, uri: room.avatarUrl, size: 72, shape: room.kind == .space ? .square : .circle)
            Text(room.name)
                .font(.title2.weight(.semibold))
                .multilineTextAlignment(.center)
            Text(what)
                .foregroundStyle(.secondary)
                .multilineTextAlignment(.center)
            if let topic = room.topic.firstLine {
                Text(topic)
                    .font(.callout)
                    .foregroundStyle(.secondary)
                    .multilineTextAlignment(.center)
                    .lineLimit(3)
            }
            if let error {
                Text(error)
                    .font(.footnote)
                    .foregroundColor(.red)
                    .multilineTextAlignment(.center)
            }
            AdaptiveStack(spacing: 12) {
                Button {
                    answer(.decline)
                } label: {
                    label("Decline", busy: answering == .decline)
                }
                .buttonStyle(.bordered)
                .tint(.secondary)
                Button {
                    answer(.accept)
                } label: {
                    label("Accept", busy: answering == .accept)
                }
                .buttonStyle(.borderedProminent)
            }
            .disabled(answering != nil)
            .padding(.top, 8)
            Spacer()
            Spacer()
        }
        .padding(24)
        .frame(maxWidth: .infinity)
    }

    private func label(_ title: String, busy: Bool) -> some View {
        ZStack {
            Text(title).opacity(busy ? 0 : 1)
            if busy {
                ProgressView()
            }
        }
        .frame(minWidth: 100)
    }

    private func answer(_ answer: Answer) {
        answering = answer
        error = nil
        let done: @MainActor () -> Void = {
            // A sheet closes; pushed, a declined room leaves its path.
            home.invite = nil
            answering = nil
        }
        let failed: @MainActor (String) -> Void = { message in
            error = message
            answering = nil
        }
        switch answer {
        case .accept: session.accept(room, done: done, failed: failed)
        case .decline: session.decline(room, done: done, failed: failed)
        }
    }
}
