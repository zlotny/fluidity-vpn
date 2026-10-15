import AppKit
import ServiceManagement
import SwiftUI

@MainActor
final class WindowManager: NSObject, NSWindowDelegate {
    static let shared = WindowManager()

    private var windows: [String: NSWindow] = [:]
    let preferencesState = PreferencesState()
    let detailsState = DetailsState()

    private func show<V: View>(_ key: String, title: String, size: NSSize, resizable: Bool = true, floating: Bool = false, @ViewBuilder content: () -> V) -> NSWindow {
        if let existing = windows[key] {
            NSApp.activate(ignoringOtherApps: true)
            existing.makeKeyAndOrderFront(nil)
            return existing
        }
        var style: NSWindow.StyleMask = [.titled, .closable, .miniaturizable]
        if resizable { style.insert(.resizable) }
        let window = NSWindow(contentRect: NSRect(origin: .zero, size: size), styleMask: style, backing: .buffered, defer: false)
        window.title = title
        window.isReleasedWhenClosed = false
        window.contentViewController = NSHostingController(rootView: content().environmentObject(ConnectionManager.shared))
        window.setContentSize(size)
        window.center()
        window.setFrameAutosaveName("Fluidity.\(key)")
        window.delegate = self
        window.identifier = NSUserInterfaceItemIdentifier(key)
        if floating { window.level = .floating }
        windows[key] = window
        NSApp.activate(ignoringOtherApps: true)
        window.makeKeyAndOrderFront(nil)
        return window
    }

    func windowWillClose(_ notification: Notification) {
        guard let window = notification.object as? NSWindow, let key = window.identifier?.rawValue else { return }
        windows[key] = nil
    }

    func close(_ key: String) {
        windows[key]?.close()
    }

    func showPreferences(tab: PreferencesTab? = nil) {
        if let tab { preferencesState.tab = tab }
        _ = show("preferences", title: "Fluidity Preferences", size: NSSize(width: 720, height: 500)) {
            PreferencesView(state: preferencesState)
        }
    }

    func showDetails(_ id: String?) {
        if let id { detailsState.selection = id }
        let floating = ConnectionManager.shared.preferences.detailsAlwaysOnTop
        let window = show("details", title: "Fluidity Details", size: NSSize(width: 640, height: 480), floating: floating) {
            DetailsView(state: detailsState)
        }
        window.level = floating ? .floating : .normal
    }

    func showEditor(id: String?) {
        let key = "editor-\(id ?? "new")"
        let model: EditorModel
        do {
            if let id {
                model = try Core.call("editor.load", IdParam(id: id))
            } else {
                model = try Core.call("editor.new")
            }
        } catch {
            NSAlert(error: error).runModal()
            return
        }
        let title = id == nil ? "New Connection" : "Edit \(model.name)"
        _ = show(key, title: title, size: NSSize(width: 680, height: 560)) {
            EditorView(id: id, model: model) { [weak self] in self?.close(key) }
        }
    }

    func showWelcome() {
        _ = show("welcome", title: "Welcome to Fluidity", size: NSSize(width: 560, height: 400), resizable: false) {
            WelcomeView { [weak self] in
                ConnectionManager.shared.preferences.firstRun = false
                ConnectionManager.shared.savePreferences()
                self?.close("welcome")
            }
        }
    }
}

@MainActor
enum LoginItem {
    static func sync(_ enabled: Bool) {
        guard Bundle.main.bundleIdentifier != nil else { return }
        let service = SMAppService.mainApp
        do {
            if enabled, service.status != .enabled {
                try service.register()
            } else if !enabled, service.status == .enabled {
                try service.unregister()
            }
        } catch {
            ConnectionManager.shared.lastError = error.localizedDescription
        }
    }
}

enum Admin {
    static func run(_ command: String, prompt: String) -> (ok: Bool, output: String) {
        let escaped = command.replacingOccurrences(of: "\\", with: "\\\\").replacingOccurrences(of: "\"", with: "\\\"")
        let promptEscaped = prompt.replacingOccurrences(of: "\"", with: "\\\"")
        let source = "do shell script \"\(escaped)\" with prompt \"\(promptEscaped)\" with administrator privileges"
        var error: NSDictionary?
        let result = NSAppleScript(source: source)?.executeAndReturnError(&error)
        if let error {
            return (false, error[NSAppleScript.errorMessage] as? String ?? "Failed")
        }
        return (true, result?.stringValue ?? "")
    }

    static func quote(_ value: String) -> String {
        "'" + value.replacingOccurrences(of: "'", with: "'\\''") + "'"
    }
}

enum HelperInstaller {
    static let plist = "/Library/LaunchDaemons/org.fluidityvpn.daemon.plist"

    static var script: String? {
        Bundle.main.path(forResource: "install-helper", ofType: "sh")
    }

    static var isInstalled: Bool {
        FileManager.default.fileExists(atPath: plist)
    }

    @MainActor
    static func offer() {
        guard script != nil else { return }
        let alert = NSAlert()
        alert.messageText = isInstalled ? "The Fluidity service is not responding" : "Install the Fluidity helper?"
        alert.informativeText = "Fluidity needs a background service to create VPN tunnels and change network settings. Installing it requires an administrator password."
        alert.addButton(withTitle: isInstalled ? "Reinstall" : "Install")
        alert.addButton(withTitle: "Not Now")
        NSApp.activate(ignoringOtherApps: true)
        if alert.runModal() == .alertFirstButtonReturn {
            install()
        }
    }

    @MainActor
    static func install() {
        guard let script else {
            NSAlert.show("Installer missing", "This build of Fluidity does not include the helper installer.")
            return
        }
        let resources = Bundle.main.resourcePath ?? ""
        let result = Admin.run("/bin/sh \(Admin.quote(script)) \(Admin.quote(resources))", prompt: "Fluidity wants to install its VPN helper.")
        if !result.ok {
            NSAlert.show("Cannot install the helper", result.output)
        }
    }

    @MainActor
    static func uninstall() {
        guard let path = Bundle.main.path(forResource: "uninstall-helper", ofType: "sh") else { return }
        let result = Admin.run("/bin/sh \(Admin.quote(path))", prompt: "Fluidity wants to remove its VPN helper.")
        if !result.ok {
            NSAlert.show("Cannot remove the helper", result.output)
        }
    }
}

extension NSAlert {
    static func show(_ title: String, _ message: String) {
        let alert = NSAlert()
        alert.messageText = title
        alert.informativeText = message
        NSApp.activate(ignoringOtherApps: true)
        alert.runModal()
    }
}
