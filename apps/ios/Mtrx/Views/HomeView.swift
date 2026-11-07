import MtrxFFI
import SwiftUI

/// Four tabs: Chats (direct chats), Rooms (the rooms, by space), Spaces
/// (the spaces, each with its rooms) and Settings.
/// Rooms open over their list, without the tab bar. A verification and an
/// invite to answer show over any of them.
struct HomeView: View {
    @EnvironmentObject private var app: AppModel
    @EnvironmentObject private var session: SessionModel
    @EnvironmentObject private var rooms: RoomListModel
    @EnvironmentObject private var home: HomeNavigation
    @EnvironmentObject private var encryption: EncryptionModel
    @EnvironmentObject private var presence: PresenceModel

    var body: some View {
        TabView(selection: Binding(get: { home.tab }, set: home.select)) {
            ChatsTab(openSettings: openSettings)
                .tabItem { Label("Chats", systemImage: "bubble.left.and.bubble.right") }
                .badge(rooms.chatsBadge)
                .tag(HomeTab.chats)
            RoomsTab(openSettings: openSettings)
                .tabItem { Label("Rooms", systemImage: "number") }
                .badge(rooms.roomsBadge)
                .tag(HomeTab.rooms)
            SpacesTab(openSettings: openSettings)
                .tabItem { Label("Spaces", systemImage: "square.stack.3d.up") }
                .badge(rooms.spacesBadge)
                .tag(HomeTab.spaces)
            SettingsView()
                .tabItem { Label("Settings", systemImage: "gearshape") }
                .tag(HomeTab.settings)
        }
        // Input anywhere in the app: not idle.
        .background(InputWatcher(onInput: presence.input))
        // Closed by its own buttons, which end the verification.
        .sheet(item: verification) { model in
            VerificationView(model: model)
                .account(session)
        }
        .sheet(item: $home.invite) { invite in
            if let room = rooms.room(invite.id) {
                InviteView(room: room)
                    .account(session)
            }
        }
        .alert("Signed out", isPresented: signedOutElsewhere) {
            Button("Sign in again") {
                Task { await app.forgetSession() }
            }
        } message: {
            Text("The server no longer accepts this session, e.g. because it was signed out on another device.")
        }
        .alert("Something went wrong", isPresented: hasError) {
            Button("OK") { session.error = nil }
        } message: {
            Text(session.error ?? "")
        }
    }

    /// The encryption banner leads to Settings.
    private func openSettings() {
        home.tab = .settings
    }

    private var verification: Binding<VerificationModel?> {
        Binding(get: { encryption.verification }, set: { _ in })
    }

    private var signedOutElsewhere: Binding<Bool> {
        Binding(get: { session.connection == .signedOut }, set: { _ in })
    }

    private var hasError: Binding<Bool> {
        Binding(get: { session.error != nil }, set: { if !$0 { session.error = nil } })
    }
}

/// The direct chats, most recently active first, with their people's
/// presence and the spaces we share with them.
struct ChatsTab: View {
    @EnvironmentObject private var rooms: RoomListModel
    @EnvironmentObject private var home: HomeNavigation
    @EnvironmentObject private var presence: PresenceModel
    let openSettings: () -> Void
    @State private var search = ""

    var body: some View {
        let lists = rooms.lists
        let searching = !search.isEmpty
        // The Notes to self at the top, unless searching: then among the
        // chats found.
        let chats = searching ? lists.search(key: nil, filter: search) : lists.chats()
        let chatsSection = ListSection(
            key: "chats", title: "", rooms: chats, folded: false, unread: 0, mentions: false, space: nil)
        NavigationStack(path: $home.chatsPath) {
            RoomList(sections: [chatsSection],
                     notes: searching ? nil : lists.notes(), searching: searching, openSettings: openSettings,
                     presence: presence.presence(of:),
                     subline: { room in
                         let spaces = lists.chatSpaces(chatId: room.id)
                         return spaces.isEmpty ? nil : spaces.joined(separator: ", ")
                     }) {
                ListPlaceholder(loaded: rooms.loaded, empty: "No chats yet")
            }
            .navigationTitle("Chats")
            .searchable(text: $search, prompt: "Search chats")
            .navigationDestination(for: String.self) { id in
                OpenRoom(id: id)
                    .backWithUnread(rooms.backBadge(besides: id)) {
                        home.chatsPath.removeAll { $0 == id }
                    }
                    .toolbar(.hidden, for: .tabBar)
            }
        }
    }
}

/// What the Spaces and Rooms tabs open over themselves.
enum SpacesSheet: Identifiable {
    case newSpace
    /// A new room in the space (ID and name), if any.
    case newRoom(parent: String?, name: String?)
    case join

    var id: String {
        switch self {
        case .newSpace: return "new space"
        case .newRoom(let parent, _): return "new room \(parent ?? "")"
        case .join: return "join"
        }
    }
}

/// The spaces, then Home; a tap shows the entry's rooms, with the
/// space's name on top (a tap on it shows its overview). A room opens in
/// the Rooms tab.
struct SpacesTab: View {
    @EnvironmentObject private var session: SessionModel
    @EnvironmentObject private var home: HomeNavigation
    let openSettings: () -> Void
    @State private var search = ""
    @State private var sheet: SpacesSheet?

    var body: some View {
        NavigationStack(path: $home.spacesPath) {
            SpacesList(search: search, openSettings: openSettings)
                .navigationTitle("Spaces")
                .searchable(text: $search, prompt: "Search spaces")
                .toolbar {
                    ToolbarItem(placement: .navigationBarTrailing) {
                        AddMenu(space: nil, sheet: $sheet)
                    }
                }
                .navigationDestination(for: SpaceKey.self) { entry in
                    SpaceRoomsPage(entry: entry, sheet: $sheet, openSettings: openSettings)
                }
        }
        .sheet(item: $sheet) { sheet in
            SpacesSheetView(sheet: sheet)
                .account(session)
        }
    }
}

/// The `+` of the Spaces tab: New space…, Join a space or room…, and in a
/// space we may add rooms to New room in it….
private struct AddMenu: View {
    /// The space shown, if any.
    let space: RoomSummary?
    @Binding var sheet: SpacesSheet?

    var body: some View {
        Menu {
            if let space, space.rights.addRooms {
                Button {
                    sheet = .newRoom(parent: space.id, name: space.name)
                } label: {
                    Label("New room in \(space.name)…", systemImage: "plus.bubble")
                }
            }
            Button {
                sheet = .newSpace
            } label: {
                Label("New space…", systemImage: "square.stack.3d.up")
            }
            Button {
                sheet = .join
            } label: {
                Label("Join a space or room…", systemImage: "magnifyingglass")
            }
        } label: {
            Image(systemName: "plus")
        }
        .accessibilityLabel("Add")
    }
}

/// A sheet of the Spaces tab, with what it made or joined opened after.
private struct SpacesSheetView: View {
    @EnvironmentObject private var session: SessionModel
    let sheet: SpacesSheet

    var body: some View {
        switch sheet {
        case .newSpace:
            NewRoomView(model: NewRoomModel(session: session.session, space: true))
        case .newRoom(let parent, let name):
            NewRoomView(
                model: NewRoomModel(
                    session: session.session, space: false,
                    parent: parent.flatMap { id in name.map { (id: id, name: $0) } }))
        case .join:
            JoinView(model: JoinModel(session: session.session, ownServer: session.serverName))
        }
    }
}

/// The top-level spaces, then Home; a tap shows the entry's rooms.
struct SpacesList: View {
    @EnvironmentObject private var rooms: RoomListModel
    let search: String
    let openSettings: () -> Void

    var body: some View {
        List {
            if search.isEmpty {
                EncryptionBannerRow(open: openSettings)
            }
            ForEach(rooms.lists.searchSpaces(filter: search)) { item in
                NavigationLink(value: item.key) {
                    SpaceRow(item: item)
                }
                .compactRow()
            }
        }
        .listStyle(.plain)
        .overlay {
            if rooms.lists.spaces().isEmpty {
                ListPlaceholder(loaded: rooms.loaded, empty: "No rooms yet")
            }
        }
        .safeAreaInset(edge: .bottom) { OfflineBanner() }
    }
}

/// A space's list: the Notes to self at the top (if kept there), then its
/// sections as the desktop arranges them, shown, folded and ordered as on
/// this phone, each in the chosen order; the space's name on top, a tap on
/// it shows the space's overview.
struct SpaceRoomsPage: View {
    @EnvironmentObject private var session: SessionModel
    @EnvironmentObject private var rooms: RoomListModel
    @EnvironmentObject private var home: HomeNavigation
    let entry: SpaceKey
    @Binding var sheet: SpacesSheet?
    let openSettings: () -> Void
    @State private var search = ""
    @State private var overview = false
    @State private var showing = false

    private var item: SpaceEntry? { rooms.lists.space(key: entry) }

    var body: some View {
        let sections = item.map { _ in
            rooms.lists.spaceList(
                key: entry, layout: home.sectionLayouts[entry].library, sort: home.roomSort, filter: search)
        } ?? []
        RoomList(sections: sections, notes: search.isEmpty ? rooms.lists.notes() : nil,
                 searching: !search.isEmpty, openSettings: openSettings, opensInRooms: true,
                 fold: { key in home.sectionLayouts.change(entry) { $0.toggleFold(key) } }) {
            if item != nil {
                Text("No rooms in this space").foregroundStyle(.secondary)
            } else {
                // A remembered or new space may come with sync.
                ProgressView("Loading…")
            }
        }
        .searchable(text: $search, prompt: "Search rooms")
        .navigationBarTitleDisplayMode(.inline)
        .toolbar {
            ToolbarItem(placement: .principal) {
                if let space = item?.space {
                    Button {
                        overview = true
                    } label: {
                        HStack(spacing: 6) {
                            AvatarView(name: space.name, uri: space.avatarUrl, size: 22, shape: .square)
                            Text(space.name)
                                .font(.headline)
                                .foregroundColor(.primary)
                                .lineLimit(1)
                            Image(systemName: "chevron.right")
                                .font(.caption.weight(.semibold))
                                .foregroundStyle(.secondary)
                        }
                    }
                    .accessibilityHint("Shows the space's overview")
                } else {
                    Text(item?.name ?? "").font(.headline)
                }
            }
            ToolbarItemGroup(placement: .navigationBarTrailing) {
                ViewMenu { showing = true }
                AddMenu(space: item?.space, sheet: $sheet)
            }
        }
        .fullScreenCover(isPresented: $overview) {
            if let space = item?.space {
                SpaceOverviewView(space: space)
                    .account(session)
            }
        }
        .sheet(isPresented: $showing) {
            if let item {
                ShowSheet(item: item)
                    .account(session)
            }
        }
    }
}

/// A space's picture and name, with a dot while any of its rooms has unread
/// messages.
private struct SpaceRow: View {
    let item: SpaceEntry
    @RowAvatarSize private var avatarSize: CGFloat

    var body: some View {
        HStack(spacing: 10) {
            if let space = item.space {
                AvatarView(name: space.name, uri: space.avatarUrl, size: avatarSize, shape: .square)
            } else {
                Image(systemName: "house")
                    .font(.body)
                    .foregroundStyle(.secondary)
                    .frame(width: avatarSize, height: avatarSize)
                    .background(Color.secondary.opacity(0.15), in: RoundedRectangle(cornerRadius: avatarSize * 0.25))
            }
            Text(item.name)
                .fontWeight(item.unread ? .semibold : .regular)
                .foregroundColor(.primary)
                .lineLimit(1, accessibility: 3)
            Spacer(minLength: 8)
            if item.unread {
                Circle()
                    .fill(Color.accentColor)
                    .frame(width: 9, height: 9)
                    .accessibilityLabel("Unread")
            }
        }
        .contentShape(Rectangle())
    }
}

/// How a space's list shows: Show… (its sections) and the order of the
/// spaces' rooms, the current one checked.
private struct ViewMenu: View {
    @EnvironmentObject private var home: HomeNavigation
    let show: () -> Void

    var body: some View {
        Menu {
            Button(action: show) {
                Label("Show…", systemImage: "checklist")
            }
            Picker("Sort", selection: Binding(get: { home.roomSort }, set: home.setRoomSort)) {
                ForEach(RoomSort.allCases) { sort in
                    Text(sort.title).tag(sort)
                }
            }
        } label: {
            Image(systemName: "line.3.horizontal.decrease.circle")
        }
        .accessibilityLabel("Show and sort")
    }
}

/// The Rooms tab: Home (the rooms in no space), then each space's rooms,
/// each newest first, folded as on this phone; a room opens over it. A
/// heading's caret folds or unfolds its group; a space's name shows the
/// space in the Spaces tab.
struct RoomsTab: View {
    @EnvironmentObject private var rooms: RoomListModel
    @EnvironmentObject private var home: HomeNavigation
    let openSettings: () -> Void

    var body: some View {
        NavigationStack(path: $home.roomsPath) {
            RoomList(sections: home.roomGroups, searching: false, openSettings: openSettings,
                     fold: home.toggleGroupFold, openSpace: { home.open($0) }) {
                ListPlaceholder(loaded: rooms.loaded, empty: "No rooms yet")
            }
            .navigationTitle("Rooms")
            .navigationDestination(for: String.self) { id in
                OpenRoom(id: id)
                    .backWithUnread(rooms.backBadge(besides: id)) {
                        home.roomsPath.removeAll { $0 == id }
                    }
                    .toolbar(.hidden, for: .tabBar)
            }
        }
    }
}

/// Rooms or chats, in sections with their titles, that open on a tap (in
/// their tabs if `opensInRooms`: rooms in Rooms, chats in Chats; else over
/// the list); an invite opens its answer. The Notes to self above them if
/// kept at the top, and the encryption banner (if `banner`), unless
/// searching. Where sections fold (`fold`), a tap on a heading folds or
/// unfolds it; where a space's heading shows the space (`openSpace`), a tap
/// on it does, and if it folds too, only a tap on its caret folds it.
struct RoomList<Empty: View>: View {
    @EnvironmentObject private var rooms: RoomListModel
    @EnvironmentObject private var home: HomeNavigation
    let sections: [ListSection]
    /// The Notes to self, at the top.
    var notes: RoomSummary?
    let searching: Bool
    var banner = true
    let openSettings: () -> Void
    /// The presence to show on a room's picture (Chats).
    var presence: (RoomSummary) -> Presence? = { _ in nil }
    /// The line under a room's name (Chats: the spaces shared with the
    /// person).
    var subline: (RoomSummary) -> String? = { _ in nil }
    /// Rooms open in their tabs (from a space's list).
    var opensInRooms = false
    /// Fold or unfold the section with this key; nil: sections don't fold.
    var fold: ((String) -> Void)?
    /// Show the space with this ID; nil: headings don't open spaces.
    var openSpace: ((String) -> Void)?
    @ViewBuilder let empty: () -> Empty

    var body: some View {
        List {
            if banner && !searching {
                EncryptionBannerRow(open: openSettings)
            }
            if let notes {
                rows([notes])
            }
            // Headings are rows of their own, not the list's section
            // headers, whose spacing is not ours to set.
            ForEach(sections) { section in
                if !section.title.isEmpty {
                    SectionHeading(section: section, title: section.title, action: headingAction(section))
                        .listRowInsets(EdgeInsets(top: 8, leading: 14, bottom: 3, trailing: 14))
                        .listRowSeparator(.hidden)
                }
                if !section.folded {
                    rows(section.rooms)
                }
            }
        }
        .listStyle(.plain)
        // Headings as low as their text; rooms keep their own height.
        .environment(\.defaultMinListRowHeight, 0)
        .overlay {
            if sections.allSatisfy(\.rooms.isEmpty) && notes == nil && !searching {
                empty()
            }
        }
        .safeAreaInset(edge: .bottom) { OfflineBanner() }
    }

    private func rows(_ rooms: [RoomSummary]) -> some View {
        ForEach(rooms, id: \.id) { room in
            Group {
                if room.membership == .invited {
                    Button {
                        home.invite = RoomRef(id: room.id)
                    } label: {
                        row(room)
                    }
                    .foregroundColor(.primary)
                } else if opensInRooms {
                    Button {
                        home.open(room.id)
                    } label: {
                        row(room)
                    }
                    .foregroundColor(.primary)
                } else {
                    NavigationLink(value: room.id) {
                        row(room)
                    }
                }
            }
            .compactRow()
        }
    }

    private func headingAction(_ section: ListSection) -> SectionHeading.Action? {
        let open = openSpace.flatMap { openSpace in section.space.map { space in { openSpace(space) } } }
        switch (fold.map { fold in { fold(section.key) } }, open) {
        case let (fold?, open?): return .foldOrOpen(fold: fold, open: open)
        case let (fold?, nil): return .fold(fold)
        case let (nil, open?): return .open(open)
        case (nil, nil): return nil
        }
    }

    private func row(_ room: RoomSummary) -> RoomRow {
        RoomRow(
            room: room, presence: presence(room), draft: rooms.drafts.contains(room.id),
            subline: subline(room))
    }
}

/// A section's heading, a compact row of the list: where it folds, a
/// caret, its title, and while folded its rooms' unread messages, as a
/// badge in the unread colour; a tap folds or unfolds it. One that does
/// not fold always shows its unread messages; a space's (`open`) leads to
/// the space. One that does both (`foldOrOpen`) folds by its caret and
/// leads to the space by the rest.
struct SectionHeading: View {
    let section: ListSection
    let title: String
    let action: Action?

    /// What a tap on it does.
    enum Action {
        case fold(() -> Void)
        /// Show its space.
        case open(() -> Void)
        /// Its caret folds it, the rest shows its space.
        case foldOrOpen(fold: () -> Void, open: () -> Void)
    }

    var body: some View {
        switch action {
        case .fold(let fold):
            Button(action: fold) {
                content(caret: true, chevron: false)
            }
            .buttonStyle(.plain)
            .accessibilityElement(children: .ignore)
            .accessibilityLabel(title)
            .accessibilityValue(section.folded ? "Folded" : "Unfolded")
            .accessibilityAddTraits([.isHeader, .isButton])
            .accessibilityHint(section.folded ? "Shows its rooms" : "Hides its rooms")
            .accessibilityAction(named: section.folded ? "Unfold" : "Fold", fold)
        case .open(let open):
            openButton(open)
        case .foldOrOpen(let fold, let open):
            HStack(spacing: 6) {
                Button(action: fold) {
                    caret
                        .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .foregroundStyle(.secondary)
                .accessibilityLabel(section.folded ? "Unfold \(title)" : "Fold \(title)")
                openButton(open)
            }
        case nil:
            content(caret: false, chevron: false)
                .accessibilityElement(children: .combine)
                .accessibilityAddTraits(.isHeader)
        }
    }

    private func openButton(_ open: @escaping () -> Void) -> some View {
        Button(action: open) {
            content(caret: false, chevron: true)
        }
        .buttonStyle(.plain)
        .accessibilityElement(children: .combine)
        .accessibilityAddTraits([.isHeader, .isButton])
        .accessibilityHint("Shows the space")
    }

    private var showsUnread: Bool {
        switch action {
        case .fold, .foldOrOpen: return section.folded
        case .open, nil: return true
        }
    }

    /// Large enough to tap on its own.
    private var caret: some View {
        Image(systemName: "chevron.down")
            .font(.caption2.weight(.bold))
            .rotationEffect(.degrees(section.folded ? -90 : 0))
            .frame(minWidth: 18, minHeight: 18)
    }

    private func content(caret: Bool, chevron: Bool) -> some View {
        HStack(spacing: 6) {
            if caret {
                self.caret
            }
            Text(title)
                .font(.footnote.weight(.semibold))
            if chevron {
                Image(systemName: "chevron.right")
                    .font(.caption2.weight(.bold))
            }
            Spacer(minLength: 8)
            let unread = section.unread
            if showsUnread && unread > 0 {
                Text(unread > 999 ? "999+" : String(unread))
                    .font(.caption2.weight(.semibold))
                    .monospacedDigit()
                    .foregroundColor(.white)
                    .padding(.horizontal, 6)
                    .padding(.vertical, 1)
                    .frame(minWidth: 18)
                    .background(section.mentions ? Color.red : Color.accentColor, in: Capsule())
                    .accessibilityLabel("\(unread) unread")
            }
        }
        .foregroundStyle(.secondary)
        .contentShape(Rectangle())
    }
}

/// Before the room list loads, or when a loaded list is empty.
struct ListPlaceholder: View {
    let loaded: Bool
    let empty: String

    var body: some View {
        if loaded {
            Text(empty).foregroundStyle(.secondary)
        } else {
            ProgressView("Loading…")
        }
    }
}

extension View {
    /// A list row with less space around it than the default.
    func compactRow() -> some View {
        listRowInsets(EdgeInsets(top: 5, leading: 14, bottom: 5, trailing: 14))
    }

    /// A back button with a number: unread messages elsewhere.
    func backWithUnread(_ count: Int, back: @escaping () -> Void) -> some View {
        modifier(BackWithUnread(count: count, back: back))
    }
}

/// The back button of a room: a chevron and the unread messages in the
/// other rooms. Swiping from the edge still goes back (`swipeBack`).
private struct BackWithUnread: ViewModifier {
    let count: Int
    let back: () -> Void

    func body(content: Content) -> some View {
        content
            .navigationBarBackButtonHidden(true)
            .swipeBack()
            .toolbar {
                ToolbarItem(placement: .navigationBarLeading) {
                    Button(action: back) {
                        HStack(spacing: 4) {
                            Image(systemName: "chevron.left")
                                .font(.body.weight(.semibold))
                            if count > 0 {
                                Text(count > 999 ? "999+" : String(count))
                                    .font(.caption.weight(.semibold))
                                    .monospacedDigit()
                                    .foregroundColor(.white)
                                    .padding(.horizontal, 6)
                                    .padding(.vertical, 1)
                                    .background(Color.accentColor, in: Capsule())
                            }
                        }
                    }
                    .accessibilityLabel(count > 0 ? "Back, \(count) unread" : "Back")
                }
            }
    }
}

/// A room by ID: its invite to answer, its timeline, or a word while it is
/// coming (just made or joined) or gone. Its state (the timeline's and the
/// info's models) is the room's: a navigation stack keeps a page's state
/// when its path changes to another room at the same place (a room opened
/// from elsewhere over another, `HomeNavigation.open`).
struct OpenRoom: View {
    @EnvironmentObject private var session: SessionModel
    @EnvironmentObject private var rooms: RoomListModel
    let id: String

    var body: some View {
        content.id(id)
    }

    @ViewBuilder
    private var content: some View {
        if let room = rooms.room(id), room.membership != .invited {
            RoomView(
                room: room, model: TimelineModel(session: session.session, room: room),
                info: RoomInfoModel(session: session.session, roomId: room.id, userId: session.userId))
        } else if let room = rooms.room(id) {
            InviteView(room: room)
        } else if rooms.joining.contains(id) || !rooms.loaded {
            ProgressView("Loading…")
        } else {
            Text("This room is not in your list.")
                .foregroundStyle(.secondary)
        }
    }
}

/// The encryption banner as the first row of a list, while it shows.
private struct EncryptionBannerRow: View {
    @EnvironmentObject private var encryption: EncryptionModel
    let open: () -> Void

    var body: some View {
        if encryption.needsSetup {
            EncryptionBanner(open: open)
                .listRowSeparator(.hidden)
                .listRowInsets(EdgeInsets(top: 6, leading: 12, bottom: 6, trailing: 12))
        }
    }
}

/// While this device cannot read all encrypted messages; a tap opens the
/// encryption settings.
struct EncryptionBanner: View {
    let open: () -> Void

    var body: some View {
        Button(action: open) {
            HStack(spacing: 10) {
                Image(systemName: "lock.trianglebadge.exclamationmark")
                    .font(.title3)
                Text("Verify this device or enter your recovery key to read encrypted messages.")
                    .font(.footnote)
                    .multilineTextAlignment(.leading)
                Spacer(minLength: 0)
                Image(systemName: "chevron.right")
                    .font(.footnote.weight(.semibold))
                    .foregroundStyle(.secondary)
            }
            .padding(12)
            .background(Color.orange.opacity(0.15), in: RoundedRectangle(cornerRadius: 12))
        }
        .buttonStyle(.plain)
    }
}

/// While the server is unreachable.
struct OfflineBanner: View {
    @EnvironmentObject private var session: SessionModel

    var body: some View {
        if session.connection == .offline {
            Label("Offline – reconnecting…", systemImage: "wifi.slash")
                .font(.footnote)
                .padding(.horizontal, 14)
                .padding(.vertical, 8)
                .background(.regularMaterial, in: Capsule())
                .padding(.bottom, 8)
        }
    }
}
