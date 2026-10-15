import AppKit
import Combine
import UniformTypeIdentifiers
import UserNotifications

@main
enum Main {
    static func main() {
        let args = CommandLine.arguments
        if args.count == 3, args[1] == "--render-iconset" {
            exit(StatusIcon.writeIconset(to: URL(fileURLWithPath: args[2])) ? 0 : 1)
        }
        let app = NSApplication.shared
        let delegate = AppDelegate()
        app.delegate = delegate
        app.setActivationPolicy(.accessory)
        app.run()
    }
}

@MainActor
final class AppDelegate: NSObject, NSApplicationDelegate, UNUserNotificationCenterDelegate {
    static private(set) weak var shared: AppDelegate?

    let manager = ConnectionManager.shared
    private var statusMenu: StatusMenu?
    private var pendingOpen: [URL] = []
    private var launched = false
    private var offeredHelper = false

    override init() {
        super.init()
        AppDelegate.shared = self
    }

    func applicationDidFinishLaunching(_ notification: Notification) {
        if Bundle.main.bundleIdentifier != nil {
            UNUserNotificationCenter.current().delegate = self
            UNUserNotificationCenter.current().requestAuthorization(options: [.alert, .sound]) { _, _ in }
        }
        NSApp.applicationIconImage = Bundle.main.image(forResource: "AppIcon") ?? StatusIcon.appIcon()
        installMainMenu()
        manager.start()
        statusMenu = StatusMenu(manager: manager)
        launched = true
        if !pendingOpen.isEmpty {
            manager.importFiles(pendingOpen)
            pendingOpen = []
        }
        if manager.preferences.firstRun {
            WindowManager.shared.showWelcome()
        }
        DispatchQueue.main.asyncAfter(deadline: .now() + 4) { [weak self] in
            guard let self, !self.manager.daemonAvailable else { return }
            self.offerHelperInstall()
        }
        LoginItem.sync(manager.preferences.startAtLogin)
        Updater.shared.scheduleChecks()
        Snapshots.start()
    }

    func applicationShouldHandleReopen(_ sender: NSApplication, hasVisibleWindows flag: Bool) -> Bool {
        if !flag { WindowManager.shared.showPreferences() }
        return true
    }

    func application(_ application: NSApplication, open urls: [URL]) {
        if launched {
            manager.importFiles(urls)
            WindowManager.shared.showPreferences()
        } else {
            pendingOpen += urls
        }
    }

    func applicationShouldTerminate(_ sender: NSApplication) -> NSApplication.TerminateReply {
        let active = manager.activeSessions
        guard !active.isEmpty else { return .terminateNow }
        let alert = NSAlert()
        alert.messageText = "Disconnect before quitting?"
        alert.informativeText = "\(active.count) connection(s) are active. Quitting Fluidity disconnects them."
        alert.addButton(withTitle: "Disconnect and Quit")
        alert.addButton(withTitle: "Cancel")
        NSApp.activate(ignoringOtherApps: true)
        guard alert.runModal() == .alertFirstButtonReturn else { return .terminateCancel }
        manager.disconnectAll()
        DispatchQueue.main.asyncAfter(deadline: .now() + 1.5) {
            NSApp.reply(toApplicationShouldTerminate: true)
        }
        return .terminateLater
    }

    nonisolated func userNotificationCenter(
        _ center: UNUserNotificationCenter, willPresent notification: UNNotification,
        withCompletionHandler completionHandler: @escaping (UNNotificationPresentationOptions) -> Void
    ) {
        completionHandler([.banner])
    }

    func offerHelperInstall() {
        guard !offeredHelper, !manager.daemonAvailable else { return }
        offeredHelper = true
        HelperInstaller.offer()
    }

    func importWithPanel() {
        let panel = NSOpenPanel()
        panel.allowsMultipleSelection = true
        panel.canChooseDirectories = true
        panel.message = "Choose OpenVPN configurations or Fluidity connections to import."
        panel.prompt = "Import"
        NSApp.activate(ignoringOtherApps: true)
        if panel.runModal() == .OK {
            manager.importFiles(panel.urls)
        }
    }

    private func installMainMenu() {
        let main = NSMenu()
        let appItem = NSMenuItem()
        let appMenu = NSMenu()
        appMenu.addItem(withTitle: "About Fluidity", action: #selector(showAbout), keyEquivalent: "").target = self
        appMenu.addItem(.separator())
        appMenu.addItem(withTitle: "Preferences…", action: #selector(showPreferences), keyEquivalent: ",").target = self
        appMenu.addItem(.separator())
        appMenu.addItem(withTitle: "Hide Fluidity", action: #selector(NSApplication.hide(_:)), keyEquivalent: "h")
        appMenu.addItem(withTitle: "Quit Fluidity", action: #selector(NSApplication.terminate(_:)), keyEquivalent: "q")
        appItem.submenu = appMenu
        main.addItem(appItem)

        let editItem = NSMenuItem()
        let edit = NSMenu(title: "Edit")
        edit.addItem(withTitle: "Undo", action: Selector(("undo:")), keyEquivalent: "z")
        edit.addItem(withTitle: "Redo", action: Selector(("redo:")), keyEquivalent: "Z")
        edit.addItem(.separator())
        edit.addItem(withTitle: "Cut", action: #selector(NSText.cut(_:)), keyEquivalent: "x")
        edit.addItem(withTitle: "Copy", action: #selector(NSText.copy(_:)), keyEquivalent: "c")
        edit.addItem(withTitle: "Paste", action: #selector(NSText.paste(_:)), keyEquivalent: "v")
        edit.addItem(withTitle: "Select All", action: #selector(NSText.selectAll(_:)), keyEquivalent: "a")
        editItem.submenu = edit
        main.addItem(editItem)

        let windowItem = NSMenuItem()
        let window = NSMenu(title: "Window")
        window.addItem(withTitle: "Close", action: #selector(NSWindow.performClose(_:)), keyEquivalent: "w")
        window.addItem(withTitle: "Minimize", action: #selector(NSWindow.performMiniaturize(_:)), keyEquivalent: "m")
        windowItem.submenu = window
        main.addItem(windowItem)
        NSApp.mainMenu = main
    }

    @objc func showAbout() { WindowManager.shared.showPreferences(tab: .about) }
    @objc func showPreferences() { WindowManager.shared.showPreferences() }
}
