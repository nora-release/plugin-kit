import ExtensionFoundation
import Foundation

/// Use the same service for scene connections and AppExtensionSceneConfiguration.configuration.
/// The SDK negotiates `plugin.handshake`; Nora sends `plugin.ready` after authenticating the connection.
public struct NoraPluginService: AppExtensionConfiguration {
    public nonisolated static let readyMethod = "plugin.ready"
    public typealias Handler = @Sendable (NoraPluginRequest, NoraPluginHostClient) async throws -> NoraPluginResponse
    public typealias ConnectionHandler = @Sendable (NoraPluginHostClient, NoraPluginConnectionContext) -> Void
    private let handler: Handler
    private let onConnect: ConnectionHandler

    public nonisolated init(handler: @escaping Handler, onConnect: @escaping ConnectionHandler = { _, _ in }) {
        self.handler = handler
        self.onConnect = onConnect
    }

    public nonisolated func accept(connection: NSXPCConnection) -> Bool {
        let host = NoraPluginHostClient(connection: connection)
        connection.exportedInterface = NSXPCInterface(with: NoraPluginExtensionXPC.self)
        connection.exportedObject = NoraPluginEndpoint(handler: handler, host: host, onConnect: onConnect)
        connection.remoteObjectInterface = NSXPCInterface(with: NoraPluginHostXPC.self)
        connection.invalidationHandler = { [weak connection] in connection?.exportedObject = nil }
        connection.resume()
        return true
    }
}

private final class NoraPluginEndpoint: NSObject, NoraPluginExtensionXPC, Sendable {
    let handler: NoraPluginService.Handler
    let host: NoraPluginHostClient
    let onConnect: NoraPluginService.ConnectionHandler
    private let readiness = ConnectionReadiness()

    init(handler: @escaping NoraPluginService.Handler, host: NoraPluginHostClient,
         onConnect: @escaping NoraPluginService.ConnectionHandler) {
        self.handler = handler
        self.host = host
        self.onConnect = onConnect
    }

    func invoke(_ request: Data, reply: @escaping @Sendable (Data) -> Void) {
        Task {
            let response: NoraPluginResponse
            do {
                let request = try NoraPluginWire.decodeRequest(request)
                switch request.method {
                case "plugin.handshake":
                    response = NoraPluginResponse(resultJSON: #"{"apiVersion":1}"#)
                case NoraPluginService.readyMethod:
                    let context = try request.parameters(as: NoraPluginConnectionContext.self)
                    try context.validate()
                    if try readiness.activate(context) { onConnect(host, context) }
                    response = try NoraPluginResponse(result: true)
                default:
                    guard readiness.isReady else {
                        throw NoraPluginValidationError.invalid("connection", "Nora has not completed authentication")
                    }
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

private final class ConnectionReadiness: @unchecked Sendable {
    private let lock = NSLock()
    private var context: NoraPluginConnectionContext?

    var isReady: Bool { lock.withLock { context != nil } }

    func activate(_ context: NoraPluginConnectionContext) throws -> Bool {
        try lock.withLock {
            guard let previous = self.context else {
                self.context = context
                return true
            }
            guard previous == context else {
                throw NoraPluginValidationError.invalid("connection", "cannot change its authenticated panel")
            }
            return false
        }
    }
}
