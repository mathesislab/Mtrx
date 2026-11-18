import MtrxFFI
import PhotosUI
import QuickLookThumbnailing
import SwiftUI
import UIKit
import UniformTypeIdentifiers

/// The paperclip left of the message field: a menu of Photos (the
/// library's photos and videos, several at once), Files (any, several) and
/// the Camera (a photo or a video); what is picked goes to `attach`.
struct AttachMenu: View {
    /// The send button's.
    let size: CGFloat
    let attach: ([Picked]) -> Void
    @State private var choosingPhotos = false
    @State private var photos: [PhotosPickerItem] = []
    @State private var choosingFiles = false
    @State private var usingCamera = false

    var body: some View {
        Menu {
            Button {
                choosingPhotos = true
            } label: {
                Label("Photos", systemImage: "photo.on.rectangle")
            }
            Button {
                choosingFiles = true
            } label: {
                Label("Files", systemImage: "folder")
            }
            if UIImagePickerController.isSourceTypeAvailable(.camera) {
                Button {
                    usingCamera = true
                } label: {
                    Label("Camera", systemImage: "camera")
                }
            }
        } label: {
            Image(systemName: "paperclip")
                .font(.system(size: size * 0.65))
                .frame(width: size, height: size)
        }
        .accessibilityLabel("Attach")
        // Compatible: HEIC photos come as JPEG, HEVC videos as H.264.
        .photosPicker(
            isPresented: $choosingPhotos, selection: $photos, matching: .any(of: [.images, .videos]),
            preferredItemEncoding: .compatible
        )
        .onChange(of: photos) { items in
            guard !items.isEmpty else { return }
            photos = []
            attach(items.map(Picked.library))
        }
        .fileImporter(isPresented: $choosingFiles, allowedContentTypes: [.item], allowsMultipleSelection: true) { result in
            if case .success(let urls) = result {
                attach(urls.map(Picked.file))
            }
        }
        .fullScreenCover(isPresented: $usingCamera) {
            CameraPicker(took: { attach([$0]) }, close: { usingCamera = false })
                .ignoresSafeArea()
        }
    }
}

/// The camera (`UIImagePickerController`), for a photo or a video: what is
/// taken goes to `took`, then it closes.
struct CameraPicker: UIViewControllerRepresentable {
    let took: (Picked) -> Void
    let close: () -> Void

    func makeUIViewController(context: Context) -> UIImagePickerController {
        let picker = UIImagePickerController()
        picker.sourceType = .camera
        picker.mediaTypes = [UTType.image.identifier, UTType.movie.identifier]
        picker.delegate = context.coordinator
        return picker
    }

    func updateUIViewController(_ picker: UIImagePickerController, context: Context) {
        context.coordinator.parent = self
    }

    func makeCoordinator() -> Coordinator {
        Coordinator(parent: self)
    }

    final class Coordinator: NSObject, UIImagePickerControllerDelegate, UINavigationControllerDelegate {
        var parent: CameraPicker

        init(parent: CameraPicker) {
            self.parent = parent
        }

        func imagePickerController(
            _ picker: UIImagePickerController, didFinishPickingMediaWithInfo info: [UIImagePickerController.InfoKey: Any]
        ) {
            if let video = info[.mediaURL] as? URL {
                parent.took(.cameraVideo(video))
            } else if let photo = info[.originalImage] as? UIImage {
                parent.took(.cameraPhoto(photo))
            }
            parent.close()
        }

        func imagePickerControllerDidCancel(_ picker: UIImagePickerController) {
            parent.close()
        }
    }
}

/// The files picked to send, above the message field, like the desktop's
/// chips: a card each with its picture (or a file's symbol), its name, and
/// ✕ to take it away; "Preparing…" until its copy is made, with how far a
/// video's conversion is ("Preparing… 40%").
struct PickedFileCards: View {
    let files: [PickedFile]
    let remove: (UUID) -> Void

    var body: some View {
        ScrollView(.horizontal, showsIndicators: false) {
            HStack(spacing: 8) {
                ForEach(files) { file in
                    PickedFileCard(file: file) { remove(file.id) }
                }
            }
            .padding(.horizontal, 12)
            .padding(.top, 8)
        }
    }
}

private struct PickedFileCard: View {
    let file: PickedFile
    let remove: () -> Void
    @State private var thumbnail: UIImage?
    @Environment(\.displayScale) private var scale

    var body: some View {
        let name = file.file?.filename
        HStack(spacing: 8) {
            ZStack {
                Color(.tertiarySystemFill)
                if let thumbnail {
                    Image(uiImage: thumbnail)
                        .resizable()
                        .scaledToFill()
                } else if name == nil {
                    ProgressView()
                } else {
                    Image(systemName: "doc")
                        .foregroundStyle(.secondary)
                }
            }
            .frame(width: 40, height: 40)
            .clipShape(RoundedRectangle(cornerRadius: 8))
            Text(name ?? preparing)
                .font(.footnote)
                .foregroundStyle(name == nil ? .secondary : .primary)
                .lineLimit(2)
                .frame(maxWidth: 140, alignment: .leading)
            Button(action: remove) {
                Image(systemName: "xmark.circle.fill")
                    .foregroundStyle(.secondary)
            }
            .buttonStyle(.plain)
            .accessibilityLabel(name.map { "Remove \($0)" } ?? "Stop preparing the file")
        }
        .padding(6)
        .background(Color(.secondarySystemBackground), in: RoundedRectangle(cornerRadius: 12))
        .task(id: file.file?.path) {
            guard let path = file.file?.path else { return }
            thumbnail = await Self.thumbnail(of: URL(fileURLWithPath: path), scale: scale)
        }
    }

    private var preparing: String {
        guard let progress = file.progress else { return "Preparing…" }
        return "Preparing… \(Int((progress * 100).rounded()))%"
    }

    /// A small picture of the file (a photo, a video's frame, a page);
    /// nil for a file without one.
    static func thumbnail(of url: URL, scale: CGFloat) async -> UIImage? {
        let request = QLThumbnailGenerator.Request(
            fileAt: url, size: CGSize(width: 40, height: 40), scale: scale, representationTypes: .thumbnail)
        return try? await QLThumbnailGenerator.shared.generateBestRepresentation(for: request).uiImage
    }
}

/// While a message's file uploads, a ring that fills as it goes; nothing
/// otherwise. It follows the uploads itself, so their progress redraws
/// only it.
struct UploadRing: View {
    /// The message's row.
    let itemId: String
    @EnvironmentObject private var uploads: UploadsModel

    var body: some View {
        if let fraction = uploads.fraction(of: itemId) {
            ZStack {
                Circle().stroke(Color.secondary.opacity(0.3), lineWidth: 3)
                Circle()
                    .trim(from: 0, to: CGFloat(fraction))
                    .stroke(Color.accentColor, style: StrokeStyle(lineWidth: 3, lineCap: .round))
                    .rotationEffect(.degrees(-90))
                    .animation(.linear(duration: 0.2), value: fraction)
            }
            .frame(width: 30, height: 30)
            .padding(4)
            .background(.ultraThinMaterial, in: Circle())
            .accessibilityElement()
            .accessibilityLabel("Uploading")
            .accessibilityValue("\(Int(fraction * 100)) percent")
        }
    }
}
