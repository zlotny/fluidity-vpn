import AppKit
import SwiftUI
import UniformTypeIdentifiers

enum PreferencesTab: String, CaseIterable, Identifiable {
    case connections, general, appearance, advanced, about
    var id: String { rawValue }

    var title: String { rawValue.capitalized }

    var symbol: String {
        switch self {
        case .connections: return "network"
        case .general: return "gearshape"
        case .appearance: return "paintbrush"
        case .advanced: return "slider.horizontal.3"
        case .about: return "info.circle"
        }
    }
}

@MainActor
final class PreferencesState: ObservableObject {
    @Published var tab: PreferencesTab = .connections
    @Published var selection: Set<String> = []
}

struct PreferencesView: View {
    @ObservedObject var state: PreferencesState

    var body: some View {
        TabView(selection: $state.tab) {
            ForEach(PreferencesTab.allCases) { tab in
                content(tab)
                    .tabItem { Label(tab.title, systemImage: tab.symbol) }
                    .tag(tab)
            }
        }
        .padding()
        .frame(minWidth: 680, minHeight: 460)
    }

    @ViewBuilder private func content(_ tab: PreferencesTab) -> some View {
        switch tab {
        case .connections: ConnectionsPane(state: state)
        case .general: GeneralPane()
        case .appearance: AppearancePane()
        case .advanced: AdvancedPane()
        case .about: AboutPane()
        }
    }
}

struct TreeRow: Identifiable, Hashable {
    var id: String
    var name: String
    var isFolder: Bool
    var children: [TreeRow]?
}

@MainActor
enum TreeEdit {
    static func newId() -> String {
        (0..<6).map { _ in String(format: "%02x", UInt8.random(in: 0...255)) }.joined()
    }

    static func rows(_ nodes: [Node], _ manager: ConnectionManager) -> [TreeRow] {
        nodes.compactMap { node in
            switch node {
            case .connection(let id):
                guard let c = manager.connections[id] else { return nil }
                return TreeRow(id: id, name: c.name, isFolder: false, children: nil)
            case .folder(let f):
                return TreeRow(id: f.id, name: f.name, isFolder: true, children: rows(f.children, manager))
            }
        }
    }

    static func folder(_ id: String, in nodes: [Node]) -> Folder? {
        for node in nodes {
            if case .folder(let f) = node {
                if f.id == id { return f }
                if let inner = folder(id, in: f.children) { return inner }
            }
        }
        return nil
    }

    static func removing(_ id: String, from nodes: [Node]) -> ([Node], Node?) {
        var removed: Node?
        var result: [Node] = []
        for node in nodes {
            switch node {
            case .connection(let cid):
                if cid == id { removed = node } else { result.append(node) }
            case .folder(var f):
                if f.id == id {
                    removed = node
                    continue
                }
                let (children, inner) = removing(id, from: f.children)
                if inner != nil { removed = inner }
                f.children = children
                result.append(.folder(f))
            }
        }
        return (result, removed)
    }

    static func inserting(_ node: Node, into folderId: String?, _ nodes: [Node]) -> [Node] {
        guard let folderId else { return nodes + [node] }
        return nodes.map { n in
            guard case .folder(var f) = n else { return n }
            if f.id == folderId {
                f.children.append(node)
            } else {
                f.children = inserting(node, into: folderId, f.children)
            }
            return .folder(f)
        }
    }

    static func updatingFolder(_ id: String, _ nodes: [Node], _ change: (inout Folder) -> Void) -> [Node] {
        nodes.map { n in
            guard case .folder(var f) = n else { return n }
            if f.id == id { change(&f) } else { f.children = updatingFolder(id, f.children, change) }
            return .folder(f)
        }
    }

    static func dissolving(_ id: String, _ nodes: [Node]) -> [Node] {
        nodes.flatMap { n -> [Node] in
            guard case .folder(var f) = n else { return [n] }
            if f.id == id { return f.children }
            f.children = dissolving(id, f.children)
            return [.folder(f)]
        }
    }

    static func moving(_ id: String, by offset: Int, _ nodes: [Node]) -> [Node] {
        var list = nodes
        if let i = list.firstIndex(where: { key($0) == id }) {
            let j = i + offset
            guard j >= 0, j < list.count else { return nodes }
            list.swapAt(i, j)
            return list
        }
        return list.map { n in
            guard case .folder(var f) = n else { return n }
            f.children = moving(id, by: offset, f.children)
            return .folder(f)
        }
    }

    static func inserting(_ node: Node, before target: String, _ nodes: [Node]) -> [Node] {
        nodes.flatMap { n -> [Node] in
            if key(n) == target { return [node, n] }
            guard case .folder(var f) = n else { return [n] }
            f.children = inserting(node, before: target, f.children)
            return [.folder(f)]
        }
    }

    static func contains(_ id: String, in nodes: [Node]) -> Bool {
        nodes.contains { n in
            if key(n) == id { return true }
            if case .folder(let f) = n { return contains(id, in: f.children) }
            return false
        }
    }

    static func key(_ node: Node) -> String {
        switch node {
        case .connection(let id): return id
        case .folder(let f): return f.id
        }
    }

    static func allFolders(_ nodes: [Node], prefix: String = "") -> [(String, String)] {
        nodes.flatMap { n -> [(String, String)] in
            guard case .folder(let f) = n else { return [] }
            let name = prefix + f.name
            return [(f.id, name)] + allFolders(f.children, prefix: name + " / ")
        }
    }
}

struct ConnectionsPane: View {
    @EnvironmentObject var manager: ConnectionManager
    @ObservedObject var state: PreferencesState
    @State private var renamingFolder: String?
    @State private var folderName = ""
    @State private var dropTargeted = false

    private var rows: [TreeRow] { TreeEdit.rows(manager.tree, manager) }
    private var selected: String? { state.selection.count == 1 ? state.selection.first : nil }
    private var selectedFolder: Folder? { selected.flatMap { TreeEdit.folder($0, in: manager.tree) } }
    private var selectedConnection: ConnectionInfo? { selected.flatMap { manager.connections[$0] } }

    var body: some View {
        HStack(spacing: 12) {
            VStack(spacing: 0) {
                List(rows, children: \.children, selection: $state.selection) { row in
                    rowView(row)
                        .contextMenu { contextMenu(row) }
                        .draggable(row.id)
                        .dropDestination(for: String.self) { ids, _ in drop(ids, on: row) }
                }
                .listStyle(.bordered(alternatesRowBackgrounds: true))
                .overlay(dropTargeted ? RoundedRectangle(cornerRadius: 6).stroke(Color.accentColor, lineWidth: 2) : nil)
                .onDrop(of: [.fileURL], isTargeted: $dropTargeted, perform: dropFiles)
                .onDeleteCommand { removeSelected() }
                toolbar
            }
            .frame(minWidth: 280)
            detail.frame(width: 300)
        }
        .sheet(item: Binding(get: { renamingFolder.map(IdBox.init) }, set: { renamingFolder = $0?.id })) { box in
            VStack(alignment: .leading, spacing: 12) {
                Text("Folder name").font(.headline)
                TextField("Name", text: $folderName).frame(width: 260)
                HStack {
                    Spacer()
                    Button("Cancel") { renamingFolder = nil }.keyboardShortcut(.cancelAction)
                    Button("Save") {
                        manager.setTree(TreeEdit.updatingFolder(box.id, manager.tree) { $0.name = folderName })
                        renamingFolder = nil
                    }.keyboardShortcut(.defaultAction).disabled(folderName.trimmingCharacters(in: .whitespaces).isEmpty)
                }
            }.padding(20)
        }
    }

    private func rowView(_ row: TreeRow) -> some View {
        HStack {
            if row.isFolder {
                Image(systemName: "folder")
            } else {
                Circle().fill(color(manager.state(of: row.id))).frame(width: 8, height: 8)
            }
            Text(row.name)
            Spacer()
            if !row.isFolder, manager.isActive(row.id) {
                Text(manager.state(of: row.id).label).font(.caption).foregroundStyle(.secondary)
            }
        }
        .contentShape(Rectangle())
        .onTapGesture(count: 2) {
            if row.isFolder { startRename(row.id) } else { WindowManager.shared.showEditor(id: row.id) }
        }
        .onTapGesture { state.selection = [row.id] }
    }

    private func color(_ s: SessionState) -> Color {
        switch s {
        case .connected: return .green
        case .disconnected: return .secondary.opacity(0.4)
        case .error: return .red
        default: return .orange
        }
    }

    @ViewBuilder private func contextMenu(_ row: TreeRow) -> some View {
        if row.isFolder {
            Button("Rename…") { startRename(row.id) }
            Button("Delete Folder") { manager.setTree(TreeEdit.dissolving(row.id, manager.tree)) }
        } else {
            Button(manager.isActive(row.id) ? "Disconnect" : "Connect") { manager.toggle(row.id) }
            Button("Edit…") { WindowManager.shared.showEditor(id: row.id) }
            Button("Duplicate") { manager.duplicate(row.id) }
            Button("Export…") { export(row.id) }
            Menu("Move to Folder") {
                Button("Top Level") { move(row.id, to: nil) }
                ForEach(TreeEdit.allFolders(manager.tree), id: \.0) { folder in
                    Button(folder.1) { move(row.id, to: folder.0) }
                }
            }
            Divider()
            Button("Remove…") { state.selection = [row.id]; removeSelected() }
        }
    }

    private var toolbar: some View {
        HStack(spacing: 2) {
            Menu {
                Button("New Connection…") { WindowManager.shared.showEditor(id: nil) }
                Button("Import from File…") { AppDelegate.shared?.importWithPanel() }
                Divider()
                Button("New Folder") { newFolder() }
            } label: { Image(systemName: "plus") }
                .menuStyle(.borderlessButton).frame(width: 36)
            Button { removeSelected() } label: { Image(systemName: "minus") }
                .buttonStyle(.borderless).frame(width: 28).disabled(state.selection.isEmpty)
            Divider().frame(height: 16)
            Button { if let s = selected { move(s, by: -1) } } label: { Image(systemName: "chevron.up") }
                .buttonStyle(.borderless).frame(width: 28).disabled(selected == nil)
            Button { if let s = selected { move(s, by: 1) } } label: { Image(systemName: "chevron.down") }
                .buttonStyle(.borderless).frame(width: 28).disabled(selected == nil)
            Spacer()
            if let c = selectedConnection {
                Button("Edit") { WindowManager.shared.showEditor(id: c.id) }
                Button(manager.isActive(c.id) ? "Disconnect" : "Connect") { manager.toggle(c.id) }
            }
        }
        .padding(4)
        .background(Color(nsColor: .controlBackgroundColor))
    }

    @ViewBuilder private var detail: some View {
        if let folder = selectedFolder {
            FolderDetail(folder: folder)
        } else if let c = selectedConnection {
            ConnectionSummary(connection: c)
        } else {
            VStack(spacing: 8) {
                Image(systemName: "tray.and.arrow.down").font(.system(size: 36)).foregroundStyle(.secondary)
                Text("Select a connection, or drop configuration files here to import them.")
                    .multilineTextAlignment(.center).foregroundStyle(.secondary)
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
        }
    }

    private func startRename(_ id: String) {
        folderName = TreeEdit.folder(id, in: manager.tree)?.name ?? ""
        renamingFolder = id
    }

    private func newFolder() {
        let folder = Folder(id: TreeEdit.newId(), name: "New Folder", children: [], sharedCredentials: false)
        manager.setTree(manager.tree + [.folder(folder)])
        state.selection = [folder.id]
        startRename(folder.id)
    }

    private func move(_ id: String, to folder: String?) {
        let (rest, node) = TreeEdit.removing(id, from: manager.tree)
        guard let node else { return }
        manager.setTree(TreeEdit.inserting(node, into: folder, rest))
    }

    private func drop(_ ids: [String], on row: TreeRow) -> Bool {
        guard let id = ids.first, id != row.id, TreeEdit.contains(id, in: manager.tree) else { return false }
        if let folder = TreeEdit.folder(id, in: manager.tree), TreeEdit.contains(row.id, in: folder.children) { return false }
        let (rest, node) = TreeEdit.removing(id, from: manager.tree)
        guard let node else { return false }
        manager.setTree(row.isFolder ? TreeEdit.inserting(node, into: row.id, rest) : TreeEdit.inserting(node, before: row.id, rest))
        state.selection = [id]
        return true
    }

    private func move(_ id: String, by offset: Int) {
        manager.setTree(TreeEdit.moving(id, by: offset, manager.tree))
    }

    private func removeSelected() {
        let ids = state.selection
        guard !ids.isEmpty else { return }
        let names = ids.compactMap { manager.connections[$0]?.name ?? TreeEdit.folder($0, in: manager.tree)?.name }
        let alert = NSAlert()
        alert.messageText = "Remove \(names.joined(separator: ", "))?"
        alert.informativeText = "Connections are deleted with their certificates and saved passwords. Folders are removed and their connections are kept."
        alert.addButton(withTitle: "Remove")
        alert.addButton(withTitle: "Cancel")
        alert.alertStyle = .warning
        guard alert.runModal() == .alertFirstButtonReturn else { return }
        var tree = manager.tree
        for id in ids where TreeEdit.folder(id, in: tree) != nil {
            tree = TreeEdit.dissolving(id, tree)
        }
        manager.setTree(tree)
        for id in ids where manager.connections[id] != nil { manager.remove(id) }
        state.selection = []
    }

    private func export(_ id: String) {
        guard let c = manager.connections[id] else { return }
        let panel = NSSavePanel()
        panel.nameFieldStringValue = c.name + ".fvpnz"
        panel.allowedContentTypes = [UTType(filenameExtension: "fvpnz") ?? .data]
        guard panel.runModal() == .OK, let url = panel.url else { return }
        do { try manager.export(id, to: url) } catch { NSAlert(error: error).runModal() }
    }

    private func dropFiles(_ providers: [NSItemProvider]) -> Bool {
        for provider in providers {
            _ = provider.loadObject(ofClass: URL.self) { url, _ in
                guard let url else { return }
                Task { @MainActor in
                    if url.pathExtension == CustomMenuItems.fileExtension {
                        try? CustomMenuItems.install(url)
                    } else {
                        ConnectionManager.shared.importFiles([url])
                    }
                }
            }
        }
        return true
    }
}

struct IdBox: Identifiable { var id: String }

struct ConnectionSummary: View {
    @EnvironmentObject var manager: ConnectionManager
    let connection: ConnectionInfo

    var body: some View {
        let session = manager.sessions[connection.id]
        Form {
            Section {
                LabeledContent("Name", value: connection.name)
                LabeledContent("Status", value: session?.state.label ?? "Disconnected")
                if let s = session, s.state == .connected {
                    LabeledContent("Address", value: s.localIpv4.isEmpty ? s.localIpv6 : s.localIpv4)
                    LabeledContent("Server", value: s.remoteAddress)
                }
                if let error = session?.lastError { LabeledContent("Last error", value: error) }
            }
            Section {
                LabeledContent("Auto-reconnect", value: connection.autoReconnect ? "On" : "Off")
                LabeledContent("Connect on launch", value: connection.connectOnLaunch ? "On" : "Off")
            }
            Section {
                HStack {
                    Button("Edit…") { WindowManager.shared.showEditor(id: connection.id) }
                    Button("Details…") { WindowManager.shared.showDetails(connection.id) }
                    Button("Forget Password") {
                        try? Core.run("credentials.clear", ScopeParam(scope: connection.credentialScope))
                    }
                }
            }
        }
        .formStyle(.grouped)
    }
}

struct FolderDetail: View {
    @EnvironmentObject var manager: ConnectionManager
    let folder: Folder

    var body: some View {
        Form {
            Section("Folder") {
                LabeledContent("Name", value: folder.name)
                Toggle("Share saved credentials between connections", isOn: binding(\.sharedCredentials))
            }
            Section("Scripts for all connections in this folder") {
                scriptRow("Before connect", \.beforeConnectScript)
                scriptRow("Connected", \.connectedScript)
                scriptRow("Disconnected", \.disconnectedScript)
            }
        }
        .formStyle(.grouped)
    }

    private func binding(_ path: WritableKeyPath<Folder, Bool>) -> Binding<Bool> {
        Binding(get: { folder[keyPath: path] }, set: { value in
            manager.setTree(TreeEdit.updatingFolder(folder.id, manager.tree) { $0[keyPath: path] = value })
        })
    }

    private func scriptRow(_ title: String, _ path: WritableKeyPath<Folder, String?>) -> some View {
        LabeledContent(title) {
            HStack {
                Text(folder[keyPath: path].map { URL(fileURLWithPath: $0).lastPathComponent } ?? "None")
                    .foregroundStyle(.secondary).lineLimit(1).truncationMode(.middle)
                Button("Choose…") {
                    let panel = NSOpenPanel()
                    panel.canChooseFiles = true
                    if panel.runModal() == .OK, let url = panel.url {
                        manager.setTree(TreeEdit.updatingFolder(folder.id, manager.tree) { $0[keyPath: path] = url.path })
                    }
                }
                if folder[keyPath: path] != nil {
                    Button { manager.setTree(TreeEdit.updatingFolder(folder.id, manager.tree) { $0[keyPath: path] = nil }) } label: {
                        Image(systemName: "xmark.circle.fill")
                    }.buttonStyle(.borderless)
                }
            }
        }
    }
}

struct GeneralPane: View {
    @EnvironmentObject var manager: ConnectionManager

    var body: some View {
        Form {
            Section {
                Toggle("Start Fluidity at login", isOn: pref(\.startAtLogin))
                Toggle("Show notifications", isOn: pref(\.notifications))
            }
            Section("Reconnect") {
                Toggle("Reconnect connections after waking from sleep", isOn: pref(\.reconnectOnWake))
                Toggle("Reconnect connections when the network comes back", isOn: pref(\.reconnectOnNetwork))
                Picker("Disconnect when idle", selection: idleBinding) {
                    Text("Never").tag(UInt32(0))
                    ForEach([5, 10, 15, 30, 60, 120] as [UInt32], id: \.self) { Text("After \($0) minutes").tag($0) }
                }
            }
            Section("While connected") {
                Toggle("Block IPv6 traffic when the VPN has no IPv6", isOn: pref(\.blockIpv6))
                Toggle("Pause Time Machine backups", isOn: pref(\.disableTimeMachine))
            }
            Section("Updates") {
                Toggle("Check for updates automatically", isOn: pref(\.checkForUpdates))
                Toggle("Include beta versions", isOn: pref(\.includeBetaUpdates)).disabled(!manager.preferences.checkForUpdates)
                Button("Check Now") { Updater.shared.check(interactive: true) }
            }
        }
        .formStyle(.grouped)
    }

    private var idleBinding: Binding<UInt32> {
        Binding(get: { manager.preferences.idleDisconnectMinutes ?? 0 }, set: { value in
            manager.preferences.idleDisconnectMinutes = value == 0 ? nil : value
            manager.savePreferences()
        })
    }

    private func pref(_ path: WritableKeyPath<Preferences, Bool>) -> Binding<Bool> {
        Binding(get: { manager.preferences[keyPath: path] }, set: { value in
            manager.preferences[keyPath: path] = value
            manager.savePreferences()
            if path == \Preferences.startAtLogin { LoginItem.sync(value) }
        })
    }
}

struct AppearancePane: View {
    @EnvironmentObject var manager: ConnectionManager

    var body: some View {
        Form {
            Section("Menu bar icon") {
                Picker("Icon style", selection: pref(\.iconSet)) {
                    ForEach(IconSet.allCases) { set in
                        HStack {
                            ForEach([SessionState.disconnected, .connecting, .connected], id: \.self) { state in
                                Image(nsImage: StatusIcon.image(set: set, state: state, frame: 3))
                            }
                            Text(set.title)
                        }.tag(set.rawValue)
                    }
                }
                .pickerStyle(.radioGroup)
            }
            Section("Menu statistics") {
                Picker("Address", selection: pref(\.addressDisplay)) {
                    Text("IPv4").tag("ipv4")
                    Text("IPv6").tag("ipv6")
                }
                Picker("Traffic", selection: pref(\.trafficDisplay)) {
                    Text("Rates").tag("rates")
                    Text("Totals").tag("totals")
                }
            }
            Section("Details window") {
                Picker("Traffic graph", selection: pref(\.graphStyle)) {
                    Text("Line").tag("line")
                    Text("Bars").tag("bars")
                    Text("None").tag("none")
                }
                Toggle("Keep the Details window on top", isOn: Binding(get: { manager.preferences.detailsAlwaysOnTop }, set: {
                    manager.preferences.detailsAlwaysOnTop = $0
                    manager.savePreferences()
                }))
            }
        }
        .formStyle(.grouped)
    }

    private func pref(_ path: WritableKeyPath<Preferences, String>) -> Binding<String> {
        Binding(get: { manager.preferences[keyPath: path] }, set: { value in
            manager.preferences[keyPath: path] = value
            manager.savePreferences()
        })
    }
}

struct AdvancedPane: View {
    @EnvironmentObject var manager: ConnectionManager
    @State private var versions: [String] = []
    @State private var allowUnsafe = false
    @State private var loaded = false

    var body: some View {
        Form {
            Section("OpenVPN") {
                Picker("Default version", selection: Binding(get: { manager.preferences.openvpnVersion ?? "" }, set: {
                    manager.preferences.openvpnVersion = $0.isEmpty ? nil : $0
                    manager.savePreferences()
                })) {
                    Text("Latest").tag("")
                    ForEach(versions, id: \.self) { Text($0).tag($0) }
                }
                Toggle("Allow unsafe OpenVPN commands", isOn: Binding(get: { allowUnsafe }, set: setUnsafe))
                Text("Unsafe commands run scripts and programs as root. Only enable this when you trust every configuration. Changing it requires an administrator password.")
                    .font(.caption).foregroundStyle(.secondary)
            }
            Section("Network") {
                Toggle("Reset network interfaces on disconnect", isOn: Binding(get: { manager.preferences.resetInterfacesOnDisconnect }, set: {
                    manager.preferences.resetInterfacesOnDisconnect = $0
                    manager.savePreferences()
                }))
            }
            Section("Data") {
                HStack {
                    Button("Clear Saved Passwords…") { clearCredentials() }
                    Button("Open Menu Items Folder") { openMenuItems() }
                }
            }
            Section("Helper") {
                LabeledContent("Service", value: manager.daemonAvailable ? "Running" : "Not running")
                HStack {
                    Button(HelperInstaller.isInstalled ? "Reinstall Helper…" : "Install Helper…") { HelperInstaller.install() }
                    if HelperInstaller.isInstalled {
                        Button("Uninstall Helper…") { HelperInstaller.uninstall() }
                    }
                    Button("Install Command-Line Tool…") { installCLI() }
                }
            }
        }
        .formStyle(.grouped)
        .task { await load() }
        .onChange(of: manager.daemonAvailable) { _ in Task { await load() } }
    }

    private func load() async {
        versions = (try? await manager.daemon.versions()) ?? []
        allowUnsafe = (try? await manager.daemon.systemSettings().allowUnsafeCommands) ?? false
    }

    private func setUnsafe(_ value: Bool) {
        guard let cli = AppInfo.cliPath else {
            NSAlert.show("Command-line tool missing", "This build of Fluidity does not include the fluidity tool.")
            return
        }
        let result = Admin.run(
            "\(Admin.quote(cli)) system-settings --allow-unsafe-commands \(value ? "true" : "false")",
            prompt: "Fluidity wants to change a system-wide security setting.")
        if result.ok { allowUnsafe = value } else { NSAlert.show("Cannot change the setting", result.output) }
    }

    private func clearCredentials() {
        let alert = NSAlert()
        alert.messageText = "Clear all saved passwords?"
        alert.informativeText = "Fluidity removes every username and password it saved in your Keychain."
        alert.addButton(withTitle: "Clear")
        alert.addButton(withTitle: "Cancel")
        guard alert.runModal() == .alertFirstButtonReturn else { return }
        do { try Core.run("credentials.clear", ScopeParam(scope: nil)) } catch { NSAlert(error: error).runModal() }
    }

    private func openMenuItems() {
        let dir = CustomMenuItems.directory
        try? FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        NSWorkspace.shared.open(dir)
    }

    private func installCLI() {
        guard let cli = AppInfo.cliPath else {
            NSAlert.show("Command-line tool missing", "This build of Fluidity does not include the fluidity tool.")
            return
        }
        let result = Admin.run(
            "mkdir -p /usr/local/bin && ln -sf \(Admin.quote(cli)) /usr/local/bin/fluidity",
            prompt: "Fluidity wants to install the fluidity command.")
        if result.ok {
            NSAlert.show("Command-line tool installed", "Run \"fluidity --help\" in Terminal.")
        } else {
            NSAlert.show("Cannot install the command-line tool", result.output)
        }
    }
}

struct AboutPane: View {
    var body: some View {
        VStack(spacing: 12) {
            Image(nsImage: NSApp.applicationIconImage).resizable().frame(width: 96, height: 96)
            Text("Fluidity").font(.largeTitle.bold())
            Text("Version \(AppInfo.version)").foregroundStyle(.secondary)
            Text("A free and open source OpenVPN client.")
            Text("Licensed under the GNU General Public License v3.0 or later.")
                .font(.caption).foregroundStyle(.secondary)
            HStack {
                Link("Website", destination: AppInfo.website)
                Link("Report an Issue", destination: AppInfo.website.appendingPathComponent("issues"))
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }
}

enum AppInfo {
    static var version: String {
        let short = Bundle.main.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String
        return short ?? ((try? Core.call("version")) ?? "dev")
    }

    static var cliPath: String? {
        let path = Bundle.main.bundleURL.appendingPathComponent("Contents/Helpers/fluidity").path
        return FileManager.default.isExecutableFile(atPath: path) ? path : nil
    }

    static let website = URL(string: "https://github.com/fluidityvpn/fluidity")!
}
