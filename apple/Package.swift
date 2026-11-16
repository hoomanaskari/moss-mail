// swift-tools-version: 6.0
// Moss's bridge to Apple Intelligence: `moss-apple` serves Apple's on-device model through the
// same OpenAI-style chat endpoint as llama-server, so Moss's Rust side talks to either the same
// way. The logic lives in MossAppleCore, where the tests reach it.
import PackageDescription

let package = Package(
    name: "MossApple",
    platforms: [.macOS("27.0")],
    products: [.executable(name: "moss-apple", targets: ["moss-apple"])],
    targets: [
        .target(name: "MossAppleCore"),
        // Its few lines of glue hand callbacks across threads the way Network.framework expects.
        .executableTarget(name: "moss-apple", dependencies: ["MossAppleCore"], swiftSettings: [.swiftLanguageMode(.v5)]),
        .testTarget(name: "MossAppleCoreTests", dependencies: ["MossAppleCore"]),
    ]
)
