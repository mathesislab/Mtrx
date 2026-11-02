import MtrxFFI
import SwiftUI

/// A space's overview, full screen: its picture, name and topic, then its
/// Rooms (to join or open), Members, or Settings. A joined subspace opens
/// its own overview over it.
struct SpaceOverviewView: View {
    let space: RoomSummary
    @EnvironmentObject private var session: SessionModel
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        NavigationStack {
            SpaceOverviewPage(
                space: space,
                overview: SpaceOverviewModel(
                    session: session.session, spaceId: space.id, cache: session.spaceChildren),
                info: session.info(for: space), close: { dismiss() }
            )
            .navigationDestination(for: RoomRef.self) { ref in
                if let subspace = session.room(ref.id) {
                    SpaceOverviewPage(
                        space: subspace,
                        overview: SpaceOverviewModel(
                            session: session.session, spaceId: subspace.id, cache: session.spaceChildren),
                        info: session.info(for: subspace), close: { dismiss() })
                }
            }
        }
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
        .task { await overview.load() }
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
                let topic = parseTopic(topic: info.shownTopic(space.topic))
                if !topic.text.isEmpty {
                    Text(Formatting.linked(topic.text))
                        .font(.callout)
                        .foregroundStyle(.secondary)
                        .multilineTextAlignment(.center)
                        .lineLimit(4)
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
                    Task { await model.load() }
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
    @EnvironmentObject private var session: SessionModel

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
                        Task { await model.load() }
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
                DirectoryRoomRow(room: model.shown(room), action: model.action(room), linksSubspaces: true) {
                    act(on: room)
                }
            }
        } footer: {
            Text("Rooms only invited people can join aren't listed.")
        }
    }

    private func act(on room: DirectoryRoom) {
        switch model.action(room) {
        case .join, .askToJoin:
            Task {
                if let id = await model.join(room) {
                    session.expect(id)
                }
            }
        case .open:
            open(room)
        case .invited:
            close()
            session.invite = RoomRef(id: room.id)
        case .asked, .inviteOnly, .busy:
            break
        }
    }

    private func open(_ room: DirectoryRoom) {
        if room.isSpace && session.lists.space(key: .space(id: room.id)) == nil {
            // A subspace: its own overview, over this one.
            return
        }
        close()
        session.open(room.id, kind: room.isSpace ? .space : .room)
    }
}

/// A room or space to join: its picture (else an icon for who may join),
/// name, details and our role, and what we may do.
struct DirectoryRoomRow: View {
    let room: DirectoryRoom
    let action: DirectoryRows.Action
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
            button
        }
        .contentShape(Rectangle())
        .onTapGesture {
            if action == .open { act() }
        }
        .modifier(OpensSubspace(room: room, action: action, enabled: linksSubspaces))
    }

    @ViewBuilder
    private var picture: some View {
        if room.avatarUrl != nil {
            AvatarView(name: room.name, uri: room.avatarUrl, size: avatarSize, shape: room.isSpace ? .square : .circle)
        } else {
            Image(systemName: Self.symbol(room.access))
                .font(.body)
                .foregroundStyle(.secondary)
                .frame(width: avatarSize, height: avatarSize)
                .background(
                    Color.secondary.opacity(0.12),
                    in: RoundedRectangle(cornerRadius: avatarSize * (room.isSpace ? 0.25 : 0.5)))
                .accessibilityLabel(Self.accessLabel(room.access))
        }
    }

    @ViewBuilder
    private var button: some View {
        if action.pressable {
            if action.primary {
                Button(action.title, action: act)
                    .buttonStyle(.borderedProminent)
                    .controlSize(.small)
            } else {
                Button(action.title, action: act)
                    .buttonStyle(.bordered)
                    .controlSize(.small)
            }
        } else {
            HStack(spacing: 4) {
                if case .busy = action {
                    ProgressView().controlSize(.small)
                }
                Text(action.title)
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
        }
    }

    static func symbol(_ access: JoinAccess) -> String {
        switch access {
        case .public: return "number"
        case .spaceMembers: return "person.2"
        case .knock: return "door.left.hand.open"
        case .invite: return "lock"
        }
    }

    static func accessLabel(_ access: JoinAccess) -> String {
        switch access {
        case .public: return "Anyone can join"
        case .spaceMembers: return "Space members can join"
        case .knock: return "Ask to join"
        case .invite: return "Invited people only"
        }
    }
}

/// A joined subspace not in the Spaces tab opens its overview: a link in
/// the overview's navigation stack.
private struct OpensSubspace: ViewModifier {
    let room: DirectoryRoom
    let action: DirectoryRows.Action
    let enabled: Bool
    @EnvironmentObject private var session: SessionModel

    func body(content: Content) -> some View {
        if enabled && room.isSpace && action == .open && session.lists.space(key: .space(id: room.id)) == nil
            && session.room(room.id) != nil
        {
            NavigationLink(value: RoomRef(id: room.id)) {
                content
            }
        } else {
            content
        }
    }
}
