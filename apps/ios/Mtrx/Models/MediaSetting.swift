import Foundation
import MtrxFFI

/// How photos and videos are sent from this device (Settings → "Photos and
/// videos"): kept in its defaults, since data plans differ; the library's
/// default (Reduced) until changed.
enum MediaSetting {
    private static let key = "mediaQuality"

    /// Reduced, then Original, with their words.
    static let choices = mediaQualityChoices()

    static func quality(in defaults: UserDefaults = .standard) -> MediaQuality {
        switch defaults.string(forKey: key) {
        case "reduced": return .reduced
        case "original": return .original
        default: return defaultMediaQuality()
        }
    }

    static func set(_ quality: MediaQuality, in defaults: UserDefaults = .standard) {
        switch quality {
        case .reduced: defaults.set("reduced", forKey: key)
        case .original: defaults.set("original", forKey: key)
        }
    }

    /// What `quality` does, below the setting.
    static func explanation(of quality: MediaQuality) -> String {
        choices.first { $0.quality == quality }?.explanation ?? ""
    }
}
