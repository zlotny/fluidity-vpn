// swift-tools-version:5.9
import Foundation
import PackageDescription

let root = URL(fileURLWithPath: #filePath).deletingLastPathComponent().deletingLastPathComponent().path
let libDir = ProcessInfo.processInfo.environment["FLUIDITY_LIB_DIR"] ?? "\(root)/target/release"

let package = Package(
    name: "Fluidity",
    platforms: [.macOS(.v13)],
    targets: [
        .systemLibrary(name: "CFluidity", path: "Sources/CFluidity"),
        .executableTarget(
            name: "Fluidity",
            dependencies: ["CFluidity"],
            path: "Sources/Fluidity",
            linkerSettings: [
                .unsafeFlags(["-L", libDir]),
                .linkedFramework("Security"),
                .linkedFramework("SystemConfiguration"),
                .linkedFramework("CoreFoundation"),
            ]
        ),
    ]
)
