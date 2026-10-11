import ExtensionFoundation
import ExtensionKit
import Foundation
import NoraPluginKit
import SwiftUI

@main @MainActor
final class NativePlugin: NSObject, AppExtension {
    private let model = NativePluginModel()

    override required init() {
        super.init()
    }

    var configuration: AppExtensionSceneConfiguration {
        let model = self.model
        let service = NoraPluginService(
            handler: { [model] request, host in try await model.invoke(request, host: host) },
            onConnect: { [model] host in Task { @MainActor in model.host = host } }
        )
        return AppExtensionSceneConfiguration(
            PrimitiveAppExtensionScene(id: "com.nora.example.native.panel", content: { [model] in
                NativePluginView(model: model)
            }, onConnection: { service.accept(connection: $0) }),
            configuration: service
        )
    }
}

@MainActor @Observable
final class NativePluginModel {
    var text = "Hello, Nora"
    var result = ""
    var host: NoraPluginHostClient?

    func invoke(_ request: NoraPluginRequest, host: NoraPluginHostClient) async throws -> NoraPluginResponse {
        guard request.method == "commands.execute" else {
            return NoraPluginResponse(error: "Unknown method: \(request.method)")
        }
        let input = try JSONDecoder().decode(CommandInput.self, from: Data(request.parametersJSON.utf8))
        if input.commandID == "com.nora.example.native.hostInfo" {
            return try await host.request(NoraPluginRequest(method: "host.info"))
        }
        guard input.commandID == "com.nora.example.native.echo", let text = input.arguments.text else {
            return NoraPluginResponse(error: "Unknown command: \(input.commandID)")
        }
        self.host = host
        result = text
        return NoraPluginResponse(resultJSON: String(decoding: try JSONEncoder().encode(["text": text]), as: UTF8.self))
    }

    func readHostInfo() async {
        guard let host else {
            result = String(localized: "Open this panel in Nora to connect.")
            return
        }
        do {
            let response = try await host.request(NoraPluginRequest(method: "host.info"))
            result = response.error ?? response.resultJSON ?? ""
        } catch {
            result = error.localizedDescription
        }
    }

    private struct CommandInput: Decodable {
        let commandID: String
        let arguments: EchoArguments
    }

    private struct EchoArguments: Codable {
        let text: String?
    }
}

private struct NativePluginView: View {
    @Bindable var model: NativePluginModel

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            Label(String(localized: "Native Plugin Example"), systemImage: "puzzlepiece.extension")
                .font(.title2)
            Text(String(localized: "This SwiftUI panel runs in its own extension process."))
                .foregroundStyle(.secondary)
            TextField(String(localized: "Text"), text: $model.text)
                .textFieldStyle(.roundedBorder)
            HStack {
                Button(String(localized: "Echo")) { model.result = model.text }
                Button(String(localized: "Get Nora Info")) { Task { await model.readHostInfo() } }
                    .disabled(model.host == nil)
            }
            ScrollView { Text(model.result).textSelection(.enabled).frame(maxWidth: .infinity, alignment: .leading) }
        }
        .padding(24)
        .frame(minWidth: 320, minHeight: 260)
    }
}
