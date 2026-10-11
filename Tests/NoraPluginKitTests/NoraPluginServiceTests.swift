import Foundation
import Testing
@testable import NoraPluginKit

@Suite("Native plugin XPC service")
struct NoraPluginServiceTests {
    @Test("Commands can request host info and SDK handshakes bypass the plugin handler", arguments: ["commands.execute", "plugin.handshake"])
    func bidirectionalConnection(method: String) async throws {
        let service = NoraPluginService { request, host in
            #expect(request.method != "plugin.handshake", "The SDK must handle the handshake without invoking plugin code.")
            guard request.method == "commands.execute" else { return NoraPluginResponse(error: "Unknown method") }
            return try await host.request(NoraPluginRequest(method: "host.info"))
        }
        let delegate = ListenerDelegate(service: service)
        let listener = NSXPCListener.anonymous()
        listener.delegate = delegate
        listener.resume()
        let connection = NSXPCConnection(listenerEndpoint: listener.endpoint)
        connection.remoteObjectInterface = NSXPCInterface(with: NoraPluginExtensionXPC.self)
        connection.exportedInterface = NSXPCInterface(with: NoraPluginHostXPC.self)
        connection.exportedObject = Host()
        connection.resume()
        defer {
            connection.invalidate()
            listener.invalidate()
            withExtendedLifetime(delegate) {}
        }
        let reply = NoraPluginPendingReply()
        let timeout = Task {
            try? await Task.sleep(for: .seconds(5))
            reply.finish(.failure(NoraPluginConnectionError.timedOut))
        }
        defer { timeout.cancel() }
        let data = try NoraPluginWire.encode(NoraPluginRequest(method: method))
        let responseData = try await withCheckedThrowingContinuation { continuation in
            guard reply.install(continuation) else { return }
            let proxy = connection.remoteObjectProxyWithErrorHandler { reply.finish(.failure($0)) } as? any NoraPluginExtensionXPC
            guard let proxy else { reply.finish(.failure(NoraPluginConnectionError.unavailable)); return }
            proxy.invoke(data) { reply.finish(.success($0)) }
        }
        let expected = method == "plugin.handshake" ? #"{"apiVersion":1}"# : #"{"name":"Nora"}"#
        #expect(try NoraPluginWire.decodeResponse(responseData).resultJSON == expected)
    }
}

private final class ListenerDelegate: NSObject, NSXPCListenerDelegate, Sendable {
    let service: NoraPluginService
    init(service: NoraPluginService) { self.service = service }
    func listener(_ listener: NSXPCListener, shouldAcceptNewConnection connection: NSXPCConnection) -> Bool {
        service.accept(connection: connection)
    }
}

private final class Host: NSObject, NoraPluginHostXPC, Sendable {
    func request(_ request: Data, reply: @escaping @Sendable (Data) -> Void) {
        do {
            let request = try NoraPluginWire.decodeRequest(request)
            let response = request.method == "host.info"
                ? NoraPluginResponse(resultJSON: #"{"name":"Nora"}"#)
                : NoraPluginResponse(error: "Permission denied")
            reply(try NoraPluginWire.encode(response))
        } catch {
            reply(Data(#"{"error":"Invalid request"}"#.utf8))
        }
    }
}
