import Foundation
import Network

enum DaemonEvent {
    case session(SessionInfo)
    case removed(String)
    case prompt(String, Prompt)
    case promptDone(String)
    case log(String, LogEntry)
    case blocked([String])
}

private struct EventEnvelope: Decodable {
    let event: String
    let session: SessionInfo?
    let id: String?
    let prompt: Prompt?
    let entry: LogEntry?
    let ids: [String]?
}

private struct ResponseEnvelope: Decodable {
    let seq: UInt64?
    let error: String?
}

private struct OkEnvelope<T: Decodable>: Decodable {
    let ok: T
}

private struct SessionsPayload: Decodable { let sessions: [SessionInfo] }
private struct LogPayload: Decodable { let log: [LogEntry] }
private struct VersionsPayload: Decodable { let versions: [String] }
private struct SettingsPayload: Decodable { let settings: SystemSettings }
private struct BlockedPayload: Decodable { let blocked: [String] }
private struct UnblockParams: Encodable { let id: String? }
private struct IdParams: Encodable { let id: String }
private struct UsagePayload: Decodable { let usage: [UsageSample] }

private struct Request<P: Encodable>: Encodable {
    let seq: UInt64
    let method: String
    let params: P?
}

final class DaemonClient: @unchecked Sendable {
    static let socketPath = ProcessInfo.processInfo.environment["FLUIDITY_SOCKET"] ?? "/var/run/fluidityd.sock"

    var onEvent: ((DaemonEvent) -> Void)?
    var onConnectionChange: ((Bool) -> Void)?

    private let queue = DispatchQueue(label: "fluidity.daemon")
    private var connection: NWConnection?
    private var buffer = Data()
    private var seq: UInt64 = 1
    private var waiters: [UInt64: (Result<Data, Error>) -> Void] = [:]
    private(set) var isConnected = false
    private var reconnectTimer: DispatchSourceTimer?

    func start() {
        queue.async { self.open() }
    }

    private func open() {
        connection?.cancel()
        buffer.removeAll()
        let conn = NWConnection(to: .unix(path: Self.socketPath), using: .tcp)
        connection = conn
        conn.stateUpdateHandler = { [weak self] state in
            guard let self else { return }
            switch state {
            case .ready:
                self.isConnected = true
                self.receive(on: conn)
                self.send(method: "subscribe", params: Optional<Empty>.none) { _ in }
                DispatchQueue.main.async { self.onConnectionChange?(true) }
            case .failed, .cancelled:
                self.handleDisconnect(conn)
            case .waiting:
                self.handleDisconnect(conn)
            default:
                break
            }
        }
        conn.start(queue: queue)
    }

    private func handleDisconnect(_ conn: NWConnection) {
        guard conn === connection else { return }
        let wasConnected = isConnected
        isConnected = false
        connection?.cancel()
        connection = nil
        let pending = waiters
        waiters.removeAll()
        for (_, waiter) in pending {
            waiter(.failure(CoreError(message: "The Fluidity service is not running.")))
        }
        if wasConnected {
            DispatchQueue.main.async { self.onConnectionChange?(false) }
        }
        scheduleReconnect()
    }

    private func scheduleReconnect() {
        reconnectTimer?.cancel()
        let timer = DispatchSource.makeTimerSource(queue: queue)
        timer.schedule(deadline: .now() + 2)
        timer.setEventHandler { [weak self] in self?.open() }
        timer.resume()
        reconnectTimer = timer
    }

    private func receive(on conn: NWConnection) {
        conn.receive(minimumIncompleteLength: 1, maximumLength: 1 << 16) { [weak self] data, _, isComplete, error in
            guard let self else { return }
            if let data, !data.isEmpty {
                self.buffer.append(data)
                self.drain()
            }
            if isComplete || error != nil {
                self.handleDisconnect(conn)
                return
            }
            self.receive(on: conn)
        }
    }

    private func drain() {
        while let newline = buffer.firstIndex(of: 0x0A) {
            let line = buffer.subdata(in: buffer.startIndex..<newline)
            buffer.removeSubrange(buffer.startIndex...newline)
            handle(line)
        }
    }

    private func handle(_ line: Data) {
        let decoder = Core.decoder
        if let envelope = try? decoder.decode(ResponseEnvelope.self, from: line), let seq = envelope.seq {
            let waiter = waiters.removeValue(forKey: seq)
            if let error = envelope.error {
                waiter?(.failure(CoreError(message: error)))
            } else {
                waiter?(.success(line))
            }
            return
        }
        guard let event = try? decoder.decode(EventEnvelope.self, from: line) else { return }
        let parsed: DaemonEvent?
        switch event.event {
        case "session": parsed = event.session.map { .session($0) }
        case "removed": parsed = event.id.map { .removed($0) }
        case "prompt":
            if let id = event.id, let prompt = event.prompt { parsed = .prompt(id, prompt) } else { parsed = nil }
        case "prompt_done": parsed = event.id.map { .promptDone($0) }
        case "log":
            if let id = event.id, let entry = event.entry { parsed = .log(id, entry) } else { parsed = nil }
        case "blocked": parsed = event.ids.map { .blocked($0) }
        default: parsed = nil
        }
        if let parsed {
            DispatchQueue.main.async { self.onEvent?(parsed) }
        }
    }

    private func send<P: Encodable>(method: String, params: P?, completion: @escaping (Result<Data, Error>) -> Void) {
        guard let connection, isConnected else {
            completion(.failure(CoreError(message: "The Fluidity service is not running.")))
            return
        }
        let id = seq
        seq += 1
        waiters[id] = completion
        do {
            var data = try Core.encoder.encode(Request(seq: id, method: method, params: params))
            data.append(0x0A)
            connection.send(content: data, completion: .contentProcessed { [weak self] error in
                if let error {
                    self?.queue.async { self?.waiters.removeValue(forKey: id)?(.failure(error)) }
                }
            })
        } catch {
            waiters.removeValue(forKey: id)
            completion(.failure(error))
        }
    }

    func request<P: Encodable>(_ method: String, _ params: P? = Optional<Empty>.none) async throws -> Data {
        try await withCheckedThrowingContinuation { continuation in
            queue.async {
                self.send(method: method, params: params) { continuation.resume(with: $0) }
            }
        }
    }

    func sessions() async throws -> [SessionInfo] {
        let data = try await request("list", Optional<Empty>.none)
        return try Core.decoder.decode(OkEnvelope<SessionsPayload>.self, from: data).ok.sessions
    }

    func log(_ id: String) async throws -> [LogEntry] {
        let data = try await request("log", IdParam(id: id))
        return try Core.decoder.decode(OkEnvelope<LogPayload>.self, from: data).ok.log
    }

    func versions() async throws -> [String] {
        let data = try await request("versions", Optional<Empty>.none)
        return try Core.decoder.decode(OkEnvelope<VersionsPayload>.self, from: data).ok.versions
    }

    func systemSettings() async throws -> SystemSettings {
        let data = try await request("system_settings", Optional<Empty>.none)
        return try Core.decoder.decode(OkEnvelope<SettingsPayload>.self, from: data).ok.settings
    }

    struct ConnectParams: Encodable {
        let id: String
        let bundle: WireBundle
        let options: ConnectOptions
    }

    struct ReplyParams: Encodable {
        let id: String
        let reply: Reply
    }

    func connect(id: String, bundle: WireBundle, options: ConnectOptions) async throws {
        _ = try await request("connect", ConnectParams(id: id, bundle: bundle, options: options))
    }

    func disconnect(id: String) async throws {
        _ = try await request("disconnect", IdParam(id: id))
    }

    func disconnectAll() async throws {
        _ = try await request("disconnect_all", Optional<Empty>.none)
    }

    func blocked() async throws -> [String] {
        let data = try await request("blocked", Optional<Empty>.none)
        return try Core.decoder.decode(OkEnvelope<BlockedPayload>.self, from: data).ok.blocked
    }

    func unblock(id: String?) async throws {
        _ = try await request("unblock", UnblockParams(id: id))
    }

    func usage(id: String) async throws -> [UsageSample] {
        let data = try await request("usage", IdParams(id: id))
        return try Core.decoder.decode(OkEnvelope<UsagePayload>.self, from: data).ok.usage
    }

    func clearUsage(id: String) async throws {
        _ = try await request("clear_usage", IdParams(id: id))
    }

    func reply(id: String, reply: Reply) async throws {
        _ = try await request("reply", ReplyParams(id: id, reply: reply))
    }
}
