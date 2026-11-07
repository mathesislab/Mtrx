import MtrxFFI
import SwiftUI

/// A room's topic panel: the topic's text in full, its links as buttons
/// (opening them), the pinned messages (a tap goes to one), and Edit where
/// we may change the topic.
struct TopicPanel: View {
    let topic: Topic
    let canEdit: Bool
    @ObservedObject var info: RoomInfoModel
    @StateObject private var pinned: PinnedModel
    /// Go to a pinned message (its event ID).
    let jump: (String) -> Void
    @Environment(\.dismiss) private var dismiss

    init(
        topic: Topic, canEdit: Bool, info: RoomInfoModel, pinned: @autoclosure @escaping () -> PinnedModel,
        jump: @escaping (String) -> Void
    ) {
        self.topic = topic
        self.canEdit = canEdit
        self.info = info
        _pinned = StateObject(wrappedValue: pinned())
        self.jump = jump
    }

    var body: some View {
        NavigationStack {
            List {
                Section {
                    TopicText(topic: topic)
                }
                if !topic.links.isEmpty {
                    Section {
                        ForEach(Array(topic.links.enumerated()), id: \.offset) { _, link in
                            LinkButton(link: link)
                        }
                    }
                }
                Section("Pinned messages") {
                    pins
                }
            }
            .navigationTitle("Topic")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                if canEdit {
                    ToolbarItem(placement: .navigationBarLeading) {
                        NavigationLink("Edit") {
                            TopicEditor(draft: TopicDraft(topic), info: info)
                        }
                    }
                }
                ToolbarItem(placement: .confirmationAction) {
                    Button("Done") { dismiss() }
                }
            }
        }
        .presentationDetents([.medium, .large])
        .task { await pinned.load() }
    }

    @ViewBuilder
    private var pins: some View {
        if pinned.loading {
            ProgressView()
                .frame(maxWidth: .infinity)
        } else if let error = pinned.error {
            Text(error).foregroundColor(.red).font(.footnote)
        } else if pinned.pins.isEmpty {
            Text("No pinned messages").foregroundStyle(.secondary)
        } else {
            ForEach(pinned.pins) { pin in
                Button {
                    if let eventId = pin.eventId {
                        jump(eventId)
                        dismiss()
                    }
                } label: {
                    HStack(alignment: .top, spacing: 8) {
                        Image(systemName: "pin.fill")
                            .font(.caption)
                            .foregroundStyle(.secondary)
                            .padding(.top, 3)
                        VStack(alignment: .leading, spacing: 1) {
                            Text(pin.sender)
                                .font(.subheadline.weight(.semibold))
                            Text(pin.text)
                                .font(.subheadline)
                                .lineLimit(3)
                        }
                    }
                    .foregroundColor(.primary)
                }
            }
        }
    }
}

/// A topic's text, to select from, or a word that there is none.
struct TopicText: View {
    let topic: Topic

    var body: some View {
        if !topic.text.isEmpty {
            Text(Formatting.linked(topic.text))
                .textSelection(.enabled)
        } else if topic.links.isEmpty {
            Text("No topic or links yet.")
                .foregroundStyle(.secondary)
        } else {
            Text("No topic").foregroundStyle(.secondary)
        }
    }
}

/// A topic's link: a call (camera) or a link, by its name, opening its
/// address; a call's passcode beside it.
struct LinkButton: View {
    let link: RoomLink
    @Environment(\.openURL) private var openURL

    var body: some View {
        Button {
            if let url = URL(string: link.url) {
                openURL(url)
            }
        } label: {
            AdaptiveStack {
                Label(link.name, systemImage: link.call ? "video" : "link")
                Spacer(minLength: 0)
                if let passcode = link.passcode, !passcode.isEmpty {
                    Text("Passcode \(passcode)")
                        .font(.footnote)
                        .foregroundStyle(.secondary)
                        .textSelection(.enabled)
                }
            }
        }
        .accessibilityHint(link.url)
    }
}

/// Editing a topic: its text, and its calls and links (title, address,
/// passcode for calls). Saved, the editor closes.
struct TopicEditor: View {
    @State var draft: TopicDraft
    @ObservedObject var info: RoomInfoModel
    @Environment(\.dismiss) private var dismiss
    @State private var saving = false

    var body: some View {
        Form {
            Section {
                TextField("What the room is about", text: $draft.text, axis: .vertical)
                    .lineLimit(3...10)
            }
            links(calls: true)
            links(calls: false)
            Section {
                Button {
                    draft.add(call: true)
                } label: {
                    Label("Add call", systemImage: "video.badge.plus")
                }
                Button {
                    draft.add(call: false)
                } label: {
                    Label("Add link", systemImage: "link.badge.plus")
                }
            } footer: {
                if draft.dropped > 0 {
                    Text("Links without a web address are left out.")
                }
            }
        }
        .navigationTitle("Edit topic")
        .navigationBarTitleDisplayMode(.inline)
        .toolbar {
            ToolbarItem(placement: .confirmationAction) {
                if saving {
                    ProgressView()
                } else {
                    Button("Save") { save() }
                }
            }
        }
    }

    @ViewBuilder
    private func links(calls: Bool) -> some View {
        if draft.links.contains(where: { $0.call == calls }) {
            Section(calls ? "Calls" : "Links") {
                ForEach($draft.links) { $link in
                    if link.call == calls {
                        LinkFields(link: $link) {
                            draft.remove(link.id)
                        }
                    }
                }
            }
        }
    }

    private func save() {
        saving = true
        info.setTopic(draft.topic) { saved in
            if saved {
                dismiss()
            }
            saving = false
        }
    }
}

/// A call's or link's fields: title, address, and a call's passcode.
private struct LinkFields: View {
    @Binding var link: TopicDraft.Link
    let remove: () -> Void

    var body: some View {
        HStack(alignment: .top, spacing: 10) {
            Image(systemName: link.call ? "video" : "link")
                .foregroundStyle(.secondary)
                .padding(.top, 4)
            VStack(spacing: 8) {
                TextField("Title", text: $link.title)
                TextField("https://…", text: $link.url)
                    .keyboardType(.URL)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                    .foregroundColor(link.url.isEmpty || link.valid ? nil : .red)
                if link.call {
                    TextField("Passcode", text: $link.passcode)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                }
            }
            Button(action: remove) {
                Image(systemName: "minus.circle.fill")
                    .foregroundColor(.red)
            }
            .buttonStyle(.borderless)
            .accessibilityLabel("Remove")
        }
        .padding(.vertical, 2)
    }
}
