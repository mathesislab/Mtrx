import MtrxFFI
import QuickLook
import SwiftUI

/// A room: its header (name and topic line), its timeline, newest at the
/// bottom, who types, and the composer. The name opens the room's info,
/// the topic line its topic panel; its space's picture, at the right, the
/// space. A direct chat's `#` shows the rooms shared with the person over
/// the timeline.
struct RoomView: View {
    let room: RoomSummary
    @StateObject private var model: TimelineModel
    @StateObject private var info: RoomInfoModel
    @EnvironmentObject private var session: SessionModel
    @EnvironmentObject private var presence: PresenceModel
    @EnvironmentObject private var downloads: DownloadsModel
    @Environment(\.scenePhase) private var scenePhase
    @State private var sheet: RoomSheet?
    @State private var showsInfo = false
    /// The message to delete, once confirmed.
    @State private var deleting: String?
    /// The message not sent yet whose status was tapped: to retry or take
    /// back.
    @State private var unsent: String?
    /// The rooms shared with a direct chat's person show.
    @State private var showsShared = false
    /// Chosen in a message's menu (on which item), done once it closed.
    @State private var chosen: (action: MessageAction, itemId: String)?
    /// Share… was chosen in a message's menu (on which item).
    @State private var sharing: String?
    /// A downloaded file shown in QuickLook.
    @State private var preview: URL?
    /// A downloaded file being shared.
    @State private var shared: URL?

    init(
        room: RoomSummary, model: @autoclosure @escaping () -> TimelineModel,
        info: @autoclosure @escaping () -> RoomInfoModel
    ) {
        self.room = room
        _model = StateObject(wrappedValue: model())
        _info = StateObject(wrappedValue: info())
    }

    var body: some View {
        VStack(spacing: 0) {
            TimelineList(model: model, actions: actions)
                .safeAreaInset(edge: .bottom) { OfflineBanner() }
            TypingLine(text: TimelineModel.typingLine(model.typing))
            Divider()
            Composer(draft: $model.draft, compose: model.compose, cancel: model.cancelCompose) {
                model.send()
            }
        }
        .navigationTitle(room.name)
        .navigationBarTitleDisplayMode(.inline)
        .toolbar {
            ToolbarItem(placement: .principal) {
                if let person = model.person {
                    PersonHeader(room: room, presence: presence.presence(of: person.userId), open: model.showPersonInfo)
                } else {
                    RoomHeader(
                        room: room, topic: topic, canEdit: room.rights.changeTopic,
                        openInfo: { showsInfo = true }, openTopic: { sheet = .topic })
                }
            }
            ToolbarItem(placement: .navigationBarTrailing) {
                if model.person != nil {
                    SharedRoomsButton(showing: showsShared) {
                        if !showsShared {
                            UIApplication.shared.sendAction(
                                #selector(UIResponder.resignFirstResponder), to: nil, from: nil, for: nil)
                        }
                        withAnimation(.easeOut(duration: 0.2)) { showsShared.toggle() }
                    }
                } else {
                    SpaceButton(roomId: room.id)
                }
            }
        }
        .overlay {
            if showsShared, let person = model.person {
                SharedRoomsPanel(userId: person.userId, name: room.name)
                    .transition(.opacity)
            }
        }
        .task { await model.open() }
        .onAppear { model.setVisible(true) }
        .onDisappear {
            model.setVisible(false)
            showsShared = false
        }
        .onChange(of: scenePhase) { phase in
            if phase != .active {
                model.leftForeground()
            }
        }
        .sheet(item: $sheet, onDismiss: performChosen) { sheet in
            RoomSheetView(sheet: sheet, room: room, model: model, info: info, topic: topic) { action, id in
                chosen = (action, id)
                self.sheet = nil
            } share: { id in
                sharing = id
                self.sheet = nil
            }
                .account(session)
        }
        .sheet(item: $model.personInfo) { info in
            PersonInfoView(model: info)
                .account(session)
        }
        .fullScreenCover(isPresented: $showsInfo) {
            RoomInfoView(room: room, model: info)
                .account(session)
        }
        .confirmationDialog("Delete this message for everyone?", isPresented: isDeleting, titleVisibility: .visible) {
            Button("Delete", role: .destructive) {
                if let deleting { model.delete(deleting) }
            }
        }
        .confirmationDialog("Not sent yet", isPresented: isUnsent) {
            if let unsent, let row = model.messageRow(unsent) {
                ForEach(row.actions.filter { $0 == .retry || $0 == .unsend }, id: \.self) { action in
                    Button(action.title) { perform(action, on: unsent) }
                }
            }
        }
        .quickLookPreview($preview)
        .sheet(isPresented: isSharing) {
            if let shared {
                ShareSheet(url: shared)
            }
        }
        .alert("Something went wrong", isPresented: hasError) {
            Button("OK") { model.error = nil }
        } message: {
            Text(model.error ?? "")
        }
    }

    /// The topic as the room list has it, one just saved too.
    private var topic: Topic {
        parseTopic(topic: room.topic ?? "")
    }

    private var actions: RowActions {
        RowActions(
            hold: openMenu,
            perform: perform,
            react: { id, key in model.react(id, key: key) },
            goTo: { quote in model.goTo(eventId: quote.eventId) },
            showReactors: { id, key in sheet = .reactors(itemId: id, key: key) },
            showReaders: { id in sheet = .readers(itemId: id) },
            unsent: { id in unsent = id },
            open: { media, eventId in fetch(media, eventId: eventId) { preview = $0 } })
    }

    /// Download a message's file (once), then hand it to `then`.
    private func fetch(_ media: Media, eventId: String?, then: @escaping @MainActor (URL) -> Void) {
        do {
            try downloads.fetch(media, roomId: room.id, eventId: eventId, then: then)
        } catch {
            model.error = errorMessage(error)
        }
    }

    /// A held message's menu, if anything applies to it.
    private func openMenu(_ id: String) {
        guard let row = model.messageRow(id), !row.actions.isEmpty else { return }
        UIImpactFeedbackGenerator(style: .medium).impactOccurred()
        sheet = .menu(itemId: id)
    }

    /// What was chosen in a message's menu, the menu closed: a sheet or
    /// dialog it opens can show.
    private func performChosen() {
        if let sharing {
            self.sharing = nil
            if let row = model.messageRow(sharing), let media = row.message?.media {
                fetch(media, eventId: row.event.eventId) { shared = $0 }
            }
        }
        guard let chosen else { return }
        self.chosen = nil
        perform(chosen.action, on: chosen.itemId)
    }

    private func perform(_ action: MessageAction, on id: String) {
        switch action {
        case .react: sheet = .emoji(itemId: id)
        case .reply: model.startReply(to: id)
        case .copy: UIPasteboard.general.string = model.messageRow(id)?.message?.body
        case .selectText:
            if let body = model.messageRow(id)?.message?.body {
                sheet = .text(body)
            }
        case .edit: model.startEdit(id)
        case .pin: model.setPinned(id, pin: true)
        case .unpin: model.setPinned(id, pin: false)
        case .delete: deleting = id
        case .retry: model.retry(id)
        case .unsend: model.unsend(id)
        }
    }

    private var isSharing: Binding<Bool> {
        Binding(get: { shared != nil }, set: { if !$0 { shared = nil } })
    }

    private var isUnsent: Binding<Bool> {
        Binding(get: { unsent != nil }, set: { if !$0 { unsent = nil } })
    }

    private var isDeleting: Binding<Bool> {
        Binding(get: { deleting != nil }, set: { if !$0 { deleting = nil } })
    }

    private var hasError: Binding<Bool> {
        Binding(get: { model.error != nil }, set: { if !$0 { model.error = nil } })
    }
}

/// A room's title: its name, which opens its info, over its topic line, a
/// caret and the topic's first line (else its links' names, else "Add a
/// topic" where we may edit it), which opens the topic panel.
struct RoomHeader: View {
    let room: RoomSummary
    let topic: Topic
    let canEdit: Bool
    let openInfo: () -> Void
    let openTopic: () -> Void

    var body: some View {
        VStack(spacing: 0) {
            Button(action: openInfo) {
                Text(room.name)
                    .font(.headline)
                    .foregroundColor(.primary)
                    .lineLimit(1)
            }
            .buttonStyle(.plain)
            .accessibilityHint("Shows the room's info and settings")
            if let line = Topics.preview(topic, canEdit: canEdit) {
                Button(action: openTopic) {
                    HStack(spacing: 3) {
                        Image(systemName: "chevron.down")
                            .font(.caption2.weight(.bold))
                            .imageScale(.small)
                        Text(line)
                            .font(.caption)
                            .lineLimit(1)
                    }
                    .foregroundStyle(.secondary)
                    .frame(maxWidth: 240)
                }
                .buttonStyle(.plain)
                .accessibilityLabel("Topic: \(line)")
                .accessibilityHint("Shows the topic, its links and the pinned messages")
            }
        }
    }
}

/// A direct chat's title: the person's picture and name, with their
/// presence; a tap opens their info sheet.
private struct PersonHeader: View {
    let room: RoomSummary
    let presence: Presence?
    let open: () -> Void

    var body: some View {
        Button(action: open) {
            HStack(spacing: 8) {
                AvatarView(name: room.name, uri: room.avatarUrl, size: 28)
                    .presenceDot(presence, size: 9, avatar: 28)
                VStack(alignment: .leading, spacing: 0) {
                    Text(room.name)
                        .font(.headline)
                        .foregroundColor(.primary)
                        .lineLimit(1)
                    PresenceLine(presence: presence, font: .caption)
                }
            }
        }
        .buttonStyle(.plain)
        .accessibilityElement(children: .combine)
        .accessibilityHint("Shows who this is")
    }
}

/// A direct chat's `#`, filled while the rooms shared with the person show:
/// shows or hides them.
private struct SharedRoomsButton: View {
    let showing: Bool
    let toggle: () -> Void

    var body: some View {
        Button(action: toggle) {
            Image(systemName: showing ? "number.square.fill" : "number")
        }
        .accessibilityLabel("Shared rooms")
        .accessibilityValue(showing ? "Shown" : "Hidden")
    }
}

/// A room's space (the first in the Spaces tab's order) at the right of its
/// header, if any: its small picture, which shows the space. It follows the
/// room lists itself, so that their updates don't draw the room again.
private struct SpaceButton: View {
    let roomId: String
    @EnvironmentObject private var rooms: RoomListModel
    @EnvironmentObject private var home: HomeNavigation

    var body: some View {
        if let space = rooms.lists.firstSpaceOf(roomId: roomId) {
            Button {
                home.open(space.id)
            } label: {
                AvatarView(name: space.name, uri: space.avatarUrl, size: 28, shape: .square)
            }
            .accessibilityLabel(space.name)
            .accessibilityHint("Shows the space")
        }
    }
}

/// The rooms shared with a direct chat's person, over the timeline and the
/// composer, under the header: grouped as in the Rooms tab (Home, then each
/// space), newest first. A room opens in the Rooms tab, a space's heading
/// shows the space.
struct SharedRoomsPanel: View {
    @EnvironmentObject private var session: SessionModel
    @EnvironmentObject private var rooms: RoomListModel
    @EnvironmentObject private var home: HomeNavigation
    let userId: String
    /// The person's.
    let name: String
    /// The shared rooms' and spaces' IDs, once loaded.
    @State private var shared: Set<String>?
    @State private var error: String?

    var body: some View {
        Group {
            if let shared {
                RoomList(
                    sections: rooms.lists.roomGroups(among: shared.sorted(), folded: []), searching: false, banner: false,
                    openSettings: {}, opensInRooms: true, openSpace: { home.open($0) }
                ) {
                    Text("No rooms shared with \(name)")
                        .foregroundStyle(.secondary)
                        .multilineTextAlignment(.center)
                        .padding()
                }
            } else if let error {
                Text(error)
                    .foregroundStyle(.secondary)
                    .multilineTextAlignment(.center)
                    .padding()
            } else {
                ProgressView()
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .background(Color(.systemBackground))
        .task {
            do {
                shared = Set(try await session.session.sharedRooms(userId: userId))
            } catch {
                self.error = errorMessage(error)
            }
        }
    }
}

/// The sheets a room opens over itself.
enum RoomSheet: Identifiable {
    /// A held message's menu.
    case menu(itemId: String)
    /// Who reacted to a message with a key.
    case reactors(itemId: String, key: String)
    /// Who read up to a message.
    case readers(itemId: String)
    /// All emoji, to react to a message with.
    case emoji(itemId: String)
    /// A message's text, to select from.
    case text(String)
    /// The topic in full, its links and the pinned messages.
    case topic

    var id: String {
        switch self {
        case .menu(let itemId): return "menu \(itemId)"
        case .reactors(let itemId, let key): return "reactors \(itemId) \(key)"
        case .readers(let itemId): return "readers \(itemId)"
        case .emoji(let itemId): return "emoji \(itemId)"
        case .text(let text): return "text \(text)"
        case .topic: return "topic"
        }
    }
}

/// A room's sheet, from the newest rows (names arrive in later updates).
private struct RoomSheetView: View {
    let sheet: RoomSheet
    let room: RoomSummary
    @ObservedObject var model: TimelineModel
    @ObservedObject var info: RoomInfoModel
    let topic: Topic
    /// An entry of a message's menu was chosen (item ID).
    let choose: (MessageAction, String) -> Void
    /// Share… was chosen in a message's menu (item ID).
    let share: (String) -> Void
    @EnvironmentObject private var session: SessionModel

    var body: some View {
        switch sheet {
        case .menu(let itemId):
            if let row = model.messageRow(itemId) {
                MessageMenu(
                    row: row, entries: row.actions,
                    react: { key in model.react(itemId, key: key) },
                    choose: { action in choose(action, itemId) },
                    share: row.message?.media == nil ? nil : { share(itemId) })
            }
        case .reactors(let itemId, let key):
            let reaction = model.messageRow(itemId)?.reactions.first { $0.key == key }
            PeopleSheet(title: "Reacted with \(key)", people: reaction?.senders ?? [])
        case .readers(let itemId):
            PeopleSheet(title: "Read by", people: model.messageRow(itemId)?.readers ?? [])
        case .emoji(let itemId):
            let ours = model.messageRow(itemId)?.reactions.filter(\.byUs).map(\.key) ?? []
            EmojiPicker(ours: Set(ours)) { key in
                model.react(itemId, key: key)
            }
        case .text(let text):
            SelectTextSheet(text: text)
        case .topic:
            TopicPanel(
                topic: topic, canEdit: room.rights.changeTopic, info: info,
                pinned: PinnedModel(session: session.session, roomId: room.id)
            ) { eventId in
                model.goTo(eventId: eventId)
            }
        }
    }
}

/// The timeline, newest message at the bottom, older ones loading when its
/// top comes into view. The scroll view and its rows are drawn upside down,
/// the rows newest first: its start, where it opens, is the newest message,
/// drawn and measured at once, and older rows load at its far end, out of
/// view, where the lazy stack's guesses at their heights move nothing in
/// view. (Anchored at its bottom instead, it opened at the end of those
/// guesses, past the rows or short of them, until they were measured.) At
/// its start, it stays there as new rows come. Scrolled up, a button leads
/// back to the bottom.
struct TimelineList: View {
    @ObservedObject var model: TimelineModel
    let actions: RowActions

    /// Below the newest row.
    private static let bottomId = "bottom"

    var body: some View {
        ScrollViewReader { proxy in
            GeometryReader { geometry in
                ScrollView {
                    rows(insets: geometry.safeAreaInsets)
                }
                .modifier(MeasuresBottom(model: model))
                .modifier(NoEdgeEffect())
                .flipped()
                // Under the bars, as without the flip: their insets are
                // the rows' padding, at the ends they are drawn at.
                .ignoresSafeArea(.container, edges: .vertical)
            }
            .onChange(of: model.rows.last?.id) { _ in
                // A new message came: in view if the newest was, also when
                // scrolled up a little.
                if model.atBottom {
                    proxy.scrollTo(Self.bottomId, anchor: .top)
                }
            }
            .onChange(of: model.scrollTarget) { target in
                guard let target else { return }
                withAnimation {
                    proxy.scrollTo(target.rowId, anchor: .center)
                }
            }
            .overlay(alignment: .bottomTrailing) {
                if model.showsScrollToBottom {
                    ScrollToBottomButton {
                        withAnimation {
                            proxy.scrollTo(Self.bottomId, anchor: .top)
                        }
                    }
                    .transition(.scale.combined(with: .opacity))
                }
            }
            .animation(.easeOut(duration: 0.2), value: model.showsScrollToBottom)
        }
        .scrollDismissesKeyboard(.interactively)
        // The header's background: the timeline, drawn upside down, is not
        // blurred under it.
        .overlay(alignment: .top) {
            Color.clear
                .frame(height: 0)
                .background(.bar, ignoresSafeAreaEdges: .top)
        }
        .overlay {
            if model.opening {
                ProgressView()
            }
        }
        .overlay(alignment: .top) {
            if model.seeking {
                ProgressView()
                    .padding(10)
                    .background(.regularMaterial, in: Circle())
                    .padding(.top, 8)
                    .accessibilityLabel("Looking for the original message")
            }
        }
    }

    /// Newest first, each upside down again; `insets`, the bars', as drawn.
    private func rows(insets: EdgeInsets) -> some View {
        LazyVStack(spacing: 0) {
            bottom
            ForEach(model.rows.reversed()) { row in
                TimelineRowView(row: row, highlighted: model.highlighted == row.id, actions: actions)
                .equatable()
                .flipped()
                .id(row.id)
            }
            top
                .flipped()
        }
        .padding(.horizontal, 12)
        .padding(.top, insets.bottom)
        .padding(.bottom, insets.top + 8)
    }

    /// Above the oldest row: a spinner until the room's start.
    private var top: some View {
        Group {
            if model.reachedStart || model.opening {
                Color.clear.frame(height: 1)
            } else {
                ProgressView()
                    .frame(maxWidth: .infinity, minHeight: 44)
            }
        }
        .onAppear { model.setTopVisible(true) }
        .onDisappear { model.setTopVisible(false) }
    }

    /// Below the newest row, the very end of the timeline, to scroll to;
    /// before iOS 18, whether the lazy stack draws it says whether the
    /// timeline is at its bottom.
    private var bottom: some View {
        Color.clear
            .frame(height: 8)
            .id(Self.bottomId)
            .onAppear { model.setBottomDrawn(true) }
            .onDisappear { model.setBottomDrawn(false) }
    }
}

private extension View {
    /// Drawn and touched upside down.
    func flipped() -> some View {
        scaleEffect(x: 1, y: -1)
    }
}

/// From iOS 26, no blur where the scroll view meets the bars: drawn
/// upside down, it blurs the wrong end. The header has a background
/// instead.
private struct NoEdgeEffect: ViewModifier {
    func body(content: Content) -> some View {
        if #available(iOS 26, *) {
            content.scrollEdgeEffectHidden()
        } else {
            content
        }
    }
}

/// From iOS 18, how far the timeline is scrolled up from its bottom: its
/// offset from its start, drawn at the bottom.
private struct MeasuresBottom: ViewModifier {
    @ObservedObject var model: TimelineModel

    func body(content: Content) -> some View {
        if #available(iOS 18, *) {
            content
                .onScrollGeometryChange(for: CGFloat.self) { geometry in
                    geometry.contentOffset.y + geometry.contentInsets.top
                } action: { _, distance in
                    model.scrolled(distanceFromBottom: distance)
                }
        } else {
            content
        }
    }
}

/// A round button with a down arrow, over the timeline's lower right
/// corner: to the newest message.
struct ScrollToBottomButton: View {
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            Image(systemName: "arrow.down")
                .font(.body.weight(.semibold))
                .foregroundColor(.accentColor)
                .frame(width: 40, height: 40)
                .background(.regularMaterial, in: Circle())
                .overlay(Circle().strokeBorder(Color.secondary.opacity(0.25), lineWidth: 0.5))
                .shadow(color: .black.opacity(0.15), radius: 3, y: 1)
        }
        .padding(12)
        .accessibilityLabel("Scroll to the newest message")
    }
}

/// Who types, in one line above the composer; it keeps its height when
/// nobody does, so the timeline does not jump.
struct TypingLine: View {
    let text: String?

    var body: some View {
        Text(text ?? " ")
            .font(.footnote)
            .foregroundStyle(.secondary)
            .lineLimit(1)
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(.horizontal, 16)
            .padding(.vertical, 3)
            .accessibilityHidden(text == nil)
    }
}

/// Writing a message (Markdown), or a reply or an edit, with a bar saying
/// which and a button to cancel it. The field grows to six lines, three at
/// the accessibility text sizes, leaving the timeline room.
struct Composer: View {
    @Binding var draft: String
    let compose: TimelineModel.Compose?
    let cancel: () -> Void
    let send: () -> Void
    @FocusState private var focused: Bool
    @ScaledMetric(relativeTo: .title) private var sendSize: CGFloat = 32
    @Environment(\.dynamicTypeSize) private var textSize

    private var blank: Bool {
        draft.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
    }

    var body: some View {
        VStack(spacing: 0) {
            if let compose {
                bar(compose)
            }
            HStack(alignment: .bottom, spacing: 8) {
                TextField("Message", text: $draft, axis: .vertical)
                    .lineLimit(1...(textSize.isAccessibilitySize ? 3 : 6))
                    .focused($focused)
                    .padding(.horizontal, 14)
                    .padding(.vertical, 8)
                    .background(Color(.secondarySystemBackground), in: RoundedRectangle(cornerRadius: 18))
                Button(action: send) {
                    Image(systemName: compose.isEdit ? "checkmark.circle.fill" : "arrow.up.circle.fill")
                        .font(.system(size: sendSize))
                }
                .disabled(blank)
                .accessibilityLabel(compose.isEdit ? "Save" : "Send")
            }
            .padding(.horizontal, 12)
            .padding(.vertical, 8)
        }
        .background(.bar)
        .onChange(of: compose) { compose in
            if compose != nil {
                focused = true
            }
        }
    }

    private func bar(_ compose: TimelineModel.Compose) -> some View {
        let reply: Bool
        if case .reply = compose { reply = true } else { reply = false }
        return HStack(spacing: 10) {
            Image(systemName: reply ? "arrowshape.turn.up.left" : "pencil")
                .foregroundColor(.accentColor)
            VStack(alignment: .leading, spacing: 1) {
                Text(reply ? "Replying to \(compose.target.sender)" : "Editing")
                    .font(.subheadline.weight(.semibold))
                    .lineLimit(1)
                Text(compose.target.preview)
                    .font(.subheadline)
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
            }
            Spacer(minLength: 0)
            Button(action: cancel) {
                Image(systemName: "xmark.circle.fill")
                    .font(.title3)
                    .foregroundStyle(.secondary)
            }
            .accessibilityLabel(reply ? "Cancel the reply" : "Cancel editing")
        }
        .padding(.horizontal, 16)
        .padding(.top, 8)
    }
}

private extension Optional where Wrapped == TimelineModel.Compose {
    var isEdit: Bool {
        if case .edit = self { return true }
        return false
    }
}
