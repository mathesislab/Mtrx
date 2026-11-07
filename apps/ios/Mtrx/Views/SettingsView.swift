import MtrxFFI
import SwiftUI

/// The Settings tab: the account, sharing our presence, its encryption on
/// this device, and signing out.
struct SettingsView: View {
    @EnvironmentObject private var app: AppModel
    @EnvironmentObject private var session: SessionModel
    @State private var confirmSignOut = false

    var body: some View {
        NavigationStack {
            Form {
                Section("Account") {
                    LabeledContent("User", value: session.userId)
                    LabeledContent("Server", value: session.homeserver)
                    LabeledContent("Device", value: session.deviceId)
                }
                .textSelection(.enabled)
                RoomListSection()
                PrivacySection()
                EncryptionSection()
                Section {
                    Button("Sign out…", role: .destructive) {
                        confirmSignOut = true
                    }
                    .disabled(app.signingOut)
                }
                Section {
                    LabeledContent("Version", value: version)
                }
            }
            .navigationTitle("Settings")
            .overlay {
                if app.signingOut {
                    ProgressView("Signing out…")
                        .padding(24)
                        .background(.regularMaterial, in: RoundedRectangle(cornerRadius: 12))
                }
            }
            .confirmationDialog("Sign out of \(session.userId)?", isPresented: $confirmSignOut, titleVisibility: .visible) {
                Button("Sign out", role: .destructive) {
                    Task { await app.signOut() }
                }
            } message: {
                Text("The account's messages and keys on this device are deleted.")
            }
        }
    }

    private var version: String {
        let info = Bundle.main.infoDictionary
        let short = info?["CFBundleShortVersionString"] as? String ?? "?"
        let build = info?["CFBundleVersion"] as? String ?? "?"
        return "\(short) (\(build))"
    }
}

/// Where the Notes to self shows, for the account.
private struct RoomListSection: View {
    @EnvironmentObject private var rooms: RoomListModel

    var body: some View {
        Section {
            Toggle(
                "Notes at the top",
                isOn: Binding(get: { rooms.notesAtTop }, set: { rooms.setNotesAtTop($0) }))
        } header: {
            Text("Room list")
        } footer: {
            Text("Your notes to self stay at the top of Chats and of every space.")
        }
    }
}

/// Whether others see when we are online or idle.
private struct PrivacySection: View {
    @EnvironmentObject private var presence: PresenceModel

    var body: some View {
        Section {
            Toggle("Share my presence", isOn: Binding(get: { presence.sharing }, set: presence.setSharing))
                .disabled(!presence.available)
        } header: {
            Text("Privacy")
        } footer: {
            Text(presence.available ? "Others see when you are online or idle." : "This server does not share presence.")
        }
    }
}

/// Whether this device is verified and recovery unlocked; while not, the
/// ways to fix it: the recovery key, or another device.
struct EncryptionSection: View {
    @EnvironmentObject private var encryption: EncryptionModel

    var body: some View {
        Section {
            LabeledContent("This device", value: deviceStatus)
            LabeledContent("Recovery", value: recoveryStatus)
            if encryption.needsSetup, let state = encryption.state {
                if state.recovery != .disabled {
                    NavigationLink("Enter recovery key") {
                        RecoveryView(model: encryption.recovery)
                    }
                }
                if !state.deviceVerified {
                    if encryption.hasOtherDevice == false {
                        Text("Verifying needs another device signed in to this account, such as the desktop app.")
                            .font(.footnote)
                            .foregroundStyle(.secondary)
                    } else {
                        Button {
                            encryption.verifyWithAnotherDevice()
                        } label: {
                            HStack {
                                Text("Verify with another device")
                                Spacer()
                                if encryption.requestingVerification {
                                    ProgressView()
                                }
                            }
                        }
                        .disabled(encryption.requestingVerification)
                    }
                }
            }
        } header: {
            Text("Encryption")
        } footer: {
            if let error = encryption.verificationError {
                Text(error).foregroundColor(.red)
            }
        }
        .task(id: encryption.state?.deviceVerified) {
            if encryption.state?.deviceVerified == false {
                await encryption.loadDevices()
            }
        }
    }

    private var deviceStatus: String {
        guard let state = encryption.state else { return "not known yet" }
        return state.deviceVerified ? "verified" : "not verified"
    }

    private var recoveryStatus: String {
        switch encryption.state?.recovery {
        case nil, .unknown: return "not known yet"
        case .enabled: return "set up"
        case .disabled: return "not set up"
        case .incomplete: return "not unlocked on this device"
        }
    }
}
