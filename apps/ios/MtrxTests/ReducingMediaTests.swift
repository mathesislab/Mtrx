import AVFoundation
import MtrxFFI
import XCTest
@testable import Mtrx

/// Photos and videos sent reduced or as taken (`docs/reducing-media.md`):
/// the setting, and a video's plan picked through to the encoders.
@MainActor
final class ReducingMediaTests: XCTestCase {
    private var session: FakeSession!
    private var defaults: UserDefaults!

    override func setUp() async throws {
        session = FakeSession(noHandle: .init())
        defaults = try XCTUnwrap(UserDefaults(suiteName: "media-\(UUID().uuidString)"))
    }

    func testTheSettingIsReducedUntilChangedWithItsWords() {
        XCTAssertEqual(MediaSetting.quality(in: defaults), .reduced)
        MediaSetting.set(.original, in: defaults)
        XCTAssertEqual(MediaSetting.quality(in: defaults), .original)
        XCTAssertEqual(MediaSetting.choices.map(\.title), ["Reduced", "Original"])
        XCTAssertEqual(
            MediaSetting.explanation(of: .reduced), "Photos up to 1 MB, videos at 720p. Location data is left out.")
        XCTAssertEqual(
            MediaSetting.explanation(of: .original), "As taken, with the photo's details such as its location.")
    }

    func testMessagesAreSentWithTheSettingOfTheMoment() async {
        let model = TimelineModel(session: session, room: Make.room("!room:example.org"), defaults: defaults)
        await model.open()
        model.draft = "Reduced"
        await model.send()?.value
        MediaSetting.set(.original, in: defaults)
        model.draft = "As taken"
        await model.send()?.value
        XCTAssertEqual(session.fakeTimeline.sent, ["Reduced", "As taken"])
        XCTAssertEqual(session.fakeTimeline.qualities, [.reduced, .original])
    }

    func testAVideosPlanIsAskedForAsStoredAndItsNumbersGoToTheEncoders() async throws {
        let plan = VideoPlan(width: 1280, height: 720, fps: 30, videoBitrate: 1_935_360, audioBitrate: 128_000)
        session.plan = plan
        let source = OutgoingVideo.Source(width: 3840, height: 2160, fps: 60, durationMs: 12_500, size: 80_000_000)
        let planned = try await OutgoingVideo.plan(source, session: session)
        XCTAssertEqual(planned, plan)
        XCTAssertEqual(session.planned, [source])

        let video = OutgoingVideo.videoSettings(plan)
        XCTAssertEqual(video[AVVideoWidthKey] as? Int, 1280)
        XCTAssertEqual(video[AVVideoHeightKey] as? Int, 720)
        let compression = try XCTUnwrap(video[AVVideoCompressionPropertiesKey] as? [String: Any])
        XCTAssertEqual(compression[AVVideoAverageBitRateKey] as? Int, 1_935_360)
        XCTAssertEqual(compression[AVVideoExpectedSourceFrameRateKey] as? Float, 30)
        let audio = OutgoingVideo.audioSettings(plan, channels: 2, sampleRate: 48_000)
        XCTAssertEqual(audio[AVEncoderBitRateKey] as? Int, 128_000)
        XCTAssertEqual(audio[AVNumberOfChannelsKey] as? Int, 2)
        XCTAssertEqual(audio[AVSampleRateKey] as? Double, 48_000)
    }

    func testAVideosStillGoesWithItsCopy() throws {
        let copy = Outgoing.newCopy(extension: "mp4")
        try Data("video".utf8).write(to: copy)
        let still = Outgoing.newCopy(extension: "jpg")
        try Data("still".utf8).write(to: still)
        let info = VideoInfo(width: 720, height: 1280, durationMs: 12_500, thumbnailPath: still.path)
        Outgoing.delete(FileToSend(path: copy.path, filename: "Video 2026-11-18 14.03.mp4", video: info))
        XCTAssertFalse(FileManager.default.fileExists(atPath: copy.path))
        XCTAssertFalse(FileManager.default.fileExists(atPath: still.path))
    }
}
