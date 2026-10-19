import AppKit
import SwiftUI

@main
enum ContainingApp {
    @MainActor static func main() {
        guard CommandLine.arguments.dropFirst().contains("--nora-register") else {
            ContainingPreviewApp.main()
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

private struct ContainingPreviewApp: App {
    var body: some Scene {
        WindowGroup(String(localized: "Nora Native Plugin Example")) {
            VStack(alignment: .leading, spacing: 16) {
                Label(String(localized: "Nora Native Plugin Example"), systemImage: "puzzlepiece.extension")
                    .font(.title)
                Text(String(localized: "Import this app in Nora’s Plugins settings, then enable its extension using the system extension browser."))
                Text(String(localized: "The plugin provides a SwiftUI panel and an Echo command. It requests access only to Nora’s basic app information."))
                    .foregroundStyle(.secondary)
            }
            .padding(32)
            .frame(width: 440)
        }
        .windowResizability(.contentSize)
    }
}
