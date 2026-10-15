import CFluidity
import Foundation

struct CoreError: LocalizedError {
    let message: String
    var errorDescription: String? { message }
}

private struct Envelope<T: Decodable>: Decodable {
    let ok: T?
    let error: String?
}

struct Empty: Codable {}

enum Core {
    static let encoder: JSONEncoder = {
        let e = JSONEncoder()
        e.keyEncodingStrategy = .convertToSnakeCase
        return e
    }()

    static let decoder: JSONDecoder = {
        let d = JSONDecoder()
        d.keyDecodingStrategy = .convertFromSnakeCase
        return d
    }()

    static func raw(_ method: String, _ params: Data?) -> Data {
        let json = params.flatMap { String(data: $0, encoding: .utf8) } ?? ""
        guard let result = fluidity_call(method, json) else { return Data() }
        defer { fluidity_free(result) }
        return Data(String(cString: result).utf8)
    }

    static func call<T: Decodable>(_ method: String, _ params: some Encodable = Empty()) throws -> T {
        let data = raw(method, try encoder.encode(params))
        let envelope = try decoder.decode(Envelope<T>.self, from: data)
        if let error = envelope.error { throw CoreError(message: error) }
        guard let ok = envelope.ok else { throw CoreError(message: "Empty response") }
        return ok
    }

    static func run(_ method: String, _ params: some Encodable = Empty()) throws {
        let data = raw(method, try encoder.encode(params))
        let envelope = try decoder.decode(Envelope<JSONValue>.self, from: data)
        if let error = envelope.error { throw CoreError(message: error) }
    }
}

enum JSONValue: Codable, Equatable {
    case null, bool(Bool), number(Double), string(String), array([JSONValue]), object([String: JSONValue])

    init(from decoder: Decoder) throws {
        let c = try decoder.singleValueContainer()
        if c.decodeNil() { self = .null }
        else if let b = try? c.decode(Bool.self) { self = .bool(b) }
        else if let n = try? c.decode(Double.self) { self = .number(n) }
        else if let s = try? c.decode(String.self) { self = .string(s) }
        else if let a = try? c.decode([JSONValue].self) { self = .array(a) }
        else { self = .object(try c.decode([String: JSONValue].self)) }
    }

    func encode(to encoder: Encoder) throws {
        var c = encoder.singleValueContainer()
        switch self {
        case .null: try c.encodeNil()
        case .bool(let b): try c.encode(b)
        case .number(let n): try c.encode(n)
        case .string(let s): try c.encode(s)
        case .array(let a): try c.encode(a)
        case .object(let o): try c.encode(o)
        }
    }
}
