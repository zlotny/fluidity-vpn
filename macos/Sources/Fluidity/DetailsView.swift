import AppKit
import SwiftUI
import UniformTypeIdentifiers

@MainActor
final class DetailsState: ObservableObject {
    @Published var selection: String?
}

struct DetailsView: View {
    @EnvironmentObject var manager: ConnectionManager
    @ObservedObject var state: DetailsState

    private var items: [ConnectionInfo] {
        let all = manager.orderedConnections
        return all.filter { manager.isActive($0.id) } + all.filter { !manager.isActive($0.id) }
    }

    var body: some View {
        NavigationSplitView {
            List(items, selection: $state.selection) { c in
                HStack {
                    Circle().fill(color(manager.state(of: c.id))).frame(width: 8, height: 8)
                    VStack(alignment: .leading) {
                        Text(c.name)
                        Text(manager.state(of: c.id).label).font(.caption).foregroundStyle(.secondary)
                    }
                }
                .tag(c.id)
            }
            .navigationSplitViewColumnWidth(min: 170, ideal: 200)
        } detail: {
            if let id = state.selection, let c = manager.connections[id] {
                ConnectionDetail(connection: c)
                    .id(id)
            } else {
                Text("Select a connection").foregroundStyle(.secondary)
            }
        }
        .onAppear {
            if state.selection == nil { state.selection = items.first?.id }
        }
    }

    private func color(_ s: SessionState) -> Color {
        switch s {
        case .connected: return .green
        case .disconnected: return .secondary.opacity(0.4)
        case .error: return .red
        default: return .orange
        }
    }
}

struct ConnectionDetail: View {
    @EnvironmentObject var manager: ConnectionManager
    let connection: ConnectionInfo
    @State private var tab = 0
    @State private var now = Date()
    private let clock = Timer.publish(every: 1, on: .main, in: .common).autoconnect()

    var body: some View {
        let session = manager.sessions[connection.id]
        VStack(alignment: .leading, spacing: 12) {
            HStack {
                VStack(alignment: .leading) {
                    Text(connection.name).font(.title2.bold())
                    Text(session.map { $0.detail.isEmpty ? $0.state.label : "\($0.state.label): \($0.detail)" } ?? "Disconnected")
                        .foregroundStyle(.secondary)
                    if let error = session?.lastError {
                        Text(error).foregroundStyle(.red).font(.callout)
                    }
                }
                Spacer()
                Button(manager.isActive(connection.id) ? "Disconnect" : "Connect") { manager.toggle(connection.id) }
                    .controlSize(.large)
            }
            Picker("", selection: $tab) {
                Text("Status").tag(0)
                Text("Traffic").tag(1)
                Text("Log").tag(2)
            }
            .pickerStyle(.segmented)
            .labelsHidden()
            switch tab {
            case 0: StatusGrid(session: session, now: now)
            case 1: TrafficPane(id: connection.id, session: session)
            default: LogPane(id: connection.id)
            }
        }
        .padding()
        .onReceive(clock) { now = $0 }
    }
}

struct StatusGrid: View {
    let session: SessionInfo?
    let now: Date

    var body: some View {
        Form {
            if let s = session {
                LabeledContent("State", value: s.state.label)
                if let since = s.connectedSince {
                    LabeledContent("Connected for", value: Format.duration(now.timeIntervalSince1970 - Double(since)))
                }
                row("IPv4 address", s.localIpv4)
                row("IPv6 address", s.localIpv6)
                row("Server", s.remotePort.isEmpty ? s.remoteAddress : "\(s.remoteAddress):\(s.remotePort)")
                row("Interface", s.interface)
                LabeledContent("Received", value: Format.bytes(Double(s.bytesIn)))
                LabeledContent("Sent", value: Format.bytes(Double(s.bytesOut)))
            } else {
                Text("Not connected").foregroundStyle(.secondary)
            }
        }
        .formStyle(.grouped)
        .textSelection(.enabled)
    }

    @ViewBuilder private func row(_ title: String, _ value: String) -> some View {
        if !value.isEmpty { LabeledContent(title, value: value) }
    }
}

struct TrafficPane: View {
    @EnvironmentObject var manager: ConnectionManager
    let id: String
    let session: SessionInfo?

    var body: some View {
        let samples = manager.traffic[id] ?? []
        VStack(alignment: .leading, spacing: 8) {
            HStack(spacing: 20) {
                legend("In", .blue, samples.last?.inRate)
                legend("Out", .green, samples.last?.outRate)
                Spacer()
                if let s = session {
                    Text("Total ↓\(Format.bytes(Double(s.bytesIn))) ↑\(Format.bytes(Double(s.bytesOut)))")
                        .foregroundStyle(.secondary)
                }
            }
            if manager.preferences.graphStyle == "none" {
                Spacer()
            } else {
                TrafficGraph(samples: samples, bars: manager.preferences.graphStyle == "bars")
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
                    .background(RoundedRectangle(cornerRadius: 6).fill(Color(nsColor: .textBackgroundColor)))
                    .overlay(RoundedRectangle(cornerRadius: 6).stroke(Color.secondary.opacity(0.3)))
            }
        }
    }

    private func legend(_ title: String, _ color: Color, _ rate: Double?) -> some View {
        HStack(spacing: 6) {
            RoundedRectangle(cornerRadius: 2).fill(color).frame(width: 12, height: 4)
            Text("\(title): \(Format.bytes(rate ?? 0))/s").monospacedDigit()
        }
    }
}

struct TrafficGraph: View {
    let samples: [TrafficSample]
    let bars: Bool

    var body: some View {
        Canvas { context, size in
            let count = ConnectionManager.historyLength
            let peak = max(samples.map { max($0.inRate, $0.outRate) }.max() ?? 0, 1024)
            let step = size.width / CGFloat(count - 1)
            let offset = count - samples.count
            for fraction in [0.25, 0.5, 0.75] {
                var grid = Path()
                let y = size.height * fraction
                grid.move(to: CGPoint(x: 0, y: y))
                grid.addLine(to: CGPoint(x: size.width, y: y))
                context.stroke(grid, with: .color(.secondary.opacity(0.15)))
            }
            context.draw(Text(Format.bytes(peak) + "/s").font(.caption2).foregroundColor(.secondary), at: CGPoint(x: 6, y: 8), anchor: .leading)
            func y(_ v: Double) -> CGFloat { size.height - CGFloat(v / peak) * (size.height - 16) }
            if bars {
                let w = max(step / 2 - 0.5, 0.5)
                for (i, s) in samples.enumerated() {
                    let x = CGFloat(i + offset) * step
                    context.fill(Path(CGRect(x: x - w, y: y(s.inRate), width: w, height: size.height - y(s.inRate))), with: .color(.blue.opacity(0.7)))
                    context.fill(Path(CGRect(x: x, y: y(s.outRate), width: w, height: size.height - y(s.outRate))), with: .color(.green.opacity(0.7)))
                }
            } else {
                for (key, color) in [(\TrafficSample.inRate, Color.blue), (\TrafficSample.outRate, Color.green)] {
                    var line = Path()
                    var area = Path()
                    for (i, s) in samples.enumerated() {
                        let p = CGPoint(x: CGFloat(i + offset) * step, y: y(s[keyPath: key]))
                        if i == 0 {
                            line.move(to: p)
                            area.move(to: CGPoint(x: p.x, y: size.height))
                        } else {
                            line.addLine(to: p)
                        }
                        area.addLine(to: p)
                    }
                    if let last = samples.indices.last {
                        area.addLine(to: CGPoint(x: CGFloat(last + offset) * step, y: size.height))
                        area.closeSubpath()
                    }
                    context.fill(area, with: .color(color.opacity(0.12)))
                    context.stroke(line, with: .color(color), lineWidth: 1.5)
                }
            }
        }
    }
}

struct LogPane: View {
    @EnvironmentObject var manager: ConnectionManager
    let id: String
    @State private var filter = ""

    private var entries: [LogEntry] {
        let all = manager.logs[id] ?? []
        guard !filter.isEmpty else { return all }
        return all.filter { $0.message.localizedCaseInsensitiveContains(filter) }
    }

    var body: some View {
        VStack(spacing: 6) {
            HStack {
                TextField("Filter", text: $filter).textFieldStyle(.roundedBorder).frame(maxWidth: 220)
                Spacer()
                Button("Copy") {
                    NSPasteboard.general.clearContents()
                    NSPasteboard.general.setString(text(entries), forType: .string)
                }
                Button("Save…", action: save)
                Button("Clear") { manager.clearLog(id) }
            }
            ScrollViewReader { proxy in
                ScrollView {
                    LazyVStack(alignment: .leading, spacing: 1) {
                        ForEach(Array(entries.enumerated()), id: \.offset) { i, e in
                            Text("\(Format.time(e.time))  \(e.message)")
                                .font(.system(size: 11, design: .monospaced))
                                .foregroundStyle(color(e.level))
                                .frame(maxWidth: .infinity, alignment: .leading)
                                .id(i)
                        }
                    }
                    .padding(6)
                    .textSelection(.enabled)
                }
                .background(Color(nsColor: .textBackgroundColor))
                .overlay(RoundedRectangle(cornerRadius: 4).stroke(Color.secondary.opacity(0.3)))
                .onChange(of: entries.count) { count in proxy.scrollTo(count - 1, anchor: .bottom) }
                .onAppear { proxy.scrollTo(entries.count - 1, anchor: .bottom) }
            }
        }
        .task { await manager.loadLog(id) }
    }

    private func color(_ level: String) -> Color {
        switch level {
        case "error", "fatal": return .red
        case "warning": return .orange
        case "debug": return .secondary
        default: return .primary
        }
    }

    private func text(_ list: [LogEntry]) -> String {
        list.map { "\(Format.time($0.time)) \($0.message)" }.joined(separator: "\n")
    }

    private func save() {
        let panel = NSSavePanel()
        panel.nameFieldStringValue = "\(manager.connections[id]?.name ?? "Fluidity") Log.txt"
        panel.allowedContentTypes = [.plainText]
        guard panel.runModal() == .OK, let url = panel.url else { return }
        try? text(manager.logs[id] ?? []).write(to: url, atomically: true, encoding: .utf8)
    }
}
