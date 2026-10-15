import AppKit

@objc(FluidityScriptConnection)
final class ScriptConnection: NSObject {
    @objc let uniqueID: String
    @objc let name: String
    @objc let state: String
    @objc let ipv4Address: String
    @objc let ipv6Address: String
    @objc let serverAddress: String
    @objc let bytesIn: Double
    @objc let bytesOut: Double

    init(connection: ConnectionInfo, session: SessionInfo?) {
        uniqueID = connection.id
        name = connection.name
        state = session?.state.label ?? SessionState.disconnected.label
        ipv4Address = session?.localIpv4 ?? ""
        ipv6Address = session?.localIpv6 ?? ""
        serverAddress = session?.remoteAddress ?? ""
        bytesIn = Double(session?.bytesIn ?? 0)
        bytesOut = Double(session?.bytesOut ?? 0)
    }

    override var objectSpecifier: NSScriptObjectSpecifier? {
        guard let app = NSApp, let description = app.classDescription as? NSScriptClassDescription else { return nil }
        return NSUniqueIDSpecifier(containerClassDescription: description, containerSpecifier: nil, key: "scriptConnections", uniqueID: uniqueID)
    }
}

extension AppDelegate {
    func application(_ sender: NSApplication, delegateHandlesKey key: String) -> Bool {
        key == "scriptConnections"
    }

    @objc var scriptConnections: [ScriptConnection] {
        let manager = ConnectionManager.shared
        return manager.orderedConnections.map { ScriptConnection(connection: $0, session: manager.sessions[$0.id]) }
    }
}

@MainActor
private func resolve(_ command: NSScriptCommand) -> [String]? {
    let manager = ConnectionManager.shared
    if let specifier = command.directParameter as? NSScriptObjectSpecifier,
       let objects = specifier.objectsByEvaluatingSpecifier {
        let list = (objects as? [ScriptConnection]) ?? [(objects as? ScriptConnection)].compactMap { $0 }
        return list.map(\.uniqueID)
    }
    if let list = command.evaluatedArguments?[""] as? [ScriptConnection] {
        return list.map(\.uniqueID)
    }
    if let single = command.evaluatedArguments?[""] as? ScriptConnection {
        return [single.uniqueID]
    }
    let name = (command.directParameter as? String) ?? (command.evaluatedArguments?[""] as? String)
    guard let name else { return nil }
    let matches = manager.orderedConnections.filter { $0.name == name || $0.id == name }
    if matches.isEmpty {
        command.scriptErrorNumber = errAENoSuchObject
        command.scriptErrorString = "No connection named \"\(name)\"."
        return nil
    }
    return matches.map(\.id)
}

@objc(FluidityConnectCommand)
final class ConnectCommand: NSScriptCommand {
    override func performDefaultImplementation() -> Any? {
        MainActor.assumeIsolated {
            for id in resolve(self) ?? [] where !ConnectionManager.shared.isActive(id) {
                ConnectionManager.shared.connect(id)
            }
        }
        return nil
    }
}

@objc(FluidityDisconnectCommand)
final class DisconnectCommand: NSScriptCommand {
    override func performDefaultImplementation() -> Any? {
        MainActor.assumeIsolated {
            for id in resolve(self) ?? [] where ConnectionManager.shared.isActive(id) {
                ConnectionManager.shared.disconnect(id)
            }
        }
        return nil
    }
}

@objc(FluidityConnectAllCommand)
final class ConnectAllCommand: NSScriptCommand {
    override func performDefaultImplementation() -> Any? {
        MainActor.assumeIsolated { ConnectionManager.shared.connectAll() }
        return nil
    }
}

@objc(FluidityDisconnectAllCommand)
final class DisconnectAllCommand: NSScriptCommand {
    override func performDefaultImplementation() -> Any? {
        MainActor.assumeIsolated { ConnectionManager.shared.disconnectAll() }
        return nil
    }
}
