import Foundation
import Testing
@testable import NoraPluginKit

@Suite("Native plugin XPC service")
struct NoraPluginServiceTests {
    @Test("Host callbacks begin once after readiness and preserve their native channel context",
          arguments: [NoraPluginConnectionContext(), NoraPluginConnectionContext(panelID: "com.example.native.panel")])
    func bidirectionalConnection(context: NoraPluginConnectionContext) async throws {
        let connected = ConnectionRecorder()
        let service = NoraPluginService(handler: { request, host in
            #expect(request.method == "commands.execute", "The SDK must handle connection negotiation itself.")
            let info = try await host.request(method: "host.info", parameters: [String: String](), as: HostInfo.self)
            return try NoraPluginResponse(result: info)
        }, onConnect: { _, context in connected.record(context) })
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
        #expect(connected.contexts.isEmpty)
        let handshake = try await invoke(connection, request: NoraPluginRequest(method: "plugin.handshake"))
        #expect(handshake.resultJSON == #"{"apiVersion":1}"#)
        #expect(connected.contexts.isEmpty)
        let premature = try await invoke(connection, request: NoraPluginRequest(method: "commands.execute"))
        #expect(premature.error != nil)
        #expect(connected.contexts.isEmpty)
        let ready = try NoraPluginRequest(method: NoraPluginService.readyMethod, parameters: context)
        let acknowledgement = try await invoke(connection, request: ready)
        #expect(try acknowledgement.result(as: Bool.self))
        #expect(connected.contexts == [context])
        _ = try await invoke(connection, request: ready)
        #expect(connected.contexts == [context])
        let response = try await invoke(connection, request: NoraPluginRequest(method: "commands.execute"))
        #expect(response.resultJSON == #"{"name":"Nora"}"#)
        let replacement = NoraPluginConnectionContext(panelID: "com.other.native.panel")
        let changed = try await invoke(connection, request: NoraPluginRequest(method: NoraPluginService.readyMethod, parameters: replacement))
        #expect(changed.error != nil)
        #expect(connected.contexts == [context])
    }

    private func invoke(_ connection: NSXPCConnection, request: NoraPluginRequest) async throws -> NoraPluginResponse {
        let reply = NoraPluginPendingReply()
        let timeout = Task {
            try? await Task.sleep(for: .seconds(5))
            reply.finish(.failure(NoraPluginConnectionError.timedOut))
        }
        defer { timeout.cancel() }
        let data = try NoraPluginWire.encode(request)
        let responseData = try await withCheckedThrowingContinuation { continuation in
            guard reply.install(continuation) else { return }
            let proxy = connection.remoteObjectProxyWithErrorHandler { reply.finish(.failure($0)) } as? any NoraPluginExtensionXPC
            guard let proxy else { reply.finish(.failure(NoraPluginConnectionError.unavailable)); return }
            proxy.invoke(data) { reply.finish(.success($0)) }
        }
        return try NoraPluginWire.decodeResponse(responseData)
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

private struct HostInfo: Codable, Sendable {
    let name: String
}

private final class ConnectionRecorder: @unchecked Sendable {
    private let lock = NSLock()
    private var recorded: [NoraPluginConnectionContext] = []
    var contexts: [NoraPluginConnectionContext] { lock.withLock { recorded } }
    func record(_ context: NoraPluginConnectionContext) { lock.withLock { recorded.append(context) } }
}
