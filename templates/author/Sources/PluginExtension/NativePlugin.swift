import ExtensionFoundation
import ExtensionKit
import Foundation
import NoraPluginKit
import SwiftUI

@main @MainActor
final class NativePlugin: NSObject, AppExtension {
    override required init() {
        super.init()
    }

    var configuration: AppExtensionSceneConfiguration {
        let service = NoraPluginService { request, _ in
            guard request.method == "commands.execute" else {
                return NoraPluginResponse(error: "Unsupported method")
            }
            let command = try request.parameters(as: EchoCommand.self)
            guard command.commandID == "com.example.nora.hello.echo" else {
                return NoraPluginResponse(error: "Unknown command")
            }
            return try NoraPluginResponse(result: ["text": command.arguments.text])
        }
        return AppExtensionSceneConfiguration(
            PrimitiveAppExtensionScene(
                id: "com.example.nora.hello.panel",
                content: { HelloPanel() },
                onConnection: { service.accept(connection: $0) }
            ),
            configuration: service
        )
    }
}

private struct EchoCommand: Decodable {
    let commandID: String
    let arguments: Arguments

    struct Arguments: Decodable {
        let text: String
    }
}

private struct HelloPanel: View {
    @State private var text = "Hello, Nora"
    @State private var result = ""

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            Label("Hello Nora", systemImage: "puzzlepiece.extension")
                .font(.title2)
            TextField("Text", text: $text)
                .textFieldStyle(.roundedBorder)
            Button("Echo") { result = text }
            Text(result).textSelection(.enabled)
        }
        .padding(24)
        .frame(minWidth: 320, minHeight: 220)
    }
}
