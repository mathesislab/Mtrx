import AVFoundation
import Foundation
import MtrxFFI
import UIKit

/// A video picked to send, made ready (`docs/reducing-media.md`): when
/// reduced, converted to the library's plan (`Session.videoPlan`) with the
/// system's encoders, H.264 and AAC; always with its still and length
/// (`VideoInfo`), so that apps show them before it is opened.
enum OutgoingVideo {
    /// A video as stored, as the library plans for it.
    struct Source: Equatable {
        var width: UInt32
        var height: UInt32
        /// Frames a second; 0 if not known.
        var fps: Float
        var durationMs: UInt64
        /// Of its file, in bytes.
        var size: UInt64
    }

    /// `copy` (in the outgoing folder) ready to send: converted into a new
    /// copy, which replaces it, if `quality` is reduced and the library
    /// plans so (`progress` gets how far, from any thread); with its
    /// details (nil for a file without pictures). The copy goes if this
    /// fails.
    static func prepare(
        _ copy: URL, quality: MediaQuality, session: Session, progress: @escaping @Sendable (Double) -> Void
    ) async throws -> (URL, VideoInfo?) {
        do {
            let asset = AVURLAsset(url: copy)
            guard let track = try await asset.loadTracks(withMediaType: .video).first else {
                return (copy, nil)
            }
            let (naturalSize, transform, fps) = try await track.load(
                .naturalSize, .preferredTransform, .nominalFrameRate)
            let duration = try await asset.load(.duration)
            var sent = copy
            var stored = naturalSize
            if quality == .reduced {
                // As stored: the conversion keeps the turn.
                let source = Source(
                    width: UInt32(naturalSize.width.rounded()), height: UInt32(naturalSize.height.rounded()),
                    fps: fps, durationMs: milliseconds(duration), size: fileSize(copy))
                if let plan = try await plan(source, session: session) {
                    let converted = Outgoing.newCopy(extension: "mp4")
                    do {
                        try await convert(
                            asset, track: track, to: converted, plan: plan, duration: duration, progress: progress)
                    } catch {
                        try? FileManager.default.removeItem(at: converted)
                        throw error
                    }
                    try? FileManager.default.removeItem(at: copy)
                    sent = converted
                    stored = CGSize(width: Int(plan.width), height: Int(plan.height))
                }
            }
            let shown = stored.applying(transform)
            let info = VideoInfo(
                width: UInt32(abs(shown.width).rounded()), height: UInt32(abs(shown.height).rounded()),
                durationMs: milliseconds(duration),
                thumbnailPath: await still(of: AVURLAsset(url: sent), duration: duration)?.path)
            return (sent, info)
        } catch {
            try? FileManager.default.removeItem(at: copy)
            throw error
        }
    }

    /// The library's plan for `source`, fitted to the server's limit; nil:
    /// it is sent as it is.
    static func plan(_ source: Source, session: Session) async throws -> VideoPlan? {
        try await session.videoPlan(
            width: source.width, height: source.height, fps: source.fps, durationMs: source.durationMs,
            size: source.size)
    }

    /// The H.264 encoder's settings for `plan`: its size, as stored, and
    /// its bit rate.
    static func videoSettings(_ plan: VideoPlan) -> [String: Any] {
        [
            AVVideoCodecKey: AVVideoCodecType.h264,
            AVVideoWidthKey: Int(plan.width),
            AVVideoHeightKey: Int(plan.height),
            AVVideoScalingModeKey: AVVideoScalingModeResizeAspectFill,
            AVVideoCompressionPropertiesKey: [
                AVVideoAverageBitRateKey: Int(plan.videoBitrate),
                AVVideoExpectedSourceFrameRateKey: plan.fps,
                AVVideoProfileLevelKey: AVVideoProfileLevelH264HighAutoLevel,
            ] as [String: Any],
        ]
    }

    /// The AAC encoder's settings for `plan`: `channels` (one or two) at
    /// `sampleRate`, as read.
    static func audioSettings(_ plan: VideoPlan, channels: Int, sampleRate: Double) -> [String: Any] {
        [
            AVFormatIDKey: kAudioFormatMPEG4AAC,
            AVNumberOfChannelsKey: channels,
            AVSampleRateKey: sampleRate,
            AVEncoderBitRateKey: Int(plan.audioBitrate),
            AVChannelLayoutKey: channelLayout(channels),
        ]
    }

    /// A JPEG of a frame near the start (a second in: the first is often
    /// black), upright, at most `videoThumbnailEdge()` px, in the outgoing
    /// folder; nil if none can be made.
    static func still(of asset: AVAsset, duration: CMTime) async -> URL? {
        let generator = AVAssetImageGenerator(asset: asset)
        generator.appliesPreferredTrackTransform = true
        let edge = CGFloat(videoThumbnailEdge())
        generator.maximumSize = CGSize(width: edge, height: edge)
        let seconds = duration.isNumeric ? min(1, duration.seconds / 2) : 0
        let time = CMTime(seconds: seconds, preferredTimescale: 600)
        guard let frame = try? await generator.image(at: time).image,
            let jpeg = UIImage(cgImage: frame).jpegData(compressionQuality: 0.8)
        else { return nil }
        let still = Outgoing.newCopy(extension: "jpg")
        guard (try? jpeg.write(to: still)) != nil else { return nil }
        return still
    }

    /// `asset` converted to `plan` into `destination` (MP4), turned as it
    /// was; frames beyond the plan's rate are dropped. Stops when the task
    /// is cancelled (a card taken away).
    private static func convert(
        _ asset: AVAsset, track: AVAssetTrack, to destination: URL, plan: VideoPlan, duration: CMTime,
        progress: @escaping @Sendable (Double) -> Void
    ) async throws {
        let reader = try AVAssetReader(asset: asset)
        let writer = try AVAssetWriter(outputURL: destination, fileType: .mp4)
        writer.shouldOptimizeForNetworkUse = true

        let pictures = AVAssetReaderTrackOutput(
            track: track,
            outputSettings: [
                kCVPixelBufferPixelFormatTypeKey as String: kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange
            ])
        pictures.alwaysCopiesSampleData = false
        reader.add(pictures)
        let pictureInput = AVAssetWriterInput(mediaType: .video, outputSettings: videoSettings(plan))
        pictureInput.transform = try await track.load(.preferredTransform)
        pictureInput.expectsMediaDataInRealTime = false
        writer.add(pictureInput)
        let gate = FrameGate(fps: Double(plan.fps), duration: duration, progress: progress)
        var tracks = [TrackCopy.Track(output: pictures, input: pictureInput, keep: gate.keep)]

        if let audio = try await asset.loadTracks(withMediaType: .audio).first {
            let format = try await audio.load(.formatDescriptions).first
            let basic = format.flatMap { CMAudioFormatDescriptionGetStreamBasicDescription($0)?.pointee }
            let channels = min(max(Int(basic?.mChannelsPerFrame ?? 2), 1), 2)
            let sampleRate = basic?.mSampleRate ?? 44_100
            let sound = AVAssetReaderTrackOutput(
                track: audio,
                outputSettings: [
                    AVFormatIDKey: kAudioFormatLinearPCM,
                    AVNumberOfChannelsKey: channels,
                    AVSampleRateKey: sampleRate,
                    AVChannelLayoutKey: channelLayout(channels),
                ])
            sound.alwaysCopiesSampleData = false
            reader.add(sound)
            let soundInput = AVAssetWriterInput(
                mediaType: .audio, outputSettings: audioSettings(plan, channels: channels, sampleRate: sampleRate))
            soundInput.expectsMediaDataInRealTime = false
            writer.add(soundInput)
            tracks.append(TrackCopy.Track(output: sound, input: soundInput, keep: { _ in true }))
        }

        guard reader.startReading() else {
            throw reader.error ?? CocoaError(.fileReadUnknown)
        }
        guard writer.startWriting() else {
            reader.cancelReading()
            throw writer.error ?? CocoaError(.fileWriteUnknown)
        }
        writer.startSession(atSourceTime: .zero)
        await withTaskCancellationHandler {
            await withCheckedContinuation { (done: CheckedContinuation<Void, Never>) in
                TrackCopy(tracks).run { done.resume() }
            }
        } onCancel: {
            reader.cancelReading()
        }
        if Task.isCancelled || reader.status == .failed || writer.status == .failed {
            let error: Error = reader.error ?? writer.error ?? CancellationError()
            writer.cancelWriting()
            throw error
        }
        await writer.finishWriting()
        guard writer.status == .completed else {
            throw writer.error ?? CocoaError(.fileWriteUnknown)
        }
    }

    /// Mono or stereo, as the encoders take it.
    private static func channelLayout(_ channels: Int) -> Data {
        var layout = AudioChannelLayout()
        layout.mChannelLayoutTag = channels == 1 ? kAudioChannelLayoutTag_Mono : kAudioChannelLayoutTag_Stereo
        return Data(bytes: &layout, count: MemoryLayout<AudioChannelLayout>.size)
    }

    private static func milliseconds(_ time: CMTime) -> UInt64 {
        time.isNumeric ? UInt64(max(0, time.seconds * 1000).rounded()) : 0
    }

    private static func fileSize(_ file: URL) -> UInt64 {
        let attributes = try? FileManager.default.attributesOfItem(atPath: file.path)
        return (attributes?[.size] as? NSNumber)?.uint64Value ?? 0
    }
}

/// Which frames a conversion keeps (none closer together than the plan's
/// rate allows), and how far it is, in whole percent.
private final class FrameGate {
    private let interval: Double
    private let seconds: Double
    private let progress: @Sendable (Double) -> Void
    private var lastKept: Double?
    private var percent = -1

    init(fps: Double, duration: CMTime, progress: @escaping @Sendable (Double) -> Void) {
        interval = fps > 0 ? 1 / fps : 0
        seconds = duration.isNumeric ? duration.seconds : 0
        self.progress = progress
    }

    func keep(_ sample: CMSampleBuffer) -> Bool {
        let time = CMSampleBufferGetPresentationTimeStamp(sample).seconds
        guard time.isFinite else { return true }
        if seconds > 0 {
            let now = Int(min(max(time / seconds, 0), 1) * 100)
            if now != percent {
                percent = now
                progress(Double(now) / 100)
            }
        }
        // A little slack: timestamps are rounded.
        if let lastKept, time - lastKept < interval * 0.9 {
            return false
        }
        lastKept = time
        return true
    }
}

/// Copies a video's tracks from the reader to the writer as the writer
/// takes their samples, all at once (it interleaves them), on one queue;
/// `finished` once every track is copied, or as soon as one fails.
private final class TrackCopy {
    struct Track {
        let output: AVAssetReaderOutput
        let input: AVAssetWriterInput
        /// Whether a sample is written.
        let keep: (CMSampleBuffer) -> Bool
    }

    private let queue = DispatchQueue(label: "org.mtrx.outgoing-video")
    private var tracks: [Track]
    private var copied: Set<Int> = []
    private var finished: (() -> Void)?

    init(_ tracks: [Track]) {
        self.tracks = tracks
    }

    func run(finished: @escaping () -> Void) {
        queue.async {
            self.finished = finished
            for index in self.tracks.indices {
                self.tracks[index].input.requestMediaDataWhenReady(on: self.queue) { self.copy(index) }
            }
        }
    }

    private func copy(_ index: Int) {
        guard finished != nil, !copied.contains(index) else { return }
        let track = tracks[index]
        while track.input.isReadyForMoreMediaData {
            guard let sample = track.output.copyNextSampleBuffer() else {
                track.input.markAsFinished()
                copied.insert(index)
                if copied.count == tracks.count {
                    end()
                }
                return
            }
            if track.keep(sample) && !track.input.append(sample) {
                // The writer failed (its error says why): the others stop too.
                end()
                return
            }
        }
    }

    private func end() {
        let finished = self.finished
        self.finished = nil
        // The inputs hold their blocks, which hold this.
        tracks = []
        finished?()
    }
}
