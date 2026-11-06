import XCTest

/// The app with made-up messages (`-ui-testing`), in the Lab room, as a
/// finger uses it: deleting one of our messages through its menu, sending,
/// and messages arriving; a room opening at its newest message; a chat's
/// shared rooms; a room opened over another; folding the Rooms tab's
/// groups. Each step is saved as a PNG where `SCREENSHOTS_DIR` says.
final class RoomUITests: XCTestCase {
    private var app: XCUIApplication!

    override func setUp() async throws {
        continueAfterFailure = false
        await MainActor.run {
            let app = XCUIApplication()
            app.launchArguments = ["-ui-testing"]
            app.launch()
            self.app = app
        }
    }

    /// The Lab room, from the Rooms tab: its last message ours.
    @MainActor
    private func openLab() {
        app.tabBars.buttons["Rooms"].tap()
        app.staticTexts["Lab"].firstMatch.tap()
        XCTAssertTrue(app.staticTexts["Mine, to delete"].waitForExistence(timeout: 15), "The room did not open")
        snap("ui-room")
    }

    @MainActor
    func testDeletingASentMessage() {
        openLab()
        app.staticTexts["Mine, to delete"].press(forDuration: 1.5)
        let delete = app.buttons["Delete"].firstMatch
        let inMenu = delete.waitForExistence(timeout: 5)
        snap("ui-delete-menu")
        XCTAssertTrue(inMenu, "No Delete in the message's menu")
        delete.tap()
        sleep(1)
        let confirm = app.buttons["Delete"].firstMatch
        let asked = confirm.waitForExistence(timeout: 5)
        snap("ui-delete-dialog")
        XCTAssertTrue(asked, "No dialog to confirm")
        confirm.tap()
        let deleted = app.staticTexts["Message deleted"].waitForExistence(timeout: 5)
        snap("ui-deleted")
        XCTAssertTrue(deleted, "The message was not deleted")
    }

    @MainActor
    func testTheTimelineStaysShownWhileSending() {
        openLab()
        let field = app.textViews.firstMatch.exists ? app.textViews.firstMatch : app.textFields.firstMatch
        field.tap()
        let newest = inView("Mine, to delete")
        snap("ui-keyboard")
        XCTAssertTrue(newest, "The newest message is under the keyboard")
        for n in 1...4 {
            field.tap()
            field.typeText("Sent number \(n), with a few words")
            snap("ui-written-\(n)")
            app.buttons["Send"].tap()
            let shown = inView("Sent number \(n), with a few words")
            snap("ui-sent-\(n)")
            XCTAssertTrue(shown, "Sent message \(n) not in view")
        }
    }

    /// A direct chat's `#` lists the rooms shared with Ann; Models opens in
    /// the Rooms tab, its space at the right of its header, which shows the
    /// space.
    @MainActor
    func testSharedRoomsAndARoomsSpace() {
        app.staticTexts["Ann"].firstMatch.tap()
        let shared = app.buttons["Shared rooms"]
        XCTAssertTrue(shared.waitForExistence(timeout: 15), "No # in the chat's header")
        shared.tap()
        let models = app.staticTexts["Models"].firstMatch
        let listed = models.waitForExistence(timeout: 5)
        snap("ui-shared-rooms")
        XCTAssertTrue(listed, "Models not among the shared rooms")
        XCTAssertTrue(app.staticTexts["Lab"].exists, "Lab not among the shared rooms")
        models.tap()
        let team = app.buttons["Team"]
        let inRoom = team.waitForExistence(timeout: 5)
        snap("ui-room-space")
        XCTAssertTrue(inRoom, "No space at the right of the room's header")
        team.tap()
        XCTAssertTrue(app.navigationBars.staticTexts["Team"].waitForExistence(timeout: 5), "The space did not open")
        snap("ui-space")
    }

    /// With a message of Ann's arriving every two seconds: the newest stays
    /// in view at the bottom, the keyboard up or down; scrolled up, the
    /// timeline is left where it is, and its button leads back to the
    /// newest, which then stays in view again.
    @MainActor
    func testTheNewestStaysInViewAsMessagesArrive() {
        app.terminate()
        app.launchArguments = ["-ui-testing", "-ui-testing-arrivals"]
        app.launch()
        openLab()
        XCTAssertTrue(arrived(2).waitForExistence(timeout: 8), "Arrived 2 did not arrive")
        XCTAssertTrue(newestInView(), "The newest not in view")
        let field = app.textViews.firstMatch.exists ? app.textViews.firstMatch : app.textFields.firstMatch
        field.tap()
        XCTAssertTrue(arrived(4).waitForExistence(timeout: 8), "Arrived 4 did not arrive")
        let up = newestInView()
        snap("ui-arrived-keyboard")
        XCTAssertTrue(up, "The newest not in view with the keyboard up")
        let window = app.windows.firstMatch
        window.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.2))
            .press(forDuration: 0.05, thenDragTo: window.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.45)))
        let button = app.buttons["Scroll to the newest message"]
        XCTAssertTrue(button.waitForExistence(timeout: 5), "No button to the newest message")
        // Left where it is while more arrive.
        sleep(3)
        snap("ui-arrived-scrolled-up")
        XCTAssertTrue(button.exists, "Back at the bottom by itself")
        button.tap()
        sleep(3)
        let back = newestInView()
        snap("ui-arrived-back")
        XCTAssertTrue(back, "The newest is not in view after the button")
        XCTAssertFalse(button.exists, "The button stayed")
    }

    /// A room whose newest messages are far taller than its older ones,
    /// loaded after it opens, shows its newest message, at the bottom.
    @MainActor
    func testARoomOpensAtItsNewestMessage() {
        app.terminate()
        app.launchArguments = ["-ui-testing", "-ui-testing-history"]
        app.launch()
        openLab()
        // Its older messages loaded.
        sleep(4)
        snap("ui-opened-loaded")
        XCTAssertTrue(inView("Mine, to delete"), "The newest message is not in view")
        XCTAssertFalse(app.buttons["Scroll to the newest message"].exists, "Not at the bottom")
    }

    /// A room opened from its space while another is open in the Rooms tab
    /// shows its own messages and header, and keeps them after a message is
    /// sent in it while another room's arrive, the rooms changing order.
    @MainActor
    func testARoomOpenedOverAnotherShowsItsOwnMessages() {
        app.terminate()
        app.launchArguments = ["-ui-testing", "-ui-testing-arrivals-papers"]
        app.launch()
        app.tabBars.buttons["Rooms"].tap()
        app.staticTexts["Papers"].firstMatch.tap()
        XCTAssertTrue(message("Papers 29").waitForExistence(timeout: 15), "Papers did not open")
        app.buttons["Team"].tap()
        XCTAssertTrue(app.navigationBars.staticTexts["Team"].waitForExistence(timeout: 5), "The space did not open")
        app.staticTexts["Models"].firstMatch.tap()
        let models = message("Models 29")
        let shown = models.waitForExistence(timeout: 5)
        snap("ui-over-another")
        XCTAssertTrue(shown, "Models' messages not shown")
        XCTAssertFalse(message("Papers 29").exists, "Papers' messages shown in Models")
        let field = app.textViews.firstMatch.exists ? app.textViews.firstMatch : app.textFields.firstMatch
        field.tap()
        field.typeText("Sent in Models")
        app.buttons["Send"].tap()
        // Papers' messages arrive every two seconds, moving it up, and
        // Models up when sent in.
        sleep(5)
        snap("ui-over-another-sent")
        XCTAssertTrue(app.navigationBars.staticTexts["Models"].exists, "Not Models' header")
        XCTAssertTrue(app.staticTexts["Sent in Models"].exists, "The message sent is not shown")
        XCTAssertTrue(models.exists, "Models' messages gone")
        XCTAssertFalse(message("Papers 29").exists, "Papers' messages shown in Models")
        XCTAssertFalse(arrived(1).exists, "Papers' arrivals shown in Models")
    }

    /// The Rooms tab's Team: its caret folds it, its unread count then on
    /// its heading, and unfolds it; its name shows it in the Spaces tab.
    @MainActor
    func testFoldingARoomsTabGroup() {
        app.tabBars.buttons["Rooms"].tap()
        let models = app.staticTexts["Models"].firstMatch
        XCTAssertTrue(models.waitForExistence(timeout: 15), "No Models in the Rooms tab")
        let heading = app.buttons.matching(NSPredicate(format: "label BEGINSWITH 'Team'")).firstMatch
        snap("ui-rooms-unfolded")
        XCTAssertFalse(heading.label.contains("unread"), "A count on the unfolded heading: \(heading.label)")
        app.buttons["Fold Team"].tap()
        XCTAssertTrue(app.buttons["Unfold Team"].waitForExistence(timeout: 5), "Team did not fold")
        snap("ui-rooms-folded")
        XCTAssertFalse(models.exists, "Models shown in folded Team")
        XCTAssertTrue(heading.label.contains("2 unread"), "No count on the folded heading: \(heading.label)")
        XCTAssertTrue(app.staticTexts["Lab"].exists, "Home folded too")
        app.buttons["Unfold Team"].tap()
        XCTAssertTrue(models.waitForExistence(timeout: 5), "Team did not unfold")
        heading.tap()
        XCTAssertTrue(app.navigationBars.staticTexts["Team"].waitForExistence(timeout: 5), "The space did not open")
    }


    /// A message whose text starts with `start` and a comma.
    @MainActor
    private func message(_ start: String) -> XCUIElement {
        app.staticTexts.matching(NSPredicate(format: "label BEGINSWITH %@", "\(start),")).firstMatch
    }

    /// Ann's arrived message `n`.
    @MainActor
    private func arrived(_ n: Int) -> XCUIElement {
        app.staticTexts.matching(NSPredicate(format: "label BEGINSWITH %@", "Arrived \(n),")).firstMatch
    }

    /// The arrived message numbered highest is in view, once the keyboard
    /// and the timeline have moved: which is newest and where it is, read
    /// from the same look, as more arrive between two.
    @MainActor
    private func newestInView() -> Bool {
        settles { texts in
            let arrived: [(number: Int, inView: Bool)] = texts.compactMap { text in
                guard text.label.hasPrefix("Arrived "),
                    let number = Int(text.label.dropFirst("Arrived ".count).prefix { $0.isNumber })
                else { return nil }
                return (number, text.inView)
            }
            return arrived.max { $0.number < $1.number }?.inView ?? false
        }
    }

    /// The message `label` is in view, once the keyboard and the timeline
    /// have moved.
    @MainActor
    private func inView(_ label: String) -> Bool {
        settles { texts in texts.contains { $0.label == label && $0.inView } }
    }

    /// Whether `shown` holds of a look at the timeline, looked at again
    /// until it does or until a look taken `settle` seconds on does not.
    /// A look can take seconds on a slow machine: the verdict is never
    /// that of a look taken before the app had `settle` seconds.
    @MainActor
    private func settles(
        within settle: TimeInterval = 5, _ shown: ([(label: String, inView: Bool)]) -> Bool
    ) -> Bool {
        let end = Date().addingTimeInterval(settle)
        while true {
            let last = Date() >= end
            if shown(timelineTexts()) {
                return true
            }
            if last {
                return false
            }
            usleep(200_000)
        }
    }

    /// The texts in the timeline, from one snapshot of the app, each with
    /// whether it is in view: its middle in the timeline's scroll view,
    /// below the header (the timeline is drawn under it) and above the
    /// keyboard. One query, however many messages, and no hit test: a
    /// second query each, which records a failure for a text scrolled off
    /// the screen in between.
    @MainActor
    private func timelineTexts() -> [(label: String, inView: Bool)] {
        guard let snapshot = try? app.snapshot() else { return [] }
        var header: CGFloat = 0
        var keyboard = CGFloat.infinity
        var texts: [(label: String, frame: CGRect, view: CGRect)] = []
        func collect(_ element: XCUIElementSnapshot, in scrollView: CGRect?) {
            switch element.elementType {
            case .navigationBar:
                header = max(header, element.frame.maxY)
            case .keyboard:
                keyboard = min(keyboard, element.frame.minY)
            case .staticText:
                if let scrollView {
                    texts.append((element.label, element.frame, scrollView))
                }
            default:
                break
            }
            let scrollView = element.elementType == .scrollView ? element.frame : scrollView
            element.children.forEach { collect($0, in: scrollView) }
        }
        collect(snapshot, in: nil)
        return texts.map { text in
            let middle = CGPoint(x: text.frame.midX, y: text.frame.midY)
            return (text.label, text.view.contains(middle) && middle.y > header && middle.y < keyboard)
        }
    }

    @MainActor
    private func snap(_ name: String) {
        let shot = XCUIScreen.main.screenshot()
        let attachment = XCTAttachment(screenshot: shot)
        attachment.name = name
        attachment.lifetime = .keepAlways
        add(attachment)
        if let folder = ProcessInfo.processInfo.environment["SCREENSHOTS_DIR"] {
            try? FileManager.default.createDirectory(atPath: folder, withIntermediateDirectories: true)
            try? shot.pngRepresentation.write(to: URL(fileURLWithPath: folder).appendingPathComponent("\(name).png"))
        }
    }
}
