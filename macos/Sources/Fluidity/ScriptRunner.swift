import Foundation

struct ScriptResult {
    var allowConnect = true
    var username: String?
    var password: String?
    var challengeResponse: String?
    var output = ""
}

enum ScriptRunner {
    static func environment(connection: ConnectionInfo, session: SessionInfo?, username: String?) -> [String: String] {
        var env = ProcessInfo.processInfo.environment
        let values: [(String, String?)] = [
            ("displayName", connection.name),
            ("connectionId", connection.id),
            ("ipv4Address", session?.localIpv4),
            ("ipv6Address", session?.localIpv6),
            ("serverIPAddress", session?.remoteAddress),
            ("interface", session?.interface),
            ("username", username),
        ]
        for (key, value) in values {
            guard let value, !value.isEmpty else { continue }
            env[key] = value
            env["FLUIDITY_" + key.uppercased()] = value
        }
        return env
    }

    static func run(path: String, environment: [String: String], timeout: TimeInterval = 120) -> (status: Int32, output: String) {
        let url = URL(fileURLWithPath: path)
        let process = Process()
        let ext = url.pathExtension.lowercased()
        if ["scpt", "applescript", "scptd"].contains(ext) {
            process.executableURL = URL(fileURLWithPath: "/usr/bin/osascript")
            process.arguments = [path]
        } else if FileManager.default.isExecutableFile(atPath: path) {
            process.executableURL = url
        } else {
            process.executableURL = URL(fileURLWithPath: "/bin/sh")
            process.arguments = [path]
        }
        process.environment = environment
        process.currentDirectoryURL = url.deletingLastPathComponent()
        let pipe = Pipe()
        process.standardOutput = pipe
        process.standardError = FileHandle.nullDevice
        let done = DispatchSemaphore(value: 0)
        process.terminationHandler = { _ in done.signal() }
        do {
            try process.run()
        } catch {
            return (-1, error.localizedDescription)
        }
        let timer = DispatchSource.makeTimerSource()
        timer.schedule(deadline: .now() + timeout)
        timer.setEventHandler { process.terminate() }
        timer.resume()
        let data = pipe.fileHandleForReading.readDataToEndOfFile()
        done.wait()
        timer.cancel()
        return (process.terminationStatus, String(decoding: data, as: UTF8.self))
    }

    static func beforeConnect(path: String, connection: ConnectionInfo) -> ScriptResult {
        let (status, output) = run(path: path, environment: environment(connection: connection, session: nil, username: nil))
        var result = ScriptResult(output: output)
        for raw in output.split(whereSeparator: \.isNewline) {
            let line = raw.trimmingCharacters(in: .whitespaces)
            if line == "FluidityNoConnect" { result.allowConnect = false }
            if line == "FluidityConnect" { result.allowConnect = true }
            if line.hasPrefix("FluidityUsername:") { result.username = String(line.dropFirst("FluidityUsername:".count)) }
            if line.hasPrefix("FluidityPassword:") { result.password = String(line.dropFirst("FluidityPassword:".count)) }
            if line.hasPrefix("FluidityChallenge:") {
                result.challengeResponse = String(line.dropFirst("FluidityChallenge:".count))
            }
        }
        if status != 0 && !output.contains("FluidityConnect") {
            result.allowConnect = false
        }
        return result
    }
}
