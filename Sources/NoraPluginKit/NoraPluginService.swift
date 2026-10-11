import ExtensionFoundation
import Foundation

/// Use the same service for scene connections and AppExtensionSceneConfiguration.configuration.
/// The SDK answers `plugin.handshake` with its API version before invoking custom handlers.
public struct NoraPluginService: AppExtensionConfiguration {
    public typealias Handler = @Sendable (NoraPluginRequest, NoraPluginHostClient) async throws -> NoraPluginResponse
    private let handler: Handler
    private let onConnect: @Sendable (NoraPluginHostClient) -> Void

    public nonisolated init(handler: @escaping Handler,
                            onConnect: @escaping @Sendable (NoraPluginHostClient) -> Void = { _ in }) {
        self.handler = handler
        self.onConnect = onConnect
    }

    public nonisolated func accept(connection: NSXPCConnection) -> Bool {
        let host = NoraPluginHostClient(connection: connection)
        connection.exportedInterface = NSXPCInterface(with: NoraPluginExtensionXPC.self)
        connection.exportedObject = NoraPluginEndpoint(handler: handler, host: host)
        connection.remoteObjectInterface = NSXPCInterface(with: NoraPluginHostXPC.self)
        connection.invalidationHandler = { [weak connection] in connection?.exportedObject = nil }
        connection.resume()
        onConnect(host)
        return true
    }
}

private final class NoraPluginEndpoint: NSObject, NoraPluginExtensionXPC, Sendable {
    let handler: NoraPluginService.Handler
    let host: NoraPluginHostClient

    init(handler: @escaping NoraPluginService.Handler, host: NoraPluginHostClient) {
        self.handler = handler
        self.host = host
    }

    func invoke(_ request: Data, reply: @escaping @Sendable (Data) -> Void) {
        Task {
            let response: NoraPluginResponse
            do {
                let request = try NoraPluginWire.decodeRequest(request)
                if request.method == "plugin.handshake" {
                    response = NoraPluginResponse(resultJSON: #"{"apiVersion":1}"#)
                } else {
                    response = try await handler(request, host)
                }
                reply(try NoraPluginWire.encode(response))
            } catch {
                let failure = NoraPluginResponse(error: String(error.localizedDescription.prefix(4096)))
                // Encoding a bounded error string cannot exceed the wire limit.
                reply((try? NoraPluginWire.encode(failure)) ?? Data(#"{"error":"Plugin request failed."}"#.utf8))
            }
        }
    }
}
