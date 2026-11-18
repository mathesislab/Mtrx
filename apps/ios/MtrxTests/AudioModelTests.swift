import MtrxFFI
import XCTest
@testable import Mtrx

/// A room's audio messages: one plays at a time, each keeps its position
/// while the room is open and goes back to the start at its end; closing
/// the room lets go of them.
@MainActor
final class AudioModelTests: XCTestCase {
    private var session: FakeSession!
    private var model: AudioModel!
    /// The players made, in order.
    private var players: [FakeAudioPlayer] = []
    /// 42 s.
    private let voice = Make.audio("mxc://example.org/voice")
    /// 10 minutes.
    private let talk = Make.audio("mxc://example.org/talk", name: "Talk.m4a", seconds: 600)

    override func setUp() async throws {
        session = FakeSession(noHandle: .init())
        session.audioFiles = [
            voice.source: .success("/cache/audio/voice.wav"), talk.source: .success("/cache/audio/talk.m4a"),
        ]
        players = []
        model = AudioModel(session: session) { [unowned self] file, _ in
            let player = FakeAudioPlayer(file: file, duration: file.lastPathComponent == "voice.wav" ? 42 : 600)
            self.players.append(player)
            return player
        }
    }

    /// Start `media`, and wait for its file.
    private func play(_ media: Media) async {
        model.toggle(media)
        await eventually { self.model.state(of: media) == .playing }
    }

    func testPlayingAnotherPausesTheFirstWhichKeepsItsPosition() async {
        await play(voice)
        let first = players[0]
        XCTAssertTrue(first.playing)
        XCTAssertEqual(model.duration, 42)
        first.currentTime = 7
        await play(talk)
        XCTAssertFalse(first.playing)
        XCTAssertEqual(model.current, talk.source)
        XCTAssertNil(model.state(of: voice))
        XCTAssertEqual(model.position(of: voice), 7)
        // Back to the first: from where it was.
        await play(voice)
        XCTAssertEqual(players.count, 3)
        XCTAssertEqual(players[2].currentTime, 7)
        XCTAssertTrue(players[2].playing)
        XCTAssertFalse(players[1].playing)
    }

    func testTheEndGoesBackToTheStart() async {
        await play(voice)
        players[0].currentTime = 40
        players[0].end()
        XCTAssertEqual(model.state(of: voice), .paused)
        XCTAssertEqual(model.position(of: voice), 0)
        // Again: from the start, with the same player.
        model.toggle(voice)
        XCTAssertEqual(model.state(of: voice), .playing)
        XCTAssertEqual(players.count, 1)
        XCTAssertTrue(players[0].playing)
        XCTAssertEqual(players[0].currentTime, 0)
    }

    func testSeekingAMessageNotPlayingSetsWhereItStarts() async {
        await play(voice)
        model.seek(talk, to: 90)
        XCTAssertEqual(model.position(of: talk), 90)
        XCTAssertTrue(players[0].playing)
        await play(talk)
        XCTAssertEqual(players[1].currentTime, 90)
        XCTAssertEqual(model.position, 90)
        XCTAssertEqual(model.duration, 600)
    }

    /// Closing the room screen releases the model, and with it the
    /// positions; its fetch is cancelled.
    func testClosingTheRoomCancelsTheFetch() async {
        let slow = Make.audio("mxc://example.org/slow")
        model.toggle(slow)
        await eventually { self.session.audioFetches == [slow.source] }
        XCTAssertEqual(model.state(of: slow), .fetching(nil))
        model = nil
        await eventually { self.session.audioCancelled == [slow.source] }
    }

    /// Closing the room screen releases the player, which stops the sound.
    func testClosingTheRoomReleasesThePlayer() async {
        await play(voice)
        weak var player = players.first
        players = []
        model = nil
        XCTAssertNil(player)
    }

    /// A tap while fetching cancels; a failure says why, and the button
    /// tries again.
    func testTheButtonCancelsAFetchAndRetriesAFailure() async {
        let slow = Make.audio("mxc://example.org/slow")
        model.toggle(slow)
        model.toggle(slow)
        XCTAssertNil(model.current)
        await eventually { self.session.audioCancelled == [slow.source] }
        session.audioFiles[voice.source] = .failure(MtrxError.Other(reason: "The file is damaged"))
        model.toggle(voice)
        await eventually { self.model.state(of: self.voice) == .failed("The file is damaged") }
        session.audioFiles[voice.source] = .success("/cache/audio/voice.wav")
        await play(voice)
        XCTAssertEqual(session.audioFetches, [slow.source, voice.source, voice.source])
    }
}
