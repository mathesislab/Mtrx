import Foundation
import MtrxFFI

/// Disappearing messages (see `docs/disappearing.md`): the choices the app
/// offers, as the library words them.
enum Disappearing {
    /// Off, then each time after read, in order: for the composer's menu
    /// and a room's mode.
    static let choices = disappearChoices()
    /// The limits Settings offers, in order.
    static let limits = disappearingLimitChoices()
}
