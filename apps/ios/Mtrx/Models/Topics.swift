import MtrxFFI
import SwiftUI

// A room's topic comes parsed from the library (`RoomSummary.topic`): its
// text, its calls (the first is the room's, which the header joins) and its
// other links, as the desktop shows and edits them.

extension Topic {
    /// No topic.
    static let none = Topic(text: "", calls: [], links: [], firstLine: nil, preview: nil)
}

/// What the room header offers of a topic's calls and links: Join for the
/// room's call, and a menu with the calls, then the links, then Edit links…
/// where we may edit the topic. None without calls or links.
struct HeaderLinks: Equatable {
    struct Group: Equatable {
        let title: String
        let call: Bool
        let links: [RoomLink]
    }

    /// The room's call: a tap on Join opens it.
    let join: RoomLink?
    /// Calls, then Links, each where there are any.
    let groups: [Group]
    let canEdit: Bool

    init?(_ topic: Topic, canEdit: Bool) {
        guard !topic.calls.isEmpty || !topic.links.isEmpty else { return nil }
        join = topic.calls.first
        groups = [
            Group(title: "Calls", call: true, links: topic.calls),
            Group(title: "Links", call: false, links: topic.links),
        ].filter { !$0.links.isEmpty }
        self.canEdit = canEdit
    }

    /// "Join Weekly, passcode 123".
    var joinLabel: String? {
        join.map { call in
            call.passcode.map { "Join \(call.name), passcode \($0)" } ?? "Join \(call.name)"
        }
    }
}

/// A topic being edited: its text, its calls and its other links, each in
/// its order and with a title, an address, and for calls a passcode.
struct TopicDraft: Equatable {
    struct Link: Identifiable, Equatable {
        let id = UUID()
        var title = ""
        var url = ""
        var passcode = ""

        init(title: String = "", url: String = "", passcode: String = "") {
            self.title = title
            self.url = url
            self.passcode = passcode
        }

        init(_ link: RoomLink) {
            self.init(title: link.title ?? "", url: link.url, passcode: link.passcode ?? "")
        }

        /// Its address is a web address the topic can keep (the library's
        /// rule: "https://" may be left out).
        var valid: Bool { linkUrl(text: url) != nil }

        /// Nothing typed in: left out when saved, without a word.
        func blank(call: Bool) -> Bool {
            [title, url, call ? passcode : ""].allSatisfy { $0.trimmingCharacters(in: .whitespaces).isEmpty }
        }

        func draft(call: Bool) -> LinkDraft {
            LinkDraft(call: call, title: title, url: url, passcode: call ? passcode : "")
        }
    }

    var text: String
    /// In order: the first is the room's call.
    var calls: [Link]
    var links: [Link]

    init(_ topic: Topic) {
        text = topic.text
        calls = topic.calls.map(Link.init)
        links = topic.links.map(Link.init)
    }

    init(text: String = "", calls: [Link] = [], links: [Link] = []) {
        self.text = text
        self.calls = calls
        self.links = links
    }

    mutating func add(call: Bool) {
        if call { calls.append(Link()) } else { links.append(Link()) }
    }

    mutating func remove(call: Bool, at offsets: IndexSet) {
        if call { calls.remove(atOffsets: offsets) } else { links.remove(atOffsets: offsets) }
    }

    mutating func move(call: Bool, from offsets: IndexSet, to offset: Int) {
        if call {
            calls.move(fromOffsets: offsets, toOffset: offset)
        } else {
            links.move(fromOffsets: offsets, toOffset: offset)
        }
    }

    /// Calls and links typed in without a web address: left out when saved.
    var dropped: Int {
        calls.filter { !$0.blank(call: true) && !$0.valid }.count
            + links.filter { !$0.blank(call: false) && !$0.valid }.count
    }

    /// The topic to save (the library's: calls first, each kind in its
    /// order).
    var topic: String {
        composeTopic(text: text, links: calls.map { $0.draft(call: true) } + links.map { $0.draft(call: false) })
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
    private let work = Work()
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
            work.follow(timeline.subscribeRows(listener: feed), feed) { [weak self] rows in
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
