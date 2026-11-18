import AVFoundation
import Combine
import Foundation
import MtrxFFI

/// A room's audio messages, played in their bubbles one at a time: the
/// file from the library (Ogg Opus as WAV), played by a player
/// (`SystemAudioPlayer`; a fake in tests). Starting one pauses the one
/// playing, which keeps its position; each keeps its position while the
/// room is open, and goes back to the start at its end. One per room
/// screen, injected like `DownloadsModel`, so that its ticks redraw only
/// the audio bubbles; closing the screen releases it and its player, which
/// stops the sound.
@MainActor
final class AudioModel: ObservableObject {
    enum State: Equatable {
        /// Getting the file: how far, nil while unknown or converting.
        case fetching(Float?)
        case playing
        case paused
        /// It can't play: why.
        case failed(String)
    }

    /// When the player can't read or decode the file.
    static let cantPlay = "Can't play this file"

    /// The message playing, paused, being fetched or failed (its
    /// `Media.source`).
    @Published private(set) var current: String?
    @Published private(set) var state: State?
    /// The current message's position.
    @Published private(set) var position: TimeInterval = 0
    /// The current message's length: the player's once loaded, else the
    /// event's.
    @Published private(set) var duration: TimeInterval?

    private let session: Session
    /// A player for a file, and whether it's a voice message.
    private let makePlayer: @MainActor (URL, Bool) throws -> AudioPlaying
    /// The current message's, once its file is here.
    private var player: AudioPlaying?
    /// Where the other messages stopped or were moved to, by source.
    private var kept: [String: TimeInterval] = [:]
    /// The fetch, and the ticks while playing.
    private let work = Work()

    init(
        session: Session,
        makePlayer: @escaping @MainActor (URL, Bool) throws -> AudioPlaying = {
            try SystemAudioPlayer(url: $0, voice: $1)
        }
    ) {
        self.session = session
        self.makePlayer = makePlayer
    }

    /// What `media` is doing, if it's the current message.
    func state(of media: Media) -> State? {
        media.source == current ? self.state : nil
    }

    /// Where `media` is: the current message's position, else where it
    /// stopped or was moved to (0 if neither).
    func position(of media: Media) -> TimeInterval {
        media.source == current ? self.position : kept[media.source] ?? 0
    }

    /// The length of `media`: the player's once loaded, else the event's.
    func duration(of media: Media) -> TimeInterval? {
        media.source == current ? self.duration : media.duration
    }

    /// The play button: play, pause, resume, cancel the fetch or try again.
    func toggle(_ media: Media) {
        switch state(of: media) {
        case .playing: pause()
        case .paused: resume()
        case .fetching: leave()
        case .failed, nil: start(media)
        }
    }

    /// Move `media` to `time`: the player, if it's the current message (it
    /// starts there if still fetched); else where it starts.
    func seek(_ media: Media, to time: TimeInterval) {
        guard media.source == current else {
            kept[media.source] = time
            return
        }
        position = time
        player?.currentTime = time
    }

    /// Pause the message playing, if any (the app goes to the background);
    /// a fetch is cancelled, so that it doesn't start there.
    func pause() {
        if case .fetching = self.state {
            return leave()
        }
        guard self.state == .playing, let player else { return }
        player.pause()
        position = player.currentTime
        state = .paused
        work.cancel("ticks")
    }

    /// Fetch `media`'s file and play it from its position, after letting go
    /// of the current message.
    private func start(_ media: Media) {
        leave()
        let source = media.source
        current = source
        position = kept.removeValue(forKey: source) ?? 0
        duration = media.duration
        state = .fetching(nil)
        let session = self.session
        let progress = FetchProgress { [weak self] fraction in
            guard let self, self.current == source, case .fetching = self.state else { return }
            self.state = .fetching(fraction)
        }
        work.load("fetch") { [weak self] in
            do {
                let path = try await session.audioFile(media: media, progress: progress)
                guard !Task.isCancelled else { return }
                self?.play(URL(fileURLWithPath: path), voice: media.voice)
            } catch {
                guard !Task.isCancelled else { return }
                self?.state = .failed(errorMessage(error) ?? AudioModel.cantPlay)
            }
        }
    }

    /// The current message's file is here: play it from its position.
    private func play(_ file: URL, voice: Bool) {
        let player: AudioPlaying
        do {
            player = try makePlayer(file, voice)
        } catch {
            state = .failed(Self.cantPlay)
            return
        }
        player.stopped = { [weak self, weak player] stop in
            guard let self, let player, self.player === player else { return }
            self.stopped(stop)
        }
        player.currentTime = position
        if player.duration > 0 {
            duration = player.duration
        }
        self.player = player
        resume()
    }

    /// Play the current message's player from where it is.
    private func resume() {
        guard let player else { return }
        guard player.play() else {
            self.player = nil
            state = .failed(Self.cantPlay)
            return
        }
        state = .playing
        work.load("ticks") { [weak self] in
            while !Task.isCancelled {
                try? await Task.sleep(nanoseconds: 100_000_000)
                guard let self, !Task.isCancelled, let player = self.player else { return }
                self.position = player.currentTime
            }
        }
    }

    /// The player stopped by itself.
    private func stopped(_ stop: AudioStop) {
        work.cancel("ticks")
        switch stop {
        case .ended:
            player?.currentTime = 0
            position = 0
            state = .paused
        case .paused:
            position = player?.currentTime ?? self.position
            state = .paused
        case .failed:
            player = nil
            state = .failed(Self.cantPlay)
        }
    }

    /// Let go of the current message, keeping its position: pause and
    /// release its player, or cancel its fetch.
    private func leave() {
        guard let source = current else { return }
        work.cancel("fetch")
        work.cancel("ticks")
        if let player {
            player.pause()
            position = player.currentTime
        }
        kept[source] = self.position > 0 ? self.position : nil
        player = nil
        current = nil
        state = nil
        position = 0
        duration = nil
    }
}

/// Hands a fetch's progress to the main actor.
private final class FetchProgress: AudioProgress {
    private let report: @MainActor @Sendable (Float?) -> Void

    init(_ report: @escaping @MainActor @Sendable (Float?) -> Void) {
        self.report = report
    }

    func onProgress(fraction: Float?) {
        let report = self.report
        Task { @MainActor in report(fraction) }
    }
}

/// Why a player stopped by itself.
enum AudioStop {
    /// It played to its end.
    case ended
    /// The file can't be decoded.
    case failed
    /// A call, or the headphones going, paused it.
    case paused
}

/// Plays a message's file: `SystemAudioPlayer`, a fake in tests.
@MainActor
protocol AudioPlaying: AnyObject {
    /// Where it is; set to move.
    var currentTime: TimeInterval { get set }
    /// The file's length.
    var duration: TimeInterval { get }
    /// Told when it stops by itself.
    var stopped: (@MainActor (AudioStop) -> Void)? { get set }
    /// Play from where it is; false if it can't.
    func play() -> Bool
    func pause()
}

/// `AVAudioPlayer`, with the app's audio session active while it plays:
/// category playback, so it plays with the silent switch on; spoken audio
/// for voice messages; no mixing, so other audio pauses. The session is
/// deactivated when it pauses, ends or goes, so that other audio resumes.
/// A call, or the headphones going, pauses it. No background audio: the
/// room pauses it in the background.
@MainActor
final class SystemAudioPlayer: NSObject, AudioPlaying {
    private let player: AVAudioPlayer
    private let voice: Bool
    /// The audio session is active for it.
    private var active = false
    var stopped: (@MainActor (AudioStop) -> Void)?

    init(url: URL, voice: Bool) throws {
        player = try AVAudioPlayer(contentsOf: url)
        self.voice = voice
        super.init()
        player.delegate = self
        player.prepareToPlay()
        let center = NotificationCenter.default
        let session = AVAudioSession.sharedInstance()
        center.addObserver(
            self, selector: #selector(interrupted(_:)), name: AVAudioSession.interruptionNotification, object: session)
        center.addObserver(
            self, selector: #selector(routeChanged(_:)), name: AVAudioSession.routeChangeNotification, object: session)
    }

    deinit {
        // Released while playing (the room closed): other audio may resume.
        if active {
            try? AVAudioSession.sharedInstance().setActive(false, options: .notifyOthersOnDeactivation)
        }
    }

    var currentTime: TimeInterval {
        get { player.currentTime }
        set { player.currentTime = newValue }
    }

    var duration: TimeInterval { player.duration }

    func play() -> Bool {
        let session = AVAudioSession.sharedInstance()
        try? session.setCategory(.playback, mode: voice ? .spokenAudio : .default)
        try? session.setActive(true)
        active = true
        guard player.play() else {
            deactivate()
            return false
        }
        return true
    }

    func pause() {
        player.pause()
        deactivate()
    }

    private func deactivate() {
        guard active else { return }
        active = false
        try? AVAudioSession.sharedInstance().setActive(false, options: .notifyOthersOnDeactivation)
    }

    /// It stopped by itself: the session goes, the model is told.
    private func finished(_ stop: AudioStop) {
        deactivate()
        stopped?(stop)
    }

    /// A call, or the headphones going: pause, if playing.
    private func interrupt() {
        guard active else { return }
        player.pause()
        finished(.paused)
    }

    /// Posted on the session's thread.
    @objc private nonisolated func interrupted(_ notification: Notification) {
        guard let type = notification.userInfo?[AVAudioSessionInterruptionTypeKey] as? UInt,
            AVAudioSession.InterruptionType(rawValue: type) == .began
        else { return }
        Task { @MainActor in self.interrupt() }
    }

    /// Posted on the session's thread.
    @objc private nonisolated func routeChanged(_ notification: Notification) {
        guard let reason = notification.userInfo?[AVAudioSessionRouteChangeReasonKey] as? UInt,
            AVAudioSession.RouteChangeReason(rawValue: reason) == .oldDeviceUnavailable
        else { return }
        Task { @MainActor in self.interrupt() }
    }
}

extension SystemAudioPlayer: AVAudioPlayerDelegate {
    nonisolated func audioPlayerDidFinishPlaying(_ player: AVAudioPlayer, successfully flag: Bool) {
        Task { @MainActor in self.finished(flag ? .ended : .failed) }
    }

    nonisolated func audioPlayerDecodeErrorDidOccur(_ player: AVAudioPlayer, error: Error?) {
        Task { @MainActor in self.finished(.failed) }
    }
}
