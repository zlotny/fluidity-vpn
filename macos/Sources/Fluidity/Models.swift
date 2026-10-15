import Foundation

struct StaticChallengePassword: Codable, Hashable {
    var text: String
    var delimiter: String
}

struct ConnectionInfo: Codable, Identifiable, Hashable {
    var id: String
    var name: String
    var folder: String?
    var credentialScope: String
    var autoReconnect: Bool
    var connectOnLaunch: Bool
    var beforeConnectScript: String?
    var connectedScript: String?
    var disconnectedScript: String?
    var staticChallengePassword: StaticChallengePassword?
    var staticChallengeAcknowledge: String?
    var systemIdentity: String?
    var openvpnVersion: String?
}

struct Folder: Codable, Hashable {
    var id: String
    var name: String
    var children: [Node]
    var sharedCredentials: Bool
    var beforeConnectScript: String?
    var connectedScript: String?
    var disconnectedScript: String?
}

indirect enum Node: Codable, Hashable {
    case connection(String)
    case folder(Folder)

    private enum Keys: String, CodingKey { case connection, folder }

    init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: Keys.self)
        if let id = try c.decodeIfPresent(String.self, forKey: .connection) {
            self = .connection(id)
        } else {
            self = .folder(try c.decode(Folder.self, forKey: .folder))
        }
    }

    func encode(to encoder: Encoder) throws {
        var c = encoder.container(keyedBy: Keys.self)
        switch self {
        case .connection(let id): try c.encode(id, forKey: .connection)
        case .folder(let f): try c.encode(f, forKey: .folder)
        }
    }
}

enum SessionState: String, Codable {
    case disconnected, connecting, authenticating, connected, reconnecting, disconnecting, error

    var isActive: Bool { self != .disconnected && self != .error }

    var label: String {
        switch self {
        case .disconnected: return "Disconnected"
        case .connecting: return "Connecting"
        case .authenticating: return "Authenticating"
        case .connected: return "Connected"
        case .reconnecting: return "Reconnecting"
        case .disconnecting: return "Disconnecting"
        case .error: return "Error"
        }
    }
}

struct SessionInfo: Codable, Equatable {
    var id: String
    var name: String
    var ownerUid: UInt32
    var state: SessionState
    var detail: String
    var localIpv4: String
    var localIpv6: String
    var remoteAddress: String
    var remotePort: String
    var interface: String
    var bytesIn: UInt64
    var bytesOut: UInt64
    var connectedSince: UInt64?
    var lastError: String?
    var killSwitch: Bool?
}

struct ChoiceOption: Codable, Equatable, Hashable {
    var value: String
    var label: String
}

struct StaticChallengePrompt: Codable, Equatable {
    var text: String
    var echo: Bool
}

enum Prompt: Codable, Equatable {
    case credentials(realm: String, staticChallenge: StaticChallengePrompt?, retry: Bool)
    case password(realm: String)
    case challenge(realm: String, text: String, echo: Bool)
    case needOk(name: String, message: String)
    case needStr(name: String, message: String)
    case webAuth(url: String)
    case choice(name: String, message: String, options: [ChoiceOption])
    case certificate(identity: String)
    case sign(identity: String, data: String, algorithm: String)

    private enum Keys: String, CodingKey { case type, realm, staticChallenge, retry, text, echo, name, message, url, options, identity, data, algorithm }

    init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: Keys.self)
        switch try c.decode(String.self, forKey: .type) {
        case "credentials":
            self = .credentials(
                realm: try c.decode(String.self, forKey: .realm),
                staticChallenge: try c.decodeIfPresent(StaticChallengePrompt.self, forKey: .staticChallenge),
                retry: try c.decodeIfPresent(Bool.self, forKey: .retry) ?? false)
        case "password": self = .password(realm: try c.decode(String.self, forKey: .realm))
        case "challenge":
            self = .challenge(
                realm: try c.decode(String.self, forKey: .realm),
                text: try c.decode(String.self, forKey: .text),
                echo: try c.decode(Bool.self, forKey: .echo))
        case "need_ok":
            self = .needOk(name: try c.decode(String.self, forKey: .name), message: try c.decode(String.self, forKey: .message))
        case "need_str":
            self = .needStr(name: try c.decode(String.self, forKey: .name), message: try c.decode(String.self, forKey: .message))
        case "choice":
            self = .choice(
                name: try c.decode(String.self, forKey: .name), message: try c.decode(String.self, forKey: .message),
                options: try c.decode([ChoiceOption].self, forKey: .options))
        case "certificate": self = .certificate(identity: try c.decode(String.self, forKey: .identity))
        case "sign":
            self = .sign(
                identity: try c.decode(String.self, forKey: .identity), data: try c.decode(String.self, forKey: .data),
                algorithm: try c.decode(String.self, forKey: .algorithm))
        default: self = .webAuth(url: try c.decode(String.self, forKey: .url))
        }
    }

    func encode(to encoder: Encoder) throws {
        var c = encoder.container(keyedBy: Keys.self)
        switch self {
        case .credentials(let realm, let sc, let retry):
            try c.encode("credentials", forKey: .type)
            try c.encode(realm, forKey: .realm)
            try c.encodeIfPresent(sc, forKey: .staticChallenge)
            try c.encode(retry, forKey: .retry)
        case .password(let realm):
            try c.encode("password", forKey: .type)
            try c.encode(realm, forKey: .realm)
        case .challenge(let realm, let text, let echo):
            try c.encode("challenge", forKey: .type)
            try c.encode(realm, forKey: .realm)
            try c.encode(text, forKey: .text)
            try c.encode(echo, forKey: .echo)
        case .needOk(let name, let message):
            try c.encode("need_ok", forKey: .type)
            try c.encode(name, forKey: .name)
            try c.encode(message, forKey: .message)
        case .needStr(let name, let message):
            try c.encode("need_str", forKey: .type)
            try c.encode(name, forKey: .name)
            try c.encode(message, forKey: .message)
        case .webAuth(let url):
            try c.encode("web_auth", forKey: .type)
            try c.encode(url, forKey: .url)
        case .choice(let name, let message, let options):
            try c.encode("choice", forKey: .type)
            try c.encode(name, forKey: .name)
            try c.encode(message, forKey: .message)
            try c.encode(options, forKey: .options)
        case .certificate(let identity):
            try c.encode("certificate", forKey: .type)
            try c.encode(identity, forKey: .identity)
        case .sign(let identity, let data, let algorithm):
            try c.encode("sign", forKey: .type)
            try c.encode(identity, forKey: .identity)
            try c.encode(data, forKey: .data)
            try c.encode(algorithm, forKey: .algorithm)
        }
    }
}

enum Reply: Encodable, Sendable {
    case credentials(realm: String, username: String, password: String, challengeResponse: String?)
    case password(realm: String, password: String)
    case needOk(name: String, ok: Bool)
    case needStr(name: String, value: String)
    case certificate(pem: String)
    case signature(String)
    case cancel

    private enum Keys: String, CodingKey { case type, realm, username, password, challengeResponse, name, ok, value, pem, signature }

    func encode(to encoder: Encoder) throws {
        var c = encoder.container(keyedBy: Keys.self)
        switch self {
        case .credentials(let realm, let username, let password, let response):
            try c.encode("credentials", forKey: .type)
            try c.encode(realm, forKey: .realm)
            try c.encode(username, forKey: .username)
            try c.encode(password, forKey: .password)
            try c.encodeIfPresent(response, forKey: .challengeResponse)
        case .password(let realm, let password):
            try c.encode("password", forKey: .type)
            try c.encode(realm, forKey: .realm)
            try c.encode(password, forKey: .password)
        case .needOk(let name, let ok):
            try c.encode("need_ok", forKey: .type)
            try c.encode(name, forKey: .name)
            try c.encode(ok, forKey: .ok)
        case .needStr(let name, let value):
            try c.encode("need_str", forKey: .type)
            try c.encode(name, forKey: .name)
            try c.encode(value, forKey: .value)
        case .certificate(let pem):
            try c.encode("certificate", forKey: .type)
            try c.encode(pem, forKey: .pem)
        case .signature(let signature):
            try c.encode("signature", forKey: .type)
            try c.encode(signature, forKey: .signature)
        case .cancel:
            try c.encode("cancel", forKey: .type)
        }
    }
}

struct LogEntry: Codable, Hashable {
    var time: UInt64
    var level: String
    var message: String
}

struct WireBundle: Codable {
    var config: String
    var files: [String: String]
}

struct ConnectOptions: Codable {
    var openvpnVersion: String?
    var autoReconnect: Bool
    var blockIpv6: Bool
    var disableTimeMachine: Bool
    var killSwitch: Bool
    var killSwitchAllowLan: Bool
}

struct SystemSettings: Codable {
    var allowUnsafeCommands: Bool
}

struct Preferences: Codable, Equatable {
    var firstRun = true
    var startAtLogin = false
    var disableTimeMachine = false
    var reconnectOnWake = true
    var reconnectOnNetwork = true
    var idleDisconnectMinutes: UInt32?
    var checkForUpdates = true
    var includeBetaUpdates = false
    var notifications = true
    var iconSet = "default"
    var addressDisplay = "ipv4"
    var trafficDisplay = "rates"
    var graphStyle = "line"
    var detailsAlwaysOnTop = false
    var resetInterfacesOnDisconnect = false
    var blockIpv6 = true
    var killSwitch = false
    var killSwitchAllowLan = true
    var openvpnVersion: String?
}

struct FileField: Codable, Equatable {
    var name: String = ""
    var data: String?

    var isSet: Bool { !name.isEmpty || data != nil }
    var displayName: String {
        if !name.isEmpty { return name }
        return data == nil ? "None" : "Inline"
    }
}

struct Remote: Codable, Equatable, Identifiable {
    var id = UUID()
    var host: String = ""
    var port: UInt16 = 1194
    var transport: String = "udp"
    var ipVersion: String = "automatic"

    private enum CodingKeys: String, CodingKey { case host, port, transport, ipVersion }
}

struct Route: Codable, Equatable, Identifiable {
    var id = UUID()
    var network: String = ""
    var mask: String = ""
    var gateway: String = ""
    var metric: String = ""
    var ipv6: Bool = false

    private enum CodingKeys: String, CodingKey { case network, mask, gateway, metric, ipv6 }
}

struct EditorModel: Codable, Equatable {
    var name = ""
    var remotes: [Remote] = []
    var remoteRandom = false
    var device = "tun"
    var dhcp = false
    var ipv6 = true
    var autoReconnect = true
    var connectOnLaunch = false

    var authType = "tls"
    var userPass = false
    var ca = FileField()
    var cert = FileField()
    var key = FileField()
    var pkcs12 = FileField()
    var staticKey = FileField()
    var staticKeyDirection: UInt8?
    var pkcs11Providers = ""
    var pkcs11Id = ""
    var pkcs11Prompt = false
    var systemIdentity = ""
    var extraType = "none"
    var extraKey = FileField()
    var extraDirection: UInt8?

    var ping: UInt32?
    var pingRestart: UInt32?
    var persistTun = false
    var persistKey = false
    var persistLocalIp = false
    var persistRemoteIp = false
    var remoteCertTls = false
    var compression = "off"
    var nobind = false
    var pull = true
    var compatibility = ""

    var redirect = "automatic"
    var routes: [Route] = []
    var defaultGateway = ""
    var dnsMode = "automatic"
    var dnsServers: [String] = []
    var dnsDomains: [String] = []
    var ignorePushedDns = false
    var shaper: UInt32?
    var fragment: UInt32?
    var tunMtu: UInt32?
    var inactive: UInt32?
    var serverTimeout: UInt32?

    var proxyType = "none"
    var proxyHost = ""
    var proxyPort: UInt16?
    var proxyAuth = "none"
    var obfuscation = ""
    var obfuscationKey = ""

    var beforeConnectScript = ""
    var connectedScript = ""
    var disconnectedScript = ""
    var extraCommands = ""
}

struct EditorSave: Encodable {
    var id: String?
    var model: EditorModel
}

struct IdParam: Encodable { var id: String }
struct ScopeParam: Encodable { var scope: String? }
struct ImportParam: Encodable { var path: String; var replace: Bool }
struct ExportParam: Encodable { var id: String; var path: String }
struct RenameParam: Encodable { var id: String; var name: String }
struct TreeParam: Encodable { var tree: [Node] }
struct SavedCredentials: Codable { var username: String?; var password: String? }
struct CredentialSave: Encodable { var scope: String; var username: String?; var password: String? }
struct IdentityParam: Encodable { var identity: String }
struct SignParam: Encodable { var identity: String; var data: String; var algorithm: String }
struct SecretParam: Encodable { var scope: String; var field: String }
struct SecretSave: Encodable { var scope: String; var field: String; var value: String }
