import Foundation
import MtrxFFI
import PhotosUI
import SwiftUI
import UIKit
import UniformTypeIdentifiers

/// A photo, video or file picked to send, as the system hands it over.
enum Picked {
    /// From the photo library (`PhotosPicker`, compatible encoding: JPEG
    /// photos, H.264 videos).
    case library(PhotosPickerItem)
    /// Taken with the camera.
    case cameraPhoto(UIImage)
    /// Recorded with the camera: its file, in the app's temporary folder.
    case cameraVideo(URL)
    /// From Files: a security-scoped file, with its name.
    case file(URL)
}

/// A file picked to send, as its card above the message field shows it:
/// ready once prepared (`file`); a video being converted says how far
/// (`progress`, 0 to 1).
struct PickedFile: Identifiable, Equatable {
    let id = UUID()
    var file: FileToSend?
    var progress: Double?
}

/// The files picked to send, prepared: each a copy in the cache, named as
/// it is sent, and deleted once sent or taken away. Photos and videos have
/// no names of their own, so they are named from the date: "Photo
/// 2026-11-18 14.03.jpg". Videos are converted here when reduced, and
/// carry their still and length (`OutgoingVideo`); the library reduces
/// photos as it sends them.
enum Outgoing {
    enum Kind {
        case photo, video
    }

    /// `picked` as a file to send: copied into `folder`, named; a video
    /// converted to `session`'s plan if `quality` is reduced
    /// (`progress` gets how far, from any thread), with its still and
    /// length.
    static func prepare(
        _ picked: Picked, quality: MediaQuality, session: Session, at date: Date = Date(),
        progress: @escaping @Sendable (Double) -> Void = { _ in }
    ) async throws -> FileToSend {
        let video: (URL) async throws -> (URL, VideoInfo?) = { copy in
            try await OutgoingVideo.prepare(copy, quality: quality, session: session, progress: progress)
        }
        switch picked {
        case .library(let item):
            guard let media = try await item.loadTransferable(type: LibraryMedia.self) else {
                throw CocoaError(.fileReadUnknown)
            }
            guard media.kind == .video else {
                return FileToSend(
                    path: media.copy.path, filename: name(.photo, at: date, extension: media.copy.pathExtension))
            }
            let (copy, info) = try await video(media.copy)
            return FileToSend(
                path: copy.path, filename: name(.video, at: date, extension: copy.pathExtension), video: info)
        case .cameraPhoto(let image):
            guard let jpeg = image.jpegData(compressionQuality: 0.9) else {
                throw CocoaError(.fileWriteUnknown)
            }
            let copy = newCopy(extension: "jpg")
            try jpeg.write(to: copy)
            return FileToSend(path: copy.path, filename: name(.photo, at: date, extension: "jpg"))
        case .cameraVideo(let url):
            let moved = newCopy(extension: url.pathExtension)
            try FileManager.default.moveItem(at: url, to: moved)
            let (copy, info) = try await video(moved)
            return FileToSend(
                path: copy.path, filename: name(.video, at: date, extension: copy.pathExtension), video: info)
        case .file(let url):
            let scoped = url.startAccessingSecurityScopedResource()
            defer {
                if scoped { url.stopAccessingSecurityScopedResource() }
            }
            return FileToSend(path: try copy(url).path, filename: url.lastPathComponent)
        }
    }

    /// The copy is no longer needed: sent (the library has read it), or
    /// taken away. A video's still goes with it.
    static func delete(_ file: FileToSend) {
        try? FileManager.default.removeItem(atPath: file.path)
        if let still = file.video?.thumbnailPath {
            try? FileManager.default.removeItem(atPath: still)
        }
    }

    /// "Photo 2026-11-18 14.03.jpg", "Video 2026-11-18 14.03.mp4": the
    /// local time; the extension lowercased, JPEG's as "jpg".
    static func name(_ kind: Kind, at date: Date, extension ext: String) -> String {
        let formatter = DateFormatter()
        formatter.locale = Locale(identifier: "en_US_POSIX")
        formatter.dateFormat = "yyyy-MM-dd HH.mm"
        var ext = ext.lowercased()
        if ext == "jpeg" {
            ext = "jpg"
        } else if ext.isEmpty {
            ext = kind == .photo ? "jpg" : "mp4"
        }
        return "\(kind == .photo ? "Photo" : "Video") \(formatter.string(from: date)).\(ext)"
    }

    /// A copy of `file` in `folder`.
    static func copy(_ file: URL) throws -> URL {
        let copy = newCopy(extension: file.pathExtension)
        try FileManager.default.copyItem(at: file, to: copy)
        return copy
    }

    /// A new place in `folder`, with `ext` so previews know the type.
    static func newCopy(extension ext: String) -> URL {
        let copy = folder.appendingPathComponent(UUID().uuidString)
        return ext.isEmpty ? copy : copy.appendingPathExtension(ext)
    }

    /// `outgoing` in the cache. Copies a run left there (it ended before
    /// sending them) go once a day old, the first time it is used in a run.
    static let folder: URL = {
        let files = FileManager.default
        let folder = files.urls(for: .cachesDirectory, in: .userDomainMask)[0]
            .appendingPathComponent("outgoing", isDirectory: true)
        try? files.createDirectory(at: folder, withIntermediateDirectories: true)
        let dayAgo = Date().addingTimeInterval(-24 * 60 * 60)
        let left = (try? files.contentsOfDirectory(at: folder, includingPropertiesForKeys: [.contentModificationDateKey])) ?? []
        for file in left {
            let modified = (try? file.resourceValues(forKeys: [.contentModificationDateKey]))?.contentModificationDate
            if let modified, modified < dayAgo {
                try? files.removeItem(at: file)
            }
        }
        return folder
    }()
}

/// A photo or video from the library, copied into the outgoing folder
/// while the system's file is there (only during the import).
private struct LibraryMedia: Transferable {
    let copy: URL
    let kind: Outgoing.Kind

    static var transferRepresentation: some TransferRepresentation {
        FileRepresentation(importedContentType: .image) { received in
            LibraryMedia(copy: try Outgoing.copy(received.file), kind: .photo)
        }
        FileRepresentation(importedContentType: .movie) { received in
            LibraryMedia(copy: try Outgoing.copy(received.file), kind: .video)
        }
    }
}
