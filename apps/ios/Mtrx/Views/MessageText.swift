import MtrxFFI
import SwiftUI
import UIKit

/// A message's text (`Message.text`), as the library finished it: blocks
/// top to bottom with their quote bars, list indents and markers. A
/// paragraph is one `Text` of styled runs with its formulas typeset inline
/// (`$tex$` in monospace until they are, or if they cannot be); display
/// math is centred, scaled down to 60 % to fit, then scrolls. Tapping it
/// shows its spoilers; links open through the environment's `openURL`
/// (mentions as `MessageText.personURL`).
struct MessageTextView: View {
    let blocks: [TextBlock]
    /// " (edited)" after the text.
    let edited: Bool
    /// The text's colour: code backgrounds and spoilers are shades of it.
    let color: Color
    /// "(edited)" and quote bars.
    let muted: Color

    @EnvironmentObject private var images: ImageCache
    @Environment(\.displayScale) private var displayScale
    @Environment(\.dynamicTypeSize) private var dynamicTypeSize
    /// Typeset since shown (the cache may drop them meanwhile).
    @State private var typeset: [FormulaKey: TypesetFormula] = [:]
    @State private var spoilersShown = false

    var body: some View {
        let hidesSpoilers = !spoilersShown && MessageText.hasSpoilers(blocks)
        VStack(alignment: .leading, spacing: 6) {
            ForEach(blocks.indices, id: \.self) { index in
                block(blocks[index], last: index == blocks.count - 1)
            }
            if edited && !MessageText.editedInline(blocks) {
                editedMark
            }
        }
        .contentShape(Rectangle())
        .gesture(TapGesture().onEnded { spoilersShown = true }, including: hidesSpoilers ? .all : .subviews)
        .task(id: emPixels) { await load(emPixels) }
    }

    /// The text size Dynamic Type sets.
    private var traits: UITraitCollection {
        UITraitCollection(preferredContentSizeCategory: UIContentSizeCategory(dynamicTypeSize))
    }

    /// The body text size in pixels.
    private var emPixels: UInt32 {
        let points = UIFont.preferredFont(forTextStyle: .body, compatibleWith: traits).pointSize
        return UInt32((points * displayScale).rounded())
    }

    /// `formula` typeset at the body text size, if it is (yet).
    private func image(_ formula: Formula) -> TypesetFormula? {
        let key = FormulaKey(formula: formula, emPixels: emPixels, scale: displayScale)
        return typeset[key] ?? images.cachedFormula(key)
    }

    /// Typeset the formulas not cached, one after another.
    private func load(_ emPixels: UInt32) async {
        for formula in MessageText.formulas(blocks) {
            let key = FormulaKey(formula: formula, emPixels: emPixels, scale: displayScale)
            guard typeset[key] == nil, images.cachedFormula(key) == nil else { continue }
            guard let image = await images.formula(key), !Task.isCancelled else { continue }
            typeset[key] = image
        }
    }

    private var look: MessageText.Look {
        MessageText.Look(
            color: color, spoilersShown: spoilersShown,
            shift: UIFontMetrics(forTextStyle: .body).scaledValue(for: 5, compatibleWith: traits))
    }

    private var editedMark: Text {
        Text(" (edited)").font(.footnote).foregroundColor(muted)
    }

    private func block(_ block: TextBlock, last: Bool) -> some View {
        HStack(alignment: .top, spacing: 0) {
            ForEach(0..<Int(block.quotes), id: \.self) { _ in
                RoundedRectangle(cornerRadius: 1.5)
                    .fill(muted)
                    .frame(width: 3)
                    .padding(.trailing, 8)
            }
            HStack(alignment: .firstTextBaseline, spacing: 0) {
                if block.lists > 0 {
                    Text(block.marker ?? "")
                        .monospacedDigit()
                        .fixedSize()
                        .frame(minWidth: 20, alignment: .leading)
                        .padding(.leading, CGFloat(block.lists - 1) * 16)
                        .accessibilityHidden(block.marker == nil)
                }
                content(block.kind, last: last)
            }
        }
        .fixedSize(horizontal: false, vertical: true)
    }

    @ViewBuilder
    private func content(_ kind: BlockKind, last: Bool) -> some View {
        switch kind {
        case .paragraph(let runs):
            paragraph(runs, last: last)
        case .heading(_, let runs):
            paragraph(runs, last: last).font(.title3.bold())
        case .code(let code, _):
            Text(code)
                .font(.callout.monospaced())
                .padding(.horizontal, 8)
                .padding(.vertical, 6)
                .background(color.opacity(0.15), in: RoundedRectangle(cornerRadius: 8))
        case .math(let formula):
            if let typeset = image(formula) {
                DisplayFormula(typeset: typeset, tex: formula.tex)
            } else {
                Text(MessageText.fallback(formula)).font(.body.monospaced())
            }
        case .rule:
            Rectangle()
                .fill(muted.opacity(0.5))
                .frame(height: 1)
                .frame(maxWidth: .infinity)
                .padding(.vertical, 4)
        }
    }

    private func paragraph(_ runs: [TextRun], last: Bool) -> Text {
        let segments = MessageText.segments(runs, look: look, typeset: image)
        let text = segments.reduce(Text(verbatim: "")) { text, segment in
            switch segment {
            case .text(let string):
                return text + Text(string)
            case .formula(let typeset, let tex):
                return text + inline(typeset, tex: tex)
            }
        }
        return edited && last ? text + editedMark : text
    }

    /// An inline formula, its baseline on the line's, in the text's colour.
    private func inline(_ typeset: TypesetFormula, tex: String) -> Text {
        guard let image = typeset.image.cgImage else { return Text(MessageText.fallback(Formula(tex: tex, display: false))) }
        return Text(Image(image, scale: typeset.image.scale, label: Text(tex)).renderingMode(.template))
            .baselineOffset(-typeset.descent)
    }
}

/// Display math, centred: scaled down to fit, to 60 % at most, then
/// scrolling sideways.
private struct DisplayFormula: View {
    let typeset: TypesetFormula
    let tex: String

    var body: some View {
        let width = typeset.image.size.width
        ViewThatFits(in: .horizontal) {
            image.frame(minWidth: width * 0.6, idealWidth: width * 0.6, maxWidth: width)
            ScrollView(.horizontal, showsIndicators: false) {
                image.frame(width: width * 0.6)
            }
        }
        .frame(maxWidth: .infinity)
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(tex)
    }

    private var image: some View {
        Image(uiImage: typeset.image)
            .renderingMode(.template)
            .resizable()
            .aspectRatio(contentMode: .fit)
    }
}

private typealias SwiftUIAttributes = AttributeScopes.SwiftUIAttributes

/// The pieces of a message's text, as `MessageTextView` draws them.
enum MessageText {
    /// What a run looks like besides its style.
    struct Look {
        /// The text's colour.
        var color: Color
        var spoilersShown = false
        /// How far superscript is raised and subscript lowered, in points.
        var shift: CGFloat = 5
    }

    /// A piece of a paragraph: styled text, or a typeset formula (with its
    /// LaTeX, for screen readers).
    enum Segment {
        case text(AttributedString)
        case formula(TypesetFormula, tex: String)
    }

    /// `runs` as text and formulas: links as `.link` (a mention's is its
    /// `personURL`), a formula `typeset` gives no image for as `$tex$` code.
    /// Text next to text is one segment.
    static func segments(
        _ runs: [TextRun], look: Look, typeset: (Formula) -> TypesetFormula? = { _ in nil }
    ) -> [Segment] {
        var segments: [Segment] = []
        func append(_ string: AttributedString) {
            if case .text(let before) = segments.last {
                segments[segments.count - 1] = .text(before + string)
            } else {
                segments.append(.text(string))
            }
        }
        for run in runs {
            switch run {
            case .text(let text, let style, let link):
                append(attributed(text, style, link: link.flatMap(URL.init(string:)), look: look))
            case .code(let code, let style, let link):
                append(inlineCode(code, style, link: link, look: look))
            case .math(let formula, let link):
                if let image = typeset(formula) {
                    segments.append(.formula(image, tex: formula.tex))
                } else {
                    append(inlineCode(fallback(formula), TextStyle.plain, link: link, look: look))
                }
            case .mention(let userId, let name, var style, let you):
                style.bold = style.bold || you
                append(attributed(name, style, link: personURL(userId: userId, name: name), look: look))
            }
        }
        return segments
    }

    /// A formula as text, before it is typeset or if it cannot be.
    static func fallback(_ formula: Formula) -> String {
        formula.display ? "$$\(formula.tex)$$" : "$\(formula.tex)$"
    }

    private static func inlineCode(_ code: String, _ style: TextStyle, link: String?, look: Look) -> AttributedString {
        var string = attributed(code, style, link: link.flatMap(URL.init(string:)), look: look)
        string.inlinePresentationIntent = (string.inlinePresentationIntent ?? []).union(.code)
        if !style.spoiler || look.spoilersShown {
            string[SwiftUIAttributes.BackgroundColorAttribute.self] = look.color.opacity(0.15)
        }
        return string
    }

    private static func attributed(_ text: String, _ style: TextStyle, link: URL?, look: Look) -> AttributedString {
        var string = AttributedString(text)
        var intent: InlinePresentationIntent = []
        if style.bold { intent.insert(.stronglyEmphasized) }
        if style.italic { intent.insert(.emphasized) }
        if !intent.isEmpty { string.inlinePresentationIntent = intent }
        if style.strikethrough { string[SwiftUIAttributes.StrikethroughStyleAttribute.self] = .single }
        if style.underline { string[SwiftUIAttributes.UnderlineStyleAttribute.self] = .single }
        if style.superscript || style.subscript {
            string[SwiftUIAttributes.FontAttribute.self] = .footnote
            string[SwiftUIAttributes.BaselineOffsetAttribute.self] = style.superscript ? look.shift : -look.shift / 2
        }
        if style.spoiler && !look.spoilersShown {
            // A filled box: the text in the colour of its background.
            string[SwiftUIAttributes.ForegroundColorAttribute.self] = look.color.opacity(0.35)
            string[SwiftUIAttributes.BackgroundColorAttribute.self] = look.color.opacity(0.35)
        }
        string.link = link
        return string
    }

    /// The link of a mention: `mtrx-person:<user ID>?name=<name>`, which the
    /// row turns into its person sheet.
    static func personURL(userId: String, name: String) -> URL? {
        var components = URLComponents()
        components.scheme = personScheme
        components.path = userId
        components.queryItems = [URLQueryItem(name: "name", value: name)]
        return components.url
    }

    /// The person a mention's link is to.
    static func person(_ url: URL) -> (userId: String, name: String)? {
        guard url.scheme == personScheme,
              let components = URLComponents(url: url, resolvingAgainstBaseURL: false),
              !components.path.isEmpty
        else { return nil }
        let name = components.queryItems?.first { $0.name == "name" }?.value ?? components.path
        return (components.path, name)
    }

    private static let personScheme = "mtrx-person"

    /// Every formula, inline or displayed, in order.
    static func formulas(_ blocks: [TextBlock]) -> [Formula] {
        blocks.flatMap { block -> [Formula] in
            switch block.kind {
            case .paragraph(let runs), .heading(_, let runs):
                return runs.compactMap { run in
                    if case .math(let formula, _) = run { return formula }
                    return nil
                }
            case .math(let formula):
                return [formula]
            case .code, .rule:
                return []
            }
        }
    }

    static func hasSpoilers(_ blocks: [TextBlock]) -> Bool {
        blocks.contains { block in
            switch block.kind {
            case .paragraph(let runs), .heading(_, let runs):
                return runs.contains { run in
                    switch run {
                    case .text(_, let style, _), .code(_, let style, _), .mention(_, _, let style, _):
                        return style.spoiler
                    case .math:
                        return false
                    }
                }
            case .code, .math, .rule:
                return false
            }
        }
    }

    /// "(edited)" goes after the last paragraph's text, else on a line of
    /// its own (after code, math or a rule).
    static func editedInline(_ blocks: [TextBlock]) -> Bool {
        switch blocks.last?.kind {
        case .paragraph, .heading: return true
        default: return false
        }
    }
}

extension TextStyle {
    static let plain = TextStyle(
        bold: false, italic: false, strikethrough: false, underline: false, superscript: false,
        subscript: false, spoiler: false)
}
