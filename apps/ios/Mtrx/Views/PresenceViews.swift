import MtrxFFI
import SwiftUI

extension View {
    /// A presence dot of `size` on the bottom right of a round avatar of
    /// `avatar` points, ringed in `ring` (the background colour); none
    /// offline or unknown.
    func presenceDot(_ presence: Presence?, size: CGFloat, avatar: CGFloat, ring: Color = Color(.systemBackground))
        -> some View
    {
        overlay(alignment: .bottomTrailing) {
            if let dot = presence?.dot {
                let outer = size + 4
                // The dot's centre on the circle, 45° below its right.
                let offset = outer / 2 - avatar / 2 * (1 - 1 / 2.squareRoot())
                Circle()
                    .fill(dot == .online ? Color.green : Color.orange)
                    .frame(width: size, height: size)
                    .padding(2)
                    .background(ring, in: Circle())
                    .offset(x: offset, y: offset)
                    .accessibilityLabel(presenceDotLabel(dot: dot))
            }
        }
    }
}

/// The line under a person's name: their status message, "Active now",
/// "Idle" or when they were last active; nothing without.
struct PresenceLine: View {
    let presence: Presence?
    var font = Font.subheadline

    var body: some View {
        // Ages update each minute.
        TimelineView(.everyMinute) { context in
            if let presence, let line = presenceLine(presence: presence, now: context.date) {
                Text(line)
                    .font(font)
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
            }
        }
    }
}
