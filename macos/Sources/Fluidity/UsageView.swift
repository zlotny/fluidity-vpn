import Charts
import SwiftUI

struct UsageDay: Identifiable {
    var date: Date
    var received: Double
    var sent: Double
    var id: Date { date }
}

enum Usage {
    static func days(_ samples: [UsageSample], count: Int = 30, now: Date = Date(), calendar: Calendar = .current) -> [UsageDay] {
        let today = calendar.startOfDay(for: now)
        var days = (0..<count).reversed().compactMap { offset in
            calendar.date(byAdding: .day, value: -offset, to: today).map { UsageDay(date: $0, received: 0, sent: 0) }
        }
        guard let first = days.first?.date else { return days }
        for s in samples {
            let day = calendar.startOfDay(for: Date(timeIntervalSince1970: TimeInterval(s.hour)))
            guard day >= first, let i = calendar.dateComponents([.day], from: first, to: day).day, i < days.count else { continue }
            days[i].received += Double(s.bytesIn)
            days[i].sent += Double(s.bytesOut)
        }
        return days
    }

    static func ticks(peak: Double) -> [Double] {
        guard peak > 0 else { return [0] }
        var unit = 1.0
        while peak / unit >= 1024 { unit *= 1024 }
        let step = [1.0, 2, 5, 10, 20, 50, 100, 200, 500, 1000].map { $0 * unit }.first { peak / $0 <= 4 } ?? 1024 * unit
        return stride(from: 0, through: peak, by: step).map { $0 }
    }

    static func total(_ days: ArraySlice<UsageDay>) -> (received: Double, sent: Double) {
        days.reduce((0, 0)) { ($0.0 + $1.received, $0.1 + $1.sent) }
    }
}

struct UsagePane: View {
    @EnvironmentObject var manager: ConnectionManager
    let id: String
    @State private var samples: [UsageSample] = []
    @State private var confirmReset = false
    private let refresh = Timer.publish(every: 30, on: .main, in: .common).autoconnect()

    var body: some View {
        let days = Usage.days(samples)
        Form {
            Section {
                row("Today", Usage.total(days.suffix(1)))
                row("Last 7 days", Usage.total(days.suffix(7)))
                row("Last 30 days", Usage.total(days.suffix(30)))
            }
            Section {
                Chart {
                    ForEach(days) { day in
                        BarMark(x: .value("Day", day.date, unit: .day), y: .value("Bytes", day.received))
                            .foregroundStyle(by: .value("Direction", "Received"))
                        BarMark(x: .value("Day", day.date, unit: .day), y: .value("Bytes", day.sent))
                            .foregroundStyle(by: .value("Direction", "Sent"))
                    }
                }
                .chartForegroundStyleScale(["Received": Color.blue, "Sent": Color.green])
                .chartYAxis {
                    AxisMarks(values: Usage.ticks(peak: days.map { $0.received + $0.sent }.max() ?? 0)) { value in
                        AxisGridLine()
                        AxisValueLabel { if let v = value.as(Double.self) { Text(Format.bytes(v)) } }
                    }
                }
                .chartXAxis {
                    AxisMarks(values: days.reversed().enumerated().filter { $0.offset % 7 == 6 }.map { $0.element.date }) { _ in
                        AxisGridLine()
                        AxisValueLabel(format: .dateTime.month(.abbreviated).day())
                    }
                }
                .chartLegend(position: .top, alignment: .leading)
                .frame(height: 170)
                .padding(.vertical, 4)
            }
            HStack {
                Text("Usage is kept on this computer for 30 days.")
                    .foregroundStyle(.secondary)
                    .font(.callout)
                Spacer()
                Button("Reset…") { confirmReset = true }
                    .disabled(samples.isEmpty)
            }
        }
        .formStyle(.grouped)
        .task { await load() }
        .onReceive(refresh) { _ in Task { await load() } }
        .confirmationDialog("Reset the usage history?", isPresented: $confirmReset) {
            Button("Reset", role: .destructive) {
                Task {
                    try? await manager.daemon.clearUsage(id: id)
                    await load()
                }
            }
        }
    }

    private func load() async {
        if let s = try? await manager.daemon.usage(id: id) { samples = s }
    }

    private func row(_ title: String, _ total: (received: Double, sent: Double)) -> some View {
        LabeledContent(title) {
            Text("↓ \(Format.bytes(total.received))   ↑ \(Format.bytes(total.sent))").monospacedDigit()
        }
    }
}
