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
            case let editor where editor.hasPrefix("editor"):
                let name = editor.split(separator: ":").dropFirst().first.map(String.init)
                let id = ConnectionManager.shared.connections.values.first { $0.name == name }?.id
                WindowManager.shared.showEditor(id: id)
            case "menu":
                DispatchQueue.main.asyncAfter(deadline: .now() + 4) { StatusMenu.current?.open() }
            case let tab where tab.hasPrefix("preferences"):
                let raw = tab.split(separator: ":").dropFirst().first.map(String.init) ?? "connections"
                WindowManager.shared.showPreferences(tab: PreferencesTab(rawValue: raw))
            default: break
            }
        }
        NSApp.activate(ignoringOtherApps: true)
        DispatchQueue.main.asyncAfter(deadline: .now() + 1) {
            NSApp.activate(ignoringOtherApps: true)
            for window in NSApp.windows { window.makeFirstResponder(nil) }
        }
        let timer = Timer(timeInterval: 1.5, repeats: true) { _ in
            MainActor.assumeIsolated { capture(to: dir) }
        }
        RunLoop.main.add(timer, forMode: .common)
        self.timer = timer
    }

    private typealias CreateImage = @convention(c) (CGRect, UInt32, UInt32, UInt32) -> Unmanaged<CGImage>?

    private static let createImage: CreateImage? = dlsym(dlopen(nil, RTLD_NOW), "CGWindowListCreateImage")
        .map { unsafeBitCast($0, to: CreateImage.self) }

    private static func capture(to dir: URL) {
        if let createImage {
            composite(to: dir, createImage)
        } else {
            render(to: dir)
        }
        if let menu = StatusMenu.current?.debugDescriptionText {
            try? menu.write(to: dir.appendingPathComponent("menu.txt"), atomically: true, encoding: .utf8)
        }
        if let image = StatusMenu.current?.iconImage, let tiff = image.tiffRepresentation,
           let rep = NSBitmapImageRep(data: tiff), let png = rep.representation(using: .png, properties: [:]) {
            try? png.write(to: dir.appendingPathComponent("status-icon.png"))
        }
    }

    private static func composite(to dir: URL, _ createImage: CreateImage) {
        let pid = Int(ProcessInfo.processInfo.processIdentifier)
        let info = CGWindowListCopyWindowInfo(.optionOnScreenOnly, kCGNullWindowID) as? [[String: Any]] ?? []
        for entry in info where entry[kCGWindowOwnerPID as String] as? Int == pid {
            guard let number = entry[kCGWindowNumber as String] as? Int,
                  let image = createImage(.null, 1 << 3, UInt32(number), 1 << 3)?.takeRetainedValue(),
                  image.width > 4, image.height > 4 else { continue }
            let layer = entry[kCGWindowLayer as String] as? Int ?? 0
            let title = NSApp.window(withWindowNumber: number)?.title ?? ""
            let name = title.isEmpty ? "layer\(layer)-\(number)" : title
            let png = NSBitmapImageRep(cgImage: image).representation(using: .png, properties: [:])
            try? png?.write(to: dir.appendingPathComponent(name.replacingOccurrences(of: "/", with: "-") + ".png"))
        }
    }

    private static func render(to dir: URL) {
        for window in NSApp.windows where window.isVisible {
            guard let view = window.contentView?.superview ?? window.contentView,
                  let rep = view.bitmapImageRepForCachingDisplay(in: view.bounds) else { continue }
            view.cacheDisplay(in: view.bounds, to: rep)
            let name = window.title.isEmpty ? "window-\(window.windowNumber)" : window.title
            if let png = rep.representation(using: .png, properties: [:]) {
                try? png.write(to: dir.appendingPathComponent(name.replacingOccurrences(of: "/", with: "-") + ".png"))
            }
        }
    }
}
