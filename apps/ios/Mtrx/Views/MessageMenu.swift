import MtrxFFI
import SwiftUI

/// A held message's menu, as a sheet: the message, the common reactions
/// (ours marked) and a button for all of them, then what can be done with
/// it. Choosing closes it; a reaction shows under the message at once.
struct MessageMenu: View {
    let row: MessageRow
    /// Only what applies to the message, in order.
    let entries: [MessageAction]
    let react: (String) -> Void
    /// After the sheet closed.
    let choose: (MessageAction) -> Void
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            preview
            if entries.contains(.react) {
                reactions
            }
            VStack(spacing: 0) {
                ForEach(entries.filter { $0 != .react }, id: \.self) { entry in
                    if entry != entries.first(where: { $0 != .react }) {
                        Divider().padding(.leading, 52)
                    }
                    button(entry)
                }
            }
            .background(Color(.secondarySystemGroupedBackground), in: RoundedRectangle(cornerRadius: 14))
            Spacer(minLength: 0)
        }
        .padding(.horizontal, 16)
        .padding(.top, 24)
        .background(Color(.systemGroupedBackground))
        .presentationDetents([.medium, .large])
        .presentationDragIndicator(.visible)
    }

    /// Who wrote it and the start of it.
    private var preview: some View {
        VStack(alignment: .leading, spacing: 2) {
            Text(row.event.sender.displayName)
                .font(.subheadline.weight(.semibold))
                .foregroundColor(AvatarColor.of(row.event.sender.userId))
            Text(row.message?.body ?? "")
                .font(.subheadline)
                .foregroundStyle(.secondary)
                .lineLimit(3)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .accessibilityElement(children: .combine)
    }

    private var reactions: some View {
        let ours = Set(row.reactions.filter(\.byUs).map(\.key))
        return HStack(spacing: 0) {
            ForEach(Emoji.common, id: \.self) { key in
                Button {
                    react(key)
                    dismiss()
                } label: {
                    Text(key)
                        .font(.title2)
                        .frame(width: 44, height: 44)
                        .background(
                            ours.contains(key) ? Color.accentColor.opacity(0.25) : Color(.secondarySystemGroupedBackground),
                            in: Circle())
                }
                .accessibilityLabel(ours.contains(key) ? "Remove \(key)" : "React with \(key)")
                Spacer(minLength: 0)
            }
            Button {
                choose(.react)
            } label: {
                Image(systemName: "face.smiling")
                    .font(.title3)
                    .foregroundColor(.primary)
                    .frame(width: 44, height: 44)
                    .background(Color(.secondarySystemGroupedBackground), in: Circle())
            }
            .accessibilityLabel("More Reactions")
        }
        .buttonStyle(.plain)
    }

    private func button(_ entry: MessageAction) -> some View {
        Button(role: entry == .delete ? .destructive : nil) {
            choose(entry)
        } label: {
            HStack(spacing: 16) {
                Image(systemName: entry.symbol)
                    .frame(width: 20)
                Text(entry.title)
                Spacer()
            }
            .foregroundColor(entry == .delete ? .red : .primary)
            .padding(.horizontal, 16)
            .frame(minHeight: 48)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
    }
}
