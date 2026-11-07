import MtrxFFI
import SwiftUI

/// A room's topic panel: the topic's text in full, its calls and links as
/// buttons (opening them), the pinned messages (a tap goes to one), and
/// Edit where we may change the topic.
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
                if !topic.calls.isEmpty || !topic.links.isEmpty {
                    Section {
                        TopicLinkButtons(topic: topic)
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
        .opensRoomLinks()
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
        } else if topic.calls.isEmpty && topic.links.isEmpty {
            Text("No topic or links yet.")
                .foregroundStyle(.secondary)
        } else {
            Text("No topic").foregroundStyle(.secondary)
        }
    }
}

/// A button for each of a topic's calls, then each of its other links: list
/// rows.
struct TopicLinkButtons: View {
    let topic: Topic

    var body: some View {
        ForEach(Array(topic.calls.enumerated()), id: \.offset) { _, link in
            LinkButton(link: link, call: true)
        }
        ForEach(Array(topic.links.enumerated()), id: \.offset) { _, link in
            LinkButton(link: link, call: false)
        }
    }
}

/// A topic's call (camera) or link, by its name, opening its address; a
/// call's passcode beside it.
struct LinkButton: View {
    let link: RoomLink
    let call: Bool
    @Environment(\.openRoomLink) private var open

    var body: some View {
        Button {
            open(link)
        } label: {
            AdaptiveStack {
                Label(link.name, systemImage: call ? "video" : "link")
                Spacer(minLength: 0)
                if let passcode = link.passcode {
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

/// A space's calls, then its links, as small capsules in rows under its
/// name, centred.
struct LinkChips: View {
    let topic: Topic
    @Environment(\.openRoomLink) private var open

    var body: some View {
        FlowLayout(spacing: 6, centered: true) {
            ForEach(Array(topic.calls.enumerated()), id: \.offset) { _, link in
                chip(link, call: true)
            }
            ForEach(Array(topic.links.enumerated()), id: \.offset) { _, link in
                chip(link, call: false)
            }
        }
    }

    private func chip(_ link: RoomLink, call: Bool) -> some View {
        Button {
            open(link)
        } label: {
            Label(link.name, systemImage: call ? "video" : "link")
                .font(.footnote)
                .lineLimit(1)
        }
        .buttonStyle(.bordered)
        .buttonBorderShape(.capsule)
        .controlSize(.small)
        .accessibilityHint(link.passcode.map { "Copies the passcode \($0), then opens \(link.url)" } ?? link.url)
    }
}

/// The room's calls and links at the right of its header: Join while it has
/// a call (a tap joins the room's call, a long press shows the menu), else a
/// link icon that opens the menu. The menu has the calls, then the links,
/// each opening, then Edit links… where we may edit them.
struct RoomLinksMenu: View {
    let links: HeaderLinks
    let editLinks: () -> Void
    @Environment(\.openRoomLink) private var open

    var body: some View {
        if let call = links.join {
            Menu {
                entries
            } label: {
                Label("Join", systemImage: "video.fill")
                    .labelStyle(.titleAndIcon)
                    .font(.subheadline.weight(.semibold))
            } primaryAction: {
                open(call)
            }
            .menuStyle(.button)
            .buttonStyle(.borderedProminent)
            .buttonBorderShape(.capsule)
            .controlSize(.small)
            .accessibilityLabel(links.joinLabel ?? "Join")
            .accessibilityHint("Hold for the room's calls and links")
        } else {
            Menu {
                entries
            } label: {
                Image(systemName: "link")
            }
            .accessibilityLabel("Links")
        }
    }

    @ViewBuilder
    private var entries: some View {
        ForEach(links.groups, id: \.title) { group in
            Section(group.title) {
                ForEach(Array(group.links.enumerated()), id: \.offset) { _, link in
                    Button {
                        open(link)
                    } label: {
                        Label {
                            Text(link.name)
                            if let passcode = link.passcode {
                                Text("Passcode \(passcode)")
                            }
                        } icon: {
                            Image(systemName: group.call ? "video" : "link")
                        }
                    }
                }
            }
        }
        if links.canEdit {
            Section {
                Button(action: editLinks) {
                    Label("Edit links…", systemImage: "pencil")
                }
            }
        }
    }
}

/// Opens a topic's call or link: a call's passcode is copied first, as the
/// call's app opens on top and pasting is the easiest way in.
struct RoomLinkAction {
    let action: @MainActor (RoomLink) -> Void

    @MainActor
    func callAsFunction(_ link: RoomLink) {
        action(link)
    }
}

private struct RoomLinkActionKey: EnvironmentKey {
    /// Without a word that the passcode was copied: see `opensRoomLinks`.
    static let defaultValue = RoomLinkAction { @MainActor link in
        guard let url = URL(string: link.url) else { return }
        if let passcode = link.passcode {
            UIPasteboard.general.string = passcode
        }
        UIApplication.shared.open(url)
    }
}

extension EnvironmentValues {
    var openRoomLink: RoomLinkAction {
        get { self[RoomLinkActionKey.self] }
        set { self[RoomLinkActionKey.self] = newValue }
    }
}

extension View {
    /// Its calls and links open here: a call with a passcode copies it
    /// first, which a banner at the top says ("Passcode 123 copied") just
    /// before the call opens.
    func opensRoomLinks() -> some View {
        modifier(OpensRoomLinks())
    }
}

private struct OpensRoomLinks: ViewModifier {
    @Environment(\.openURL) private var openURL
    /// The passcode copied, while the banner shows.
    @State private var copied: String?
    /// The banner's number: only the last one's timer hides it.
    @State private var shown = 0

    func body(content: Content) -> some View {
        content
            .environment(\.openRoomLink, RoomLinkAction(action: open))
            .overlay(alignment: .top) {
                if let copied {
                    Text("Passcode \(copied) copied")
                        .font(.footnote.weight(.semibold))
                        .padding(.horizontal, 14)
                        .padding(.vertical, 8)
                        .background(.regularMaterial, in: Capsule())
                        .padding(.top, 8)
                        .transition(.move(edge: .top).combined(with: .opacity))
                        .accessibilityAddTraits(.isStaticText)
                }
            }
    }

    @MainActor
    private func open(_ link: RoomLink) {
        guard let url = URL(string: link.url) else { return }
        guard let passcode = link.passcode else {
            openURL(url)
            return
        }
        UIPasteboard.general.string = passcode
        let message = "Passcode \(passcode) copied"
        UIAccessibility.post(notification: .announcement, argument: message)
        withAnimation(.easeOut(duration: 0.2)) { copied = passcode }
        shown += 1
        let number = shown
        Task { @MainActor in
            // Seen before the call's app opens on top.
            try? await Task.sleep(nanoseconds: 700_000_000)
            openURL(url)
            try? await Task.sleep(nanoseconds: 2_300_000_000)
            if shown == number {
                withAnimation(.easeIn(duration: 0.2)) { copied = nil }
            }
        }
    }
}

/// Editing a topic: its text, its calls and its links (title, address,
/// passcode for calls), each dragged by its handle to reorder and swiped or
/// tapped away. The first call is the room's. Saved, the editor closes.
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
            if !draft.calls.isEmpty {
                Section {
                    ForEach($draft.calls) { $link in
                        LinkFields(link: $link, call: true)
                    }
                    .onMove { draft.move(call: true, from: $0, to: $1) }
                    .onDelete { draft.remove(call: true, at: $0) }
                } header: {
                    Text("Calls")
                } footer: {
                    Text("The first call is the room's: Join opens it.")
                }
            }
            if !draft.links.isEmpty {
                Section("Links") {
                    ForEach($draft.links) { $link in
                        LinkFields(link: $link, call: false)
                    }
                    .onMove { draft.move(call: false, from: $0, to: $1) }
                    .onDelete { draft.remove(call: false, at: $0) }
                }
            }
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
        // The handles to drag by, always shown.
        .environment(\.editMode, .constant(.active))
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

/// The topic editor on its own, from the header's Edit links…: with Cancel.
struct TopicEditorSheet: View {
    let topic: Topic
    @ObservedObject var info: RoomInfoModel
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        NavigationStack {
            TopicEditor(draft: TopicDraft(topic), info: info)
                .toolbar {
                    ToolbarItem(placement: .cancellationAction) {
                        Button("Cancel") { dismiss() }
                    }
                }
        }
    }
}

/// A call's or link's fields: title, address (red while it isn't a web
/// address), and a call's passcode.
private struct LinkFields: View {
    @Binding var link: TopicDraft.Link
    let call: Bool

    var body: some View {
        HStack(alignment: .top, spacing: 10) {
            Image(systemName: call ? "video" : "link")
                .foregroundStyle(.secondary)
                .padding(.top, 4)
            VStack(spacing: 8) {
                TextField("Title", text: $link.title)
                TextField(call ? "https://zoom.us/j/123456789" : "https://…", text: $link.url)
                    .keyboardType(.URL)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                    .foregroundColor(link.url.isEmpty || link.valid ? nil : .red)
                if call {
                    TextField("Passcode", text: $link.passcode)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                }
            }
        }
        .padding(.vertical, 2)
    }
}
