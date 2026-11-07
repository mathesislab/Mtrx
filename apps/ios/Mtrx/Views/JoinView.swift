import MtrxFFI
import SwiftUI

/// Joining a space or room: a server's directory (ours, one browsed before,
/// or another), searched, all or only spaces or rooms; or the room an
/// address or link names, pasted into the search field.
struct JoinView: View {
    @StateObject private var model: JoinModel
    @EnvironmentObject private var rooms: RoomListModel
    @EnvironmentObject private var home: HomeNavigation
    @Environment(\.dismiss) private var dismiss
    @State private var askingServer = false
    @State private var typedServer = ""

    init(model: @autoclosure @escaping () -> JoinModel) {
        _model = StateObject(wrappedValue: model())
    }

    var body: some View {
        NavigationStack {
            List {
                Section {
                    serverMenu
                    TextField("Search, or paste #room:server or a link", text: $model.query)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                        .submitLabel(.search)
                        .onSubmit { model.searchSoon(delay: 0) }
                    Picker("Show", selection: $model.kind) {
                        Text("All").tag(DirectoryKind.all)
                        Text("Spaces").tag(DirectoryKind.spaces)
                        Text("Rooms").tag(DirectoryKind.rooms)
                    }
                    .pickerStyle(.segmented)
                }
                Section {
                    if let preview = model.preview {
                        row(preview)
                    }
                    ForEach(model.results, id: \.id) { room in
                        row(room)
                    }
                    if model.searching && model.results.isEmpty && model.preview == nil {
                        HStack(spacing: 8) {
                            ProgressView()
                            Text(model.message ?? "Looking…").foregroundStyle(.secondary)
                        }
                    } else if let message = model.message {
                        Text(message).foregroundStyle(.secondary)
                    }
                    if model.next != nil {
                        Button("More") {
                            Task { await model.more() }
                        }
                        .disabled(model.searching)
                    }
                }
            }
            .navigationTitle("Join a space or room")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .confirmationAction) {
                    Button("Done") { dismiss() }
                }
            }
        }
        .onChange(of: model.query) { _ in model.searchSoon() }
        .onChange(of: model.kind) { _ in model.searchSoon(delay: 0) }
        .task { model.searchSoon(delay: 0) }
        .alert("Other server", isPresented: $askingServer) {
            TextField("Server, such as uni-hamburg.de", text: $typedServer)
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled()
            Button("Browse") { model.choose(server: typedServer) }
            Button("Cancel", role: .cancel) {}
        }
        .alert(
            "Something went wrong",
            isPresented: Binding(get: { model.error != nil }, set: { if !$0 { model.error = nil } })
        ) {
            Button("OK") { model.error = nil }
        } message: {
            Text(model.error ?? "")
        }
    }

    /// The server whose directory is searched: ours, those browsed before,
    /// or another.
    private var serverMenu: some View {
        Menu {
            Button {
                model.choose(server: nil)
            } label: {
                choice(model.ownServer, chosen: model.server == nil)
            }
            ForEach(model.servers, id: \.self) { server in
                Button {
                    model.choose(server: server)
                } label: {
                    choice(server, chosen: model.server == server)
                }
            }
            Divider()
            Button("Other server…") {
                typedServer = ""
                askingServer = true
            }
        } label: {
            LabeledContent("Server") {
                HStack(spacing: 4) {
                    Text(model.server ?? model.ownServer)
                    Image(systemName: "chevron.up.chevron.down")
                        .font(.caption)
                }
            }
        }
        .foregroundColor(.primary)
    }

    @ViewBuilder
    private func choice(_ server: String, chosen: Bool) -> some View {
        if chosen {
            Label(server, systemImage: "checkmark")
        } else {
            Text(server)
        }
    }

    private func row(_ room: DirectoryRoom) -> some View {
        DirectoryRoomRow(room: room, button: rooms.button(for: room), busy: model.isBusy(room)) {
            act(on: room)
        }
    }

    private func act(on room: DirectoryRoom) {
        guard !model.isBusy(room) else { return }
        switch rooms.button(for: room).action {
        case .join, .askToJoin:
            Task { await model.join(room) }
        case .open:
            dismiss()
            home.open(room.id, kind: room.isSpace ? .space : .room)
        case .invited:
            dismiss()
            home.invite = RoomRef(id: room.id)
        case .asked, .inviteOnly:
            break
        }
    }
}
