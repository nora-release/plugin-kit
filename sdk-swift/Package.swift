// swift-tools-version: 6.0
import PackageDescription

let package = Package(
    name: "NoraPluginProcessSDK",
    platforms: [.macOS(.v14)],
    products: [
        .library(name: "NoraPluginProcessSDK", targets: ["NoraPluginProcessSDK"]),
        .executable(name: "nora-plugin-echo", targets: ["NoraPluginEcho"]),
    ],
    targets: [
        .target(name: "NoraPluginProcessSDK"),
        .executableTarget(name: "NoraPluginEcho", dependencies: ["NoraPluginProcessSDK"]),
        .testTarget(name: "NoraPluginProcessSDKTests", dependencies: ["NoraPluginProcessSDK"]),
    ]
)
