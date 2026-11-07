import SwiftUI

@main
struct MtrxApp: App {
    @StateObject private var app = AppModel.live()
    @Environment(\.scenePhase) private var scenePhase

    var body: some Scene {
        WindowGroup {
            RootView()
                .environmentObject(app)
                .task { app.start() }
                .onAppear { app.scenePhaseChanged(scenePhase) }
                .onChange(of: scenePhase) { app.scenePhaseChanged($0) }
        }
    }
}
