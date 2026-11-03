import Foundation
import MtrxFFI

/// A room's topic: its text and the links on lines of their own (calls and
/// other links), as the desktop shows and edits them.
enum Topics {
    /// The line the room's header previews the topic with: its text's first
    /// line, else its links' names, else "Add a topic" if we may edit it.
    static func preview(_ topic: Topic, canEdit: Bool) -> String? {
        if let first = topic.text.split(separator: "\n", omittingEmptySubsequences: false).first,
           !topic.text.isEmpty
        {
            return String(first)
        }
        if !topic.links.isEmpty {
            return topic.links.map(\.name).joined(separator: " · ")
        }
        return canEdit ? "Add a topic" : nil
    }

    /// The first non-empty line of a topic's text, for a room's details.
    static func firstLine(_ topic: String?) -> String? {
        guard let topic else { return nil }
        return parseTopic(topic: topic).text
            .split(separator: "\n")
            .map { $0.trimmingCharacters(in: .whitespaces) }
            .first { !$0.isEmpty }
    }
}

/// A topic being edited: its text, and its calls and links, each with a
/// title, an address, and for calls a passcode.
struct TopicDraft: Equatable {
    struct Link: Identifiable, Equatable {
        let id = UUID()
        var call: Bool
        var title = ""
        var url = ""
        var passcode = ""

        /// Its address is a web link the topic can keep.
        var valid: Bool {
            let url = url.trimmingCharacters(in: .whitespaces)
            guard let parsed = URL(string: url), let scheme = parsed.scheme?.lowercased() else { return false }
            return (scheme == "https" || scheme == "http") && parsed.host != nil
        }
    }

    var text: String
    var links: [Link]

    init(_ topic: Topic) {
        text = topic.text
        links = topic.links.map {
            Link(call: $0.call, title: $0.title ?? "", url: $0.url, passcode: $0.passcode ?? "")
        }
    }

    init(text: String = "", links: [Link] = []) {
        self.text = text
        self.links = links
    }

    /// The calls first, as the topic keeps them, then the other links.
    var calls: [Link] { links.filter(\.call) }
    var others: [Link] { links.filter { !$0.call } }

    mutating func add(call: Bool) {
        links.append(Link(call: call))
    }

    mutating func remove(_ id: UUID) {
        links.removeAll { $0.id == id }
    }

    /// Links without an address the topic can keep: left out when saved.
    var dropped: Int {
        links.filter { !$0.valid }.count
    }

    /// The topic to save.
    var topic: String {
        composeTopic(
            topic: Topic(
                text: text.trimmingCharacters(in: .whitespacesAndNewlines),
                links: (calls + others).map {
                    RoomLink(
                        call: $0.call, title: $0.title, url: $0.url, passcode: $0.call ? $0.passcode : nil, name: "")
                }))
    }
}

/// A room's pinned messages, as their own timeline gives them, first pinned
/// first.
@MainActor
final class PinnedModel: ObservableObject {
    struct Pin: Identifiable, Equatable {
        let id: String
        let eventId: String?
        let sender: String
        let text: String
    }

    @Published private(set) var pins: [Pin] = []
    @Published private(set) var loading = true
    @Published private(set) var error: String?

    private let session: Session
    private let roomId: String
    private let subscriptions = Subscriptions()
    private var timeline: Timeline?

    init(session: Session, roomId: String) {
        self.session = session
        self.roomId = roomId
    }

    func load() async {
        guard timeline == nil else { return }
        do {
            let timeline = try await session.pinnedTimeline(roomId: roomId)
            self.timeline = timeline
            let feed = Feed<[TimelineRow]>()
            subscriptions.add(timeline.subscribeRows(listener: feed), feed) { [weak self] rows in
                self?.apply(rows)
            }
        } catch {
            self.error = errorMessage(error)
            loading = false
        }
    }

    func apply(_ rows: [TimelineRow]) {
        loading = false
        pins = rows.compactMap { row in
            guard let event = row.messageRow?.event, case .message(let message) = event.content else {
                return nil
            }
            return Pin(id: row.id, eventId: event.eventId, sender: event.sender.displayName, text: message.summary)
        }
    }
}
