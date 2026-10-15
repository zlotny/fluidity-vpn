import AppKit
import Security
import SwiftUI

struct WelcomeView: View {
    @EnvironmentObject var manager: ConnectionManager
    let done: () -> Void

    var body: some View {
        VStack(spacing: 18) {
            Image(nsImage: NSApp.applicationIconImage).resizable().frame(width: 96, height: 96)
            Text("Welcome to Fluidity").font(.largeTitle.bold())
            Text("Fluidity lives in the menu bar. Add a connection to get started.")
                .foregroundStyle(.secondary)
            if !manager.daemonAvailable {
                GroupBox {
                    HStack {
                        Image(systemName: "exclamationmark.shield").font(.title2)
                        Text("Fluidity needs its helper service to connect.")
                        Spacer()
                        Button("Install Helper…") { HelperInstaller.install() }
                    }.padding(4)
                }
            }
            HStack(spacing: 12) {
                Button {
                    AppDelegate.shared?.importWithPanel()
                } label: {
                    Label("Import Configuration…", systemImage: "square.and.arrow.down").frame(minWidth: 170)
                }
                Button {
                    WindowManager.shared.showEditor(id: nil)
                } label: {
                    Label("Create Connection…", systemImage: "plus").frame(minWidth: 170)
                }
            }
            .controlSize(.large)
            Spacer()
            HStack {
                Toggle("Start Fluidity at login", isOn: Binding(get: { manager.preferences.startAtLogin }, set: {
                    manager.preferences.startAtLogin = $0
                    manager.savePreferences()
                    LoginItem.sync($0)
                }))
                Spacer()
                Button("Done", action: done).keyboardShortcut(.defaultAction)
            }
        }
        .padding(28)
        .frame(width: 560, height: 420)
    }
}

enum Keychain {
    static func identityNames() -> [String] {
        let query: [String: Any] = [
            kSecClass as String: kSecClassIdentity,
            kSecMatchLimit as String: kSecMatchLimitAll,
            kSecReturnRef as String: true,
        ]
        var result: CFTypeRef?
        guard SecItemCopyMatching(query as CFDictionary, &result) == errSecSuccess, let items = result as? [SecIdentity] else { return [] }
        var names: [String] = []
        for identity in items {
            var cert: SecCertificate?
            guard SecIdentityCopyCertificate(identity, &cert) == errSecSuccess, let cert else { continue }
            if let name = SecCertificateCopySubjectSummary(cert) as String?, !names.contains(name) {
                names.append(name)
            }
        }
        return names.sorted()
    }
}

@MainActor
final class Updater {
    static let shared = Updater()
    static let feed = URL(string: "https://api.github.com/repos/fluidityvpn/fluidity/releases")!

    private struct Release: Decodable {
        let tagName: String
        let htmlUrl: String
        let prerelease: Bool
        let draft: Bool
    }

    func scheduleChecks() {
        Timer.scheduledTimer(withTimeInterval: 24 * 3600, repeats: true) { _ in
            Task { @MainActor in Updater.shared.check(interactive: false) }
        }
        DispatchQueue.main.asyncAfter(deadline: .now() + 30) { self.check(interactive: false) }
    }

    func check(interactive: Bool) {
        let prefs = ConnectionManager.shared.preferences
        guard interactive || prefs.checkForUpdates else { return }
        Task {
            do {
                let (data, _) = try await URLSession.shared.data(from: Self.feed)
                let releases = try Core.decoder.decode([Release].self, from: data)
                let latest = releases.first { !$0.draft && (prefs.includeBetaUpdates || !$0.prerelease) }
                let current = AppInfo.version
                if let latest, Self.newer(latest.tagName, than: current) {
                    let alert = NSAlert()
                    alert.messageText = "Fluidity \(latest.tagName.trimmingCharacters(in: CharacterSet(charactersIn: "v"))) is available"
                    alert.informativeText = "You have version \(current)."
                    alert.addButton(withTitle: "Download")
                    alert.addButton(withTitle: "Later")
                    NSApp.activate(ignoringOtherApps: true)
                    if alert.runModal() == .alertFirstButtonReturn, let url = URL(string: latest.htmlUrl) {
                        NSWorkspace.shared.open(url)
                    }
                } else if interactive {
                    NSAlert.show("Fluidity is up to date", "Version \(current) is the latest version.")
                }
            } catch {
                if interactive { NSAlert.show("Cannot check for updates", error.localizedDescription) }
            }
        }
    }

    static func newer(_ tag: String, than current: String) -> Bool {
        func parts(_ s: String) -> [Int] {
            s.trimmingCharacters(in: CharacterSet(charactersIn: "v")).split(separator: "-").first.map {
                $0.split(separator: ".").compactMap { Int($0) }
            } ?? []
        }
        let a = parts(tag), b = parts(current)
        for i in 0..<max(a.count, b.count) {
            let x = i < a.count ? a[i] : 0, y = i < b.count ? b[i] : 0
            if x != y { return x > y }
        }
        return false
    }
}
