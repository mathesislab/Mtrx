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
                .environmentObject(model)
                .environmentObject(model.images)
                .environmentObject(model.presence)
                .environmentObject(model.downloads)
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
                    Task { await app.start() }
                }
                .buttonStyle(.borderedProminent)
            }
            .padding(32)
        }
    }
}
