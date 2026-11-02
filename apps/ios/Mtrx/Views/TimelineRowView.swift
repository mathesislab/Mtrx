import MtrxFFI
import SwiftUI

/// A row of the timeline. Equal while what it shows is: the actions act on
/// the same room whichever copy is kept, so a row is not drawn again each
/// time the room's view is (once per message, sent or arrived).
struct TimelineRowView: View, Equatable {
    let row: TimelineRow
    /// Gone to from a quote: marked for a moment.
    var highlighted = false
    let actions: RowActions

    static func == (lhs: Self, rhs: Self) -> Bool {
        lhs.row == rhs.row && lhs.highlighted == rhs.highlighted
    }

    var body: some View {
        switch row.content {
        case .start:
            Text("This is the start of the room.")
                .font(.footnote)
                .foregroundStyle(.secondary)
                .frame(maxWidth: .infinity)
                .padding(.vertical, 16)
        case .day(let date):
            HStack(spacing: 10) {
                line
                Text(Formatting.day(date))
                    .font(.caption.weight(.semibold))
                    .foregroundStyle(.secondary)
                    .fixedSize()
                line
            }
            .padding(.vertical, 12)
        case .readMarker:
            HStack(spacing: 8) {
                Rectangle().fill(Color.red.opacity(0.6)).frame(height: 1)
                Text("New")
                    .font(.caption2.weight(.semibold))
                    .foregroundColor(.red)
            }
            .padding(.vertical, 6)
            .accessibilityLabel("New messages below")
        case .summary(let text):
            Text(text)
                .font(.footnote)
                .foregroundStyle(.secondary)
                .multilineTextAlignment(.center)
                .frame(maxWidth: .infinity)
                .padding(.vertical, 6)
        case .message:
            if let message = row.messageRow {
                MessageView(id: row.id, row: message, highlighted: highlighted, actions: actions)
            }
        }
    }

    private var line: some View {
        Rectangle().fill(Color.secondary.opacity(0.3)).frame(height: 1)
    }
}

/// What a message's row asks of the room, with the row's ID.
struct RowActions {
    /// A message was held: its menu (item ID).
    let hold: (String) -> Void
    /// An entry of the message's menu (`react`: the full picker).
    let perform: (MessageAction, String) -> Void
    /// Add or remove our reaction (item ID, key).
    let react: (String, String) -> Void
    /// Go to the original of a quote.
    let goTo: (ReplyQuote) -> Void
    /// List who reacted with a key (item ID, key).
    let showReactors: (String, String) -> Void
    /// List who read up to the message.
    let showReaders: (String) -> Void
}

/// A message: in a bubble on the right if ours, else on the left with the
/// sender's name over the first of a group and their avatar beside it; its
/// reactions under it, and who read up to it at the right. Holding it opens
/// its menu (`MessageMenu`); swiping it to the left replies to it.
struct MessageView: View {
    let id: String
    let row: MessageRow
    let highlighted: Bool
    let actions: RowActions
    @State private var swipe = SwipeToReply()

    static let avatarSize: CGFloat = 32
    private static let avatarSpacing: CGFloat = 8

    private var event: TimelineEvent { row.event }
    private var own: Bool { event.own }

    var body: some View {
        VStack(alignment: own ? .trailing : .leading, spacing: 3) {
            if row.showSender {
                Text(event.displayName)
                    .fontWeight(.semibold)
                    .foregroundColor(AvatarColor.of(event.senderId))
                    .lineLimit(1)
                    .padding(.leading, (row.avatarColumn ? Self.avatarSize + Self.avatarSpacing : 0) + 12)
            }
            HStack(alignment: .top, spacing: Self.avatarSpacing) {
                if own { Spacer(minLength: 48) }
                if row.avatarColumn {
                    if row.showSender {
                        AvatarView(name: event.displayName, uri: event.senderAvatarUrl, size: Self.avatarSize)
                    } else {
                        Color.clear.frame(width: Self.avatarSize, height: 1)
                    }
                }
                VStack(alignment: own ? .trailing : .leading, spacing: 4) {
                    // Not a context menu: lifted out of the timeline, drawn
                    // upside down, it turned over.
                    content
                        .onLongPressGesture { actions.hold(id) }
                        .accessibilityAction(named: "Actions") { actions.hold(id) }
                    if !row.reactions.isEmpty {
                        ReactionChips(reactions: row.reactions, trailing: own) { key in
                            actions.react(id, key)
                        } showReactors: { key in
                            actions.showReactors(id, key)
                        }
                    }
                    if case .failed(let error) = event.sendState {
                        Text("Not sent: \(error)")
                            .font(.caption2)
                            .foregroundColor(.red)
                            .padding(.horizontal, 12)
                    }
                }
                if !own { Spacer(minLength: 48) }
            }
            .offset(x: swipe.offset)
            .background(alignment: .trailing) {
                if swipe.progress > 0 {
                    replyIcon
                }
            }
            .modifier(SwipesToReply(enabled: row.canSwipeToReply, swipe: $swipe) {
                actions.perform(.reply, id)
            })
            if !row.readers.isEmpty {
                ReaderAvatarsView(readers: row.readers) {
                    actions.showReaders(id)
                }
                .frame(maxWidth: .infinity, alignment: .trailing)
            }
        }
        .padding(.top, row.firstInGroup ? 10 : 2)
        .background(
            RoundedRectangle(cornerRadius: 12)
                .fill(Color.accentColor.opacity(highlighted ? 0.15 : 0))
                .padding(.horizontal, -6)
                .padding(.vertical, -2)
        )
        .animation(.easeInOut(duration: 0.3), value: highlighted)
    }

    /// Revealed at the right while the row is swiped.
    private var replyIcon: some View {
        Image(systemName: "arrowshape.turn.up.left.fill")
            .font(.subheadline.weight(.semibold))
            .dynamicTypeSize(...DynamicTypeSize.xxxLarge)
            .foregroundColor(swipe.triggers ? .white : .accentColor)
            .frame(width: 32, height: 32)
            .background(swipe.triggers ? Color.accentColor : Color.accentColor.opacity(0.15), in: Circle())
            .scaleEffect(0.6 + 0.4 * swipe.progress)
            .opacity(swipe.progress)
            .padding(.trailing, 8)
            .accessibilityHidden(true)
    }

    @ViewBuilder
    private var content: some View {
        switch event.content {
        case .message(let message):
            if let media = message.media, media.kind == .image || media.kind == .video {
                VStack(alignment: own ? .trailing : .leading, spacing: 4) {
                    if let quote = message.replyTo {
                        quoteView(quote)
                            .padding(.horizontal, 12)
                            .padding(.vertical, 7)
                            .background(bubbleColor, in: RoundedRectangle(cornerRadius: 18))
                    }
                    MediaImageView(media: media)
                        .overlay(alignment: .bottomTrailing) {
                            footer.padding(6).background(.ultraThinMaterial, in: Capsule()).padding(6)
                        }
                    if message.body != media.filename && !message.body.isEmpty {
                        bubble(text(message), quote: nil, muted: message.kind == .notice)
                    }
                }
            } else if let media = message.media {
                bubble(FileView(media: media), quote: message.replyTo)
            } else {
                bubble(text(message), quote: message.replyTo, muted: message.kind == .notice)
            }
        case .redacted:
            bubble(Text("Message deleted").italic(), quote: nil, muted: true)
        case .unableToDecrypt:
            // Turns into the message by itself once its key arrives (after a
            // recovery or verification).
            bubble(Text(row.undecryptableText ?? "").italic(), quote: nil, muted: true)
        case .other(let description):
            bubble(Text("sent a \(description)").italic(), quote: nil, muted: true)
        case .membership, .displayNameChange, .otherState:
            // Folded into summaries by the library.
            EmptyView()
        }
    }

    private func text(_ message: Message) -> Text {
        let text: Text
        switch message.kind {
        case .emote:
            text = Text("\(event.displayName) ").italic() + Text(Formatting.linked(message.body)).italic()
        default:
            text = Text(Formatting.linked(message.body))
        }
        guard message.edited else { return text }
        return text + Text(" (edited)").font(.footnote).foregroundColor(muted)
    }

    /// Times, notices, and what stands for a message (deleted, not
    /// decrypted).
    private var muted: Color {
        own ? .ownBubbleSecondary : .secondary
    }

    /// Ours a light shade of the brand blue (a dark one in dark mode), with
    /// the primary text colour; links and quote bars on it in the brand
    /// blue (lighter in dark mode, `ownBubbleLink`).
    private var bubbleColor: Color {
        own ? .ownBubble : Color(.secondarySystemBackground)
    }

    private func quoteView(_ quote: ReplyQuote) -> some View {
        QuoteView(quote: quote, own: own) {
            actions.goTo(quote)
        }
        .foregroundStyle(Color.primary)
    }

    /// `content` in the bubble, under the message it answers (`quote`); in
    /// the muted colour if `muted`, set on the view: a colour set on the
    /// text may not hold (a deleted message showed grey on ours).
    private func bubble<Content: View>(_ content: Content, quote: ReplyQuote?, muted: Bool = false) -> some View {
        VStack(alignment: .leading, spacing: 6) {
            if let quote {
                quoteView(quote)
            }
            VStack(alignment: .trailing, spacing: 2) {
                content
                    .foregroundStyle(muted ? self.muted : .primary)
                    .frame(minWidth: 0, alignment: .leading)
                footer
            }
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 7)
        .foregroundStyle(Color.primary)
        .tint(own ? .ownBubbleLink : .accentColor)
        .background(bubbleColor, in: RoundedRectangle(cornerRadius: 18))
    }

    /// Time and send state.
    private var footer: some View {
        HStack(spacing: 4) {
            Text(Formatting.time(event.timestamp))
            switch event.sendState {
            case .sent:
                EmptyView()
            case .sending:
                Image(systemName: "clock").accessibilityLabel("Sending")
            case .waiting:
                Image(systemName: "wifi.slash").accessibilityLabel("Waiting for connection")
            case .failed:
                Image(systemName: "exclamationmark.circle").foregroundColor(.red).accessibilityLabel("Not sent")
            }
        }
        .font(.caption2)
        .foregroundStyle(muted)
    }
}

/// From iOS 18, a message swiped to the left replies to it (before, Reply
/// in its menu does): a UIKit pan that begins only for a leftward drag, so
/// the list's scrolling and the back gesture are left alone.
private struct SwipesToReply: ViewModifier {
    let enabled: Bool
    @Binding var swipe: SwipeToReply
    let reply: () -> Void

    func body(content: Content) -> some View {
        if #available(iOS 18, *) {
            if enabled {
                content
                    .gesture(LeftwardPan { translation in
                        swipe.update(translation)
                    } ended: { finished in
                        if finished && swipe.triggers {
                            reply()
                        }
                        withAnimation(.spring(response: 0.3, dampingFraction: 0.8)) {
                            swipe = SwipeToReply()
                        }
                    })
                    .onChange(of: swipe.triggers) { triggers in
                        if triggers {
                            UIImpactFeedbackGenerator(style: .medium).impactOccurred()
                        }
                    }
            } else {
                content
            }
        } else {
            content
        }
    }
}

/// A pan that begins only when the finger first moves leftward, as
/// `SwipeToReply.begins` says; its translation is in the window's
/// coordinates.
@available(iOS 18, *)
private struct LeftwardPan: UIGestureRecognizerRepresentable {
    let changed: (CGPoint) -> Void
    /// Let go (true), or cancelled.
    let ended: (Bool) -> Void

    func makeCoordinator(converter: CoordinateSpaceConverter) -> Coordinator {
        Coordinator()
    }

    func makeUIGestureRecognizer(context: Context) -> UIPanGestureRecognizer {
        let pan = UIPanGestureRecognizer()
        pan.delegate = context.coordinator
        return pan
    }

    func handleUIGestureRecognizerAction(_ recognizer: UIPanGestureRecognizer, context: Context) {
        switch recognizer.state {
        case .began, .changed:
            changed(recognizer.translation(in: recognizer.view?.window))
        case .ended:
            ended(true)
        case .cancelled, .failed:
            ended(false)
        default:
            break
        }
    }

    final class Coordinator: NSObject, UIGestureRecognizerDelegate {
        func gestureRecognizerShouldBegin(_ recognizer: UIGestureRecognizer) -> Bool {
            guard let pan = recognizer as? UIPanGestureRecognizer else { return false }
            return SwipeToReply.begins(velocity: pan.velocity(in: pan.view?.window))
        }
    }
}

/// The message a reply answers, above the reply's text: a bar, the sender
/// and a few lines. Tapping it goes to the original, once loaded.
struct QuoteView: View {
    let quote: ReplyQuote
    let own: Bool
    let goTo: () -> Void

    var body: some View {
        HStack(alignment: .top, spacing: 8) {
            RoundedRectangle(cornerRadius: 1.5)
                .fill(own ? Color.ownBubbleLink : Color.accentColor)
                .frame(width: 3)
            VStack(alignment: .leading, spacing: 1) {
                if let sender = quote.senderLabel {
                    Text(sender)
                        .font(.subheadline.weight(.semibold))
                        .lineLimit(1)
                }
                Text(quote.preview)
                    .font(.subheadline)
                    .lineLimit(3)
                    .opacity(0.8)
            }
        }
        .fixedSize(horizontal: false, vertical: true)
        .contentShape(Rectangle())
        .onTapGesture {
            if quote.canGoTo { goTo() }
        }
        .accessibilityElement(children: .combine)
        .accessibilityHint(quote.canGoTo ? "Goes to the original message" : "")
        .accessibilityAddTraits(quote.canGoTo ? .isButton : [])
    }
}

/// The reactions under a message: each key and its count, ours marked.
/// Tap: add or remove ours; hold: who reacted.
struct ReactionChips: View {
    let reactions: [Reaction]
    /// Lined up on the right (under our messages).
    let trailing: Bool
    let react: (String) -> Void
    let showReactors: (String) -> Void

    var body: some View {
        FlowLayout(spacing: 4, trailing: trailing) {
            ForEach(reactions, id: \.key) { reaction in
                HStack(spacing: 4) {
                    Text(reaction.key)
                    Text("\(reaction.count)")
                        .font(.caption.weight(.semibold))
                        .foregroundColor(reaction.byUs ? .accentColor : .secondary)
                }
                .padding(.horizontal, 8)
                .padding(.vertical, 4)
                .background(
                    reaction.byUs ? Color.accentColor.opacity(0.18) : Color(.secondarySystemBackground),
                    in: Capsule())
                .overlay(Capsule().strokeBorder(reaction.byUs ? Color.accentColor : .clear, lineWidth: 1))
                .contentShape(Capsule())
                .onTapGesture { react(reaction.key) }
                .onLongPressGesture { showReactors(reaction.key) }
                .accessibilityElement(children: .ignore)
                .accessibilityLabel("\(reaction.key) \(reaction.count)\(reaction.byUs ? ", yours" : "")")
                .accessibilityAddTraits(.isButton)
                .accessibilityAction { react(reaction.key) }
                .accessibilityAction(named: "Who reacted") { showReactors(reaction.key) }
            }
        }
    }
}

/// Who read up to a message: small avatars, at most three, then "+N".
/// Tapping lists them.
struct ReaderAvatarsView: View {
    let readers: [Reader]
    let show: () -> Void

    var body: some View {
        let (shown, more) = ReaderAvatars.split(readers)
        HStack(spacing: -4) {
            ForEach(shown, id: \.userId) { reader in
                AvatarView(name: reader.displayName, uri: reader.avatarUrl, size: 16)
                    .overlay(Circle().stroke(Color(.systemBackground), lineWidth: 1.5))
            }
            if more > 0 {
                Text("+\(more)")
                    .font(.caption2)
                    .foregroundStyle(.secondary)
                    .padding(.leading, 7)
            }
        }
        .padding(.vertical, 2)
        .contentShape(Rectangle())
        .onTapGesture(perform: show)
        .accessibilityElement(children: .ignore)
        .accessibilityLabel("Read by \(readers.map(\.displayName).joined(separator: ", "))")
        .accessibilityAddTraits(.isButton)
    }
}

extension MessageAction {
    var title: String {
        switch self {
        case .react: return "React"
        case .reply: return "Reply"
        case .copy: return "Copy"
        case .selectText: return "Select Text"
        case .edit: return "Edit"
        case .pin: return "Pin"
        case .unpin: return "Unpin"
        case .delete: return "Delete"
        case .retry: return "Retry"
        case .discard: return "Discard"
        }
    }

    var symbol: String {
        switch self {
        case .react: return "face.smiling"
        case .reply: return "arrowshape.turn.up.left"
        case .copy: return "doc.on.doc"
        case .selectText: return "selection.pin.in.out"
        case .edit: return "pencil"
        case .pin: return "pin"
        case .unpin: return "pin.slash"
        case .delete: return "trash"
        case .retry: return "arrow.clockwise"
        case .discard: return "xmark.bin"
        }
    }
}

/// Lays out its views in rows, wrapping to the next row when one is full.
struct FlowLayout: Layout {
    var spacing: CGFloat
    /// Rows lined up on the right.
    var trailing = false

    func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize {
        let rows = arrange(subviews, width: proposal.width ?? .infinity)
        let width = rows.map(\.width).max() ?? 0
        let height = rows.map(\.height).reduce(0, +) + spacing * CGFloat(max(0, rows.count - 1))
        return CGSize(width: width, height: height)
    }

    func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) {
        var y = bounds.minY
        for row in arrange(subviews, width: bounds.width) {
            var x = trailing ? bounds.maxX - row.width : bounds.minX
            for index in row.indices {
                let size = subviews[index].sizeThatFits(.unspecified)
                subviews[index].place(at: CGPoint(x: x, y: y), proposal: ProposedViewSize(size))
                x += size.width + spacing
            }
            y += row.height + spacing
        }
    }

    private struct Row {
        var indices: [Int] = []
        var width: CGFloat = 0
        var height: CGFloat = 0
    }

    private func arrange(_ subviews: Subviews, width: CGFloat) -> [Row] {
        var rows: [Row] = []
        var row = Row()
        for index in subviews.indices {
            let size = subviews[index].sizeThatFits(.unspecified)
            let needed = row.indices.isEmpty ? size.width : row.width + spacing + size.width
            if needed > width && !row.indices.isEmpty {
                rows.append(row)
                row = Row()
            }
            row.width = row.indices.isEmpty ? size.width : row.width + spacing + size.width
            row.height = max(row.height, size.height)
            row.indices.append(index)
        }
        if !row.indices.isEmpty {
            rows.append(row)
        }
        return rows
    }
}
