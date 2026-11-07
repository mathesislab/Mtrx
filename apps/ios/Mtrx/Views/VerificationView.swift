import MtrxFFI
import SwiftUI

/// Verifying this device with another of ours by comparing emoji, in either
/// direction: each state of the verification as it comes.
struct VerificationView: View {
    @ObservedObject var model: VerificationModel
    @EnvironmentObject private var encryption: EncryptionModel

    var body: some View {
        NavigationStack {
            ScrollView {
                VStack(spacing: 24) {
                    content
                    if let error = model.error {
                        Text(error)
                            .font(.callout)
                            .foregroundColor(.red)
                            .multilineTextAlignment(.center)
                    }
                }
                .padding(24)
                .frame(maxWidth: 520)
                .frame(maxWidth: .infinity)
            }
            .navigationTitle("Verify this device")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                if !model.ended {
                    ToolbarItem(placement: .cancellationAction) {
                        Button("Cancel", action: cancel)
                    }
                }
            }
        }
        // Closed only by its buttons, which cancel or end the verification.
        .interactiveDismissDisabled()
    }

    @ViewBuilder
    private var content: some View {
        if model.asking && !model.ended {
            message("Another device wants to verify this one.", systemImage: "laptopcomputer.and.iphone")
            HStack(spacing: 12) {
                Button("Decline", role: .destructive, action: cancel)
                    .buttonStyle(.bordered)
                Button("Accept") { model.accept() }
                    .buttonStyle(.borderedProminent)
            }
            .disabled(model.busy)
        } else {
            switch model.state {
            case .waiting:
                message(
                    model.incoming ? "Waiting for the other device…" : "Accept the request on your other device.",
                    systemImage: "laptopcomputer.and.iphone")
                ProgressView()
                Button("Cancel", role: .destructive, action: cancel)
                    .buttonStyle(.bordered)
            case .emojis(let emojis):
                EmojiGrid(emojis: emojis)
                Text("Do they match the other device?")
                    .font(.headline)
                VStack(spacing: 12) {
                    Button { model.confirm() } label: {
                        Text("They match").frame(maxWidth: .infinity)
                    }
                    .buttonStyle(.borderedProminent)
                    Button(role: .destructive) { model.mismatch() } label: {
                        Text("They do not match").frame(maxWidth: .infinity)
                    }
                    .buttonStyle(.bordered)
                }
                .controlSize(.large)
                .disabled(model.busy)
            case .confirmed:
                message("Waiting for the other device to confirm…", systemImage: "hourglass")
                ProgressView()
            case .sharingKeys:
                message("Finishing… keep the app open.", systemImage: "key")
                ProgressView()
            case .done:
                message("This device is verified.", systemImage: "checkmark.shield")
                    .foregroundColor(.green)
                Button("Done") { encryption.closeVerification() }
                    .buttonStyle(.borderedProminent)
            case .cancelled(let reason):
                VStack(spacing: 8) {
                    message("Verification was cancelled.", systemImage: "xmark.shield")
                    Text(reason)
                        .font(.callout)
                        .foregroundStyle(.secondary)
                        .multilineTextAlignment(.center)
                }
                Button("Close") { encryption.closeVerification() }
                    .buttonStyle(.borderedProminent)
            }
        }
    }

    private func message(_ text: String, systemImage: String) -> some View {
        VStack(spacing: 12) {
            Image(systemName: systemImage)
                .font(.largeTitle)
                .imageScale(.large)
                .foregroundStyle(.secondary)
            Text(text)
                .font(.title3)
                .multilineTextAlignment(.center)
        }
    }

    /// Stop (or decline) and close.
    private func cancel() {
        model.cancel()
        encryption.closeVerification()
    }
}

/// The seven emoji in order: the symbol large, its name below.
private struct EmojiGrid: View {
    let emojis: [VerificationEmoji]

    var body: some View {
        LazyVGrid(columns: [GridItem(.adaptive(minimum: 72), spacing: 12)], spacing: 16) {
            ForEach(Array(emojis.enumerated()), id: \.offset) { _, emoji in
                VStack(spacing: 4) {
                    Text(emoji.symbol)
                        .font(.largeTitle)
                        .imageScale(.large)
                    Text(emoji.description)
                        .font(.caption)
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                        .minimumScaleFactor(0.7)
                }
                .accessibilityElement(children: .combine)
            }
        }
    }
}
