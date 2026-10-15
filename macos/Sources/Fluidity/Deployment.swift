import AppKit

struct DeploymentManifest: Decodable { var version: String; var connections: String }
struct PendingDeployment: Decodable { var dir: String; var manifest: DeploymentManifest; var connections: [String] }
struct DeploymentReport: Decodable { var added: [String]; var updated: [String]; var removed: [String]; var skipped: [String]; var errors: [String] }

private struct PendingParam: Encodable { var dirs: [String] }
private struct ApplyParam: Encodable { var dir: String; var mode: String? }

@MainActor
enum Deployment {
    static func check(_ manager: ConnectionManager) {
        let bundled = Bundle.main.resourceURL?.appendingPathComponent("Deploy").path
        let found: PendingDeployment? = try? Core.call("deploy.pending", PendingParam(dirs: bundled.map { [$0] } ?? []))
        guard let pending = found else { return }
        var mode: String? = pending.manifest.connections
        if mode == "prompt" {
            let alert = NSAlert()
            alert.messageText = "Install connections from your organization?"
            alert.informativeText = "\(pending.connections.count) preconfigured connection(s) are available."
            alert.addButton(withTitle: "Replace Existing")
            alert.addButton(withTitle: "Add New Only")
            alert.addButton(withTitle: "Skip")
            NSApp.activate(ignoringOtherApps: true)
            switch alert.runModal() {
            case .alertFirstButtonReturn: mode = "replace"
            case .alertSecondButtonReturn: mode = "append"
            default: mode = nil
            }
        }
        do {
            let report: DeploymentReport = try Core.call("deploy.apply", ApplyParam(dir: pending.dir, mode: mode))
            manager.reload()
            manager.preferences = (try? Core.call("preferences.load")) ?? manager.preferences
            if !report.errors.isEmpty {
                NSAlert.show("Some connections could not be installed", report.errors.joined(separator: "\n"))
            }
        } catch {
            NSAlert.show("Cannot apply the deployment", error.localizedDescription)
        }
    }
}
