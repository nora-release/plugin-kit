import Foundation

public enum NoraPluginConnectionError: Error, LocalizedError, Sendable {
    case unavailable
    case timedOut
    case host(String)

    public var errorDescription: String? {
        switch self {
        case .unavailable: "The Nora connection is unavailable."
        case .timedOut: "The Nora request timed out."
        case let .host(message): message
        }
    }
}

/// NSXPCConnection supports concurrent proxy calls; each request owns a locked, single-use reply.
public final class NoraPluginHostClient: @unchecked Sendable {
    private let connection: NSXPCConnection

    public init(connection: NSXPCConnection) {
        self.connection = connection
    }

    public func request(_ request: NoraPluginRequest, timeout: Duration = .seconds(30)) async throws -> NoraPluginResponse {
        let data = try NoraPluginWire.encode(request)
        let pending = NoraPluginPendingReply()
        let timeoutTask = Task {
            do {
                try await Task.sleep(for: timeout)
                pending.finish(.failure(NoraPluginConnectionError.timedOut))
            } catch {}
        }
        defer { timeoutTask.cancel() }
        let responseData = try await withTaskCancellationHandler {
            try await withCheckedThrowingContinuation { continuation in
                guard pending.install(continuation) else { return }
                let proxy = connection.remoteObjectProxyWithErrorHandler { error in
                    pending.finish(.failure(error))
                } as? any NoraPluginHostXPC
                guard let proxy else {
                    pending.finish(.failure(NoraPluginConnectionError.unavailable))
                    return
                }
                proxy.request(data) { pending.finish(.success($0)) }
            }
        } onCancel: {
            pending.finish(.failure(CancellationError()))
        }
        return try NoraPluginWire.decodeResponse(responseData)
    }
}

final class NoraPluginPendingReply: @unchecked Sendable {
    private let lock = NSLock()
    private var continuation: CheckedContinuation<Data, any Error>?
    private var result: Result<Data, any Error>?

    func install(_ continuation: CheckedContinuation<Data, any Error>) -> Bool {
        lock.lock()
        if let result {
            lock.unlock()
            continuation.resume(with: result)
            return false
        }
        self.continuation = continuation
        lock.unlock()
        return true
    }

    func finish(_ result: Result<Data, any Error>) {
        lock.lock()
        guard self.result == nil else { lock.unlock(); return }
        self.result = result
        let continuation = self.continuation
        self.continuation = nil
        lock.unlock()
        continuation?.resume(with: result)
    }
}
