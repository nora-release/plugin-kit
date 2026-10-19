import AppKit
import SwiftUI

@main
enum PluginHost {
    @MainActor static func main() {
        guard CommandLine.arguments.dropFirst().contains("--nora-register") else {
            PluginPreviewApp.main()
            return
        }
        let application = NSApplication.shared
        let delegate = RegistrationDelegate()
        application.delegate = delegate
        withExtendedLifetime(delegate) { application.run() }
    }
}

@MainActor
private final class RegistrationDelegate: NSObject, NSApplicationDelegate {
    func applicationWillFinishLaunching(_ notification: Notification) {
        NSApp.setActivationPolicy(.prohibited)
    }

    func applicationDidFinishLaunching(_ notification: Notification) {
        DispatchQueue.main.async { NSApp.terminate(nil) }
    }
}

private struct PluginPreviewApp: App {
    var body: some Scene {
        WindowGroup("Hello Nora") {
            VStack(alignment: .leading, spacing: 16) {
                Label("Hello Nora", systemImage: "puzzlepiece.extension")
                    .font(.title)
                Text("Import this app in Nora’s Plugins settings to use its native panel and Echo command.")
            }
            .padding(32)
            .frame(width: 420)
        }
        .windowResizability(.contentSize)
    }
}
