import SwiftUI

@main
struct PluginHost: App {
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
