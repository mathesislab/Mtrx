import MtrxFFI
import SwiftUI

/// Pausing notifications and quiet hours, on all our devices with Mtrx:
/// the pause (for an hour, until tomorrow morning, until a time picked,
/// until turned back on), quiet hours, and whether mentions get through.
struct QuietView: View {
    @EnvironmentObject private var quiet: QuietModel
    /// "Until…": a date and time being picked.
    @State private var picking = false
    @State private var until = Date().addingTimeInterval(3600)

    var body: some View {
        Form {
            Section {
                HStack {
                    Text(quiet.state?.line ?? "")
                    Spacer()
                    if quiet.state?.paused == true {
                        Button("Resume") { quiet.resume() }
                            .buttonStyle(.borderedProminent)
                    }
                }
            }
            Section("Pause notifications") {
                ForEach(Array(quiet.pauseMenu().enumerated()), id: \.offset) { _, item in
                    Button(item.title) {
                        switch item.entry {
                        case .pause(let pause):
                            quiet.pause(pause)
                        case .pickTime:
                            until = Date().addingTimeInterval(3600)
                            picking = true
                        }
                    }
                }
            }
            if let state = quiet.state {
                hoursSection(state.hours, line: state.hoursLine)
                Section {
                    Toggle(
                        "Let mentions and keywords through",
                        isOn: Binding(get: { state.mentionsThrough }, set: { quiet.setMentionsThrough($0) }))
                } footer: {
                    Text("Mtrx on all your devices. Other Matrix apps aren't paused.")
                }
            }
            if let error = quiet.error {
                Section {
                    Text(error).foregroundColor(.red).font(.footnote)
                }
            }
        }
        .navigationTitle("Notifications")
        .sheet(isPresented: $picking) {
            NavigationStack {
                Form {
                    DatePicker("Until", selection: $until, in: Date()..., displayedComponents: [.date, .hourAndMinute])
                        .datePickerStyle(.graphical)
                }
                .navigationTitle("Pause until")
                .navigationBarTitleDisplayMode(.inline)
                .toolbar {
                    ToolbarItem(placement: .cancellationAction) {
                        Button("Cancel") { picking = false }
                    }
                    ToolbarItem(placement: .confirmationAction) {
                        Button("Pause") {
                            quiet.pause(.at(at: until))
                            picking = false
                        }
                    }
                }
            }
        }
    }

    /// On or off, from and to, and the days a quiet period starts on.
    private func hoursSection(_ hours: QuietHours, line: String) -> some View {
        Section {
            Toggle("Quiet hours", isOn: Binding(get: { hours.on }, set: { on in
                var changed = hours
                changed.on = on
                quiet.setHours(changed)
            }))
            DatePicker(
                "From",
                selection: Binding(get: { QuietHours.date(hours.fromMinute) }, set: { date in
                    var changed = hours
                    changed.fromMinute = QuietHours.minute(date)
                    quiet.setHours(changed)
                }),
                displayedComponents: .hourAndMinute)
            DatePicker(
                "To",
                selection: Binding(get: { QuietHours.date(hours.toMinute) }, set: { date in
                    var changed = hours
                    changed.toMinute = QuietHours.minute(date)
                    quiet.setHours(changed)
                }),
                displayedComponents: .hourAndMinute)
            HStack(spacing: 6) {
                ForEach(Self.days.indices, id: \.self) { index in
                    let (day, letter) = Self.days[index]
                    let on = hours.days.contains(day)
                    Button {
                        var changed = hours
                        changed.days = on ? hours.days.filter { $0 != day } : Self.ordered(hours.days + [day])
                        quiet.setHours(changed)
                    } label: {
                        Text(letter)
                            .font(.footnote.weight(.semibold))
                            .frame(maxWidth: .infinity, minHeight: 32)
                            .foregroundColor(on ? .white : .accentColor)
                            .background(on ? Color.accentColor : Color.clear, in: Circle())
                            .overlay(Circle().stroke(Color.accentColor, lineWidth: 1))
                    }
                    .buttonStyle(.plain)
                    .accessibilityLabel(String(describing: day))
                    .accessibilityAddTraits(on ? .isSelected : [])
                }
            }
        } header: {
            Text("Quiet hours")
        } footer: {
            Text(line)
        }
    }

    private static let days: [(Weekday, String)] = [
        (.mon, "M"), (.tue, "T"), (.wed, "W"), (.thu, "T"), (.fri, "F"), (.sat, "S"), (.sun, "S"),
    ]

    /// `days` in the week's order, Monday first.
    private static func ordered(_ days: [Weekday]) -> [Weekday] {
        Self.days.map(\.0).filter(days.contains)
    }
}

/// A bell struck through while notifications are paused or in quiet hours;
/// a tap opens `QuietView`. Nothing otherwise.
struct QuietBell: View {
    @EnvironmentObject private var quiet: QuietModel
    @EnvironmentObject private var session: SessionModel
    @State private var showing = false

    var body: some View {
        if quiet.quiet {
            Button {
                showing = true
            } label: {
                Image(systemName: "bell.slash")
            }
            .accessibilityLabel(quiet.state?.line ?? "Notifications paused")
            .sheet(isPresented: $showing) {
                NavigationStack {
                    QuietView()
                        .toolbar {
                            ToolbarItem(placement: .confirmationAction) {
                                Button("Done") { showing = false }
                            }
                        }
                }
                .account(session)
            }
        }
    }
}
