import Foundation
import MtrxFFI

/// The library's rows (`Timeline.subscribeRows`) are the list's: their ID
/// is stable while the row stays.
extension TimelineRow: Identifiable {
    /// Its message, for a message's row.
    var messageRow: MessageRow? {
        guard
            case .message(let event, let firstInGroup, let showSender, let avatarColumn, let decrypting, let actions) =
                content
        else { return nil }
        return MessageRow(
            event: event, firstInGroup: firstInGroup, showSender: showSender, avatarColumn: avatarColumn,
            decrypting: decrypting, actions: actions)
    }
}

/// A message's row (`RowContent.message`), as the views take it.
struct MessageRow: Equatable {
    let event: TimelineEvent
    /// Starts a group of messages by one sender.
    let firstInGroup: Bool
    /// Show the sender's name above it and their avatar beside it (the
    /// first of a group by someone else, outside direct chats).
    let showSender: Bool
    /// Someone else's message outside a direct chat: the avatars' column
    /// is on its left (empty but for the first of a group).
    let avatarColumn: Bool
    /// An undecryptable message while message keys arrive: it may decrypt
    /// meanwhile.
    let decrypting: Bool
    /// What holding the message offers, in the menu's order.
    let actions: [MessageAction]

    /// The row's content.
    var content: RowContent {
        .message(
            event: event, firstInGroup: firstInGroup, showSender: showSender, avatarColumn: avatarColumn,
            decrypting: decrypting, actions: actions)
    }

    var message: Message? {
        if case .message(let message) = event.content { return message }
        return nil
    }

    /// In the order first given.
    var reactions: [Reaction] { message?.reactions ?? [] }
    /// The message it answers.
    var quote: ReplyQuote? { message?.replyTo }
    var edited: Bool { message?.edited ?? false }
    /// The others who read up to here.
    var readers: [Reader] { event.readBy }

    /// What an undecryptable message says instead of its text; nil for
    /// others.
    var undecryptableText: String? {
        guard event.content == .unableToDecrypt else { return nil }
        return decrypting ? "Decrypting…" : "Could not decrypt this message."
    }

    /// Swiping it to the left answers it, as Reply in its menu.
    var canSwipeToReply: Bool {
        actions.contains(.reply)
    }
}

/// `alice` of `@alice:example.org`.
func localpart(_ userId: String) -> String {
    let name = userId.hasPrefix("@") ? userId.dropFirst() : Substring(userId)
    return String(name.prefix { $0 != ":" })
}

/// Who read up to a message, as its row shows them: at most `shown`
/// avatars, then "+N".
enum ReaderAvatars {
    static let shown = 3

    static func split(_ readers: [Reader]) -> (shown: [Reader], more: Int) {
        (Array(readers.prefix(shown)), max(0, readers.count - shown))
    }
}

extension Reader {
    /// The display name, else the user ID's local part (until the name is
    /// known).
    var displayName: String {
        name ?? localpart(userId)
    }
}

extension ReplyQuote {
    /// The original's sender, once loaded.
    var senderLabel: String? {
        senderName ?? senderId.map(localpart)
    }

    /// The original in a few words: its text, or for a file the kind and
    /// its name; "Loading…" while it is fetched.
    var preview: String {
        switch state {
        case .loading:
            return "Loading…"
        case .failed:
            return "Could not load the message"
        case .loaded:
            let body = body ?? ""
            guard let label = kind?.fileLabel else { return body }
            return body.isEmpty ? label : "\(label): \(body)"
        }
    }

    /// Tapping it goes to the original.
    var canGoTo: Bool {
        state == .loaded
    }
}

extension MessageKind {
    /// "Image", "Video", "Audio" or "File" for a message with a file.
    var fileLabel: String? {
        switch self {
        case .image: return "Image"
        case .video: return "Video"
        case .audio: return "Audio"
        case .file: return "File"
        default: return nil
        }
    }
}

extension TimelineEvent {
    /// The sender's display name, else their user ID's local part.
    var displayName: String {
        senderName ?? localpart(senderId)
    }
}
