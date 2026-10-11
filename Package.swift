// swift-tools-version: 6.1
import PackageDescription

let package = Package(
    name: "plugin-kit",
    platforms: [.macOS(.v14)],
    products: [
        .library(name: "NoraPluginKit", targets: ["NoraPluginKit"]),
        .executable(name: "NoraPluginExample", targets: ["NoraPluginExample"]),
        .executable(name: "NoraPluginExampleHost", targets: ["NoraPluginExampleHost"]),
        .executable(name: "NoraPluginSmokeHost", targets: ["NoraPluginSmokeHost"]),
    ],
    targets: [
        .target(name: "NoraPluginKit", resources: [.process("Resources")]),
        // Foundation initializes ExtensionKit before dispatching to the Swift @main entry.
        .executableTarget(name: "NoraPluginExample", dependencies: ["NoraPluginKit"], path: "Examples/NativePlugin",
                          swiftSettings: [.unsafeFlags(["-application-extension"])],
                          linkerSettings: [.unsafeFlags(["-Xlinker", "-application_extension", "-Xlinker", "-e", "-Xlinker", "_NSExtensionMain"])]),
        .executableTarget(name: "NoraPluginExampleHost", path: "Examples/ContainingApp"),
        .executableTarget(name: "NoraPluginSmokeHost", dependencies: ["NoraPluginKit"], path: "Examples/SmokeHost"),
        .testTarget(name: "NoraPluginKitTests", dependencies: ["NoraPluginKit"]),
    ],
    swiftLanguageModes: [.v6]
)
