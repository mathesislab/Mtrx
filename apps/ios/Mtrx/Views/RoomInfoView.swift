import MtrxFFI
import SwiftUI

/// A room's or space's info, full screen: its picture, name (renamed in
/// place where we may) and address, Invite… and Leave…, the topic with
/// Edit, its settings (each only where we may change it) and its members.
struct RoomInfoView: View {
    let room: RoomSummary
    @ObservedObject var model: RoomInfoModel
    @EnvironmentObject private var session: SessionModel
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        NavigationStack {
            List {
                RoomInfoHeader(room: room, model: model) {
                    dismiss()
                }
                TopicSection(room: room, model: model)
                RoomSettingsSections(room: room, model: model)
                MembersSection(room: room, model: model)
            }
            .listStyle(.insetGrouped)
            .navigationTitle(room.kind == .space ? "Space info" : "Room info")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .confirmationAction) {
                    Button("Done") { dismiss() }
                }
            }
            // The settings follow the library; the members are read.
            .refreshable { await model.loadMembers() }
        }
        .task { await model.load() }
        .infoErrors(model)
    }
}

extension View {
    /// The info's failed changes, as an alert.
    func infoErrors(_ model: RoomInfoModel) -> some View {
        modifier(InfoErrors(model: model))
    }
}

private struct InfoErrors: ViewModifier {
    @ObservedObject var model: RoomInfoModel

    func body(content: Content) -> some View {
        content.alert(
            "Something went wrong",
            isPresented: Binding(get: { model.error != nil }, set: { if !$0 { model.error = nil } })
        ) {
            Button("OK") { model.error = nil }
        } message: {
            Text(model.error ?? "")
        }
    }
}

/// Picture, name (a field where we may rename it), the address under it,
/// and Invite… (where we may invite) and Leave…. `left`: after leaving.
struct RoomInfoHeader: View {
    let room: RoomSummary
    @ObservedObject var model: RoomInfoModel
    let left: () -> Void
    @State private var name = ""

    private var address: String? {
        model.settings?.address ?? room.alias
    }

    var body: some View {
        Section {
            VStack(spacing: 8) {
                AvatarView(
                    name: room.name, uri: room.avatarUrl, size: 64, shape: room.kind == .space ? .square : .circle)
                if room.rights.rename {
                    HStack {
                        TextField("Room name", text: $name)
                            .font(.title3.weight(.semibold))
                            .multilineTextAlignment(.center)
                            .submitLabel(.done)
                            .onSubmit(rename)
                        if changed {
                            Button("Save", action: rename)
                                .buttonStyle(.borderedProminent)
                                .controlSize(.small)
                        }
                    }
                } else {
                    Text(room.name)
                        .font(.title3.weight(.semibold))
                        .multilineTextAlignment(.center)
                }
                if let address {
                    Text(address)
                        .font(.caption)
                        .foregroundStyle(.secondary)
                        .textSelection(.enabled)
                }
                InviteLeaveButtons(room: room, model: model, left: left)
                    .padding(.top, 4)
            }
            .frame(maxWidth: .infinity)
            .listRowBackground(Color.clear)
        }
        .onAppear { name = room.name }
        .onChange(of: room.name) { name = $0 }
    }

    private var changed: Bool {
        let name = name.trimmingCharacters(in: .whitespaces)
        return !name.isEmpty && name != room.name
    }

    private func rename() {
        guard changed else { return }
        model.rename(name)
    }
}

/// Invite… (prominent, where we may invite) and Leave… (grey, asking
/// first). `left`: after leaving.
struct InviteLeaveButtons: View {
    let room: RoomSummary
    @ObservedObject var model: RoomInfoModel
    let left: () -> Void
    @EnvironmentObject private var session: SessionModel
    @State private var inviting = false
    @State private var leaving = false

    var body: some View {
        AdaptiveStack(spacing: 10) {
            if room.rights.invite {
                Button {
                    inviting = true
                } label: {
                    Label("Invite…", systemImage: "person.badge.plus")
                        .frame(maxWidth: .infinity)
                }
                .buttonStyle(.bordered)
                .tint(.accentColor)
            }
            Button {
                leaving = true
            } label: {
                Label("Leave…", systemImage: "rectangle.portrait.and.arrow.right")
                    .frame(maxWidth: .infinity)
            }
            .buttonStyle(.bordered)
            .tint(.gray)
        }
        .sheet(isPresented: $inviting) {
            InviteSheet(name: room.name, model: model)
        }
        .confirmationDialog("Leave \(room.name)?", isPresented: $leaving, titleVisibility: .visible) {
            Button("Leave", role: .destructive) {
                Task {
                    if await model.leave() {
                        session.left(room.id)
                        left()
                    }
                }
            }
        } message: {
            Text("You can only come back if the room is public or someone invites you.")
        }
    }
}

/// The topic and its links in a box, with Edit where we may change it.
struct TopicSection: View {
    let room: RoomSummary
    @ObservedObject var model: RoomInfoModel

    var body: some View {
        let topic = parseTopic(topic: room.topic ?? "")
        Section {
            TopicText(topic: topic)
            ForEach(Array(topic.links.enumerated()), id: \.offset) { _, link in
                LinkButton(link: link)
            }
        } header: {
            HStack {
                Text("Topic")
                Spacer()
                if room.rights.changeTopic {
                    NavigationLink("Edit") {
                        TopicEditor(draft: TopicDraft(topic), info: model)
                    }
                    .font(.subheadline)
                    .textCase(nil)
                }
            }
        }
    }
}

/// A room's settings, each only where we may change it, its spaces, its
/// facts and who is banned.
struct RoomSettingsSections: View {
    let room: RoomSummary
    @ObservedObject var model: RoomInfoModel
    @EnvironmentObject private var session: SessionModel
    @State private var address = ""
    @State private var encrypting = false

    var body: some View {
        if let settings = model.settings {
            settingsSection(settings)
            spacesSection(settings)
            Section {
                Text(RoomFacts.line(room, settings))
                    .font(.footnote)
                    .foregroundStyle(.secondary)
                    .textSelection(.enabled)
            }
            if !settings.banned.isEmpty {
                bannedSection(settings)
            }
        } else if let error = model.loadError {
            Section {
                Text(error).foregroundColor(.red).font(.footnote)
            }
        } else {
            Section {
                ProgressView().frame(maxWidth: .infinity)
            }
        }
    }

    private func settingsSection(_ settings: RoomSettings) -> some View {
        let direct = room.kind == .direct
        return Section("Settings") {
            Picker("Notifications", selection: Binding(get: { settings.notify }, set: { model.setNotify($0) })) {
                ForEach(settings.notifyChoices, id: \.mode) { choice in
                    Text(choice.title).tag(choice.mode)
                }
            }
            if room.rights.changeJoinRule && !direct {
                Picker("Who can join", selection: Binding(get: { settings.joinRule }, set: { model.setJoinRule($0) })) {
                    ForEach(settings.joinRuleChoices, id: \.rule) { choice in
                        Text(choice.title).tag(choice.rule)
                    }
                }
            }
            if let listed = settings.listed, room.rights.changeJoinRule, settings.joinRule == .public || listed {
                Toggle("Listed in the directory", isOn: Binding(get: { listed }, set: { model.setListed($0) }))
            }
            ForEach(settings.spaces.filter(\.canChange), id: \.spaceId) { link in
                Toggle(
                    "Suggested in \(session.room(link.spaceId)?.name ?? "the space")",
                    isOn: Binding(get: { link.suggested }, set: { model.setSuggested(in: link.spaceId, $0) }))
            }
            if room.rights.changeHistory && !direct {
                Picker(
                    "Who can read the history",
                    selection: Binding(get: { settings.history }, set: { model.setHistory($0) })
                ) {
                    ForEach(settings.historyChoices, id: \.history) { choice in
                        Text(choice.title).tag(choice.history)
                    }
                }
            }
            if room.rights.changeLevels && !direct {
                Picker(
                    "Who can edit",
                    selection: Binding(get: { settings.permissions }, set: { model.setPermissions($0) })
                ) {
                    ForEach(settings.permissionsChoices, id: \.permissions) { choice in
                        Text(choice.title).tag(choice.permissions)
                    }
                }
            }
            if room.kind != .space {
                LabeledContent("Encryption") {
                    if settings.encrypted {
                        Text("On")
                    } else if room.rights.encrypt {
                        Button("Turn on…") { encrypting = true }
                    } else {
                        Text("Off")
                    }
                }
                .confirmationDialog("Encrypt \(room.name)?", isPresented: $encrypting, titleVisibility: .visible) {
                    Button("Encrypt") { model.encrypt() }
                } message: {
                    Text("Only members can read messages. Cannot be turned off later.")
                }
            }
            addressRow(settings)
        }
    }

    @ViewBuilder
    private func addressRow(_ settings: RoomSettings) -> some View {
        if room.rights.changeAddress && room.kind != .direct {
            VStack(alignment: .leading, spacing: 6) {
                Text("Address")
                HStack(spacing: 2) {
                    Text("#").foregroundStyle(.secondary)
                    TextField("name", text: $address)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                        .onSubmit(saveAddress)
                    Text(":\(session.serverName)")
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                    if address.trimmingCharacters(in: .whitespaces) != Self.addressName(settings) {
                        Button("Save", action: saveAddress)
                            .buttonStyle(.borderedProminent)
                            .controlSize(.small)
                            .padding(.leading, 6)
                    }
                }
                .font(.callout.monospaced())
            }
            .onAppear { address = Self.addressName(settings) }
            .onChange(of: settings.address) { _ in address = Self.addressName(settings) }
        } else {
            LabeledContent(settings.address == nil ? "Room ID" : "Address") {
                Text(settings.address ?? room.id)
                    .textSelection(.enabled)
            }
        }
    }

    private func saveAddress() {
        model.setAddress(address)
    }

    /// The name of the address, if on our server: "ice" of "#ice:example.org".
    private static func addressName(_ settings: RoomSettings) -> String {
        guard let address = settings.address, address.hasPrefix("#"),
              let colon = address.firstIndex(of: ":")
        else { return "" }
        return String(address[address.index(after: address.startIndex)..<colon])
    }

    @ViewBuilder
    private func spacesSection(_ settings: RoomSettings) -> some View {
        if room.kind != .direct && !(settings.spaces.isEmpty && settings.canAddTo.isEmpty) {
            Section("Spaces") {
                if settings.spaces.isEmpty {
                    Text("In no space").foregroundStyle(.secondary)
                } else {
                    FlowLayout(spacing: 6) {
                        ForEach(settings.spaces, id: \.spaceId) { link in
                            SpaceChip(name: session.room(link.spaceId)?.name ?? link.spaceId, removable: link.canChange) {
                                model.removeFromSpace(link.spaceId)
                            }
                        }
                    }
                    .padding(.vertical, 2)
                }
                if !settings.canAddTo.isEmpty {
                    Menu {
                        ForEach(settings.canAddTo, id: \.self) { space in
                            Button(session.room(space)?.name ?? space) {
                                model.addToSpace(space)
                            }
                        }
                    } label: {
                        Label("Add to space…", systemImage: "plus")
                    }
                }
            }
        }
    }

    private func bannedSection(_ settings: RoomSettings) -> some View {
        Section("Banned") {
            ForEach(settings.banned, id: \.user.userId) { banned in
                HStack {
                    VStack(alignment: .leading, spacing: 1) {
                        Text(banned.user.displayName)
                        Text(banned.reason.map { "\(banned.user.userId) · \($0)" } ?? banned.user.userId)
                            .font(.caption)
                            .foregroundStyle(.secondary)
                            .textSelection(.enabled)
                    }
                    Spacer()
                    if room.rights.ban {
                        Button("Unban") { model.unban(banned.user.userId) }
                            .buttonStyle(.bordered)
                            .controlSize(.small)
                    }
                }
            }
        }
    }
}

/// A space a room is in, with × to take it out where we may.
private struct SpaceChip: View {
    let name: String
    let removable: Bool
    let remove: () -> Void

    var body: some View {
        HStack(spacing: 4) {
            Text(name)
                .font(.subheadline)
                .lineLimit(1)
            if removable {
                Button(action: remove) {
                    Image(systemName: "xmark")
                        .font(.caption2.weight(.bold))
                }
                .buttonStyle(.borderless)
                .accessibilityLabel("Take it out of \(name)")
            }
        }
        .padding(.horizontal, 10)
        .padding(.vertical, 5)
        .background(Color.accentColor.opacity(0.12), in: Capsule())
    }
}

/// "Room · Public: anyone can join · 4 members · encrypted · room version 12".
enum RoomFacts {
    static func line(_ room: RoomSummary, _ settings: RoomSettings) -> String {
        var facts: [String] = []
        switch room.kind {
        case .space: facts.append("Space")
        case .direct: facts.append("Direct chat")
        case .room: facts.append("Room")
        }
        if room.kind != .direct {
            facts.append(settings.joinRuleFact)
        }
        facts.append(room.joinedMembers == 1 ? "1 member" : "\(room.joinedMembers) members")
        if room.kind != .space {
            facts.append(settings.encrypted ? "encrypted" : "not encrypted")
        }
        if let version = settings.version {
            facts.append("room version \(version)")
        }
        return facts.joined(separator: " · ")
    }
}

/// Inviting people to a room by user ID.
struct InviteSheet: View {
    let name: String
    @ObservedObject var model: RoomInfoModel
    @Environment(\.dismiss) private var dismiss
    @State private var person = ""
    @State private var invited: [String] = []
    @State private var sending = false

    var body: some View {
        NavigationStack {
            Form {
                Section {
                    HStack {
                        TextField("@user:server", text: $person)
                            .textInputAutocapitalization(.never)
                            .autocorrectionDisabled()
                            .keyboardType(.emailAddress)
                            .onSubmit(invite)
                        if sending {
                            ProgressView()
                        } else {
                            Button("Invite", action: invite)
                                .disabled(!NewRoomModel.isUserId(person.trimmingCharacters(in: .whitespaces)))
                        }
                    }
                } footer: {
                    if !invited.isEmpty {
                        Text("Invited \(invited.joined(separator: ", ")).")
                    }
                }
            }
            .navigationTitle("Invite to \(name)")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .confirmationAction) {
                    Button("Done") { dismiss() }
                }
            }
        }
        .presentationDetents([.medium])
    }

    private func invite() {
        let id = person.trimmingCharacters(in: .whitespaces)
        guard NewRoomModel.isUserId(id), !sending else { return }
        sending = true
        Task {
            if await model.invite(id) {
                invited.append(id)
                person = ""
            }
            sending = false
        }
    }
}
