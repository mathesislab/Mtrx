import SwiftUI

/// Sign-in or the signed-in account, as the app's phase says.
struct RootView: View {
    @EnvironmentObject private var app: AppModel

    var body: some View {
        switch app.phase {
        case .starting:
            ProgressView()
        case .signedOut(let model):
            SignInView(model: model)
        case .signedIn(let model):
            HomeView()
                .account(model)
        case .failed(let message):
            VStack(spacing: 16) {
                Image(systemName: "exclamationmark.triangle")
                    .font(.largeTitle)
                    .foregroundStyle(.secondary)
                Text("Could not open the account")
                    .font(.headline)
                Text(message)
                    .font(.callout)
                    .foregroundStyle(.secondary)
                    .multilineTextAlignment(.center)
                Button("Try again") {
                    app.start()
                }
                .buttonStyle(.borderedProminent)
            }
            .padding(32)
        }
    }
}

extension View {
    /// The signed-in account's models, for the views to observe: each only
    /// those it shows. Sheets and covers are given them again.
    func account(_ session: SessionModel) -> some View {
        environmentObject(session)
            .environmentObject(session.rooms)
            .environmentObject(session.home)
            .environmentObject(session.encryption)
            .environmentObject(session.images)
            .environmentObject(session.presence)
            .environmentObject(session.downloads)
    }
}
