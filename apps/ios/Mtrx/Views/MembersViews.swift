import MtrxFFI
import SwiftUI

/// A room's members (joined, then invited): picture, name and role; a tap on
/// one we may manage opens their menu (in the navigation stack it is in).
struct MembersSection: View {
    let room: RoomSummary
    @ObservedObject var model: RoomInfoModel

    var body: some View {
        if let members = model.members {
            let joined = members.filter { $0.membership == .joined }
            let invited = members.filter { $0.membership == .invited }
            Section("Joined · \(joined.count)") {
                rows(joined)
            }
            if !invited.isEmpty {
                Section("Invited · \(invited.count)") {
                    rows(invited)
                }
            }
        } else if model.loadError == nil {
            Section("Members") {
                ProgressView().frame(maxWidth: .infinity)
            }
        }
    }

    private func rows(_ members: [Member]) -> some View {
        ForEach(members, id: \.user.userId) { member in
            if member.manageable {
                NavigationLink {
                    MemberMenu(member: member, rights: room.rights, model: model)
                } label: {
                    MemberRow(member: member)
                }
            } else {
                MemberRow(member: member)
            }
        }
    }
}

/// A member: picture, name and user ID, and role.
struct MemberRow: View {
    let member: Member

    @RowAvatarSize(30) private var avatarSize: CGFloat

    var body: some View {
        HStack(spacing: 10) {
            AvatarView(name: member.user.displayName, uri: member.user.avatarUrl, size: avatarSize)
            AdaptiveStack(spacing: 2) {
                VStack(alignment: .leading, spacing: 0) {
                    Text(member.user.displayName)
                        .lineLimit(1, accessibility: 3)
                    Text(member.user.userId)
                        .font(.caption)
                        .foregroundStyle(.secondary)
                        .lineLimit(1, accessibility: 3)
                }
                Spacer(minLength: 8)
                if let role = member.label {
                    Text(role)
                        .font(.caption.weight(.medium))
                        .foregroundStyle(.secondary)
                }
            }
        }
        .contentShape(Rectangle())
    }
}

/// What we may do to a member: the levels we may give them (theirs
/// ticked), a custom level, Remove… and Ban…. Asks first where a change
/// can't be undone.
struct MemberMenu: View {
    let member: Member
    /// Ours in the room.
    let rights: Rights
    @ObservedObject var model: RoomInfoModel
    @Environment(\.dismiss) private var dismiss
    @State private var confirming: (level: Int64, confirmation: LevelConfirmation)?
    @State private var customLevel = false
    @State private var typed = ""
    @State private var removing: Bool?
    @State private var reason = ""

    private var name: String { member.user.displayName }

    var body: some View {
        List {
            Section {
                HStack(spacing: 12) {
                    AvatarView(name: name, uri: member.user.avatarUrl, size: 44)
                    VStack(alignment: .leading, spacing: 1) {
                        Text(name).font(.headline)
                        Text(member.user.userId)
                            .font(.caption)
                            .foregroundStyle(.secondary)
                            .textSelection(.enabled)
                    }
                }
                .listRowBackground(Color.clear)
            }
            if member.manage.upTo != nil {
                Section("Role") {
                    ForEach(member.choices, id: \.level) { choice in
                        Button {
                            give(choice.level, asking: choice.confirmation)
                        } label: {
                            HStack {
                                Image(systemName: "checkmark")
                                    .font(.footnote.weight(.semibold))
                                    .opacity(choice.current ? 1 : 0)
                                Text(choice.title)
                                    .foregroundColor(.primary)
                                Spacer()
                                Text(String(choice.level))
                                    .foregroundColor(.secondary)
                                    .monospacedDigit()
                            }
                        }
                        .disabled(choice.current)
                    }
                    Button("Custom level…") {
                        typed = member.level.map(String.init) ?? ""
                        customLevel = true
                    }
                }
            }
            if member.manage.remove || member.manage.ban {
                Section {
                    if member.manage.remove {
                        Button("Remove…", role: .destructive) { remove(ban: false) }
                    }
                    if member.manage.ban {
                        Button("Ban…", role: .destructive) { remove(ban: true) }
                    }
                }
            }
        }
        .navigationTitle(name)
        .navigationBarTitleDisplayMode(.inline)
        .alert(
            confirming?.confirmation.title ?? "",
            isPresented: Binding(get: { confirming != nil }, set: { if !$0 { confirming = nil } }),
            presenting: confirming
        ) { confirming in
            Button(confirming.confirmation.button, role: .destructive) {
                model.setLevel(confirming.level, of: member.user.userId)
                dismiss()
            }
            Button("Cancel", role: .cancel) {}
        } message: { confirming in
            Text(confirming.confirmation.message)
        }
        .alert(member.own ? "Your power level" : "Power level for \(name)", isPresented: $customLevel) {
            TextField("Level", text: $typed)
                .keyboardType(.numbersAndPunctuation)
            Button("Set") {
                if let upTo = member.manage.upTo, let level = typedLevel(text: typed, upTo: upTo) {
                    give(level, asking: levelConfirmation(member: member, level: level))
                }
            }
            Button("Cancel", role: .cancel) {}
        } message: {
            Text(levelsHint)
        }
        .alert(
            removing == true ? "Ban \(name)?" : "Remove \(name)?",
            isPresented: Binding(get: { removing != nil }, set: { if !$0 { removing = nil } })
        ) {
            TextField("Reason (optional, they see it)", text: $reason)
            Button(removing == true ? "Ban" : "Remove", role: .destructive) {
                model.remove(member.user.userId, reason: reason, ban: removing == true)
                dismiss()
            }
            Button("Cancel", role: .cancel) {}
        } message: {
            Text(
                removing == true
                    ? "They can't come back until someone lifts the ban."
                    : "They can come back if the room lets them join or someone invites them.")
        }
    }

    /// "Admin 100 · Moderator 50 · Member 0 · at most 50".
    private var levelsHint: String {
        var hint = "Admin 100 · Moderator 50 · Member \(rights.defaultLevel)"
        if let upTo = member.manage.upTo, upTo < 100 {
            hint += " · at most \(upTo)"
        }
        return hint
    }

    /// Give `level`, asking `asking` first where it can't be undone.
    private func give(_ level: Int64, asking: LevelConfirmation?) {
        guard level != member.level else { return }
        if let confirmation = asking {
            confirming = (level, confirmation)
        } else {
            model.setLevel(level, of: member.user.userId)
            dismiss()
        }
    }

    private func remove(ban: Bool) {
        reason = ""
        removing = ban
    }
}
