import Foundation
#if canImport(Darwin)
    import Darwin
#elseif canImport(Glibc)
    import Glibc
#endif

public enum PluginServerError: Error, Sendable {
    case invalidOptions
    case alreadyRunning
    case frameTooLarge
    case truncatedFrame
    case inputQueueFull
    case invalidFrame
    case unknownResponse
}

private struct HostServiceContext: Sendable {
    let server: PluginServer
    let parentRequestID: UInt64
    let invocationID: UUID
}

public enum PluginHostServices {
    @TaskLocal fileprivate static var context: HostServiceContext?

    public static func call(service: String, method: String, arguments: JSONValue = .object([:])) async throws -> JSONValue {
        guard let context else { throw RPCError(code: -32003, message: "No host service context") }
        return try await context.server.callService(parentID: context.parentRequestID, invocationID: context.invocationID,
                                                    service: service, method: method, arguments: arguments)
    }
}

public actor PluginServer {
    private let plugin: any ProcessPlugin
    private let maxFrameBytes: Int
    private let maxInFlight: Int
    private let streamWindow: UInt64
    private var streams: StreamTable?
    private var version: UInt32 = 0
    private var output: FileHandle?
    private var inputContinuation: AsyncThrowingStream<Data, any Error>.Continuation?
    private var requests: [UInt64: Task<Void, Never>] = [:]
    private var requestInvocations: [UInt64: UUID] = [:]
    private var services: [UInt64: CheckedContinuation<JSONValue, any Error>] = [:]
    private var serviceID: UInt64 = 0
    private var grants: Set<String> = []
    private var provided: Set<String> = []
    private var started = false
    private var initialized = false
    private var running = false
    private var closing = false

    /// `streamWindow` is, in protocol 2, the credit this plugin gives each
    /// stream it reads.
    public init(plugin: any ProcessPlugin, maxFrameBytes: Int = 1024 * 1024, maxInFlight: Int = 32,
                streamWindow: UInt64 = defaultStreamWindow) throws
    {
        guard (1 ... 16 * 1024 * 1024).contains(maxFrameBytes), (1 ... 1024).contains(maxInFlight) else {
            throw PluginServerError.invalidOptions
        }
        self.plugin = plugin
        self.maxFrameBytes = maxFrameBytes
        self.maxInFlight = maxInFlight
        self.streamWindow = streamWindow
    }

    /// Owns input for this connection and closes it to release pending reads.
    public func serve(input: FileHandle = .standardInput, output: FileHandle = .standardOutput) async throws {
        guard !running, !closing else { throw PluginServerError.alreadyRunning }
        running = true
        self.output = output
        streams = StreamTable(side: .plugin, readWindow: streamWindow) { [weak self] frame in
            guard let self else { throw CancellationError() }
            try await self.writeFrame(frame)
        }
        let stream = AsyncThrowingStream<Data, any Error>.makeStream(bufferingPolicy: .bufferingOldest(maxInFlight))
        let slots = DispatchSemaphore(value: maxInFlight)
        inputContinuation = stream.continuation
        let limit = maxFrameBytes
        let reader = Task.detached { Self.read(input, into: stream.continuation, slots: slots, limit: limit) }
        do {
            try await withTaskCancellationHandler {
                for try await frame in stream.stream {
                    slots.signal()
                    try Task.checkCancellation()
                    if try receive(frame) {
                        break
                    }
                }
            } onCancel: {
                stream.continuation.finish(throwing: CancellationError())
            }
        } catch {
            await stop(input: input, reader: reader)
            throw error
        }
        await stop(input: input, reader: reader)
    }

    private func stop(input: FileHandle, reader: Task<Void, Never>) async {
        closing = true
        running = false
        streams?.closeAll(RPCError(code: -32000, message: "The plugin connection is closed"))
        for task in requests.values {
            task.cancel()
        }
        requests.removeAll()
        requestInvocations.removeAll()
        for continuation in services.values {
            continuation.resume(throwing: CancellationError())
        }
        services.removeAll()
        inputContinuation?.finish()
        reader.cancel()
        await reader.value
        try? input.close()
        output = nil
        inputContinuation = nil
    }

    private nonisolated static func read(_ input: FileHandle, into continuation: AsyncThrowingStream<Data, any Error>.Continuation, slots: DispatchSemaphore, limit: Int) {
        var buffered = Data()
        do {
            while !Task.isCancelled {
                guard let data = try readChunk(input.fileDescriptor), !data.isEmpty else {
                    if !buffered.isEmpty {
                        throw PluginServerError.truncatedFrame
                    }
                    continuation.finish()
                    return
                }
                buffered.append(data)
                while let end = buffered.firstIndex(of: 0x0A) {
                    let count = buffered.distance(from: buffered.startIndex, to: end)
                    guard count <= limit else { throw PluginServerError.frameTooLarge }
                    let frame = Data(buffered[..<end])
                    buffered.removeSubrange(...end)
                    // Notifications and reverse replies do not consume request
                    // slots. Backpressure preserves valid bursts while keeping
                    // the input queue bounded and shutdown cancellable.
                    while slots.wait(timeout: .now() + .milliseconds(100)) == .timedOut {
                        try Task.checkCancellation()
                    }
                    if Task.isCancelled {
                        slots.signal()
                        throw CancellationError()
                    }
                    switch continuation.yield(frame) {
                    case .enqueued: break
                    case .dropped:
                        slots.signal()
                        throw PluginServerError.inputQueueFull
                    case .terminated:
                        slots.signal()
                        return
                    @unknown default:
                        slots.signal()
                        return
                    }
                }
                guard buffered.count <= limit else { throw PluginServerError.frameTooLarge }
            }
            continuation.finish()
        } catch {
            continuation.finish(throwing: error)
        }
    }

    private nonisolated static func readChunk(_ descriptor: Int32) throws -> Data? {
        var readiness = pollfd(fd: descriptor, events: Int16(POLLIN), revents: 0)
        var buffer = [UInt8](repeating: 0, count: 4096)
        while !Task.isCancelled {
            #if canImport(Darwin)
                let ready = Darwin.poll(&readiness, 1, 100)
            #else
                let ready = Glibc.poll(&readiness, 1, 100)
            #endif
            if ready == 0 {
                continue
            }
            if ready < 0 {
                if errno == EINTR {
                    continue
                }
                throw POSIXError(POSIXErrorCode(rawValue: errno) ?? .EIO)
            }
            // A single read returns the currently available pipe bytes; a
            // request must not wait for a larger buffer to fill before dispatch.
            #if canImport(Darwin)
                let count = Darwin.read(descriptor, &buffer, buffer.count)
            #else
                let count = Glibc.read(descriptor, &buffer, buffer.count)
            #endif
            if count > 0 {
                return Data(buffer.prefix(count))
            }
            if count == 0 {
                return nil
            }
            if errno == EINTR {
                continue
            }
            throw POSIXError(POSIXErrorCode(rawValue: errno) ?? .EIO)
        }
        throw CancellationError()
    }

    private func receive(_ data: Data) throws -> Bool {
        let frame = try JSONDecoder().decode(Frame.self, from: data)
        guard frame.jsonrpc == "2.0" else { throw PluginServerError.invalidFrame }
        guard let method = frame.method else {
            try receiveResponse(frame)
            return false
        }
        guard frame.result == nil, frame.error == nil else { throw PluginServerError.invalidFrame }
        if method == "streams/data" || method == "streams/credit" {
            guard version == pluginStreamsProtocolVersion, let streams else {
                throw StreamViolation(reason: "a stream frame outside protocol 2")
            }
            if method == "streams/data" {
                try streams.onData(frame.params)
            } else {
                try streams.onCredit(frame.params)
            }
            return false
        }
        if method == "notifications/cancelled" {
            if let id = frame.params?["requestId"]?.unsignedValue {
                // a reply already sent may still have its output stream running
                streams?.closeCall(StreamCallKey(local: false, id: id),
                                   RPCError(code: -32800, message: "The plugin request was cancelled"))
                requests[id]?.cancel()
            }
            return false
        }
        if method == "shutdown" {
            if let id = frame.id {
                try reply(id, result: .bool(true))
            }
            return true
        }
        guard let id = frame.id else { return false }
        guard requests[id] == nil else { throw PluginServerError.invalidFrame }
        guard requests.count < maxInFlight else {
            try reply(id, error: RPCError(code: -32001, message: "Too many in-flight requests"))
            return false
        }
        if method == "initialize" {
            guard !started else {
                try reply(id, error: RPCError(code: -32600, message: "Plugin is already initialized"))
                return false
            }
            started = true
        } else if !initialized {
            try reply(id, error: RPCError(code: -32000, message: "Initialize the plugin first"))
            return false
        }
        let invocationID = UUID()
        requestInvocations[id] = invocationID
        requests[id] = Task { [weak self] in
            guard let self else { return }
            await execute(id: id, invocationID: invocationID, method: method, params: frame.params ?? .null)
        }
        return false
    }

    private func execute(id: UInt64, invocationID: UUID, method: String, params: JSONValue) async {
        defer {
            requests.removeValue(forKey: id)
            requestInvocations.removeValue(forKey: id)
        }
        do {
            let result = try await PluginHostServices.$context.withValue(.init(server: self, parentRequestID: id, invocationID: invocationID)) {
                try await invoke(id: id, method: method, params: params)
            }
            try Task.checkCancellation()
            guard !closing else { return }
            try reply(id, result: result)
        } catch {
            guard !closing else { return }
            let rpcError = error as? RPCError ?? RPCError(
                code: error is CancellationError ? -32800 : -32603,
                message: error is CancellationError ? "The plugin request was cancelled" : error.localizedDescription
            )
            do { try reply(id, error: rpcError) }
            catch { inputContinuation?.finish(throwing: error) }
        }
    }

    private func invoke(id: UInt64, method: String, params: JSONValue) async throws -> JSONValue {
        let data = try JSONEncoder().encode(params)
        switch method {
        case "initialize":
            let parameters = try JSONDecoder().decode(InitializeParameters.self, from: data)
            let negotiated = parameters.protocolVersion
            guard [pluginProtocolVersion, pluginStreamsProtocolVersion].contains(negotiated),
                  !parameters.pluginId.isEmpty
            else {
                throw RPCError(code: -32602, message: "Unsupported plugin initialization")
            }
            if negotiated == pluginStreamsProtocolVersion {
                do { try streams?.setWriteWindow(parameters.streamWindow) }
                catch { throw RPCError(code: -32602, message: "streamWindow is out of range") }
            } else if parameters.streamWindow != nil {
                throw RPCError(code: -32602, message: "streamWindow needs protocol 2")
            }
            grants = Set(parameters.grantedServices)
            let result = try await plugin.initialize(parameters)
            try Task.checkCancellation()
            let services = Set(result.services)
            guard services.count == result.services.count,
                  services.allSatisfy({ !$0.isEmpty && $0.utf8.count <= 255 })
            else { throw RPCError(code: -32602, message: "Invalid or duplicate service name") }
            provided = services
            initialized = true
            version = negotiated
            // the version is the host's to choose; the plugin speaks it
            guard case var .object(answer) = try value(result) else { return try value(result) }
            answer["protocolVersion"] = .unsigned(UInt64(negotiated))
            answer["streamWindow"] = negotiated == pluginStreamsProtocolVersion
                ? .unsigned(streams?.readWindow ?? defaultStreamWindow) : nil
            return .object(answer)
        case "tools/call":
            let parameters = try JSONDecoder().decode(ToolCallParameters.self, from: data)
            guard !parameters.name.isEmpty, case .object = parameters.arguments else {
                throw RPCError(code: -32602, message: "Invalid tool call parameters")
            }
            let calls = try callStreams(id: id, data: data)
            return try await PluginStreams.$current.withValue(calls) {
                try await value(plugin.callTool(parameters))
            }
        case "services/call":
            let parameters = try JSONDecoder().decode(ServiceCallParameters.self, from: data)
            guard !parameters.service.isEmpty, !parameters.method.isEmpty else {
                throw RPCError(code: -32602, message: "Invalid service call parameters")
            }
            guard provided.contains(parameters.service) else {
                throw RPCError(code: -32601, message: "The plugin service is not supported")
            }
            let calls = try callStreams(id: id, data: data)
            return try await PluginStreams.$current.withValue(calls) {
                try await plugin.callService(parameters)
            }
        default:
            throw RPCError(code: -32601, message: "The plugin method is not supported")
        }
    }

    /// The streams a call names, for its handler.
    private func callStreams(id: UInt64, data: Data) throws -> PluginStreams.CallStreams? {
        struct StreamIDs: Decodable {
            let inputStream: UInt64?
            let outputStream: UInt64?
        }
        let ids = try JSONDecoder().decode(StreamIDs.self, from: data)
        guard ids.inputStream != nil || ids.outputStream != nil else { return nil }
        guard version == pluginStreamsProtocolVersion, let streams else {
            throw RPCError(code: -32602, message: "Streams need protocol 2")
        }
        let (input, output) = try streams.accept(StreamCallKey(local: false, id: id),
                                                 input: ids.inputStream, output: ids.outputStream)
        return PluginStreams.CallStreams(input: input, output: output)
    }

    fileprivate func writeFrame(_ frame: JSONValue) throws {
        try write(frame)
    }

    private func value(_ value: some Encodable) throws -> JSONValue {
        try JSONDecoder().decode(JSONValue.self, from: JSONEncoder().encode(value))
    }

    private func write(_ value: JSONValue) throws {
        guard let output else { throw CancellationError() }
        var data = try JSONEncoder().encode(value)
        guard data.count <= maxFrameBytes else { throw PluginServerError.frameTooLarge }
        data.append(0x0A)
        try output.write(contentsOf: data)
    }

    private func reply(_ id: UInt64, result: JSONValue) throws {
        try write(.object(["jsonrpc": .string("2.0"), "id": .unsigned(id), "result": result]))
    }

    private func reply(_ id: UInt64, error: RPCError) throws {
        try write(.object(["jsonrpc": .string("2.0"), "id": .unsigned(id), "error": value(error)]))
    }

    private func receiveResponse(_ frame: Frame) throws {
        guard let id = frame.id, (frame.result != nil) != (frame.error != nil) else { throw PluginServerError.invalidFrame }
        guard let continuation = services.removeValue(forKey: id) else {
            guard id <= serviceID else { throw PluginServerError.unknownResponse }
            return
        }
        if let error = frame.error {
            continuation.resume(throwing: error)
        } else {
            continuation.resume(returning: frame.result ?? .null)
        }
    }

    fileprivate func callService(parentID: UInt64, invocationID: UUID, service: String, method: String, arguments: JSONValue) async throws -> JSONValue {
        try Task.checkCancellation()
        guard !closing, requestInvocations[parentID] == invocationID, grants.contains(service) else {
            throw RPCError(code: -32003, message: "The plugin has no grant for this service")
        }
        guard services.count < maxInFlight, serviceID < UInt64.max else {
            throw RPCError(code: -32001, message: "Too many in-flight host calls")
        }
        serviceID += 1
        let id = serviceID
        return try await withTaskCancellationHandler {
            try await withCheckedThrowingContinuation { continuation in
                services[id] = continuation
                do {
                    try write(.object([
                        "jsonrpc": .string("2.0"), "id": .unsigned(id), "method": .string("services/call"),
                        "params": .object(["service": .string(service), "method": .string(method),
                                           "arguments": arguments, "parentRequestId": .unsigned(parentID)]),
                    ]))
                } catch {
                    services.removeValue(forKey: id)?.resume(throwing: error)
                }
            }
        } onCancel: {
            Task { await self.cancelService(id) }
        }
    }

    private func cancelService(_ id: UInt64) {
        guard let continuation = services.removeValue(forKey: id) else { return }
        continuation.resume(throwing: CancellationError())
        try? write(.object(["jsonrpc": .string("2.0"), "method": .string("notifications/cancelled"),
                            "params": .object(["requestId": .unsigned(id)])]))
    }
}

private struct Frame: Decodable, Sendable {
    let jsonrpc: String
    let id: UInt64?
    let method: String?
    let params: JSONValue?
    let result: JSONValue?
    let error: RPCError?

    private enum CodingKeys: String, CodingKey { case jsonrpc, id, method, params, result, error }

    init(from decoder: any Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        jsonrpc = try container.decode(String.self, forKey: .jsonrpc)
        id = try container.decodeIfPresent(UInt64.self, forKey: .id)
        method = try container.decodeIfPresent(String.self, forKey: .method)
        params = container.contains(.params) ? try container.decode(JSONValue.self, forKey: .params) : nil
        result = container.contains(.result) ? try container.decode(JSONValue.self, forKey: .result) : nil
        error = try container.decodeIfPresent(RPCError.self, forKey: .error)
    }
}
