import MtrxFFI
import SwiftUI
import XCTest
@testable import Mtrx

final class MessageTextTests: XCTestCase {
    private let look = MessageText.Look(color: .primary)

    /// The one text segment of `runs`.
    private func text(_ runs: [TextRun]) -> AttributedString? {
        let segments = MessageText.segments(runs, look: look)
        guard segments.count == 1, case .text(let string) = segments[0] else { return nil }
        return string
    }

    func testLinksAndStylesAreAttributes() throws {
        let bold = TextStyle(
            bold: true, italic: false, strikethrough: false, underline: false, superscript: false,
            subscript: false, spoiler: false)
        let string = try XCTUnwrap(
            text([
                Make.plain("see "),
                .text(text: "this", style: bold, link: "https://example.org/a"),
            ]))
        XCTAssertEqual(String(string.characters), "see this")
        let runs = string.runs.map { (String(string[$0.range].characters), $0.link, $0.inlinePresentationIntent) }
        XCTAssertEqual(runs.count, 2)
        XCTAssertEqual(runs[0].0, "see ")
        XCTAssertNil(runs[0].1)
        XCTAssertEqual(runs[1].0, "this")
        XCTAssertEqual(runs[1].1, URL(string: "https://example.org/a"))
        XCTAssertEqual(runs[1].2, .stronglyEmphasized)
    }

    func testAMentionLinksToThePersonBoldIfUs() throws {
        let string = try XCTUnwrap(
            text([.mention(userId: "@ann:example.org", name: "Ann", style: .plain, you: true)]))
        let run = try XCTUnwrap(string.runs.first)
        let url = try XCTUnwrap(run.link)
        let person = try XCTUnwrap(MessageText.person(url))
        XCTAssertEqual(person.userId, "@ann:example.org")
        XCTAssertEqual(person.name, "Ann")
        XCTAssertEqual(run.inlinePresentationIntent, .stronglyEmphasized)
        // Other links are no person.
        XCTAssertNil(MessageText.person(try XCTUnwrap(URL(string: "https://matrix.to/#/@ann:example.org"))))
    }

    func testAFormulaNotTypesetIsLatexCode() throws {
        let formula = Formula(tex: "E=mc^2", display: false)
        let string = try XCTUnwrap(
            text([Make.plain("Energy "), .math(formula: formula, link: nil)]))
        XCTAssertEqual(String(string.characters), "Energy $E=mc^2$")
        let code = try XCTUnwrap(string.runs.last)
        XCTAssertEqual(String(string[code.range].characters), "$E=mc^2$")
        XCTAssertEqual(code.inlinePresentationIntent, .code)
        XCTAssertEqual(MessageText.fallback(Formula(tex: "x", display: true)), "$$x$$")
    }

    func testATypesetFormulaIsASegmentOfItsOwn() {
        let formula = Formula(tex: "x", display: false)
        let typeset = TypesetFormula(image: UIImage(), descent: 2)
        let segments = MessageText.segments(
            [Make.plain("a "), .math(formula: formula, link: nil), Make.plain(" b")], look: look
        ) { $0 == formula ? typeset : nil }
        XCTAssertEqual(segments.count, 3)
        guard case .formula(let image, let tex) = segments[1] else { return XCTFail("not a formula") }
        XCTAssertTrue(image === typeset)
        XCTAssertEqual(tex, "x")
    }

    func testFormulasAndWhereEditedGoes() {
        let inline = Formula(tex: "a", display: false)
        let shown = Formula(tex: "b", display: true)
        let blocks = [
            Make.paragraph([.math(formula: inline, link: nil)]),
            TextBlock(kind: .math(formula: shown), quotes: 0, lists: 0, marker: nil),
        ]
        XCTAssertEqual(MessageText.formulas(blocks), [inline, shown])
        // After display math, on a line of its own.
        XCTAssertFalse(MessageText.editedInline(blocks))
        XCTAssertTrue(MessageText.editedInline([Make.paragraph([Make.plain("hi")])]))
    }
}
