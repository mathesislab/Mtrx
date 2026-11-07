import MtrxFFI
import SwiftUI

/// A space's overview, full screen: its picture, name, topic and links, then its
/// Rooms (to join or open), Members, or Settings. A joined subspace opens
/// its own overview over it.
struct SpaceOverviewView: View {
    let space: RoomSummary
    @EnvironmentObject private var session: SessionModel
    @EnvironmentObject private var rooms: RoomListModel
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        NavigationStack {
            page(space)
                .navigationDestination(for: RoomRef.self) { ref in
                    if let subspace = rooms.room(ref.id) {
                        page(subspace)
                    }
                }
        }
        .opensRoomLinks()
    }

    private func page(_ space: RoomSummary) -> SpaceOverviewPage {
        SpaceOverviewPage(
            space: space,
            overview: SpaceOverviewModel(session: session.session, spaceId: space.id),
            info: RoomInfoModel(session: session.session, roomId: space.id, userId: session.userId),
            close: { dismiss() })
    }
}

/// The parts of a space's overview.
enum OverviewTab: Hashable {
    case rooms
    case members
    case settings
}

/// One space's overview page: header, the segmented control, and the part
/// chosen. `close` closes the overview (to open a room, or after leaving).
struct SpaceOverviewPage: View {
    let space: RoomSummary
    @StateObject private var overview: SpaceOverviewModel
    @StateObject private var info: RoomInfoModel
    let close: () -> Void
    @EnvironmentObject private var session: SessionModel
    @State private var tab = OverviewTab.rooms

    init(
        space: RoomSummary, overview: @autoclosure @escaping () -> SpaceOverviewModel,
        info: @autoclosure @escaping () -> RoomInfoModel, close: @escaping () -> Void,
        tab: OverviewTab = .rooms
    ) {
        self.space = space
        _overview = StateObject(wrappedValue: overview())
        _info = StateObject(wrappedValue: info())
        self.close = close
        _tab = State(initialValue: tab)
    }

    var body: some View {
        List {
            header
            Section {
                Picker("Show", selection: $tab) {
                    Text("Rooms").tag(OverviewTab.rooms)
                    Text("Members").tag(OverviewTab.members)
                    Text("Settings").tag(OverviewTab.settings)
                }
                .pickerStyle(.segmented)
                .listRowBackground(Color.clear)
                .listRowInsets(EdgeInsets())
            }
            switch tab {
            case .rooms:
                SpaceRoomsSection(space: space, model: overview, close: close)
            case .members:
                MembersSection(room: space, model: info)
            case .settings:
                Section {
                    InviteLeaveButtons(room: space, model: info, left: close)
                        .listRowBackground(Color.clear)
                        .listRowInsets(EdgeInsets())
                }
                TopicSection(room: space, model: info)
                RoomSettingsSections(room: space, model: info)
            }
        }
        .listStyle(.insetGrouped)
        .navigationTitle(space.name)
        .navigationBarTitleDisplayMode(.inline)
        .toolbar {
            ToolbarItem(placement: .confirmationAction) {
                Button("Done", action: close)
            }
        }
        // Below the list, which ends above it: the last row stays clear.
        .safeAreaInset(edge: .bottom, alignment: .trailing) {
            if tab == .rooms {
                // Our server lists the space's rooms, as on the desktop.
                UpdatePill(model: overview, server: session.serverName)
            }
        }
        .task { overview.load() }
        .task { await info.load() }
        .infoErrors(info)
        .alert(
            "Something went wrong",
            isPresented: Binding(get: { overview.error != nil }, set: { if !$0 { overview.error = nil } })
        ) {
            Button("OK") { overview.error = nil }
        } message: {
            Text(overview.error ?? "")
        }
    }

    private var header: some View {
        Section {
            VStack(spacing: 6) {
                AvatarView(name: space.name, uri: space.avatarUrl, size: 56, shape: .square)
                Text(space.name)
                    .font(.title3.weight(.semibold))
                    .multilineTextAlignment(.center)
                let topic = space.topic
                if !topic.text.isEmpty {
                    Text(Formatting.linked(topic.text))
                        .font(.callout)
                        .foregroundStyle(.secondary)
                        .multilineTextAlignment(.center)
                        .lineLimit(4)
                }
                if !topic.calls.isEmpty || !topic.links.isEmpty {
                    LinkChips(topic: topic)
                        .padding(.top, 2)
                }
            }
            .frame(maxWidth: .infinity)
            .listRowBackground(Color.clear)
        }
    }
}

/// While a space's rooms shown are read again, at the lower right, clear of
/// the scrollbar: "Updating…", or "<server> didn't answer · Try again".
private struct UpdatePill: View {
    @ObservedObject var model: SpaceOverviewModel
    let server: String

    var body: some View {
        Group {
            if model.unknown {
                // Nothing shown yet: the list itself says so.
                EmptyView()
            } else if model.updating {
                HStack(spacing: 6) {
                    ProgressView().controlSize(.small)
                    Text("Updating…")
                }
                .pill()
            } else if model.failed {
                Button {
                    model.load()
                } label: {
                    Text("\(server) didn't answer · ").foregroundColor(.secondary) + Text("Try again")
                }
                .pill()
            }
        }
        .padding(.trailing, 20)
        .padding(.bottom, 8)
    }
}

private extension View {
    func pill() -> some View {
        font(.footnote)
            .padding(.horizontal, 12)
            .padding(.vertical, 7)
            .background(.regularMaterial, in: Capsule())
            .overlay(Capsule().strokeBorder(Color.secondary.opacity(0.2), lineWidth: 0.5))
    }
}

/// A space's rooms: a filter, the rooms as last listed with Join, Ask to
/// join or Open, and the state of reading them again.
private struct SpaceRoomsSection: View {
    let space: RoomSummary
    @ObservedObject var model: SpaceOverviewModel
    let close: () -> Void
    @EnvironmentObject private var rooms: RoomListModel
    @EnvironmentObject private var home: HomeNavigation

    var body: some View {
        Section {
            TextField("Filter rooms", text: $model.filter)
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled()
            if model.failed && model.unknown {
                HStack {
                    Text("Couldn't list the rooms.")
                        .foregroundStyle(.secondary)
                    Spacer()
                    Button("Try again") {
                        model.load()
                    }
                }
            }
            if model.unknown && model.updating {
                HStack(spacing: 8) {
                    ProgressView()
                    Text("Loading…").foregroundStyle(.secondary)
                }
            } else if model.shown.isEmpty && !model.failed {
                Text(model.rooms.isEmpty ? "No rooms you can see." : "No rooms match.")
                    .foregroundStyle(.secondary)
            }
            ForEach(model.shown, id: \.id) { room in
                DirectoryRoomRow(
                    room: room, button: rooms.button(for: room), busy: model.isBusy(room), linksSubspaces: true
                ) {
                    act(on: room)
                }
            }
        } footer: {
            Text("Rooms only invited people can join aren't listed.")
        }
    }

    private func act(on room: DirectoryRoom) {
        guard !model.isBusy(room) else { return }
        switch rooms.button(for: room).action {
        case .join, .askToJoin:
            model.join(room)
        case .open:
            open(room)
        case .invited:
            close()
            home.invite = RoomRef(id: room.id)
        case .asked, .inviteOnly:
            break
        }
    }

    private func open(_ room: DirectoryRoom) {
        if room.isSpace && rooms.lists.space(key: .space(id: room.id)) == nil {
            // A subspace: its own overview, over this one.
            return
        }
        close()
        home.open(room.id, kind: room.isSpace ? .space : .room)
    }
}

/// A room or space to join: its picture (else an icon for who may join),
/// name, details and our role, and what we may do.
struct DirectoryRoomRow: View {
    let room: DirectoryRoom
    let button: JoinButton
    /// Joining or asking to join, on its way.
    let busy: Bool
    /// A joined subspace not in the Spaces tab links to its overview (in an
    /// overview's navigation stack).
    var linksSubspaces = false
    let act: () -> Void
    @RowAvatarSize private var avatarSize: CGFloat

    var body: some View {
        AdaptiveStack(spacing: 10) {
            HStack(spacing: 10) {
                picture
                VStack(alignment: .leading, spacing: 1) {
                    AdaptiveStack(spacing: 6) {
                        Text(room.name)
                            .lineLimit(1, accessibility: 3)
                        if let role = DirectoryRows.role(room) {
                            Text(role)
                                .font(.caption2.weight(.semibold))
                                .foregroundStyle(.secondary)
                                .padding(.horizontal, 5)
                                .padding(.vertical, 1)
                                .background(Color.secondary.opacity(0.15), in: Capsule())
                        }
                    }
                    Text(DirectoryRows.details(room))
                        .font(.caption)
                        .foregroundStyle(.secondary)
                        .lineLimit(2, accessibility: 4)
                }
                Spacer(minLength: 8)
            }
            trailing
        }
        .contentShape(Rectangle())
        .onTapGesture {
            if button.action == .open && !busy { act() }
        }
        .modifier(OpensSubspace(room: room, action: button.action, enabled: linksSubspaces))
    }

    @ViewBuilder
    private var picture: some View {
        if room.avatarUrl != nil {
            AvatarView(name: room.name, uri: room.avatarUrl, size: avatarSize, shape: room.isSpace ? .square : .circle)
        } else {
            Image(systemName: Self.symbol(room.joinRule))
                .font(.body)
                .foregroundStyle(.secondary)
                .frame(width: avatarSize, height: avatarSize)
                .background(
                    Color.secondary.opacity(0.12),
                    in: RoundedRectangle(cornerRadius: avatarSize * (room.isSpace ? 0.25 : 0.5)))
                .accessibilityLabel(room.joinRuleFact)
        }
    }

    @ViewBuilder
    private var trailing: some View {
        let action = button.action
        if busy {
            HStack(spacing: 4) {
                ProgressView().controlSize(.small)
                Text(room.joinRule == .knock ? "Asking…" : "Joining…")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
        } else if action.pressable {
            if action.primary {
                Button(button.title, action: act)
                    .buttonStyle(.borderedProminent)
                    .controlSize(.small)
            } else {
                Button(button.title, action: act)
                    .buttonStyle(.bordered)
                    .controlSize(.small)
            }
        } else {
            Text(button.title)
                .font(.caption)
                .foregroundStyle(.secondary)
        }
    }

    static func symbol(_ rule: JoinRule) -> String {
        switch rule {
        case .public: return "number"
        case .spaceMembers: return "person.2"
        case .knock: return "door.left.hand.open"
        case .invite: return "lock"
        }
    }
}

/// A joined subspace not in the Spaces tab opens its overview: a link in
/// the overview's navigation stack.
private struct OpensSubspace: ViewModifier {
    let room: DirectoryRoom
    let action: JoinAction
    let enabled: Bool
    @EnvironmentObject private var rooms: RoomListModel

    func body(content: Content) -> some View {
        if enabled && room.isSpace && action == .open && rooms.lists.space(key: .space(id: room.id)) == nil
            && rooms.room(room.id) != nil
        {
            NavigationLink(value: RoomRef(id: room.id)) {
                content
            }
        } else {
            content
        }
    }
}
