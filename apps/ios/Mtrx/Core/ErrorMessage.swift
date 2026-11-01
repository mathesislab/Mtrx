import AuthenticationServices
import Foundation
import MtrxFFI

/// What to tell the user about `error`, or nil when there is nothing to
/// say: the user cancelled.
func errorMessage(_ error: Error) -> String? {
    switch error {
    case is CancellationError:
        return nil
    case let error as ASWebAuthenticationSessionError where error.code == .canceledLogin:
        return nil
    case let error as MtrxError:
        switch error {
        case .LoginCancelled:
            return nil
        case .NotLoggedIn:
            return "Not signed in."
        case .AlreadyLoggedIn:
            return "Already signed in."
        case .RoomNotFound:
            return "The room is not in the room list (any more)."
        case .InvalidInput(let reason), .Other(let reason):
            return reason
        case .PresenceUnavailable:
            return "This server does not share presence."
        case .SecretStorage(let reason):
            return "The Keychain failed: \(reason)"
        }
    default:
        return error.localizedDescription
    }
}
