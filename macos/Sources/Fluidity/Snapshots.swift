import AppKit

@MainActor
enum Snapshots {
    private static var timer: Timer?
    static var detailsTab = 0

    static func start() {
        guard let path = ProcessInfo.processInfo.environment["FLUIDITY_SNAPSHOT_DIR"] else { return }
        let dir = URL(fileURLWithPath: path)
        try? FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        let env = ProcessInfo.processInfo.environment
        for name in (env["FLUIDITY_SNAPSHOT_SHOW"] ?? "").split(separator: ",") {
            switch name {
            case "welcome": WindowManager.shared.showWelcome()
            case let tab where tab.hasPrefix("details"):
                detailsTab = ["status", "traffic", "log", "usage"].firstIndex(of: tab.split(separator: ":").dropFirst().first.map(String.init) ?? "") ?? 0
                WindowManager.shared.showDetails(nil)
            case "editor": WindowManager.shared.showEditor(id: nil)
            case let tab where tab.hasPrefix("preferences"):
                let raw = tab.split(separator: ":").dropFirst().first.map(String.init) ?? "connections"
                WindowManager.shared.showPreferences(tab: PreferencesTab(rawValue: raw))
            default: break
            }
        }
        timer = Timer.scheduledTimer(withTimeInterval: 1.5, repeats: true) { _ in
            Task { @MainActor in capture(to: dir) }
        }
    }

    private static func capture(to dir: URL) {
        for window in NSApp.windows where window.isVisible {
            guard let view = window.contentView?.superview ?? window.contentView,
                  let rep = view.bitmapImageRepForCachingDisplay(in: view.bounds) else { continue }
            view.cacheDisplay(in: view.bounds, to: rep)
            let name = window.title.isEmpty ? "window-\(window.windowNumber)" : window.title
            if let png = rep.representation(using: .png, properties: [:]) {
                try? png.write(to: dir.appendingPathComponent(name.replacingOccurrences(of: "/", with: "-") + ".png"))
            }
        }
        if let menu = StatusMenu.current?.debugDescriptionText {
            try? menu.write(to: dir.appendingPathComponent("menu.txt"), atomically: true, encoding: .utf8)
        }
        if let image = StatusMenu.current?.iconImage, let tiff = image.tiffRepresentation,
           let rep = NSBitmapImageRep(data: tiff), let png = rep.representation(using: .png, properties: [:]) {
            try? png.write(to: dir.appendingPathComponent("status-icon.png"))
        }
    }
}
