import AppKit
import Carbon.HIToolbox

extension ConnectionManager {
    func connection(named name: String) -> ConnectionInfo? {
        let all = orderedConnections
        return all.first { $0.id == name } ?? all.first { $0.name == name }
            ?? all.first { $0.name.localizedCaseInsensitiveCompare(name) == .orderedSame }
    }

    var lastUsedConnection: ConnectionInfo? {
        preferences.lastConnection.flatMap { connections[$0] } ?? orderedConnections.first
    }

    func toggleLastUsed() {
        if !activeSessions.isEmpty {
            disconnectAll()
        } else if let c = lastUsedConnection {
            connect(c.id)
        }
    }

    func rememberLastUsed(_ id: String) {
        guard preferences.lastConnection != id else { return }
        preferences.lastConnection = id
        savePreferences()
    }

    @discardableResult
    func handle(url: URL) -> Bool {
        guard url.scheme?.lowercased() == "fluidity", let action = url.host?.lowercased() else { return false }
        let name = url.path.trimmingCharacters(in: CharacterSet(charactersIn: "/"))
        let target = name.isEmpty ? nil : connection(named: name)
        if !name.isEmpty && target == nil {
            lastError = "No connection is named “\(name)”."
            return true
        }
        switch action {
        case "connect":
            if let c = target ?? lastUsedConnection, !isActive(c.id) { connect(c.id) }
        case "disconnect":
            if let c = target { disconnect(c.id) } else { disconnectAll() }
        case "toggle":
            if let c = target { toggle(c.id) } else { toggleLastUsed() }
        default:
            return false
        }
        return true
    }
}

struct Shortcut: Identifiable, Hashable {
    let id: String
    let title: String
    let keyCode: UInt32
    let modifiers: UInt32

    static let all = [
        Shortcut(id: "ctrl+alt+cmd+v", title: "⌃⌥⌘V", keyCode: UInt32(kVK_ANSI_V), modifiers: UInt32(controlKey | optionKey | cmdKey)),
        Shortcut(id: "ctrl+alt+v", title: "⌃⌥V", keyCode: UInt32(kVK_ANSI_V), modifiers: UInt32(controlKey | optionKey)),
        Shortcut(id: "ctrl+shift+cmd+v", title: "⌃⇧⌘V", keyCode: UInt32(kVK_ANSI_V), modifiers: UInt32(controlKey | shiftKey | cmdKey)),
        Shortcut(id: "alt+shift+cmd+v", title: "⌥⇧⌘V", keyCode: UInt32(kVK_ANSI_V), modifiers: UInt32(optionKey | shiftKey | cmdKey)),
    ]
}

@MainActor
final class HotKey {
    static let shared = HotKey()
    private var ref: EventHotKeyRef?
    private var handler: EventHandlerRef?
    private var current: String?
    var action: (() -> Void)?

    func register(_ id: String?) {
        guard id != current else { return }
        current = id
        if let ref { UnregisterEventHotKey(ref) }
        ref = nil
        guard let shortcut = Shortcut.all.first(where: { $0.id == id }) else { return }
        if handler == nil {
            var spec = EventTypeSpec(eventClass: OSType(kEventClassKeyboard), eventKind: UInt32(kEventHotKeyPressed))
            InstallEventHandler(GetApplicationEventTarget(), { _, _, _ in
                DispatchQueue.main.async { HotKey.shared.action?() }
                return noErr
            }, 1, &spec, nil, &handler)
        }
        let hotKeyID = EventHotKeyID(signature: OSType(0x464C_5644), id: 1)
        RegisterEventHotKey(shortcut.keyCode, shortcut.modifiers, hotKeyID, GetApplicationEventTarget(), 0, &ref)
    }
}
