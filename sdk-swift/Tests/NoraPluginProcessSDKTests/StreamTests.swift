import Foundation
import NoraPluginProcessSDK
import Testing

private enum StreamHarnessError: Error { case timeout, unexpectedFrame }

/// A host table and a plugin table, each one's frames handed to the other in
/// order. Violations are collected.
private final class Pair: @unchecked Sendable {
    var host: StreamTable!
    var plugin: StreamTable!
    private let lock = NSLock()
    private var found: [StreamViolation] = []
    private let toPlugin: AsyncStream<JSONValue>.Continuation
    private let toHost: AsyncStream<JSONValue>.Continuation

    init(window: UInt64) throws {
        let (pluginFrames, toPlugin) = AsyncStream<JSONValue>.makeStream()
        let (hostFrames, toHost) = AsyncStream<JSONValue>.makeStream()
        self.toPlugin = toPlugin
        self.toHost = toHost
        host = StreamTable(side: .host, readWindow: window) { toPlugin.yield($0) }
        plugin = StreamTable(side: .plugin, readWindow: window) { toHost.yield($0) }
        try host.setWriteWindow(plugin.readWindow)
        try plugin.setWriteWindow(host.readWindow)
        pump(pluginFrames, to: plugin)
        pump(hostFrames, to: host)
    }

    deinit {
        toPlugin.finish()
        toHost.finish()
    }

    private func pump(_ frames: AsyncStream<JSONValue>, to table: StreamTable) {
        Task { [weak self] in
            for await frame in frames {
                do {
                    if frame["method"] == .string("streams/data") {
                        try table.onData(frame["params"])
                    } else {
                        try table.onCredit(frame["params"])
                    }
                } catch let violation as StreamViolation {
                    self?.record(violation)
                } catch {}
            }
        }
    }

    private func record(_ violation: StreamViolation) {
        lock.lock()
        found.append(violation)
        lock.unlock()
    }

    var violations: [StreamViolation] {
        lock.lock()
        defer { lock.unlock() }
        return found
    }
}

private let call = StreamCallKey(local: true, id: 7)
private let accepted = StreamCallKey(local: false, id: 7)

private func withTimeout<T: Sendable>(_ seconds: Int = 10, _ work: @escaping @Sendable () async throws -> T) async throws -> T {
    try await withThrowingTaskGroup(of: T.self) { group in
        group.addTask { try await work() }
        group.addTask {
            try await Task.sleep(for: .seconds(seconds))
            throw StreamHarnessError.timeout
        }
        defer { group.cancelAll() }
        return try await group.next()!
    }
}

private func pattern(_ count: Int, modulo: Int) -> Data {
    Data((0 ..< count).map { UInt8($0 % modulo) })
}

@Test func aStreamLargerThanTheWindowArrivesWholeEvenAfterAShortFrame() async throws {
    let pair = try Pair(window: minStreamWindow)
    let writer = try pair.host.openWriter(call)
    let (reader, _) = try pair.plugin.accept(accepted, input: writer.id, output: nil)
    let sent = pattern(1_000_000, modulo: 251)
    Task {
        // a short frame first: the writer then has less credit than a chunk
        try await writer.write(sent.prefix(4096))
        try await writer.write(sent.dropFirst(4096))
        try await writer.finish()
    }
    let received = try await withTimeout { try await reader!.readToEnd(limit: 2_000_000) }
    #expect(received == sent)
    #expect(pair.violations.isEmpty)
    #expect(!pair.host.hasOpenStreams(call))
}

@Test func aCancelledCallEndsItsStreamsOnBothSides() async throws {
    let pair = try Pair(window: defaultStreamWindow)
    let reader = try pair.host.openReader(call)
    let (_, writer) = try pair.plugin.accept(accepted, input: nil, output: reader.id)
    try await writer!.write(Data("head".utf8))
    #expect(try await reader.next() == Data("head".utf8))
    let cancelled = RPCError(code: -32800, message: "cancelled")
    pair.host.closeCall(call, cancelled)
    pair.plugin.closeCall(accepted, cancelled)
    await #expect(throws: RPCError.self) { try await reader.next() }
    await #expect(throws: RPCError.self) { try await writer!.write(Data("late".utf8)) }
    // a frame of the closed stream already on the wire is ignored
    try pair.host.onData(.object(["stream": .unsigned(reader.id), "data": .string("eA==")]))
    #expect(pair.violations.isEmpty)
}

@Test func aWriterThatGivesUpTellsItsReader() async throws {
    let pair = try Pair(window: defaultStreamWindow)
    let reader = try pair.host.openReader(call)
    let (_, writer) = try pair.plugin.accept(accepted, input: nil, output: reader.id)
    try await writer!.abort(RPCError(code: -32050, message: "upstream failed"))
    do {
        _ = try await withTimeout { try await reader.next() }
        Issue.record("the stream ended without its error")
    } catch let error as RPCError {
        #expect(error.code == -32050)
    }
}

@Test func streamIDsAreTheCallersAndFramesAPeerShouldNotSendAreViolations() throws {
    let pair = try Pair(window: minStreamWindow)
    let reader = try pair.host.openReader(call)
    let writer = try pair.host.openWriter(call)
    #expect(reader.id % 2 == 1)
    #expect(throws: RPCError.self) { try pair.plugin.accept(accepted, input: 2, output: nil) }
    #expect(throws: RPCError.self) { try pair.plugin.accept(accepted, input: 9, output: 9) }
    let over = Data(count: Int(minStreamWindow)).base64EncodedString()
    let frames: [(Bool, JSONValue)] = [
        (true, .object(["stream": .unsigned(99)])),
        (false, .object(["stream": .unsigned(99), "bytes": .unsigned(1)])),
        (true, .object(["stream": .unsigned(writer.id)])),
        (false, .object(["stream": .unsigned(reader.id), "bytes": .unsigned(1)])),
        (true, .object(["stream": .unsigned(reader.id), "error": .object(["code": .integer(1), "message": .string("x")])])),
        (true, .object(["stream": .unsigned(reader.id), "data": .string("!")])),
        (true, .object(["stream": .unsigned(reader.id), "data": .string(over)])),
    ]
    for (data, frame) in frames {
        #expect(throws: StreamViolation.self) {
            if data { try pair.host.onData(frame) } else { try pair.host.onCredit(frame) }
        }
    }
}

/// copy answers at once, then copies its input stream to its output stream;
/// spill writes `bytes` bytes and ends, or gives up with `fail`.
private struct StreamPlugin: ProcessPlugin {
    func initialize(_: InitializeParameters) async throws -> InitializeResult {
        InitializeResult(tools: [], services: ["example.echo"])
    }

    func callTool(_: ToolCallParameters) async throws -> ToolResult {
        .text("")
    }

    func callService(_ parameters: ServiceCallParameters) async throws -> JSONValue {
        switch parameters.method {
        case "copy":
            guard let input = PluginStreams.input(), let output = PluginStreams.output() else {
                throw RPCError(code: -32602, message: "copy needs both streams")
            }
            Task {
                while let chunk = try await input.next() {
                    try await output.write(chunk)
                }
                try await output.finish()
            }
            return .object(["head": .bool(true)])
        case "spill":
            guard let output = PluginStreams.output() else {
                throw RPCError(code: -32602, message: "spill needs an output stream")
            }
            let count = Int(parameters.arguments["bytes"]?.unsignedValue ?? 0)
            try await output.write(Data(repeating: 0x78, count: count))
            if parameters.arguments["fail"] == .bool(true) {
                try await output.abort(RPCError(code: -32050, message: "spill failed"))
            } else {
                try await output.finish()
            }
            return .object(["wrote": .unsigned(UInt64(count))])
        default:
            return .object([:])
        }
    }
}

/// Frames other than stream frames, in the order they came.
private actor Mailbox {
    private var frames: [[String: JSONValue]] = []
    private var waiters: [CheckedContinuation<[String: JSONValue], Never>] = []

    func put(_ frame: [String: JSONValue]) {
        if waiters.isEmpty {
            frames.append(frame)
        } else {
            waiters.removeFirst().resume(returning: frame)
        }
    }

    func take() async -> [String: JSONValue] {
        if !frames.isEmpty {
            return frames.removeFirst()
        }
        return await withCheckedContinuation { waiters.append($0) }
    }
}

/// Drives a served plugin as a protocol 2 host: its stream frames go to a
/// table, everything else to `replies`.
private final class StreamingHost: @unchecked Sendable {
    let table: StreamTable
    let input: FileHandle
    let run: Task<Void, any Error>
    private let mailbox = Mailbox()
    private let writeLock = NSLock()

    init(version: UInt32) async throws {
        let inputPipe = Pipe()
        let outputPipe = Pipe()
        let input = inputPipe.fileHandleForWriting
        self.input = input
        let server = try PluginServer(plugin: StreamPlugin())
        run = Task { try await server.serve(input: inputPipe.fileHandleForReading, output: outputPipe.fileHandleForWriting) }
        let lock = writeLock
        table = StreamTable(side: .host, readWindow: minStreamWindow) { frame in
            try StreamingHost.write(frame, to: input, lock: lock)
        }
        let mailbox = mailbox
        let table = table
        let output = outputPipe.fileHandleForReading
        Task.detached {
            var line = Data()
            while true {
                let chunk = output.availableData
                if chunk.isEmpty { break }
                for byte in chunk {
                    guard byte == 0x0A else {
                        line.append(byte)
                        continue
                    }
                    guard case let .object(frame) = try JSONDecoder().decode(JSONValue.self, from: line) else {
                        throw StreamHarnessError.unexpectedFrame
                    }
                    line.removeAll(keepingCapacity: true)
                    switch frame["method"] {
                    case .string("streams/data"): try table.onData(frame["params"])
                    case .string("streams/credit"): try table.onCredit(frame["params"])
                    default: await mailbox.put(frame)
                    }
                }
            }
        }
        var params: [String: JSONValue] = ["protocolVersion": .unsigned(UInt64(version)), "pluginId": .string("test"),
                                           "config": .object([:]), "grantedServices": .array([])]
        if version == pluginStreamsProtocolVersion {
            params["streamWindow"] = .unsigned(table.readWindow)
        }
        try send(["jsonrpc": .string("2.0"), "id": .unsigned(1), "method": .string("initialize"), "params": .object(params)])
        let initialized = try await reply()
        #expect(initialized["result"]?["protocolVersion"]?.unsignedValue == UInt64(version))
        try table.setWriteWindow(initialized["result"]?["streamWindow"]?.unsignedValue)
    }

    private static func write(_ frame: JSONValue, to input: FileHandle, lock: NSLock) throws {
        var data = try JSONEncoder().encode(frame)
        data.append(0x0A)
        lock.lock()
        defer { lock.unlock() }
        try input.write(contentsOf: data)
    }

    func send(_ fields: [String: JSONValue]) throws {
        try Self.write(.object(fields), to: input, lock: writeLock)
    }

    func reply() async throws -> [String: JSONValue] {
        let mailbox = mailbox
        return try await withTimeout(5) { await mailbox.take() }
    }

    func call(_ id: UInt64, _ method: String, _ arguments: JSONValue = .object([:]), input: UInt64? = nil, output: UInt64? = nil) throws {
        var params: [String: JSONValue] = ["service": .string("example.echo"), "method": .string(method), "arguments": arguments]
        if let input { params["inputStream"] = .unsigned(input) }
        if let output { params["outputStream"] = .unsigned(output) }
        try send(["jsonrpc": .string("2.0"), "id": .unsigned(id), "method": .string("services/call"), "params": .object(params)])
    }

    func close() {
        run.cancel()
        try? input.close()
    }
}

@Test func protocolTwoAnswersFirstThenStreamsABodyLargerThanAnyFrame() async throws {
    let host = try await StreamingHost(version: pluginStreamsProtocolVersion)
    defer { host.close() }
    let key = StreamCallKey(local: true, id: 2)
    let input = try host.table.openWriter(key)
    let output = try host.table.openReader(key)
    try host.call(2, "copy", input: input.id, output: output.id)
    #expect(try await host.reply()["result"] == .object(["head": .bool(true)]))
    let body = pattern(1_500_000, modulo: 253)
    Task {
        try await input.write(body)
        try await input.finish()
    }
    let copied = try await withTimeout(20) { try await output.readToEnd(limit: 2_000_000) }
    #expect(copied == body)
}

@Test func aPluginThatGivesUpMidStreamEndsItWithItsError() async throws {
    let host = try await StreamingHost(version: pluginStreamsProtocolVersion)
    defer { host.close() }
    let output = try host.table.openReader(StreamCallKey(local: true, id: 2))
    try host.call(2, "spill", .object(["bytes": .unsigned(100_000), "fail": .bool(true)]), output: output.id)
    var read = 0
    do {
        while let chunk = try await withTimeout(10, { try await output.next() }) {
            read += chunk.count
        }
        Issue.record("the stream ended without its error")
    } catch let error as RPCError {
        #expect(error.code == -32050)
    }
    #expect(read == 100_000)
    #expect(try await host.reply()["result"]?["wrote"]?.unsignedValue == 100_000)
}

@Test func protocolOneTakesNoStreams() async throws {
    let host = try await StreamingHost(version: pluginProtocolVersion)
    defer { host.close() }
    try host.call(2, "spill", output: 1)
    let refused = try await host.reply()
    #expect(refused["error"]?["code"] == .integer(-32602))
    try host.send(["jsonrpc": .string("2.0"), "method": .string("streams/credit"),
                   "params": .object(["stream": .unsigned(1), "bytes": .unsigned(1)])])
    await #expect(throws: StreamViolation.self) { try await withTimeout(5) { try await host.run.value } }
}
