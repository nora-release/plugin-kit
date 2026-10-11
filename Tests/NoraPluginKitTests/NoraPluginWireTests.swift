import Foundation
import Testing
@testable import NoraPluginKit

@Suite("Native plugin messages")
struct NoraPluginWireTests {
    @Test("Command requests preserve nested arguments")
    func requestRoundTrip() throws {
        let request = NoraPluginRequest(method: "commands.execute", parametersJSON: #"{"commandID":"com.example.echo","arguments":{"text":"你好"}}"#)
        #expect(try NoraPluginWire.decodeRequest(NoraPluginWire.encode(request)) == request)
    }

    @Test("A response carries exactly one result or error")
    func responseRoundTrip() throws {
        for response in [NoraPluginResponse(resultJSON: #"{"text":"hello"}"#), NoraPluginResponse(error: "Permission denied")] {
            #expect(try NoraPluginWire.decodeResponse(NoraPluginWire.encode(response)) == response)
        }
        #expect(throws: NoraPluginValidationError.self) { try NoraPluginWire.encode(NoraPluginResponse()) }
        #expect(throws: NoraPluginValidationError.self) { try NoraPluginWire.encode(NoraPluginResponse(resultJSON: "{}", error: "Error")) }
    }

    @Test("Non-object parameters and oversized requests never reach the handler")
    func rejectsRequests() {
        #expect(throws: NoraPluginValidationError.self) {
            try NoraPluginWire.encode(NoraPluginRequest(method: "host.info", parametersJSON: "[]"))
        }
        #expect(throws: NoraPluginValidationError.self) {
            try NoraPluginWire.decodeRequest(Data(repeating: 0, count: NoraPluginWire.maximumMessageBytes + 1))
        }
    }

    @Test("The first reply wins even when cancellation arrives before continuation setup")
    func replyBeforeSetup() async throws {
        let reply = NoraPluginPendingReply()
        reply.finish(.success(Data("first".utf8)))
        reply.finish(.failure(CancellationError()))
        let result = try await withCheckedThrowingContinuation { continuation in
            #expect(!reply.install(continuation))
        }
        #expect(result == Data("first".utf8))
    }
}
