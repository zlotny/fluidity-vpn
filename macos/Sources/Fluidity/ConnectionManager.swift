import AppKit
import Combine
import Network
import UserNotifications

struct TrafficSample: Hashable {
    var time: Date
    var inRate: Double
    var outRate: Double
}

@MainActor
final class ConnectionManager: ObservableObject {
    static let shared = ConnectionManager()

    @Published private(set) var tree: [Node] = []
    @Published private(set) var connections: [String: ConnectionInfo] = [:]
    @Published private(set) var sessions: [String: SessionInfo] = [:]
    @Published private(set) var logs: [String: [LogEntry]] = [:]
    @Published private(set) var traffic: [String: [TrafficSample]] = [:]
    @Published private(set) var daemonAvailable = false
    @Published private(set) var blocked: [String] = []
    @Published var preferences = Preferences()
    @Published var lastError: String?

    let daemon = DaemonClient()
    let prompts = PromptController()
    static let historyLength = 300

    private var userInitiatedDisconnect: Set<String> = []
    private var failureAlerts: Set<String> = []
    private var wantedBeforeSleep: Set<String> = []
    private var wantedBeforeNetworkLoss: Set<String> = []
    private var previousStates: [String: SessionState] = [:]
    private var pathMonitor: NWPathMonitor?
    private var lastPathSatisfied = true
    private var idleTimer: Timer?
    private var pendingChallenge: [String: String] = [:]
    private var usernames: [String: String] = [:]
    private var autoConnected: Set<String> = []

    var orderedConnections: [ConnectionInfo] {
        var result: [ConnectionInfo] = []
        func walk(_ nodes: [Node]) {
            for node in nodes {
                switch node {
                case .connection(let id): if let c = connections[id] { result.append(c) }
                case .folder(let f): walk(f.children)
                }
            }
        }
        walk(tree)
        let seen = Set(result.map(\.id))
        result += connections.values.filter { !seen.contains($0.id) }.sorted { $0.name < $1.name }
        return result
    }

    private var knownSessions: [SessionInfo] {
        sessions.values.filter { connections[$0.id] != nil }
    }

    var activeSessions: [SessionInfo] {
        knownSessions.filter { $0.state.isActive }.sorted { $0.name < $1.name }
    }

    var overallState: SessionState {
        let states = knownSessions.map(\.state)
        if states.contains(.connected) { return .connected }
        if states.contains(where: { $0.isActive }) { return .connecting }
        return .disconnected
    }

    func start() {
        reload()
        preferences = (try? Core.call("preferences.load")) ?? Preferences()
        daemon.onEvent = { [weak self] event in
            Task { @MainActor in self?.handle(event) }
        }
        daemon.onConnectionChange = { [weak self] up in
            Task { @MainActor in await self?.daemonChanged(up) }
        }
        daemon.start()
        observeSystem()
        scheduleIdleCheck()
        WiFiWatcher.shared.onChange = { [weak self] ssid in self?.wifiChanged(ssid) }
        WiFiWatcher.shared.start()
    }

    func reload() {
        do {
            tree = try Core.call("store.tree")
            let list: [ConnectionInfo] = try Core.call("store.connections")
            connections = Dictionary(uniqueKeysWithValues: list.map { ($0.id, $0) })
        } catch {
            lastError = error.localizedDescription
        }
    }

    func savePreferences() {
        do {
            try Core.run("preferences.save", preferences)
        } catch {
            lastError = error.localizedDescription
        }
        scheduleIdleCheck()
    }

    func setTree(_ nodes: [Node]) {
        do {
            try Core.run("store.set_tree", TreeParam(tree: nodes))
        } catch {
            lastError = error.localizedDescription
        }
        reload()
    }

    private func daemonChanged(_ up: Bool) async {
        daemonAvailable = up
        guard up else { return }
        if let list = try? await daemon.sessions() {
            sessions = Dictionary(uniqueKeysWithValues: list.map { ($0.id, $0) })
            for s in list { previousStates[s.id] = s.state }
        }
        blocked = (try? await daemon.blocked()) ?? []
        let ready = readyActions
        readyActions = []
        for action in ready { action(self) }
        if !didLaunchConnect {
            didLaunchConnect = true
            for c in orderedConnections where c.connectOnLaunch && sessions[c.id] == nil {
                connect(c.id)
            }
            wifiChanged(WiFiWatcher.shared.ssid)
        }
    }

    private var didLaunchConnect = false
    private var readyActions: [(ConnectionManager) -> Void] = []

    func whenReady(_ action: @escaping (ConnectionManager) -> Void) {
        if daemonAvailable && didLaunchConnect { action(self) } else { readyActions.append(action) }
    }

    func state(of id: String) -> SessionState {
        sessions[id]?.state ?? .disconnected
    }

    func isActive(_ id: String) -> Bool {
        state(of: id).isActive
    }

    func toggle(_ id: String) {
        if isActive(id) { disconnect(id) } else { connect(id) }
    }

    func connect(_ id: String) {
        guard let connection = connections[id] else { return }
        autoConnected.remove(id)
        userInitiatedDisconnect.remove(id)
        rememberLastUsed(id)
        Task { await performConnect(connection) }
    }

    private func performConnect(_ connection: ConnectionInfo) async {
        if let script = connection.beforeConnectScript {
            let result = await Task.detached { ScriptRunner.beforeConnect(path: script, connection: connection) }.value
            if !result.allowConnect {
                appendLocalLog(connection.id, "warning", "Before-connect script cancelled the connection")
                return
            }
            if let username = result.username { usernames[connection.id] = username }
            if result.username != nil || result.password != nil {
                try? Core.run(
                    "credentials.save",
                    CredentialSave(scope: "script:\(connection.id)", username: result.username, password: result.password))
            }
            if let response = result.challengeResponse { pendingChallenge[connection.id] = response }
        }
        do {
            let bundle: WireBundle = try Core.call("store.bundle", IdParam(id: connection.id))
            let options = ConnectOptions(
                openvpnVersion: connection.openvpnVersion ?? preferences.openvpnVersion,
                autoReconnect: connection.autoReconnect,
                blockIpv6: preferences.blockIpv6,
                disableTimeMachine: preferences.disableTimeMachine,
                killSwitch: preferences.killSwitch,
                killSwitchAllowLan: preferences.killSwitchAllowLan)
            try await daemon.connect(id: connection.id, bundle: bundle, options: options)
        } catch {
            report(error, connection: connection)
        }
    }

    private func wifiChanged(_ ssid: String?) {
        guard let ssid, daemonAvailable, let id = preferences.autoConnect, let connection = connections[id] else { return }
        if preferences.trustedNetworks.contains(ssid) {
            if preferences.disconnectOnTrusted && isActive(id) && autoConnected.contains(id) {
                appendLocalLog(id, "info", "Disconnecting on trusted network \(ssid)")
                disconnect(id)
            }
        } else if !isActive(id) {
            appendLocalLog(id, "info", "Connecting on untrusted network \(ssid)")
            notify(title: connection.name, body: "Connecting on untrusted Wi-Fi network \(ssid)")
            connect(id)
            autoConnected.insert(id)
        }
    }

    func disconnect(_ id: String) {
        autoConnected.remove(id)
        userInitiatedDisconnect.insert(id)
        wantedBeforeSleep.remove(id)
        wantedBeforeNetworkLoss.remove(id)
        prompts.dismiss(id)
        Task {
            do { try await daemon.disconnect(id: id) } catch { lastError = error.localizedDescription }
        }
    }

    func unblock(_ id: String? = nil) {
        Task {
            do { try await daemon.unblock(id: id) } catch { lastError = error.localizedDescription }
        }
    }

    func connectAll() {
        for c in orderedConnections where !isActive(c.id) { connect(c.id) }
    }

    func disconnectAll() {
        for s in activeSessions { userInitiatedDisconnect.insert(s.id) }
        prompts.dismissAll()
        Task {
            do { try await daemon.disconnectAll() } catch { lastError = error.localizedDescription }
        }
    }

    func loadLog(_ id: String) async {
        if let entries = try? await daemon.log(id) {
            logs[id] = entries
        }
    }

    func clearLog(_ id: String) {
        logs[id] = []
    }

    private func report(_ error: Error, connection: ConnectionInfo) {
        let message = error.localizedDescription
        appendLocalLog(connection.id, "error", message)
        lastError = message
        notify(title: connection.name, body: message)
        if daemonAvailable {
            showFailure(message, connection: connection)
        } else {
            AppDelegate.shared?.offerHelperInstall()
        }
    }

    private func showFailure(_ message: String, connection: ConnectionInfo) {
        guard failureAlerts.insert(connection.id).inserted else { return }
        DispatchQueue.main.async { self.presentFailure(message, connection: connection) }
    }

    private func presentFailure(_ message: String, connection: ConnectionInfo) {
        let alert = NSAlert()
        alert.alertStyle = .warning
        alert.messageText = "Cannot connect to \(connection.name)"
        alert.informativeText = message
        alert.addButton(withTitle: "OK")
        alert.addButton(withTitle: "Edit Connection")
        alert.addButton(withTitle: "Show Log")
        NSApp.activate(ignoringOtherApps: true)
        let response = alert.runModal()
        failureAlerts.remove(connection.id)
        switch response {
        case .alertSecondButtonReturn: WindowManager.shared.showEditor(id: connection.id)
        case .alertThirdButtonReturn: WindowManager.shared.showDetails(connection.id)
        default: break
        }
    }

    private func appendLocalLog(_ id: String, _ level: String, _ message: String) {
        let entry = LogEntry(time: UInt64(Date().timeIntervalSince1970), level: level, message: message)
        logs[id, default: []].append(entry)
    }

    private func handle(_ event: DaemonEvent) {
        switch event {
        case .session(let info):
            update(info)
        case .removed(let id):
            if let old = sessions[id] { transition(old, to: .disconnected) }
            sessions[id] = nil
            previousStates[id] = nil
            traffic[id] = nil
            prompts.dismiss(id)
        case .prompt(let id, let prompt):
            answer(id, prompt)
        case .promptDone(let id):
            prompts.dismiss(id)
        case .blocked(let ids):
            if ids.count > blocked.count, let id = ids.first(where: { !blocked.contains($0) }),
               let connection = connections[id] {
                notify(title: connection.name, body: "Connection lost. The kill switch is blocking internet traffic.")
            }
            blocked = ids
        case .log(let id, let entry):
            var list = logs[id, default: []]
            list.append(entry)
            if list.count > 5000 { list.removeFirst(list.count - 5000) }
            logs[id] = list
        }
    }

    private func update(_ info: SessionInfo) {
        let old = sessions[info.id]
        sessions[info.id] = info
        if let old, old.state == .connected, info.state == .connected {
            let now = Date()
            var history = traffic[info.id, default: []]
            let elapsed = history.last.map { now.timeIntervalSince($0.time) } ?? 1
            let dt = max(elapsed, 0.2)
            let inRate = Double(info.bytesIn &- old.bytesIn) / dt
            let outRate = Double(info.bytesOut &- old.bytesOut) / dt
            history.append(TrafficSample(time: now, inRate: max(inRate, 0), outRate: max(outRate, 0)))
            if history.count > Self.historyLength { history.removeFirst(history.count - Self.historyLength) }
            traffic[info.id] = history
        }
        let previous = previousStates[info.id] ?? .disconnected
        previousStates[info.id] = info.state
        if previous != info.state, let old {
            transition(old, to: info.state, current: info)
        } else if previous != info.state {
            transition(SessionInfo.placeholder(info), to: info.state, current: info)
        }
    }

    private func transition(_ old: SessionInfo, to state: SessionState, current: SessionInfo? = nil) {
        guard let connection = connections[old.id] else { return }
        let info = current ?? old
        switch state {
        case .connected where old.state != .connected:
            notify(title: connection.name, body: "Connected" + (info.localIpv4.isEmpty ? "" : " (\(info.localIpv4))"))
            runScript(connection.connectedScript, connection: connection, session: info)
        case .disconnected, .error:
            if old.state == .connected {
                if userInitiatedDisconnect.contains(old.id) || state == .error {
                    notify(title: connection.name, body: info.lastError ?? "Disconnected")
                }
                runScript(connection.disconnectedScript, connection: connection, session: old)
            } else if state == .error, let error = info.lastError {
                notify(title: connection.name, body: error)
                showFailure(error, connection: connection)
            }
            userInitiatedDisconnect.remove(old.id)
        case .reconnecting where old.state == .connected:
            notify(title: connection.name, body: "Connection lost, reconnecting")
        default:
            break
        }
    }

    private func runScript(_ path: String?, connection: ConnectionInfo, session: SessionInfo) {
        guard let path else { return }
        let username = usernames[connection.id]
        Task.detached {
            let env = ScriptRunner.environment(connection: connection, session: session, username: username)
            _ = ScriptRunner.run(path: path, environment: env)
        }
    }

    func notify(title: String, body: String) {
        guard preferences.notifications, Bundle.main.bundleIdentifier != nil else { return }
        let content = UNMutableNotificationContent()
        content.title = title
        content.body = body
        let request = UNNotificationRequest(identifier: UUID().uuidString, content: content, trigger: nil)
        UNUserNotificationCenter.current().add(request)
    }

    private func credentialScope(_ connection: ConnectionInfo, realm: String) -> String {
        realm == "Auth" ? connection.credentialScope : "\(connection.credentialScope):\(secretField(realm))"
    }

    private func savedCredentials(_ connection: ConnectionInfo, realm: String) -> SavedCredentials {
        if realm == "Auth" {
            let scripted: SavedCredentials? = try? Core.call("credentials.load", ScopeParam(scope: "script:\(connection.id)"))
            if let scripted, scripted.username != nil || scripted.password != nil {
                try? Core.run("credentials.clear", ScopeParam(scope: "script:\(connection.id)"))
                return scripted
            }
        }
        return (try? Core.call("credentials.load", ScopeParam(scope: credentialScope(connection, realm: realm)))) ?? SavedCredentials()
    }

    private func useIdentity(_ id: String, _ prompt: Prompt) -> Bool {
        let work: @Sendable () throws -> Reply
        switch prompt {
        case .certificate(let identity):
            work = { .certificate(pem: try Core.call("identity.certificate", IdentityParam(identity: identity))) }
        case .sign(let identity, let data, let algorithm):
            work = { .signature(try Core.call("identity.sign", SignParam(identity: identity, data: data, algorithm: algorithm))) }
        default:
            return false
        }
        Task { [weak self] in
            let result = await Task.detached { Result { try work() } }.value
            self?.finishIdentity(id, result)
        }
        return true
    }

    private func finishIdentity(_ id: String, _ result: Result<Reply, Error>) {
        switch result {
        case .success(let reply): send(id, reply)
        case .failure(let error):
            if let connection = connections[id] { report(error, connection: connection) }
            send(id, .cancel)
        }
    }

    private func answer(_ id: String, _ prompt: Prompt) {
        if useIdentity(id, prompt) { return }
        guard let connection = connections[id] else {
            prompts.present(id: id, title: sessions[id]?.name ?? id, prompt: prompt, saved: SavedCredentials()) { [weak self] reply, _ in
                self?.send(id, reply)
            }
            return
        }
        if case .credentials(let realm, let challenge, let retry) = prompt, !retry {
            let saved = savedCredentials(connection, realm: realm)
            if let username = saved.username, let password = saved.password, !password.isEmpty {
                let response: String?
                if let challenge {
                    if let scripted = pendingChallenge.removeValue(forKey: id) {
                        response = scripted
                    } else if let ack = connection.staticChallengeAcknowledge {
                        response = ack
                    } else {
                        response = nil
                    }
                    if response == nil {
                        presentChallengeOnly(id, connection: connection, realm: realm, username: username, password: password, challenge: challenge)
                        return
                    }
                } else {
                    response = nil
                }
                usernames[id] = username
                send(id, .credentials(realm: realm, username: username, password: password, challengeResponse: response))
                return
            }
        }
        if case .password(let realm) = prompt {
            let stored: String? = try? Core.call("credentials.secret", SecretParam(scope: connection.credentialScope, field: secretField(realm)))
            if let password = stored, !password.isEmpty {
                send(id, .password(realm: realm, password: password))
                return
            }
        }
        var saved = SavedCredentials()
        var splitter: StaticChallengePassword?
        if case .credentials(let realm, _, _) = prompt {
            saved = savedCredentials(connection, realm: realm)
            if realm == "Auth" { splitter = connection.staticChallengePassword }
        }
        prompts.present(id: id, title: connection.name, prompt: prompt, saved: saved, splitter: splitter) { [weak self] reply, remember in
            guard let self else { return }
            if case .credentials(let realm, let username, let password, _) = reply {
                if realm == "Auth" { self.usernames[id] = username }
                if remember {
                    try? Core.run("credentials.save", CredentialSave(scope: self.credentialScope(connection, realm: realm), username: username, password: password))
                }
            }
            if case .password(let realm, let password) = reply, remember, case .password = prompt {
                try? Core.run("credentials.save_secret", SecretSave(scope: connection.credentialScope, field: self.secretField(realm), value: password))
            }
            self.send(id, reply)
        }
    }

    private func secretField(_ realm: String) -> String {
        realm.lowercased().replacingOccurrences(of: " ", with: "-")
    }

    private func presentChallengeOnly(_ id: String, connection: ConnectionInfo, realm: String, username: String, password: String, challenge: StaticChallengePrompt) {
        prompts.present(
            id: id, title: connection.name,
            prompt: .challenge(realm: realm, text: challenge.text, echo: challenge.echo),
            saved: SavedCredentials()
        ) { [weak self] reply, _ in
            guard let self else { return }
            switch reply {
            case .password(_, let response):
                self.usernames[id] = username
                self.send(id, .credentials(realm: realm, username: username, password: password, challengeResponse: response))
            default:
                self.send(id, reply)
            }
        }
    }

    private func send(_ id: String, _ reply: Reply) {
        if case .cancel = reply { userInitiatedDisconnect.insert(id) }
        Task {
            do { try await daemon.reply(id: id, reply: reply) } catch { lastError = error.localizedDescription }
        }
    }

    private func observeSystem() {
        let center = NSWorkspace.shared.notificationCenter
        center.addObserver(forName: NSWorkspace.willSleepNotification, object: nil, queue: .main) { [weak self] _ in
            Task { @MainActor in self?.willSleep() }
        }
        center.addObserver(forName: NSWorkspace.didWakeNotification, object: nil, queue: .main) { [weak self] _ in
            Task { @MainActor in self?.didWake() }
        }
        let monitor = NWPathMonitor()
        monitor.pathUpdateHandler = { [weak self] path in
            let ok = path.status == .satisfied
            Task { @MainActor in self?.pathChanged(ok) }
        }
        monitor.start(queue: DispatchQueue(label: "fluidity.path"))
        pathMonitor = monitor
    }

    private func willSleep() {
        guard preferences.reconnectOnWake else { return }
        wantedBeforeSleep = Set(activeSessions.map(\.id))
    }

    private func didWake() {
        let wanted = wantedBeforeSleep
        wantedBeforeSleep = []
        guard preferences.reconnectOnWake else { return }
        DispatchQueue.main.asyncAfter(deadline: .now() + 3) { [weak self] in
            guard let self else { return }
            for id in wanted where !self.isActive(id) { self.connect(id) }
        }
    }

    private func pathChanged(_ satisfied: Bool) {
        defer { lastPathSatisfied = satisfied }
        guard preferences.reconnectOnNetwork else { return }
        if !satisfied && lastPathSatisfied {
            wantedBeforeNetworkLoss = Set(activeSessions.map(\.id))
        } else if satisfied && !lastPathSatisfied {
            let wanted = wantedBeforeNetworkLoss
            wantedBeforeNetworkLoss = []
            for id in wanted where !isActive(id) { connect(id) }
        }
    }

    private func scheduleIdleCheck() {
        idleTimer?.invalidate()
        guard let minutes = preferences.idleDisconnectMinutes, minutes > 0 else { return }
        idleTimer = Timer.scheduledTimer(withTimeInterval: 30, repeats: true) { [weak self] _ in
            Task { @MainActor in self?.checkIdle(minutes: minutes) }
        }
    }

    private func checkIdle(minutes: UInt32) {
        let limit = Double(minutes) * 60
        let now = Date()
        for s in activeSessions where s.state == .connected {
            guard let samples = traffic[s.id], let since = s.connectedSince else { continue }
            if now.timeIntervalSince1970 - Double(since) < limit { continue }
            let recent = samples.filter { now.timeIntervalSince($0.time) < limit }
            let busy = recent.contains { $0.inRate > 64 || $0.outRate > 64 }
            if !busy {
                appendLocalLog(s.id, "info", "Disconnecting after \(minutes) idle minutes")
                disconnect(s.id)
            }
        }
    }

    func importFiles(_ urls: [URL]) {
        var imported: [String] = []
        for url in urls {
            do {
                let c: ConnectionInfo = try Core.call("store.import", ImportParam(path: url.path, replace: false))
                imported.append(c.name)
            } catch {
                let alert = NSAlert()
                alert.messageText = "Cannot import \(url.lastPathComponent)"
                alert.informativeText = error.localizedDescription
                alert.runModal()
            }
        }
        reload()
        if !imported.isEmpty {
            notify(title: "Imported", body: imported.joined(separator: ", "))
        }
    }

    func export(_ id: String, to url: URL) throws {
        try Core.run("store.export", ExportParam(id: id, path: url.path))
    }

    func remove(_ id: String) {
        if isActive(id) { disconnect(id) }
        do { try Core.run("store.remove", IdParam(id: id)) } catch { lastError = error.localizedDescription }
        reload()
    }

    func duplicate(_ id: String) {
        do {
            let _: ConnectionInfo = try Core.call("store.duplicate", IdParam(id: id))
        } catch {
            lastError = error.localizedDescription
        }
        reload()
    }
}

extension SessionInfo {
    static func placeholder(_ info: SessionInfo) -> SessionInfo {
        var s = info
        s.state = .disconnected
        return s
    }
}
