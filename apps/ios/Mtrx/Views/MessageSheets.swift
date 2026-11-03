import MtrxFFI
import SwiftUI

/// People who reacted to or read a message: avatar and name (the ID's local
/// part until the name is known).
struct PeopleSheet: View {
    let title: String
    let people: [User]
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        NavigationStack {
            List(people, id: \.userId) { person in
                HStack(spacing: 12) {
                    AvatarView(name: person.displayName, uri: person.avatarUrl, size: 36)
                    VStack(alignment: .leading, spacing: 2) {
                        Text(person.displayName)
                        Text(person.userId)
                            .font(.caption)
                            .foregroundStyle(.secondary)
                    }
                }
            }
            .listStyle(.plain)
            .navigationTitle(title)
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .confirmationAction) {
                    Button("Done") { dismiss() }
                }
            }
        }
        .presentationDetents([.medium, .large])
    }
}

/// All emoji, searchable by name, to react with; ours are marked.
struct EmojiPicker: View {
    let ours: Set<String>
    let pick: (String) -> Void
    /// An emoji's square, growing with the text.
    @ScaledMetric(relativeTo: .title) private var cell: CGFloat = 44
    @State private var query = ""
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        NavigationStack {
            ScrollView {
                LazyVGrid(columns: [GridItem(.adaptive(minimum: cell), spacing: 4)], spacing: 4) {
                    ForEach(Emoji.search(query)) { emoji in
                        Button {
                            pick(emoji.symbol)
                            dismiss()
                        } label: {
                            Text(emoji.symbol)
                                .font(.title)
                                .frame(width: cell, height: cell)
                                .background(
                                    ours.contains(emoji.symbol) ? Color.accentColor.opacity(0.2) : .clear,
                                    in: RoundedRectangle(cornerRadius: 10))
                        }
                        .accessibilityLabel(emoji.name)
                    }
                }
                .padding(.horizontal, 12)
            }
            .searchable(text: $query, placement: .navigationBarDrawer(displayMode: .always), prompt: "Search emoji")
            .navigationTitle("React")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Cancel") { dismiss() }
                }
            }
        }
    }
}

/// A message's text, to select any part of.
struct SelectTextSheet: View {
    let text: String
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        NavigationStack {
            SelectableText(text: text)
                .navigationTitle("Select Text")
                .navigationBarTitleDisplayMode(.inline)
                .toolbar {
                    ToolbarItem(placement: .confirmationAction) {
                        Button("Done") { dismiss() }
                    }
                }
        }
        .presentationDetents([.medium, .large])
    }
}

/// Text that can be selected in part (SwiftUI's `Text` selects only all of
/// it).
private struct SelectableText: UIViewRepresentable {
    let text: String

    func makeUIView(context: Context) -> UITextView {
        let view = UITextView()
        view.isEditable = false
        view.isSelectable = true
        view.dataDetectorTypes = .link
        view.font = .preferredFont(forTextStyle: .body)
        view.adjustsFontForContentSizeCategory = true
        view.backgroundColor = .clear
        view.textContainerInset = UIEdgeInsets(top: 16, left: 12, bottom: 16, right: 12)
        return view
    }

    func updateUIView(_ view: UITextView, context: Context) {
        if view.text != text {
            view.text = text
        }
    }
}
