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

/// A file, audio or other attachment: its name, and its size or how its
/// download goes.
struct FileView: View {
    let media: Media
    @EnvironmentObject private var downloads: DownloadsModel

    var body: some View {
        let download = downloads.download(of: media)
        HStack(spacing: 10) {
            Image(systemName: symbol)
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

    private var symbol: String {
        switch media.kind {
        case .audio: return "waveform"
        case .video: return "film"
        case .image: return "photo"
        case .file: return "doc"
        }
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
