// swift-tools-version: 6.1
import PackageDescription

let package = Package(
    name: "HelloNora",
    platforms: [.macOS(.v14)],
    products: [
        .executable(name: "PluginHost", targets: ["PluginHost"]),
        .executable(name: "PluginExtension", targets: ["PluginExtension"]),
    ],
    dependencies: [
        .package(url: "https://github.com/nora-release/plugin-kit.git", exact: "__PLUGIN_KIT_VERSION__"),
    ],
    targets: [
        .executableTarget(name: "PluginHost"),
        .executableTarget(
            name: "PluginExtension",
            dependencies: [.product(name: "NoraPluginKit", package: "plugin-kit")],
            swiftSettings: [.unsafeFlags(["-application-extension"])],
            // ExtensionKit must receive the process entry point before Swift's @main.
            linkerSettings: [.unsafeFlags(["-Xlinker", "-application_extension", "-Xlinker", "-e", "-Xlinker", "_NSExtensionMain"])]
        ),
    ],
    swiftLanguageModes: [.v6]
)
