import SwiftUI

@main
struct ContainingApp: App {
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
