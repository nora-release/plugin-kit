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
            try NoraPluginWire.decodeRequest(Data(repeating: 0, count: NoraPluginWire.maximumRequestBytes + 1))
        }
    }

    @Test("Response capacity never enlarges native request limits")
    func requestLimit() throws {
        #expect(NoraPluginWire.maximumRequestBytes == 1024 * 1024)
        #expect(NoraPluginWire.maximumMessageBytes == 8 * 1024 * 1024)
        let oversized = NoraPluginRequest(method: "network.request", parametersJSON: "{\"body\":\"" + String(repeating: "x", count: NoraPluginWire.maximumRequestBytes) + "\"}")
        #expect(throws: NoraPluginValidationError.self) { try NoraPluginWire.encode(oversized) }
        #expect(throws: NoraPluginValidationError.self) { try NoraPluginWire.decodeRequest(JSONEncoder().encode(oversized)) }
        let nearLimit = NoraPluginRequest(method: "network.request", parametersJSON: "{\"body\":\"" + String(repeating: "x", count: NoraPluginWire.maximumRequestBytes - 256) + "\"}")
        #expect(try NoraPluginWire.decodeRequest(NoraPluginWire.encode(nearLimit)) == nearLimit)
    }

    @Test("A four MiB network payload fits through native text and base64 responses", arguments: [false, true])
    func networkResponseLimit(base64: Bool) throws {
        let bytes = Data(repeating: 0x61, count: 4 * 1024 * 1024)
        let body = base64 ? bytes.base64EncodedString() : String(decoding: bytes, as: UTF8.self)
        let result = [base64 ? "bodyBase64" : "bodyText": body]
        let response = try NoraPluginResponse(result: result)
        let encoded = try NoraPluginWire.encode(response)
        #expect(encoded.count > NoraPluginWire.maximumRequestBytes)
        #expect(encoded.count < NoraPluginWire.maximumMessageBytes)
        #expect(try NoraPluginWire.decodeResponse(encoded).result(as: [String: String].self) == result)
    }

    @Test("Final encoded response limits still apply after JSON escaping")
    func responseLimit() throws {
        #expect(throws: NoraPluginValidationError.self) {
            try NoraPluginWire.decodeResponse(Data(repeating: 0x20, count: NoraPluginWire.maximumMessageBytes + 1))
        }
        let excessive = try NoraPluginResponse(result: String(repeating: "x", count: NoraPluginWire.maximumMessageBytes))
        #expect(throws: NoraPluginValidationError.self) { try NoraPluginWire.encode(excessive) }
        let escaped = try NoraPluginResponse(result: ["bodyText": String(repeating: "\"", count: 4 * 1024 * 1024)])
        #expect(throws: NoraPluginValidationError.self) { try NoraPluginWire.encode(escaped) }
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
