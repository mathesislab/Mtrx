import MtrxFFI
import UIKit

/// A picture to show: an avatar at a size, or a message's image.
enum ImageKey: Hashable {
    /// `mxc://` URI and size in pixels.
    case avatar(String, pixels: UInt32)
    case media(Media)

    fileprivate var name: NSString {
        switch self {
        case .avatar(let uri, let pixels): return "avatar \(pixels) \(uri)" as NSString
        case .media(let media): return "media \(media.source)" as NSString
        }
    }
}

/// A formula at a text size: `emPixels` pixels per em, on a screen of
/// `scale` pixels per point.
struct FormulaKey: Hashable {
    let formula: Formula
    let emPixels: UInt32
    let scale: CGFloat

    fileprivate var name: NSString {
        "\(emPixels) \(scale) \(formula.display) \(formula.tex)" as NSString
    }
}

/// A formula typeset: a template image (tint it) at the screen's scale, and
/// how far below its baseline it reaches, in points.
final class TypesetFormula {
    let image: UIImage
    let descent: CGFloat

    init(image: UIImage, descent: CGFloat) {
        self.image = image
        self.descent = descent
    }
}

/// Decoded avatars, message images and typeset formulas in memory, on top
/// of the library's download cache. One load per picture at a time;
/// failures are not remembered, so a picture is tried again when shown
/// again, except a formula's (its LaTeX does not change).
@MainActor
final class ImageCache: ObservableObject {
    private let session: Session
    private let images = NSCache<NSString, UIImage>()
    private var loading: [ImageKey: Task<UIImage?, Never>] = [:]
    private let formulas = NSCache<NSString, TypesetFormula>()
    private var typesetting: [FormulaKey: Task<TypesetFormula?, Never>] = [:]
    /// Formulas that could not be typeset.
    private var unreadable: Set<FormulaKey> = []

    init(session: Session) {
        self.session = session
        images.totalCostLimit = 64 << 20
        formulas.totalCostLimit = 16 << 20
    }

    func cached(_ key: ImageKey) -> UIImage? {
        images.object(forKey: key.name)
    }

    func load(_ key: ImageKey) async -> UIImage? {
        if let image = cached(key) {
            return image
        }
        if let task = loading[key] {
            return await task.value
        }
        let session = self.session
        let task = Task.detached(priority: .userInitiated) { () -> UIImage? in
            let data: Data
            do {
                switch key {
                case .avatar(let uri, let pixels):
                    data = try await session.avatar(mxcUri: uri, size: pixels)
                case .media(let media):
                    data = try await session.mediaPreview(media: media)
                }
            } catch {
                return nil
            }
            return UIImage(data: data)?.preparingForDisplay()
        }
        loading[key] = task
        let image = await task.value
        loading[key] = nil
        if let image {
            images.setObject(image, forKey: key.name, cost: Self.cost(image))
        }
        return image
    }

    func cachedFormula(_ key: FormulaKey) -> TypesetFormula? {
        formulas.object(forKey: key.name)
    }

    /// `nil` for LaTeX the library cannot read.
    func formula(_ key: FormulaKey) async -> TypesetFormula? {
        if let typeset = cachedFormula(key) {
            return typeset
        }
        if let task = typesetting[key] {
            return await task.value
        }
        guard !unreadable.contains(key) else { return nil }
        let task = Task.detached(priority: .userInitiated) { () -> TypesetFormula? in
            guard let typeset = try? await typesetFormula(formula: key.formula, emPixels: Float(key.emPixels)),
                  let image = UIImage(data: typeset.png, scale: key.scale)
            else { return nil }
            return TypesetFormula(
                image: image.withRenderingMode(.alwaysTemplate),
                descent: CGFloat(typeset.height - typeset.baseline) / key.scale)
        }
        typesetting[key] = task
        let typeset = await task.value
        typesetting[key] = nil
        if let typeset {
            formulas.setObject(typeset, forKey: key.name, cost: Self.cost(typeset.image))
        } else {
            unreadable.insert(key)
        }
        return typeset
    }

    /// Bytes decoded.
    private static func cost(_ image: UIImage) -> Int {
        Int(image.size.width * image.size.height * image.scale * image.scale) * 4
    }
}
