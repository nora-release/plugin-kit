import Foundation
import NoraPluginProcessSDK

struct Echo: ProcessPlugin {
    func initialize(_: InitializeParameters) async throws -> InitializeResult {
        .init(tools: [.init(name: "echo", description: "Echo a JSON object", inputSchema: .object(["type": .string("object")]))], services: ["example.echo"])
    }

    func callTool(_ parameters: ToolCallParameters) async throws -> ToolResult {
        let data = try JSONEncoder().encode(parameters.arguments)
        return .text(String(decoding: data, as: UTF8.self))
    }

    func callService(_ parameters: ServiceCallParameters) async throws -> JSONValue {
        parameters.arguments
    }
}

@main
struct Main {
    static func main() async {
        do { try await PluginServer(plugin: Echo()).serve() }
        catch {
            FileHandle.standardError.write(Data((error.localizedDescription + "\n").utf8))
            exit(1)
        }
    }
}
