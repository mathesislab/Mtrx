import Combine
import Foundation
import MtrxFFI

/// A room's timeline while it is shown: its rows, loading older messages,
/// sending, replying, editing and acting on messages, who types, and
/// marking it read.
@MainActor
final class TimelineModel: ObservableObject {
    /// Older messages loaded per request.
    static let page: UInt16 = 40
    /// How long a message gone to stays marked.
    static let highlightTime: UInt64 = 2_000_000_000
    /// Scrolled up less than this (points), the timeline counts as at the
    /// bottom.
    static let bottomTolerance: CGFloat = 40

    let roomId: String
    /// The other person of a direct chat (not the Notes to self), whose
    /// picture and name the header shows, opening their info sheet.
    let person: String?
    @Published private(set) var rows: [TimelineRow] = []
    /// Not opened yet.
    @Published private(set) var opening = true
    /// Back-pagination reached the room's beginning.
    @Published private(set) var reachedStart = false
    @Published private(set) var loadingOlder = false
    /// The composer's text; kept for the room on this device, with the
    /// message it answers (`DraftKeeper`).
    @Published var draft = "" {
        didSet {
            if draft != oldValue {
                setTyping(!draft.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                drafts.changed(draft: keptDraft)
            }
        }
    }
    /// What sending the draft does instead of sending a new message.
    @Published private(set) var compose: Compose? {
        didSet { drafts.changed(draft: keptDraft) }
    }
    /// The names of the others typing.
    @Published private(set) var typing: [String] = []
    /// Looking for a quoted message in older events.
    @Published private(set) var seeking = false
    /// The row gone to, marked for a moment.
    @Published private(set) var highlighted: String?
    /// The row to scroll to: a new value for each request.
    @Published private(set) var scrollTarget: ScrollTarget?
    /// The newest message is in view; else a button leads back to it.
    @Published private(set) var atBottom = true
    /// The other person's info sheet, while open (direct chats).
    @Published var personInfo: PersonInfoModel?
    @Published var error: String?

    private let session: Session
    private let room: RoomSummary
    /// The library's rows, as last given.
    private var delivered: [TimelineRow] = []
    private var timeline: Timeline?
    private let subscriptions = Subscriptions()
    /// Who types, while the room is on screen.
    private let typingSubscription = Subscriptions()
    private var watchingTyping = false
    /// The top of the list (the oldest row) is on screen.
    private var topVisible = false
    private var visible = false
    /// The newest item when the room was last marked read.
    private var readUpTo: String?
    /// The draft before an edit took the field; back after it.
    private var savedDraft: String?
    /// The last `setTyping` call: each waits for the one before.
    private var typingCall: Task<Void, Never>?
    /// The row of the message gone to, found in older events: scrolled to
    /// once it is among the rows.
    private var revealing: String?
    /// Keeps the draft on this device, saved once typing stops.
    private let drafts: DraftKeeper
    /// The message the draft left in the room answers (its event ID), until
    /// it is answered or another reply or edit starts.
    private var restoringReply: String?
    /// That message's row, found by the library: answered once it is
    /// among the rows.
    private var replyRow: String?
    /// Our reactions added or taken away here, until the library's items
    /// show them: whether we want each one, shown so at once.
    private var reacting: [ReactionKey: Bool] = [:]

    init(session: Session, room: RoomSummary) {
        self.session = session
        self.room = room
        drafts = session.draftKeeper(roomId: room.id)
        roomId = room.id
        let ours = session.userId()
        person = room.kind == .direct ? room.directUserId.flatMap { $0 == ours ? nil : $0 } : nil
    }

    /// Open the timeline, with the draft left in the room unless something
    /// was written meanwhile: its text, and the reply once its message is
    /// in the timeline.
    func open() async {
        guard timeline == nil else { return }
        // Before the timeline: bringing it back is not typing.
        if let left = await drafts.restore(), draft.isEmpty, compose == nil {
            restoringReply = left.replyTo
            draft = left.text
        }
        do {
            let timeline = try await session.timeline(roomId: roomId)
            self.timeline = timeline
            let feed = Feed<[TimelineRow]>()
            subscriptions.add(timeline.subscribeRows(listener: feed), feed) { [weak self] rows in
                self?.apply(rows)
            }
            if let eventId = restoringReply {
                waitForReply(eventId, in: timeline)
            }
            opening = false
            watchTyping()
            if topVisible {
                await loadOlder()
            }
        } catch {
            opening = false
            self.error = errorMessage(error)
        }
    }

    func apply(_ rows: [TimelineRow]) {
        delivered = rows
        // Reactions the library shows as wanted (or whose message went)
        // are no longer ours to show.
        reacting = reacting.filter { key, wanted in
            guard let row = Self.messageRow(key.itemId, in: rows) else { return false }
            return row.reactions.contains { $0.key == key.key && $0.byUs } != wanted
        }
        self.rows = withOurReactions(rows)
        if rows.contains(where: { $0.content == .start }) {
            reachedStart = true
        }
        answerReply()
        if let row = revealing, show(row) {
            revealing = nil
        }
        markRead()
    }

    /// `rows` with the reactions we changed and the library does not show
    /// yet.
    private func withOurReactions(_ rows: [TimelineRow]) -> [TimelineRow] {
        guard !reacting.isEmpty else { return rows }
        return rows.map { row in
            guard let message = row.messageRow else { return row }
            let changes = reacting.filter { $0.key.itemId == row.id }
            guard !changes.isEmpty else { return row }
            var reactions = message.reactions
            for (key, wanted) in changes {
                reactions = Reactions.setting(key.key, ours: wanted, in: reactions)
            }
            return TimelineRow(id: row.id, content: message.with(reactions: reactions).content)
        }
    }

    /// The timeline is on screen (or not): mark it read and show who types
    /// while it is; leaving it, we stop typing and the draft is saved.
    func setVisible(_ visible: Bool) {
        self.visible = visible
        if visible {
            watchTyping()
        } else {
            typingSubscription.cancelAll()
            watchingTyping = false
            typing = []
            setTyping(false)
            let drafts = self.drafts
            Task { await drafts.flush() }
        }
        markRead()
    }

    /// The oldest row is on screen (or not): load older ones while it is.
    func setTopVisible(_ visible: Bool) async {
        topVisible = visible
        if visible {
            await loadOlder()
        }
    }

    /// Load older messages until the top of the list is off screen or the
    /// room's start is reached.
    func loadOlder() async {
        guard timeline != nil, !loadingOlder, !reachedStart else { return }
        loadingOlder = true
        defer { loadingOlder = false }
        do {
            while let timeline, topVisible && !reachedStart {
                if try await timeline.paginateBack(events: Self.page) {
                    reachedStart = true
                }
            }
        } catch {
            self.error = errorMessage(error)
        }
    }

    /// The timeline is scrolled `distance` points up from its bottom
    /// (negative: pulled past it).
    func scrolled(distanceFromBottom distance: CGFloat) {
        setAtBottom(distance <= Self.bottomTolerance)
    }

    /// Before iOS 18 (no scroll geometry): the end of the timeline is drawn
    /// (on screen or near it), or no longer.
    func setBottomDrawn(_ drawn: Bool) {
        guard #unavailable(iOS 18) else { return }
        setAtBottom(drawn)
    }

    private func setAtBottom(_ atBottom: Bool) {
        if atBottom != self.atBottom {
            self.atBottom = atBottom
        }
    }

    /// The button to the newest message shows: scrolled up, once open.
    var showsScrollToBottom: Bool {
        !atBottom && !opening
    }

    /// A direct chat's header was tapped: the other person's info sheet.
    func showPersonInfo() {
        guard let person, personInfo == nil else { return }
        let info = PersonInfoModel(session: session, userId: person, room: room)
        info.load()
        personInfo = info
    }

    // MARK: Composing

    /// Send the draft: a new message, a reply or an edit, as `compose`
    /// says. It shows in the timeline at once, and is sent in the
    /// background.
    func send() async {
        let text = draft.trimmingCharacters(in: .whitespacesAndNewlines)
        guard let timeline, !text.isEmpty else { return }
        let compose = self.compose
        // Its message not found: sent as a new one.
        restoringReply = nil
        // After an edit, the draft from before it is back.
        endCompose()
        if case .edit = compose {} else {
            draft = ""
        }
        let drafts = self.drafts
        Task { await drafts.flush() }
        let left = draft
        do {
            switch compose {
            case nil:
                try await timeline.sendText(text: text)
            case .reply(let target):
                try await timeline.reply(itemId: target.itemId, text: text)
            case .edit(let target):
                try await timeline.edit(itemId: target.itemId, text: text)
            }
        } catch {
            // Back as it was, unless something else was begun meanwhile.
            if self.compose == nil && draft == left {
                if case .edit = compose {
                    savedDraft = left
                }
                draft = text
                self.compose = compose
            }
            self.error = errorMessage(error)
        }
    }

    /// Answer row `itemId`'s message with the next one sent.
    func startReply(to itemId: String) {
        guard let row = messageRow(itemId) else { return }
        if case .edit = compose {
            endCompose()
        }
        restoringReply = nil
        compose = .reply(ComposeTarget(itemId: itemId, row: row))
    }

    /// Change row `itemId`'s message (ours): the field holds its text as we
    /// wrote it.
    func startEdit(_ itemId: String) {
        guard let row = messageRow(itemId), let message = row.message else { return }
        if case .edit = compose {} else {
            savedDraft = draft
        }
        restoringReply = nil
        compose = .edit(ComposeTarget(itemId: itemId, row: row))
        draft = message.rawBody.isEmpty ? message.body : message.rawBody
    }

    /// Stop replying or editing; after an edit, the draft from before is
    /// back.
    func cancelCompose() {
        endCompose()
    }

    private func endCompose() {
        if case .edit = compose {
            draft = savedDraft ?? ""
            savedDraft = nil
        }
        compose = nil
    }

    // MARK: Drafts

    /// The draft as kept: during an edit, the one from before it (an edit
    /// is not a draft).
    private var keptDraft: Draft {
        switch compose {
        case .edit: return Draft(text: savedDraft ?? "", replyTo: nil)
        case .reply(let target): return Draft(text: draft, replyTo: target.eventId)
        case nil: return Draft(text: draft, replyTo: restoringReply)
        }
    }

    /// Answer the restored draft's message once the library has it (it
    /// loads nothing for it).
    private func waitForReply(_ eventId: String, in timeline: Timeline) {
        subscriptions.add(
            Task { [weak self] in
                guard let row = try? await timeline.itemOf(eventId: eventId),
                    let self, restoringReply == eventId
                else { return }
                replyRow = row
                answerReply()
            })
    }

    /// Reply to the restored draft's message, once its row is among the
    /// rows, unless another reply or edit started.
    private func answerReply() {
        guard let itemId = replyRow, let message = messageRow(itemId) else { return }
        replyRow = nil
        guard restoringReply != nil, compose == nil else { return }
        restoringReply = nil
        compose = .reply(ComposeTarget(itemId: itemId, row: message))
    }

    /// The app leaves the foreground: the draft is saved, in the time iOS
    /// grants, before it may end the app.
    func leftForeground() {
        let background = BackgroundTask(name: "Save the draft")
        let drafts = self.drafts
        Task {
            await drafts.flush()
            background.end()
        }
    }

    /// Tell the others whether we type (the library sends at most a notice
    /// every few seconds).
    private func setTyping(_ typing: Bool) {
        guard let timeline else { return }
        let previous = typingCall
        typingCall = Task {
            await previous?.value
            try? await timeline.setTyping(typing: typing)
        }
    }

    private func watchTyping() {
        guard visible, !watchingTyping, let timeline else { return }
        watchingTyping = true
        let feed = Feed<[String]>()
        typingSubscription.add(timeline.subscribeTyping(listener: feed), feed) { [weak self] names in
            self?.typing = names
        }
    }

    /// One line saying who types, or nil if nobody does.
    static func typingLine(_ names: [String]) -> String? {
        switch names.count {
        case 0: return nil
        case 1: return "\(names[0]) is typing…"
        case 2: return "\(names[0]) and \(names[1]) are typing…"
        case 3: return "\(names[0]), \(names[1]) and \(names[2]) are typing…"
        default: return "\(names[0]), \(names[1]) and \(names.count - 2) others are typing…"
        }
    }

    // MARK: Acting on messages

    /// Add our reaction `key` to row `itemId`'s message, or remove it: shown
    /// so at once, until the library's items show it (or it failed).
    @discardableResult
    func react(_ itemId: String, key: String) -> Task<Void, Never> {
        guard timeline != nil, let row = messageRow(itemId) else { return act { try await $0.react(itemId: itemId, key: key) } }
        let change = ReactionKey(itemId: itemId, key: key)
        reacting[change] = !row.reactions.contains { $0.key == key && $0.byUs }
        rows = withOurReactions(delivered)
        return act {
            try await $0.react(itemId: itemId, key: key)
        } failed: { [weak self] in
            guard let self else { return }
            reacting[change] = nil
            rows = withOurReactions(delivered)
        }
    }

    /// Pin row `itemId`'s message (`pin`), or unpin it.
    @discardableResult
    func setPinned(_ itemId: String, pin: Bool) -> Task<Void, Never> {
        guard let eventId = messageRow(itemId)?.event.eventId else { return Task {} }
        let (session, roomId) = (self.session, self.roomId)
        return Task {
            do {
                try await session.setPinned(roomId: roomId, eventId: eventId, pin: pin)
            } catch {
                self.error = errorMessage(error)
            }
        }
    }

    /// Delete row `itemId`'s message (ours) for everyone.
    @discardableResult
    func delete(_ itemId: String) -> Task<Void, Never> {
        act { try await $0.delete(itemId: itemId) }
    }

    /// Send row `itemId`'s refused message again.
    @discardableResult
    func retry(_ itemId: String) -> Task<Void, Never> {
        act { try await $0.retry(itemId: itemId) }
    }

    /// Drop row `itemId`'s refused message.
    @discardableResult
    func discard(_ itemId: String) -> Task<Void, Never> {
        act { try await $0.discard(itemId: itemId) }
    }

    /// Run `action` in a task of the model's (leaving the room does not
    /// cancel it); a failure shows as `error`.
    private func act(
        _ action: @escaping (Timeline) async throws -> Void, failed: @escaping @MainActor () -> Void = {}
    ) -> Task<Void, Never> {
        guard let timeline else { return Task {} }
        return Task {
            do {
                try await action(timeline)
            } catch {
                failed()
                self.error = errorMessage(error)
            }
        }
    }

    func messageRow(_ itemId: String) -> MessageRow? {
        Self.messageRow(itemId, in: rows)
    }

    /// The row of the message with `eventId`, if loaded.
    private func eventRow(_ eventId: String) -> TimelineRow? {
        rows.first { row in row.messageRow?.event.eventId == eventId }
    }

    private static func messageRow(_ itemId: String, in rows: [TimelineRow]) -> MessageRow? {
        rows.first { $0.id == itemId }?.messageRow
    }

    // MARK: Going to a quoted message

    /// Scroll to the message with `eventId` and mark it, once the library
    /// loaded the older events it is in. A second call is ignored while
    /// one is under way.
    func goTo(eventId: String) async {
        guard let timeline else { return }
        if let row = eventRow(eventId) {
            show(row.id)
            return
        }
        guard !seeking else { return }
        seeking = true
        defer { seeking = false }
        do {
            let row = try await timeline.seek(eventId: eventId)
            // The rows may not have it yet.
            if !show(row) {
                revealing = row
            }
        } catch {
            self.error = errorMessage(error)
        }
    }

    /// Scroll to row `rowId` and mark it, if it is there.
    @discardableResult
    private func show(_ rowId: String) -> Bool {
        guard rows.contains(where: { $0.id == rowId }) else { return false }
        scrollTarget = ScrollTarget(rowId: rowId, serial: (scrollTarget?.serial ?? 0) + 1)
        highlighted = rowId
        Task {
            try? await Task.sleep(nanoseconds: Self.highlightTime)
            if highlighted == rowId {
                highlighted = nil
            }
        }
        return true
    }

    private func markRead() {
        guard visible, let timeline, let newest = rows.last?.id, newest != readUpTo else { return }
        readUpTo = newest
        Task {
            try? await timeline.markRead()
        }
    }
}

extension TimelineModel {
    /// Replying to a message, or editing one of ours.
    enum Compose: Equatable {
        case reply(ComposeTarget)
        case edit(ComposeTarget)

        var target: ComposeTarget {
            switch self {
            case .reply(let target), .edit(let target): return target
            }
        }
    }

    struct ScrollTarget: Equatable {
        let rowId: String
        let serial: Int
    }

    /// A reaction of ours to a row's message.
    private struct ReactionKey: Hashable {
        let itemId: String
        let key: String
    }
}

/// Reactions as they are once ours changed.
enum Reactions {
    /// `reactions` with ours with `key` added (`ours`) or taken away; the
    /// others' stay. A new key goes last.
    static func setting(_ key: String, ours: Bool, in reactions: [Reaction]) -> [Reaction] {
        var reactions = reactions
        if let at = reactions.firstIndex(where: { $0.key == key }) {
            var reaction = reactions[at]
            guard reaction.byUs != ours else { return reactions }
            reaction.byUs = ours
            reaction.count = ours ? reaction.count + 1 : reaction.count - min(reaction.count, 1)
            if reaction.count == 0 {
                reactions.remove(at: at)
            } else {
                reactions[at] = reaction
            }
        } else if ours {
            reactions.append(Reaction(key: key, count: 1, byUs: true, senders: []))
        }
        return reactions
    }
}

extension MessageRow {
    /// The same row with other reactions.
    func with(reactions: [Reaction]) -> MessageRow {
        var event = event
        if case .message(var message) = event.content {
            message.reactions = reactions
            event.content = .message(message: message)
        }
        return MessageRow(
            event: event, firstInGroup: firstInGroup, showSender: showSender, avatarColumn: avatarColumn,
            decrypting: decrypting, actions: actions)
    }
}

/// The message replied to or edited, as the composer's bar shows it.
struct ComposeTarget: Equatable {
    let itemId: String
    /// Its event ID, kept with a draft that answers it.
    let eventId: String?
    let sender: String
    /// Its first line.
    let preview: String

    init(itemId: String, eventId: String? = nil, sender: String, preview: String) {
        self.itemId = itemId
        self.eventId = eventId
        self.sender = sender
        self.preview = preview
    }

    init(itemId: String, row: MessageRow) {
        self.init(
            itemId: itemId, eventId: row.event.eventId, sender: row.event.displayName,
            preview: row.message?.summary ?? "")
    }
}

extension Message {
    /// Its first line, or for a file its kind and name.
    var summary: String {
        let line = body.split(separator: "\n", maxSplits: 1).first.map(String.init) ?? ""
        guard media != nil, let label = kind.fileLabel else { return line }
        return line.isEmpty ? label : "\(label): \(line)"
    }
}
