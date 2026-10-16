import AppKit
import Combine

@MainActor
final class StatusMenu: NSObject, NSMenuDelegate {
    static private(set) weak var current: StatusMenu?
    private let manager: ConnectionManager
    private let item = NSStatusBar.system.statusItem(withLength: NSStatusItem.squareLength)
    private let menu = NSMenu()
    private var cancellables: Set<AnyCancellable> = []
    private var animation: Timer?
    private var frame = 0
    private var liveTimer: Timer?
    private var statItems: [String: NSMenuItem] = [:]

    init(manager: ConnectionManager) {
        self.manager = manager
        super.init()
        StatusMenu.current = self
        menu.delegate = self
        menu.autoenablesItems = false
        item.menu = menu
        item.button?.toolTip = "Fluidity"
        manager.$sessions
            .combineLatest(manager.$preferences.map(\.iconSet).removeDuplicates(), manager.$blocked)
            .receive(on: RunLoop.main)
            .sink { [weak self] _ in self?.refreshIcon() }
            .store(in: &cancellables)
        refreshIcon()
    }

    private var iconSet: IconSet { IconSet(rawValue: manager.preferences.iconSet) ?? .droplet }

    private func refreshIcon() {
        let state = manager.overallState
        let hasError = manager.sessions.values.contains { $0.state == .error } || !manager.blocked.isEmpty
        let shown: SessionState = state == .disconnected && hasError ? .error : state
        if state == .connecting {
            if animation == nil {
                animation = Timer.scheduledTimer(withTimeInterval: 0.12, repeats: true) { [weak self] _ in
                    Task { @MainActor in self?.tick() }
                }
                RunLoop.main.add(animation!, forMode: .common)
            }
        } else {
            animation?.invalidate()
            animation = nil
            item.button?.image = StatusIcon.image(set: iconSet, state: shown)
        }
        var names = manager.activeSessions.map { "\($0.name): \($0.state.label)" }
        if !manager.blocked.isEmpty { names.append("Kill switch: internet traffic blocked") }
        item.button?.toolTip = names.isEmpty ? "Fluidity: Not connected" : names.joined(separator: "\n")
    }

    var iconImage: NSImage? { item.button?.image }

    func open() { item.button?.performClick(nil) }

    var debugDescriptionText: String {
        rebuild()
        func dump(_ menu: NSMenu, _ indent: String) -> [String] {
            menu.items.flatMap { item -> [String] in
                if item.isSeparatorItem { return [indent + "---"] }
                let mark = item.state == .on ? "[x] " : item.state == .mixed ? "[-] " : ""
                let line = indent + mark + item.title + (item.isEnabled ? "" : " (disabled)")
                return [line] + (item.submenu.map { dump($0, indent + "  ") } ?? [])
            }
        }
        return dump(menu, "").joined(separator: "\n")
    }

    private func tick() {
        frame = (frame + 1) % StatusIcon.frameCount
        item.button?.image = StatusIcon.image(set: iconSet, state: .connecting, frame: frame)
    }

    func menuNeedsUpdate(_ menu: NSMenu) {
        rebuild()
    }

    func menuWillOpen(_ menu: NSMenu) {
        liveTimer = Timer(timeInterval: 1, repeats: true) { [weak self] _ in
            Task { @MainActor in self?.updateStats() }
        }
        RunLoop.main.add(liveTimer!, forMode: .common)
    }

    func menuDidClose(_ menu: NSMenu) {
        liveTimer?.invalidate()
        liveTimer = nil
    }

    private func rebuild() {
        menu.removeAllItems()
        statItems = [:]
        if !manager.daemonAvailable {
            let warn = NSMenuItem(title: "Fluidity service is not running", action: nil, keyEquivalent: "")
            warn.isEnabled = false
            menu.addItem(warn)
            menu.addItem(action("Install Helper…", #selector(installHelper)))
            menu.addItem(.separator())
        }
        if !manager.blocked.isEmpty {
            let warn = NSMenuItem(title: "Internet Traffic Blocked", action: nil, keyEquivalent: "")
            warn.isEnabled = false
            warn.image = NSImage(systemSymbolName: "exclamationmark.shield", accessibilityDescription: nil)
            menu.addItem(warn)
            let names = manager.blocked.compactMap { manager.connections[$0]?.name }
            if !names.isEmpty {
                let reconnect = action("Reconnect \(names.joined(separator: ", "))", #selector(reconnectBlocked))
                reconnect.indentationLevel = 1
                menu.addItem(reconnect)
            }
            let allow = action("Allow Internet Traffic", #selector(unblock))
            allow.indentationLevel = 1
            menu.addItem(allow)
            menu.addItem(.separator())
        }
        if manager.connections.isEmpty {
            let empty = NSMenuItem(title: "No Connections", action: nil, keyEquivalent: "")
            empty.isEnabled = false
            menu.addItem(empty)
        } else {
            add(nodes: manager.tree, to: menu)
            let listed = Set(flatten(manager.tree))
            for c in manager.orderedConnections where !listed.contains(c.id) {
                addConnection(c, to: menu)
            }
        }
        let active = manager.activeSessions
        menu.addItem(.separator())
        if !active.isEmpty {
            let count = active.count == 1 ? "1 Active Connection" : "\(active.count) Active Connections"
            let header = NSMenuItem(title: count, action: nil, keyEquivalent: "")
            header.isEnabled = false
            menu.addItem(header)
        }
        if active.count > 0 {
            menu.addItem(action("Disconnect All", #selector(disconnectAll)))
        }
        if manager.connections.count > 1 {
            menu.addItem(action("Connect All", #selector(connectAll)))
        }
        menu.addItem(action("Details…", #selector(showDetails)))
        menu.addItem(action("Preferences…", #selector(showPreferences), key: ","))
        let importItem = NSMenuItem(title: "Import Connection", action: nil, keyEquivalent: "")
        let importMenu = NSMenu()
        importMenu.addItem(action("From File…", #selector(importFile)))
        importMenu.addItem(action("New Connection…", #selector(newConnection)))
        importItem.submenu = importMenu
        menu.addItem(importItem)
        let custom = CustomMenuItems.load()
        if !custom.isEmpty {
            menu.addItem(.separator())
            for entry in custom {
                let mi = action(entry.title, #selector(runCustom(_:)))
                mi.representedObject = entry.path
                if let icon = entry.icon, let image = NSImage(contentsOf: icon) {
                    image.size = NSSize(width: 16, height: 16)
                    mi.image = image
                }
                menu.addItem(mi)
            }
        }
        menu.addItem(.separator())
        menu.addItem(action("Quit Fluidity", #selector(quit), key: "q"))
    }

    private func flatten(_ nodes: [Node]) -> [String] {
        nodes.flatMap { node -> [String] in
            switch node {
            case .connection(let id): return [id]
            case .folder(let f): return flatten(f.children)
            }
        }
    }

    private func add(nodes: [Node], to menu: NSMenu) {
        for node in nodes {
            switch node {
            case .connection(let id):
                if let c = manager.connections[id] { addConnection(c, to: menu) }
            case .folder(let folder):
                let item = NSMenuItem(title: folder.name, action: nil, keyEquivalent: "")
                item.image = NSImage(systemSymbolName: "folder", accessibilityDescription: nil)
                let sub = NSMenu()
                sub.autoenablesItems = false
                add(nodes: folder.children, to: sub)
                let ids = flatten(folder.children)
                if ids.count > 1 {
                    sub.addItem(.separator())
                    let all = action("Connect All in Folder", #selector(connectFolder(_:)))
                    all.representedObject = ids
                    sub.addItem(all)
                    let none = action("Disconnect All in Folder", #selector(disconnectFolder(_:)))
                    none.representedObject = ids
                    sub.addItem(none)
                }
                item.submenu = sub
                if ids.contains(where: { manager.state(of: $0) == .connected }) { item.state = .mixed }
                menu.addItem(item)
            }
        }
    }

    private func addConnection(_ c: ConnectionInfo, to menu: NSMenu) {
        menu.addItem(connectionItem(c))
        guard let s = manager.sessions[c.id], s.state == .connected else { return }
        let stat = NSMenuItem(title: "", action: nil, keyEquivalent: "")
        stat.isEnabled = false
        stat.indentationLevel = 1
        statItems[s.id] = stat
        updateStat(stat, s)
        menu.addItem(stat)
    }

    private func updateStat(_ item: NSMenuItem, _ s: SessionInfo) {
        let font = NSFont.menuFont(ofSize: NSFont.smallSystemFontSize)
        let title = NSMutableAttributedString(string: statTitle(s) + "  ", attributes: [
            .font: font,
            .foregroundColor: NSColor.secondaryLabelColor,
        ])
        let spark = NSTextAttachment()
        spark.image = Sparkline.image(manager.traffic[s.id] ?? [])
        spark.bounds = NSRect(x: 0, y: font.descender, width: 36, height: 12)
        title.append(NSAttributedString(attachment: spark))
        item.attributedTitle = title
    }

    private func connectionItem(_ c: ConnectionInfo) -> NSMenuItem {
        let state = manager.state(of: c.id)
        let item = action(c.name, #selector(toggle(_:)))
        item.representedObject = c.id
        item.image = dot(for: state)
        if state.isActive && state != .connected {
            item.title = "\(c.name) (\(state.label))"
        }
        item.toolTip = (state.isActive ? "Disconnect" : "Connect") + ". Option-click for details."
        return item
    }

    private func dot(for state: SessionState) -> NSImage {
        let color: NSColor
        switch state {
        case .connected: color = .systemGreen
        case .disconnected: color = .tertiaryLabelColor
        case .error: color = .systemRed
        default: color = .systemOrange
        }
        return NSImage(size: NSSize(width: 10, height: 10), flipped: false) { rect in
            color.setFill()
            NSBezierPath(ovalIn: rect.insetBy(dx: 1, dy: 1)).fill()
            return true
        }
    }

    private func statTitle(_ s: SessionInfo) -> String {
        var parts: [String] = []
        let address = manager.preferences.addressDisplay == "ipv6" && !s.localIpv6.isEmpty ? s.localIpv6 : s.localIpv4
        if !address.isEmpty { parts.append(address) }
        if manager.preferences.trafficDisplay == "totals" {
            parts.append("↓\(Format.bytes(Double(s.bytesIn))) ↑\(Format.bytes(Double(s.bytesOut)))")
        } else if let last = manager.traffic[s.id]?.last {
            parts.append("↓\(Format.bytes(last.inRate))/s ↑\(Format.bytes(last.outRate))/s")
        }
        if let since = s.connectedSince {
            parts.append(Format.duration(Date().timeIntervalSince1970 - Double(since)))
        }
        return parts.joined(separator: "  ")
    }

    private func updateStats() {
        for (id, item) in statItems {
            if let s = manager.sessions[id] { updateStat(item, s) }
        }
    }

    private func action(_ title: String, _ selector: Selector, key: String = "") -> NSMenuItem {
        let item = NSMenuItem(title: title, action: selector, keyEquivalent: key)
        item.target = self
        return item
    }

    @objc private func toggle(_ sender: NSMenuItem) {
        guard let id = sender.representedObject as? String else { return }
        if NSEvent.modifierFlags.contains(.option) {
            WindowManager.shared.showDetails(id)
            return
        }
        manager.toggle(id)
    }


    @objc private func connectFolder(_ sender: NSMenuItem) {
        for id in sender.representedObject as? [String] ?? [] where !manager.isActive(id) { manager.connect(id) }
    }

    @objc private func disconnectFolder(_ sender: NSMenuItem) {
        for id in sender.representedObject as? [String] ?? [] where manager.isActive(id) { manager.disconnect(id) }
    }

    @objc private func disconnectAll() { manager.disconnectAll() }
    @objc private func reconnectBlocked() { for id in manager.blocked { manager.connect(id) } }
    @objc private func unblock() { manager.unblock() }
    @objc private func connectAll() { manager.connectAll() }
    @objc private func showDetails() { WindowManager.shared.showDetails(manager.activeSessions.first?.id) }
    @objc private func showPreferences() { WindowManager.shared.showPreferences() }
    @objc private func importFile() { AppDelegate.shared?.importWithPanel() }
    @objc private func newConnection() { WindowManager.shared.showEditor(id: nil) }
    @objc private func installHelper() { HelperInstaller.install() }
    @objc private func quit() { NSApp.terminate(nil) }

    @objc private func runCustom(_ sender: NSMenuItem) {
        guard let path = sender.representedObject as? String else { return }
        let manager = self.manager
        let sessions = manager.activeSessions
        let first = sessions.first
        let connection = first.flatMap { manager.connections[$0.id] }
        Task.detached {
            var env = ProcessInfo.processInfo.environment
            if let connection {
                env = ScriptRunner.environment(connection: connection, session: first, username: nil)
            }
            _ = ScriptRunner.run(path: path, environment: env)
        }
    }
}

enum CustomMenuItems {
    static let fileExtension = "fluiditymenuitem"

    struct Entry {
        var title: String
        var path: String
        var icon: URL?
    }

    static var directory: URL {
        let base: String = (try? Core.call("data_dir")) ?? NSHomeDirectory()
        return URL(fileURLWithPath: base).appendingPathComponent("Menu Items")
    }

    static func load() -> [Entry] {
        let fm = FileManager.default
        guard let names = try? fm.contentsOfDirectory(atPath: directory.path) else { return [] }
        return names.filter { $0.hasSuffix("." + fileExtension) }.sorted().compactMap { name in
            let url = directory.appendingPathComponent(name)
            var isDir: ObjCBool = false
            fm.fileExists(atPath: url.path, isDirectory: &isDir)
            let script: URL
            var icon: URL?
            if isDir.boolValue {
                guard let inner = try? fm.contentsOfDirectory(atPath: url.path).sorted().first(where: { $0.hasPrefix("script") }) else { return nil }
                script = url.appendingPathComponent(inner)
                let png = url.appendingPathComponent("icon.png")
                if fm.fileExists(atPath: png.path) { icon = png }
            } else {
                script = url
            }
            return Entry(title: url.deletingPathExtension().lastPathComponent, path: script.path, icon: icon)
        }
    }

    static func install(_ url: URL) throws {
        let fm = FileManager.default
        try fm.createDirectory(at: directory, withIntermediateDirectories: true)
        let target = directory.appendingPathComponent(url.lastPathComponent)
        if fm.fileExists(atPath: target.path) { try fm.removeItem(at: target) }
        try fm.copyItem(at: url, to: target)
    }
}

enum Format {
    static func bytes(_ value: Double) -> String {
        let units = ["B", "KB", "MB", "GB", "TB"]
        var v = value
        var i = 0
        while v >= 1024 && i < units.count - 1 {
            v /= 1024
            i += 1
        }
        return i == 0 ? "\(Int(v)) B" : String(format: "%.1f %@", v, units[i])
    }

    static func duration(_ seconds: Double) -> String {
        let s = Int(max(seconds, 0))
        let h = s / 3600, m = (s % 3600) / 60, sec = s % 60
        if h > 0 { return String(format: "%d:%02d:%02d", h, m, sec) }
        return String(format: "%d:%02d", m, sec)
    }

    static func time(_ epoch: UInt64) -> String {
        let f = DateFormatter()
        f.dateFormat = "yyyy-MM-dd HH:mm:ss"
        return f.string(from: Date(timeIntervalSince1970: TimeInterval(epoch)))
    }
}

enum Sparkline {
    static func image(_ samples: [TrafficSample]) -> NSImage {
        let size = NSSize(width: 36, height: 12)
        let recent = Array(samples.suffix(30))
        return NSImage(size: size, flipped: false) { rect in
            let peak = recent.map { max($0.inRate, $0.outRate) }.max() ?? 0
            guard recent.count > 1, peak > 0 else {
                NSColor.tertiaryLabelColor.setFill()
                NSRect(x: 0, y: 0, width: rect.width, height: 1).fill()
                return true
            }
            let step = rect.width / CGFloat(recent.count - 1)
            func path(_ value: (TrafficSample) -> Double) -> NSBezierPath {
                let p = NSBezierPath()
                for (i, sample) in recent.enumerated() {
                    let point = NSPoint(x: CGFloat(i) * step, y: 1 + CGFloat(value(sample) / peak) * (rect.height - 2))
                    i == 0 ? p.move(to: point) : p.line(to: point)
                }
                p.lineWidth = 1
                return p
            }
            NSColor.systemGreen.setStroke()
            path(\.outRate).stroke()
            NSColor.systemBlue.setStroke()
            path(\.inRate).stroke()
            return true
        }
    }
}
