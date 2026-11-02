import MtrxFFI
import SwiftUI

/// Making a room or space: name, topic, encryption (rooms), who edits it,
/// and people to invite, each a co-owner where the server offers it. Made,
/// it opens.
struct NewRoomView: View {
    @StateObject private var model: NewRoomModel
    @EnvironmentObject private var session: SessionModel
    @Environment(\.dismiss) private var dismiss

    init(model: @autoclosure @escaping () -> NewRoomModel) {
        _model = StateObject(wrappedValue: model())
    }

    var body: some View {
        NavigationStack {
            Form {
                Section {
                    TextField("Name", text: $model.name)
                    TextField("Topic (optional)", text: $model.topic, axis: .vertical)
                        .lineLimit(1...4)
                }
                if !model.space {
                    Section {
                        Toggle("Encrypted", isOn: $model.encrypted)
                    } footer: {
                        Text("Only members can read messages. Cannot be turned off later.")
                    }
                }
                Section {
                    Toggle("Only moderators can edit", isOn: $model.moderated)
                } footer: {
                    Text("Otherwise everyone can invite, rename and change the topic.")
                }
                invites
                if let error = model.error {
                    Section {
                        Text(error).foregroundColor(.red)
                    }
                }
            }
            .navigationTitle(model.title)
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Cancel") { dismiss() }
                }
                ToolbarItem(placement: .confirmationAction) {
                    if model.creating {
                        ProgressView()
                    } else {
                        Button("Create", action: create)
                            .disabled(!model.canCreate)
                    }
                }
            }
            .disabled(model.creating)
        }
        .task { await model.load() }
    }

    private var invites: some View {
        Section {
            HStack {
                TextField("@user:server", text: $model.person)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                    .keyboardType(.emailAddress)
                    .onSubmit(model.addPerson)
                Button("Add", action: model.addPerson)
                    .disabled(!model.personValid)
            }
            ForEach(model.invited, id: \.self) { person in
                HStack {
                    Text(person)
                        .lineLimit(1)
                    Spacer()
                    if model.offersOwners {
                        Toggle(
                            "Co-owner",
                            isOn: Binding(get: { model.coOwners.contains(person) }, set: { model.setCoOwner(person, $0) })
                        )
                        .fixedSize()
                        .font(.subheadline)
                    }
                    Button {
                        model.removePerson(person)
                    } label: {
                        Image(systemName: "xmark.circle.fill")
                            .foregroundStyle(.secondary)
                    }
                    .buttonStyle(.borderless)
                    .accessibilityLabel("Don't invite \(person)")
                }
            }
        } header: {
            Text("Invite people (optional)")
        } footer: {
            if model.creating {
                Text(model.space ? "Creating the space on the server…" : "Creating the room on the server…")
            } else if model.offersOwners && !model.invited.isEmpty {
                Text("A co-owner owns the room as you do: nobody can demote them, and they can't step down.")
            }
        }
    }

    private func create() {
        Task {
            guard let id = await model.create() else { return }
            dismiss()
            session.open(id, kind: model.space ? .space : .room)
        }
    }
}
