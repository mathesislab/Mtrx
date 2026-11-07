import MtrxFFI
import SwiftUI

/// The other person of a direct chat, or someone mentioned: their picture,
/// name, presence and user ID (to copy), and inviting them to one of our
/// spaces.
struct PersonInfoView: View {
    @ObservedObject var model: PersonInfoModel
    @EnvironmentObject private var rooms: RoomListModel
    @EnvironmentObject private var presence: PresenceModel
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        // Those we may invite to.
        let spaces = rooms.lists.topLevelSpaces().filter(\.rights.invite)
        NavigationStack {
            List {
                Section {
                    header
                        .frame(maxWidth: .infinity)
                        .listRowBackground(Color.clear)
                }
                if !spaces.isEmpty {
                    Section {
                        Button {
                            model.choosingSpace = true
                        } label: {
                            Label("Add to space…", systemImage: "person.badge.plus")
                        }
                    } footer: {
                        if let confirmation = model.confirmation {
                            Text(confirmation)
                        }
                    }
                }
            }
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .confirmationAction) {
                    Button("Done") { dismiss() }
                }
            }
            .navigationDestination(isPresented: $model.choosingSpace) {
                SpaceChoice(model: model, spaces: spaces)
            }
        }
        .presentationDetents([.medium, .large])
        // Fresher than the subscription's, and for someone not in it.
        .task { await presence.refresh(model.userId) }
        .alert("Could not invite", isPresented: hasError) {
            Button("OK") { model.inviteError = nil }
        } message: {
            Text(model.inviteError ?? "")
        }
    }

    private var header: some View {
        VStack(spacing: 10) {
            let shown = presence.presence(of: model.userId)
            AvatarView(name: model.name, uri: model.avatarUrl, size: 80)
                .presenceDot(shown, size: 20, avatar: 80, ring: Color(.systemGroupedBackground))
            Text(model.name)
                .font(.title2.weight(.semibold))
                .multilineTextAlignment(.center)
            PresenceLine(presence: shown)
            HStack(spacing: 8) {
                Text(model.userId)
                    .font(.subheadline)
                    .foregroundColor(.secondary)
                    .textSelection(.enabled)
                Button {
                    model.copyId()
                } label: {
                    if model.copied {
                        Label("Copied", systemImage: "checkmark")
                            .font(.subheadline)
                    } else {
                        Image(systemName: "doc.on.doc")
                            .accessibilityLabel("Copy the user ID")
                    }
                }
                .buttonStyle(.borderless)
            }
        }
    }

    private var hasError: Binding<Bool> {
        Binding(get: { model.inviteError != nil }, set: { if !$0 { model.inviteError = nil } })
    }
}

/// The top-level spaces, to invite the person to one; a spinner on the
/// space while inviting.
private struct SpaceChoice: View {
    @ObservedObject var model: PersonInfoModel
    let spaces: [RoomSummary]

    var body: some View {
        List(spaces, id: \.id) { space in
            Button {
                model.invite(to: space)
            } label: {
                HStack(spacing: 12) {
                    AvatarView(name: space.name, uri: space.avatarUrl, size: 32, shape: .square)
                    Text(space.name)
                        .foregroundColor(.primary)
                    Spacer()
                    if model.inviting == space.id {
                        ProgressView()
                    }
                }
            }
            .disabled(model.inviting != nil)
        }
        .navigationTitle("Add to space")
        .navigationBarTitleDisplayMode(.inline)
    }
}
