import MtrxFFI
import SwiftUI

/// A room in a list, compact: picture (with a direct chat's person's
/// presence), name, a line under it (for an invite, or as given), last
/// activity, a pencil if it has a draft, and unread count. At the
/// accessibility text sizes, the last three go under the name, which takes
/// up to three lines.
struct RoomRow: View {
    static let avatarSize: CGFloat = 34

    let room: RoomSummary
    var presence: Presence?
    /// Something was written in it and not sent.
    var draft = false
    /// The line under the name, unless it is an invite (Chats: the spaces
    /// shared with the person).
    var subline: String?
    @RowAvatarSize private var avatarSize: CGFloat
    @Environment(\.dynamicTypeSize) private var textSize

    private var unread: Bool { room.unreadMessages > 0 || room.unreadMentions > 0 }

    private var subtitle: String? {
        guard room.membership == .invited else { return subline }
        switch room.kind {
        case .space: return "Invited to the space"
        case .direct: return "Wants to chat"
        case .room: return "Invited"
        }
    }

    var body: some View {
        if textSize.isAccessibilitySize {
            VStack(alignment: .leading, spacing: 2) {
                HStack(spacing: 10) {
                    avatar
                    names
                }
                HStack(spacing: 6) {
                    lastActivity
                    Spacer(minLength: 8)
                    marks
                }
            }
        } else {
            HStack(spacing: 10) {
                avatar
                names
                Spacer(minLength: 8)
                VStack(alignment: .trailing, spacing: 2) {
                    lastActivity
                    marks
                }
            }
        }
    }

    private var avatar: some View {
        AvatarView(
            name: room.name, uri: room.avatarUrl, size: avatarSize, shape: room.kind == .space ? .square : .circle
        )
        .presenceDot(presence, size: 10, avatar: avatarSize)
    }

    private var names: some View {
        VStack(alignment: .leading, spacing: 0) {
            Text(room.name)
                .fontWeight(unread ? .semibold : .regular)
                .lineLimit(1, accessibility: 3)
            if let subtitle {
                Text(subtitle)
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .lineLimit(1, accessibility: 2)
            }
        }
    }

    @ViewBuilder
    private var lastActivity: some View {
        if let last = room.lastActivity {
            Text(Formatting.lastActivity(last))
                .font(.caption2)
                .foregroundStyle(unread ? Color.accentColor : .secondary)
        }
    }

    /// The draft's pencil and the unread count.
    private var marks: some View {
        HStack(spacing: 4) {
            if draft {
                Image(systemName: "pencil")
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .accessibilityLabel("Draft")
            }
            UnreadBadge(room: room)
        }
    }
}

/// Unread messages, in red when they mention the user.
struct UnreadBadge: View {
    let room: RoomSummary

    var body: some View {
        let mentions = room.unreadMentions > 0
        let count = mentions ? room.unreadMentions : room.unreadMessages
        if count > 0 {
            Text(count > 999 ? "999+" : String(count))
                .font(.caption2.weight(.semibold))
                .monospacedDigit()
                .foregroundColor(.white)
                .padding(.horizontal, 6)
                .padding(.vertical, 1)
                .frame(minWidth: 18)
                .background(mentions ? Color.red : Color.accentColor, in: Capsule())
                .accessibilityLabel(mentions ? "\(count) mentions" : "\(count) unread")
        }
    }
}
