import MtrxFFI
import SwiftUI
import UIKit

/// A message's image, or a video's preview, laid out at its size before it
/// loads.
struct MediaImageView: View {
    let media: Media

    @EnvironmentObject private var images: ImageCache
    @State private var loaded: UIImage?
    @State private var failed = false

    static let maxSize = CGSize(width: 260, height: 320)

    var body: some View {
        let image = loaded ?? images.cached(.media(media))
        let size = Self.fitted(media)
        ZStack {
            Color(.secondarySystemBackground)
            if let image {
                Image(uiImage: image)
                    .resizable()
                    .scaledToFill()
            } else if failed {
                Image(systemName: "photo")
                    .font(.title)
                    .foregroundStyle(.secondary)
                    .accessibilityLabel("Could not load the image")
            } else {
                ProgressView()
            }
            if media.kind == .video {
                Image(systemName: "play.circle.fill")
                    .font(.largeTitle)
                    .imageScale(.large)
                    .dynamicTypeSize(...DynamicTypeSize.xxxLarge)
                    .foregroundStyle(.white, .black.opacity(0.4))
                    .accessibilityLabel("Video")
            }
        }
        .frame(width: size.width, height: size.height)
        .clipShape(RoundedRectangle(cornerRadius: 16))
        .accessibilityLabel(media.filename)
        .task(id: media.source) {
            guard images.cached(.media(media)) == nil else { return }
            loaded = await images.load(.media(media))
            failed = loaded == nil
        }
    }

    /// The image's size scaled down to fit `maxSize`; a square if unknown.
    static func fitted(_ media: Media) -> CGSize {
        guard let width = media.width, let height = media.height, width > 0, height > 0 else {
            return CGSize(width: 200, height: 200)
        }
        let size = CGSize(width: CGFloat(width), height: CGFloat(height))
        let scale = min(1, maxSize.width / size.width, maxSize.height / size.height)
        return CGSize(width: max(size.width * scale, 60), height: max(size.height * scale, 60))
    }
}

/// A file attachment: its name, and its size or how its download goes.
/// (Images and videos show as `MediaImageView`, audio as `AudioView`.)
struct FileView: View {
    let media: Media
    @EnvironmentObject private var downloads: DownloadsModel

    var body: some View {
        let download = downloads.download(of: media)
        HStack(spacing: 10) {
            Image(systemName: "doc")
                .font(.title2)
            VStack(alignment: .leading, spacing: 2) {
                Text(media.filename)
                    .lineLimit(2)
                if let download, case .downloading = download.state {
                    Text(download.status)
                        .font(.caption)
                        .monospacedDigit()
                        .opacity(0.7)
                } else if let size = media.size {
                    Text(Formatting.bytes(size))
                        .font(.caption)
                        .opacity(0.7)
                }
            }
            DownloadBadge(media: media)
        }
    }
}

/// An audio message, played in its bubble (`AudioModel`): a voice
/// message's play button, waveform (a slider without one) and time; other
/// audio's name and size over the button, a slider and the time. The time
/// is the position once started, else the length (a voice message's size
/// if unknown). While the file is fetched, a ring round the button shows
/// how far, and a tap cancels; if it can't play, the button tries again
/// and the reason shows under it.
struct AudioView: View {
    let media: Media
    @EnvironmentObject private var audio: AudioModel
    /// Where the scrubber is dragged to, until let go.
    @State private var dragged: TimeInterval?

    /// How far VoiceOver moves the scrubber a step.
    private static let step: TimeInterval = 5

    var body: some View {
        let state = audio.state(of: media)
        let total = audio.duration(of: media)
        let position = dragged ?? audio.position(of: media)
        VStack(alignment: .leading, spacing: 4) {
            if !media.voice {
                Text(media.filename)
                    .lineLimit(2)
                if let size = media.size {
                    Text(Formatting.bytes(size))
                        .font(.caption)
                        .opacity(0.7)
                }
            }
            HStack(spacing: 8) {
                button(state)
                scrubber(position: position, total: total)
                if let time = shownTime(position: position, state: state) {
                    Text(time)
                        .font(.caption)
                        .monospacedDigit()
                        .opacity(0.7)
                        .fixedSize()
                }
            }
            .frame(maxWidth: 260)
            if case .failed(let reason) = state {
                Text(reason)
                    .font(.caption)
                    .foregroundColor(.red)
            }
        }
    }

    /// Play, pause, cancel the fetch (a ring round it shows how far), or
    /// try again.
    private func button(_ state: AudioModel.State?) -> some View {
        Button {
            audio.toggle(media)
        } label: {
            Group {
                if case .fetching(let fraction) = state {
                    ZStack {
                        if let fraction {
                            Circle().stroke(Color.secondary.opacity(0.3), lineWidth: 3)
                            Circle()
                                .trim(from: 0, to: CGFloat(fraction))
                                .stroke(.tint, style: StrokeStyle(lineWidth: 3, lineCap: .round))
                                .rotationEffect(.degrees(-90))
                        } else {
                            ProgressView()
                        }
                        Image(systemName: "xmark")
                            .font(.caption.weight(.bold))
                    }
                    .padding(3)
                } else {
                    Image(systemName: Self.symbol(state))
                        .resizable()
                        .foregroundStyle(.tint)
                }
            }
            .frame(width: 44, height: 44)
            .contentShape(Circle())
        }
        .buttonStyle(.plain)
        .accessibilityLabel(Self.label(state))
    }

    /// A voice message's waveform, else a slider: shows where it is, and
    /// moves it once let go. VoiceOver reads "0:12 of 0:42" and moves it 5 s
    /// a step.
    private func scrubber(position: TimeInterval, total: TimeInterval?) -> some View {
        let length = max(total ?? 0, position, 1)
        return Group {
            if media.voice && !media.waveform.isEmpty {
                WaveformView(
                    bars: media.waveform, played: position / length,
                    drag: { dragged = $0 * length },
                    release: {
                        dragged = nil
                        audio.seek(media, to: $0 * length)
                    })
            } else {
                Slider(
                    value: Binding(get: { min(position, length) }, set: { dragged = $0 }),
                    in: 0...length,
                    onEditingChanged: { editing in
                        if !editing, let dragged {
                            audio.seek(media, to: dragged)
                            self.dragged = nil
                        }
                    })
            }
        }
        .disabled(total == nil)
        .allowsHitTesting(total != nil)
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(media.voice ? "Voice message" : "Position")
        .accessibilityValue(Self.spoken(position, of: total))
        .accessibilityAdjustableAction { direction in
            guard let total else { return }
            let now = audio.position(of: media)
            switch direction {
            case .increment: audio.seek(media, to: min(now + Self.step, total))
            case .decrement: audio.seek(media, to: max(now - Self.step, 0))
            @unknown default: break
            }
        }
    }

    /// The position once started (or while dragged), else the length; a
    /// voice message's size if that's unknown (other audio shows it above).
    private func shownTime(position: TimeInterval, state: AudioModel.State?) -> String? {
        if position > 0 || state == .playing {
            return playbackTime(time: position)
        }
        return media.durationLabel ?? (media.voice ? media.size.map { Formatting.bytes($0) } : nil)
    }

    private static func symbol(_ state: AudioModel.State?) -> String {
        switch state {
        case .playing: return "pause.circle.fill"
        case .failed: return "arrow.clockwise.circle.fill"
        default: return "play.circle.fill"
        }
    }

    private static func label(_ state: AudioModel.State?) -> String {
        switch state {
        case .playing: return "Pause"
        case .fetching: return "Cancel loading"
        case .failed: return "Try again"
        default: return "Play"
        }
    }

    /// "0:12 of 0:42".
    private static func spoken(_ position: TimeInterval, of total: TimeInterval?) -> String {
        let at = playbackTime(time: position)
        guard let total else { return at }
        return "\(at) of \(playbackTime(time: total))"
    }
}

/// A voice message's waveform (`Media.waveform`): its bars across the
/// width, those played in the tint colour. A drag, or a tap, moves the
/// position: `drag` while it moves, `release` where it ends, each as a
/// fraction of the length.
struct WaveformView: View {
    let bars: [Float]
    /// How much is played, from 0 to 1.
    let played: Double
    let drag: (Double) -> Void
    let release: (Double) -> Void

    var body: some View {
        GeometryReader { geometry in
            let width = geometry.size.width
            Canvas { context, size in
                guard !bars.isEmpty else { return }
                let step = size.width / CGFloat(bars.count)
                let barWidth = max(1, step * 0.6)
                let playedBar = GraphicsContext.Shading.style(.tint)
                let otherBar = GraphicsContext.Shading.style(.secondary)
                for (index, bar) in bars.enumerated() {
                    let height = max(barWidth, size.height * CGFloat(bar))
                    let rect = CGRect(
                        x: CGFloat(index) * step + (step - barWidth) / 2, y: (size.height - height) / 2,
                        width: barWidth, height: height)
                    let isPlayed = (Double(index) + 0.5) / Double(bars.count) <= played
                    context.fill(
                        Path(roundedRect: rect, cornerRadius: barWidth / 2), with: isPlayed ? playedBar : otherBar)
                }
            }
            .contentShape(Rectangle())
            .gesture(
                DragGesture(minimumDistance: 0)
                    .onChanged { drag(Self.fraction($0.location.x, of: width)) }
                    .onEnded { release(Self.fraction($0.location.x, of: width)) })
        }
        .frame(height: 28)
    }

    private static func fraction(_ x: CGFloat, of width: CGFloat) -> Double {
        width > 0 ? Double(min(max(x / width, 0), 1)) : 0
    }
}

/// How a message's file's download goes, beside or over it: a ring with ✕
/// (cancel) while it downloads, a spinner while its length is unknown; a
/// retry once it failed. Nothing before or once done.
struct DownloadBadge: View {
    let media: Media
    @EnvironmentObject private var downloads: DownloadsModel

    var body: some View {
        if let download = downloads.download(of: media) {
            switch download.state {
            case .downloading:
                Button {
                    downloads.cancel(download.id)
                } label: {
                    ZStack {
                        if let fraction = download.fraction {
                            Circle().stroke(Color.secondary.opacity(0.3), lineWidth: 3)
                            Circle()
                                .trim(from: 0, to: CGFloat(fraction))
                                .stroke(Color.accentColor, style: StrokeStyle(lineWidth: 3, lineCap: .round))
                                .rotationEffect(.degrees(-90))
                        } else {
                            ProgressView()
                        }
                        Image(systemName: "xmark")
                            .font(.caption2.weight(.bold))
                    }
                    .frame(width: 30, height: 30)
                    .padding(4)
                    .background(.ultraThinMaterial, in: Circle())
                }
                .buttonStyle(.plain)
                .accessibilityLabel("Cancel the download")
                .accessibilityValue(download.status)
            case .failed, .cancelled:
                Button {
                    downloads.retry(download.id)
                } label: {
                    Label("Couldn't download", systemImage: "arrow.clockwise")
                        .font(.caption)
                        .padding(.horizontal, 8)
                        .padding(.vertical, 5)
                        .background(.ultraThinMaterial, in: Capsule())
                }
                .buttonStyle(.plain)
                .accessibilityHint("Tries again")
            case .done:
                EmptyView()
            }
        }
    }
}

/// The system's share sheet, for a downloaded file.
struct ShareSheet: UIViewControllerRepresentable {
    let url: URL

    func makeUIViewController(context: Context) -> UIActivityViewController {
        UIActivityViewController(activityItems: [url], applicationActivities: nil)
    }

    func updateUIViewController(_ controller: UIActivityViewController, context: Context) {}
}
