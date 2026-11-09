import Foundation

// Protocol 2's streams: flow-controlled bytes beside a call. The rules are
// docs/plugin-system.md §6.1.1; the Rust SDK's streams.rs and the Go SDK's
// streams.go keep the same ones, rule for rule.

public let pluginStreamsProtocolVersion: UInt32 = 2
public let defaultStreamWindow: UInt64 = 1024 * 1024
public let minStreamWindow: UInt64 = 64 * 1024
public let maxStreamWindow: UInt64 = 64 * 1024 * 1024
/// What each `streams/data` costs besides its bytes.
public let streamFrameCost: UInt64 = 1024
/// The most raw bytes one `streams/data` carries.
public let maxStreamChunk = 256 * 1024

func streamFrameCost(bytes: Int) -> UInt64 {
    UInt64(bytes) + streamFrameCost
}

/// The end of the connection a table serves: the host allocates odd stream
/// ids, a plugin even ones.
public enum StreamSide: Sendable {
    case host
    case plugin

    func owns(_ id: UInt64) -> Bool {
        id != 0 && (id % 2 == 1) == (self == .host)
    }
}

/// The call a stream belongs to: one this side sent (`local`) or one the
/// other side sent. The two sides' request ids are separate.
public struct StreamCallKey: Hashable, Sendable {
    public let local: Bool
    public let id: UInt64

    public init(local: Bool, id: UInt64) {
        self.local = local
        self.id = id
    }
}

/// A frame the other side should not have sent; the connection fails.
public struct StreamViolation: Error, Sendable, Equatable {
    public let reason: String
}

/// One connection's streams. `send` writes one frame to the connection; the
/// table never calls it with its lock held.
public final class StreamTable: @unchecked Sendable {
    public let side: StreamSide
    public let readWindow: UInt64
    private let send: @Sendable (JSONValue) async throws -> Void
    private let lock = NSLock()
    private var writeWindow = defaultStreamWindow
    private var nextLocal: UInt64
    private var maxRemote: UInt64 = 0
    private var readers: [UInt64: ReaderSlot] = [:]
    private var writers: [UInt64: WriterSlot] = [:]
    private var calls: [StreamCallKey: Set<UInt64>] = [:]
    private var closed: RPCError?

    final class ReaderSlot {
        let call: StreamCallKey
        let chunks: AsyncStream<Result<Data, RPCError>>.Continuation
        var granted: UInt64

        init(call: StreamCallKey, chunks: AsyncStream<Result<Data, RPCError>>.Continuation, granted: UInt64) {
            self.call = call
            self.chunks = chunks
            self.granted = granted
        }
    }

    final class WriterSlot {
        let call: StreamCallKey
        var credit: UInt64
        var closed: RPCError?
        /// Writers waiting for credit; resumed with nil to look again.
        var waiters: [CheckedContinuation<Result<Int, RPCError>?, Never>] = []

        init(call: StreamCallKey, credit: UInt64) {
            self.call = call
            self.credit = credit
        }

        func wake() {
            let waiting = waiters
            waiters.removeAll()
            for waiter in waiting {
                waiter.resume(returning: nil)
            }
        }
    }

    /// `readWindow` is the credit this side gives each stream it reads.
    public init(side: StreamSide, readWindow: UInt64, send: @escaping @Sendable (JSONValue) async throws -> Void) {
        self.side = side
        self.readWindow = min(max(readWindow, minStreamWindow), maxStreamWindow)
        self.send = send
        nextLocal = side == .host ? 1 : 2
    }

    private func locked<T>(_ body: () throws -> T) rethrows -> T {
        lock.lock()
        defer { lock.unlock() }
        return try body()
    }

    /// Takes the other side's `streamWindow` from `initialize`.
    public func setWriteWindow(_ window: UInt64?) throws {
        let value = window ?? defaultStreamWindow
        guard (minStreamWindow ... maxStreamWindow).contains(value) else {
            throw StreamViolation(reason: "streamWindow is out of range")
        }
        locked { writeWindow = value }
    }

    /// A stream this side reads, for a call it sends.
    public func openReader(_ call: StreamCallKey) throws -> PluginStreamReader {
        try locked {
            let id = try allocate()
            return registerReader(call, id)
        }
    }

    /// A stream this side writes, for a call it sends.
    public func openWriter(_ call: StreamCallKey) throws -> PluginStreamWriter {
        try locked {
            let id = try allocate()
            return registerWriter(call, id)
        }
    }

    /// The streams a call from the other side names: `input` this side
    /// reads, `output` it writes. Ids must be the other side's and new.
    public func accept(_ call: StreamCallKey, input: UInt64?, output: UInt64?) throws
        -> (PluginStreamReader?, PluginStreamWriter?)
    {
        try locked {
            if let closed { throw closed }
            let ids = [input, output].compactMap(\.self)
            guard Set(ids).count == ids.count, ids.allSatisfy({ !side.owns($0) && $0 > maxRemote }) else {
                throw RPCError(code: -32602, message: "Invalid or reused stream id")
            }
            maxRemote = ids.max() ?? maxRemote
            return (input.map { registerReader(call, $0) }, output.map { registerWriter(call, $0) })
        }
    }

    private func allocate() throws -> UInt64 {
        if let closed { throw closed }
        let id = nextLocal
        guard id < UInt64.max - 2 else { throw RPCError(code: -32000, message: "Stream ids are exhausted") }
        nextLocal = id + 2
        return id
    }

    private func registerReader(_ call: StreamCallKey, _ id: UInt64) -> PluginStreamReader {
        let (chunks, continuation) = AsyncStream<Result<Data, RPCError>>.makeStream()
        readers[id] = ReaderSlot(call: call, chunks: continuation, granted: readWindow)
        calls[call, default: []].insert(id)
        return PluginStreamReader(table: self, id: id, chunks: chunks)
    }

    private func registerWriter(_ call: StreamCallKey, _ id: UInt64) -> PluginStreamWriter {
        writers[id] = WriterSlot(call: call, credit: writeWindow)
        calls[call, default: []].insert(id)
        return PluginStreamWriter(table: self, id: id)
    }

    /// Whether `id` was ever a stream of this connection.
    private func known(_ id: UInt64) -> Bool {
        side.owns(id) ? id < nextLocal : id != 0 && id <= maxRemote
    }

    private func untrack(_ call: StreamCallKey, _ id: UInt64) {
        calls[call]?.remove(id)
        if calls[call]?.isEmpty == true {
            calls.removeValue(forKey: call)
        }
    }

    /// Handles a `streams/data` from the other side.
    public func onData(_ params: JSONValue?) throws {
        guard case let .object(fields) = params, let stream = fields["stream"]?.unsignedValue,
              Set(fields.keys).isSubset(of: ["stream", "data", "end", "error"])
        else { throw StreamViolation(reason: "malformed streams/data") }
        var encoded = ""
        if let data = fields["data"] {
            guard case let .string(text) = data else { throw StreamViolation(reason: "malformed streams/data") }
            encoded = text
        }
        var end = false
        if let value = fields["end"] {
            guard case let .bool(flag) = value else { throw StreamViolation(reason: "malformed streams/data") }
            end = flag
        }
        var error: RPCError?
        if let value = fields["error"] {
            error = try? JSONDecoder().decode(RPCError.self, from: JSONEncoder().encode(value))
            guard error != nil else { throw StreamViolation(reason: "malformed streams/data") }
        }
        if error != nil, !end {
            throw StreamViolation(reason: "an error without end")
        }
        guard let bytes = Data(base64Encoded: encoded) else {
            throw StreamViolation(reason: "data is not base64")
        }
        try locked {
            if closed != nil { return }
            if writers[stream] != nil { throw StreamViolation(reason: "data from the stream's reader") }
            guard let slot = readers[stream] else {
                if known(stream) { return }
                throw StreamViolation(reason: "data for an unknown stream")
            }
            // the closing frame is free: there is one per stream
            let cost = end && bytes.isEmpty ? 0 : streamFrameCost(bytes: bytes.count)
            guard cost <= slot.granted else { throw StreamViolation(reason: "data beyond the reader's credit") }
            slot.granted -= cost
            if !bytes.isEmpty {
                slot.chunks.yield(.success(bytes))
            }
            if end {
                if let error {
                    slot.chunks.yield(.failure(error))
                }
                slot.chunks.finish()
                readers.removeValue(forKey: stream)
                untrack(slot.call, stream)
            }
        }
    }

    /// Handles a `streams/credit` from the other side.
    public func onCredit(_ params: JSONValue?) throws {
        guard case let .object(fields) = params, Set(fields.keys) == ["stream", "bytes"],
              let stream = fields["stream"]?.unsignedValue, let bytes = fields["bytes"]?.unsignedValue
        else { throw StreamViolation(reason: "malformed streams/credit") }
        try locked {
            if closed != nil { return }
            if readers[stream] != nil { throw StreamViolation(reason: "credit from the stream's writer") }
            guard let slot = writers[stream] else {
                if known(stream) { return }
                throw StreamViolation(reason: "credit for an unknown stream")
            }
            slot.credit = slot.credit.addingReportingOverflow(bytes).overflow ? .max : slot.credit + bytes
            slot.wake()
        }
    }

    /// Ends one stream with `error`. The lock is held.
    private func closeStream(_ id: UInt64, _ error: RPCError) {
        if let slot = readers.removeValue(forKey: id) {
            slot.chunks.yield(.failure(error))
            slot.chunks.finish()
            untrack(slot.call, id)
        } else if let slot = writers[id] {
            // kept until its writer goes, so it reads why it was closed
            slot.closed = error
            slot.wake()
            untrack(slot.call, id)
        }
    }

    /// Ends every stream of `call`: its readers read `error`, its writers
    /// stop. Frames of these streams still on the wire are ignored.
    public func closeCall(_ call: StreamCallKey, _ error: RPCError) {
        locked {
            for id in calls[call] ?? [] {
                closeStream(id, error)
            }
            calls.removeValue(forKey: call)
        }
    }

    /// Whether `call` has a stream that hasn't ended.
    public func hasOpenStreams(_ call: StreamCallKey) -> Bool {
        locked { !(calls[call]?.isEmpty ?? true) }
    }

    /// The connection ended: every stream ends with `error`.
    public func closeAll(_ error: RPCError) {
        locked {
            for id in Array(readers.keys) + Array(writers.keys) {
                closeStream(id, error)
            }
            calls.removeAll()
            closed = error
        }
    }

    /// Gives the writer `bytes` more. Awaited, never dropped: a lost credit
    /// would leave the writer waiting for good.
    func grant(_ id: UInt64, _ bytes: UInt64) async throws {
        let open = locked { () -> Bool in
            guard let slot = readers[id] else { return false }
            slot.granted += bytes
            return true
        }
        guard open else { return }
        try await send(notification("streams/credit", ["stream": .unsigned(id), "bytes": .unsigned(bytes)]))
    }

    func forgetReader(_ id: UInt64) {
        locked {
            if let slot = readers.removeValue(forKey: id) {
                slot.chunks.finish()
                untrack(slot.call, id)
            }
        }
    }

    /// Takes credit for up to `wanted` bytes, at least one, waiting while
    /// there is no room for a frame; returns how many bytes it took.
    func reserve(_ id: UInt64, wanted: Int) async throws -> Int {
        while true {
            let taken: Result<Int, RPCError>? = await withCheckedContinuation { waiter in
                let outcome = locked { () -> Result<Int, RPCError>?? in
                    guard let slot = writers[id] else {
                        return .some(.failure(closed ?? RPCError(code: -32000, message: "The stream was closed")))
                    }
                    if let error = slot.closed { return .some(.failure(error)) }
                    if slot.credit > streamFrameCost {
                        let count = min(UInt64(wanted), slot.credit - streamFrameCost)
                        slot.credit -= streamFrameCost(bytes: Int(count))
                        return .some(.success(Int(count)))
                    }
                    slot.waiters.append(waiter)
                    return .none
                }
                if case let .some(result) = outcome {
                    waiter.resume(returning: result)
                }
            } as Result<Int, RPCError>?
            if let taken {
                return try taken.get()
            }
        }
    }

    /// Ends this side's writer `id`; the error it was closed with, if any.
    func finishWriter(_ id: UInt64) -> RPCError? {
        locked {
            guard let slot = writers.removeValue(forKey: id) else { return closed }
            untrack(slot.call, id)
            return slot.closed
        }
    }

    var chunkSize: Int {
        let window = locked { writeWindow }
        return Int(min(max(window - streamFrameCost, 1), UInt64(maxStreamChunk)))
    }

    func sendData(_ id: UInt64, _ bytes: Data, end: Bool, error: RPCError?) async throws {
        var params: [String: JSONValue] = ["stream": .unsigned(id), "data": .string(bytes.base64EncodedString())]
        if end { params["end"] = .bool(true) }
        if let error {
            params["error"] = try JSONDecoder().decode(JSONValue.self, from: JSONEncoder().encode(error))
        }
        try await send(notification("streams/data", params))
    }

    private func notification(_ method: String, _ params: [String: JSONValue]) -> JSONValue {
        .object(["jsonrpc": .string("2.0"), "method": .string(method), "params": .object(params)])
    }
}

/// Reads one stream; credit goes back as bytes are taken. One task reads it.
public final class PluginStreamReader: @unchecked Sendable {
    public let id: UInt64
    private let table: StreamTable
    private var iterator: AsyncStream<Result<Data, RPCError>>.Iterator
    private var spent: UInt64 = 0
    private var done = false

    init(table: StreamTable, id: UInt64, chunks: AsyncStream<Result<Data, RPCError>>) {
        self.table = table
        self.id = id
        iterator = chunks.makeAsyncIterator()
    }

    deinit {
        table.forgetReader(id)
    }

    /// The next bytes; nil once the writer ended the stream.
    public func next() async throws -> Data? {
        guard !done else { return nil }
        guard let chunk = await iterator.next() else {
            done = true
            return nil
        }
        switch chunk {
        case let .success(bytes):
            spent += streamFrameCost(bytes: bytes.count)
            if spent >= table.readWindow / 2 {
                let granted = spent
                spent = 0
                try await table.grant(id, granted)
            }
            return bytes
        case let .failure(error):
            done = true
            throw error
        }
    }

    /// Every remaining byte, up to `limit`.
    public func readToEnd(limit: Int) async throws -> Data {
        var all = Data()
        while let chunk = try await next() {
            guard all.count + chunk.count <= limit else {
                throw RPCError(code: -32000, message: "The stream exceeds its limit")
            }
            all.append(chunk)
        }
        return all
    }
}

/// Writes one stream, waiting for credit. One task writes it, and finishes
/// or aborts it in the end.
public final class PluginStreamWriter: @unchecked Sendable {
    public let id: UInt64
    private let table: StreamTable
    private var done = false

    init(table: StreamTable, id: UInt64) {
        self.table = table
        self.id = id
    }

    /// Sends `bytes` in as many frames as the reader's credit allows. A
    /// frame takes what the credit has room for rather than waiting for a
    /// whole chunk: the reader returns credit only once half its window is
    /// spent.
    public func write(_ bytes: Data) async throws {
        guard !done else { throw RPCError(code: -32000, message: "The stream has ended") }
        let chunk = table.chunkSize
        var offset = bytes.startIndex
        while offset < bytes.endIndex {
            let count = try await table.reserve(id, wanted: min(bytes.endIndex - offset, chunk))
            try await table.sendData(id, bytes[offset ..< offset + count], end: false, error: nil)
            offset += count
        }
    }

    /// Ends the stream.
    public func finish() async throws {
        try await end(nil)
    }

    /// Ends the stream with an error the reader reads.
    public func abort(_ error: RPCError) async throws {
        try await end(error)
    }

    private func end(_ error: RPCError?) async throws {
        guard !done else { return }
        done = true
        if let closed = table.finishWriter(id) {
            throw closed
        }
        try await table.sendData(id, Data(), end: true, error: error)
    }
}

/// Protocol 2: the streams the host named for the call being handled.
public enum PluginStreams {
    @TaskLocal static var current: CallStreams?

    final class CallStreams: @unchecked Sendable {
        private let lock = NSLock()
        private var input: PluginStreamReader?
        private var output: PluginStreamWriter?

        init(input: PluginStreamReader?, output: PluginStreamWriter?) {
            self.input = input
            self.output = output
        }

        func takeInput() -> PluginStreamReader? {
            lock.lock()
            defer { lock.unlock() }
            defer { input = nil }
            return input
        }

        func takeOutput() -> PluginStreamWriter? {
            lock.lock()
            defer { lock.unlock() }
            defer { output = nil }
            return output
        }
    }

    /// The stream the host writes for this call, once.
    public static func input() -> PluginStreamReader? {
        current?.takeInput()
    }

    /// The stream this call's answer is written to, once. It may outlive the
    /// handler: return the reply first (say an HTTP status and headers),
    /// then keep writing from a task of its own.
    public static func output() -> PluginStreamWriter? {
        current?.takeOutput()
    }
}
