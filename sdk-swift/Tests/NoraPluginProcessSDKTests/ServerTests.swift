import Foundation
import NoraPluginProcessSDK
import Testing

private enum HarnessError: Error { case timeout, disconnected, invalidFrame }

private struct Session: Sendable {
    let input: FileHandle
    let output: FileHandle
    let run: Task<Void, any Error>

    init(plugin: any ProcessPlugin, maxFrameBytes: Int = 1024 * 1024, maxInFlight: Int = 32) throws {
        let inputPipe = Pipe()
        let outputPipe = Pipe()
        input = inputPipe.fileHandleForWriting
        output = outputPipe.fileHandleForReading
        let server = try PluginServer(plugin: plugin, maxFrameBytes: maxFrameBytes, maxInFlight: maxInFlight)
        run = Task {
            try await server.serve(input: inputPipe.fileHandleForReading, output: outputPipe.fileHandleForWriting)
        }
    }

    func send(_ fields: [String: JSONValue]) throws {
        var data = try JSONEncoder().encode(JSONValue.object(fields))
        data.append(0x0A)
        try input.write(contentsOf: data)
    }

    func receive() async throws -> [String: JSONValue] {
        try await withThrowingTaskGroup(of: [String: JSONValue].self) { group in
            group.addTask {
                var frame = Data()
                while let byte = try output.read(upToCount: 1), !byte.isEmpty {
                    if byte[0] == 0x0A {
                        guard case let .object(fields) = try JSONDecoder().decode(JSONValue.self, from: frame) else {
                            throw HarnessError.invalidFrame
                        }
                        return fields
                    }
                    frame.append(byte)
                    if frame.count > 1024 * 1024 {
                        throw HarnessError.invalidFrame
                    }
                }
                throw HarnessError.disconnected
            }
            group.addTask {
                try await Task.sleep(for: .seconds(5))
                try? output.close()
                throw HarnessError.timeout
            }
            defer { group.cancelAll() }
            guard let value = try await group.next() else { throw HarnessError.disconnected }
            return value
        }
    }

    func initialize(services: [String] = []) async throws {
        try send([
            "jsonrpc": .string("2.0"), "id": .unsigned(1), "method": .string("initialize"),
            "params": .object(["protocolVersion": .unsigned(1), "pluginId": .string("test"),
                               "config": .object([:]), "grantedServices": .array(services.map(JSONValue.string))]),
        ])
        let reply = try await receive()
        #expect(reply["error"] == nil)
        #expect(reply["result"]?["protocolVersion"]?.unsignedValue == 1)
    }

    func call() throws {
        try send([
            "jsonrpc": .string("2.0"), "id": .unsigned(2), "method": .string("tools/call"),
            "params": .object(["name": .string("echo"), "arguments": .object(["hello": .string("Nora")])]),
        ])
    }

    func shutdown() async throws {
        try send(["jsonrpc": .string("2.0"), "id": .unsigned(99), "method": .string("shutdown")])
        let reply = try await receive()
        #expect(reply["result"] == .bool(true))
        try await run.value
        try? input.close()
        try? output.close()
    }

    func close() {
        run.cancel()
        try? input.close()
        try? output.close()
    }
}

@Test func notificationBurstsApplyBackpressureWithoutDroppingValidFrames() async throws {
    let session = try Session(plugin: Echo(), maxInFlight: 1)
    defer { session.close() }
    try await session.initialize()
    var burst = Data()
    for _ in 0 ..< 64 {
        burst.append(try JSONEncoder().encode(JSONValue.object([
            "jsonrpc": .string("2.0"), "method": .string("notifications/cancelled"),
            "params": .object(["requestId": .unsigned(1000)]),
        ])))
        burst.append(0x0A)
    }
    try session.input.write(contentsOf: burst)
    try session.call()
    let response = try await session.receive()
    #expect(response["error"] == nil)
    #expect(response["result"]?["isError"] == .bool(false))
    try await session.shutdown()
}

private struct Echo: ProcessPlugin {
    enum Behavior: Sendable { case echo, wait, host }
    let behavior: Behavior

    init(_ behavior: Behavior = .echo) {
        self.behavior = behavior
    }

    func initialize(_: InitializeParameters) async throws -> InitializeResult {
        .init(tools: [.init(name: "echo", description: "Echo JSON", inputSchema: .object(["type": .string("object")]))])
    }

    func callTool(_ parameters: ToolCallParameters) async throws -> ToolResult {
        switch behavior {
        case .echo:
            return try .text(String(decoding: JSONEncoder().encode(parameters.arguments), as: UTF8.self))
        case .wait:
            try await Task.sleep(for: .seconds(60))
            return .text("finished")
        case .host:
            let response = try await PluginHostServices.call(service: "host.info", method: "get")
            return try .text(String(decoding: JSONEncoder().encode(response), as: UTF8.self))
        }
    }
}

private struct ServiceEcho: ProcessPlugin {
    let usesHost: Bool

    func initialize(_: InitializeParameters) async throws -> InitializeResult {
        .init(tools: [], services: ["example.echo"])
    }

    func callTool(_: ToolCallParameters) async throws -> ToolResult {
        throw RPCError(code: -32601, message: "No tools")
    }

    func callService(_ parameters: ServiceCallParameters) async throws -> JSONValue {
        if usesHost {
            return try await PluginHostServices.call(service: "host.info", method: "get")
        }
        return parameters.arguments
    }
}

@Test func declaredServicesReceiveCallsAndUndeclaredServicesAreRejected() async throws {
    let session = try Session(plugin: ServiceEcho(usesHost: false))
    defer { session.close() }
    try await session.initialize()
    try session.send([
        "jsonrpc": .string("2.0"), "id": .unsigned(2), "method": .string("services/call"),
        "params": .object(["service": .string("example.echo"), "method": .string("echo"),
                           "arguments": .object(["hello": .string("Nora")])]),
    ])
    let reply = try await session.receive()
    #expect(reply["result"]?["hello"] == .string("Nora"))
    try session.send([
        "jsonrpc": .string("2.0"), "id": .unsigned(3), "method": .string("services/call"),
        "params": .object(["service": .string("undeclared"), "method": .string("echo"), "arguments": .null]),
    ])
    let unknown = try await session.receive()
    #expect(unknown["error"]?["code"] == .integer(-32601))
    try await session.shutdown()
}

@Test func serviceCallbacksUseTheirActualParentAndPreserveNullResults() async throws {
    let session = try Session(plugin: ServiceEcho(usesHost: true))
    defer { session.close() }
    try await session.initialize(services: ["host.info"])
    try session.send([
        "jsonrpc": .string("2.0"), "id": .unsigned(7), "method": .string("services/call"),
        "params": .object(["service": .string("example.echo"), "method": .string("echo"),
                           "arguments": .null, "parentRequestId": .unsigned(9999)]),
    ])
    let callback = try await session.receive()
    #expect(callback["params"]?["parentRequestId"]?.unsignedValue == 7)
    let id = try #require(callback["id"]?.unsignedValue)
    try session.send(["jsonrpc": .string("2.0"), "id": .unsigned(id), "result": .null])
    let reply = try await session.receive()
    #expect(reply["result"] == .null)
    try await session.shutdown()
}

@Test func stdioInitializeCallAndShutdown() async throws {
    let session = try Session(plugin: Echo())
    defer { session.close() }
    try await session.initialize()
    try session.call()
    let reply = try await session.receive()
    #expect(reply["error"] == nil)
    let encoded = try String(decoding: JSONEncoder().encode(reply["result"]), as: UTF8.self)
    #expect(encoded.contains("Nora"))
    try await session.shutdown()
}

@Test func cancellationStopsTheCurrentHandler() async throws {
    let session = try Session(plugin: Echo(.wait))
    defer { session.close() }
    try await session.initialize()
    try session.call()
    try session.send(["jsonrpc": .string("2.0"), "method": .string("notifications/cancelled"),
                      "params": .object(["requestId": .unsigned(2)])])
    let reply = try await session.receive()
    #expect(reply["error"]?["code"] == .integer(-32800))
    try await session.shutdown()
}

@Test func hostServiceUsesTheCurrentParentRequest() async throws {
    let session = try Session(plugin: Echo(.host))
    defer { session.close() }
    try await session.initialize(services: ["host.info"])
    try session.call()
    let call = try await session.receive()
    #expect(call["method"] == .string("services/call"))
    #expect(call["params"]?["parentRequestId"]?.unsignedValue == 2)
    #expect(call["params"]?["service"] == .string("host.info"))
    let id = try #require(call["id"]?.unsignedValue)
    try session.send(["jsonrpc": .string("2.0"), "id": .unsigned(id), "result": .object(["name": .string("Nora")])])
    let reply = try await session.receive()
    #expect(reply["error"] == nil)
    try await session.shutdown()
}

@Test func ungrantedHostServicesAreRejected() async throws {
    let session = try Session(plugin: Echo(.host))
    defer { session.close() }
    try await session.initialize()
    try session.call()
    let reply = try await session.receive()
    #expect(reply["error"]?["code"] == .integer(-32003))
    try await session.shutdown()
}

@Test func truncatedInputFailsWithoutWaitingForAnotherFrame() async throws {
    let input = Pipe()
    let output = Pipe()
    let server = try PluginServer(plugin: Echo())
    let run = Task { try await server.serve(input: input.fileHandleForReading, output: output.fileHandleForWriting) }
    try input.fileHandleForWriting.write(contentsOf: Data("{\"jsonrpc\":\"2.0\"}".utf8))
    try input.fileHandleForWriting.close()
    do {
        try await run.value
        Issue.record("Expected a truncated-frame failure")
    } catch PluginServerError.truncatedFrame {}
    try? output.fileHandleForReading.close()
    try? output.fileHandleForWriting.close()
}

@Test func requiredEmptyTextAndLargeIntegerSurviveTheWire() throws {
    let data = try JSONEncoder().encode(ToolResult.text(""))
    let fields = try JSONDecoder().decode(JSONValue.self, from: data)
    #expect(fields["content"] == .array([.object(["type": .string("text"), "text": .string("")])]))
    let maximum = JSONValue.unsigned(UInt64.max)
    #expect(try JSONDecoder().decode(JSONValue.self, from: JSONEncoder().encode(maximum)) == maximum)
}

private actor Signal {
    private var ready = false
    private var waiting: [CheckedContinuation<Void, Never>] = []

    func wait() async {
        if ready {
            return
        }
        await withCheckedContinuation { waiting.append($0) }
    }

    func send() {
        ready = true
        let continuations = waiting
        waiting.removeAll()
        for continuation in continuations {
            continuation.resume()
        }
    }
}

private actor DeferredServicePlugin: ProcessPlugin {
    let releaseOldCall = Signal()
    let secondCallStarted = Signal()
    let releaseSecondCall = Signal()
    private var count = 0
    private var deferred: Task<JSONValue, any Error>?

    func initialize(_: InitializeParameters) async throws -> InitializeResult {
        .init(tools: [])
    }

    func callTool(_: ToolCallParameters) async throws -> ToolResult {
        count += 1
        if count == 1 {
            deferred = Task {
                await releaseOldCall.wait()
                return try await PluginHostServices.call(service: "host.info", method: "get")
            }
            return .text("first")
        }
        await secondCallStarted.send()
        await releaseSecondCall.wait()
        return .text("second")
    }

    func deferredResult() async throws -> JSONValue {
        guard let deferred else { throw HarnessError.invalidFrame }
        return try await deferred.value
    }
}

@Test func aCompletedCallbackCannotBorrowAReusedRequestID() async throws {
    let plugin = DeferredServicePlugin()
    let session = try Session(plugin: plugin)
    defer { session.close() }
    try await session.initialize(services: ["host.info"])
    try session.call()
    _ = try await session.receive()
    try session.call()
    await plugin.secondCallStarted.wait()
    await plugin.releaseOldCall.send()
    do {
        _ = try await plugin.deferredResult()
        Issue.record("A finished callback reused a newer callback's host grant")
    } catch let error as RPCError {
        #expect(error.code == -32003)
    }
    await plugin.releaseSecondCall.send()
    let reply = try await session.receive()
    #expect(reply["error"] == nil)
    try await session.shutdown()
}

@Test func cancellationCleansOneReverseCallAndIgnoresItsLateReply() async throws {
    let session = try Session(plugin: Echo(.host))
    defer { session.close() }
    try await session.initialize(services: ["host.info"])
    try session.call()
    let first = try await session.receive()
    let firstID = try #require(first["id"]?.unsignedValue)
    #expect(first["params"]?["parentRequestId"]?.unsignedValue == 2)
    try session.send([
        "jsonrpc": .string("2.0"), "id": .unsigned(3), "method": .string("tools/call"),
        "params": .object(["name": .string("echo"), "arguments": .object([:])]),
    ])
    let second = try await session.receive()
    let secondID = try #require(second["id"]?.unsignedValue)
    #expect(second["params"]?["parentRequestId"]?.unsignedValue == 3)
    try session.send(["jsonrpc": .string("2.0"), "method": .string("notifications/cancelled"),
                      "params": .object(["requestId": .unsigned(2)])])
    let frames = [try await session.receive(), try await session.receive()]
    #expect(frames.contains { $0["method"] == .string("notifications/cancelled") && $0["params"]?["requestId"]?.unsignedValue == firstID })
    #expect(frames.contains { $0["id"]?.unsignedValue == 2 && $0["error"]?["code"] == .integer(-32800) })
    try session.send(["jsonrpc": .string("2.0"), "id": .unsigned(firstID), "result": .object(["late": .bool(true)])])
    try session.send(["jsonrpc": .string("2.0"), "id": .unsigned(secondID), "result": .object(["second": .bool(true)])])
    let reply = try await session.receive()
    #expect(reply["id"]?.unsignedValue == 3)
    #expect(reply["error"] == nil)
    let encoded = try String(decoding: JSONEncoder().encode(reply["result"]), as: UTF8.self)
    #expect(encoded.contains("second"))
    try await session.shutdown()
}
