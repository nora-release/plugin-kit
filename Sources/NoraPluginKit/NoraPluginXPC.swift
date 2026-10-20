import Foundation

public struct NoraPluginRequest: Codable, Hashable, Sendable {
    public let method: String
    public let parametersJSON: String

    public init<Parameters: Encodable>(method: String, parameters: Parameters) throws {
        self.init(method: method, parametersJSON: String(decoding: try JSONEncoder().encode(parameters), as: UTF8.self))
    }

    public func parameters<Parameters: Decodable>(as type: Parameters.Type) throws -> Parameters {
        try JSONDecoder().decode(type, from: Data(parametersJSON.utf8))
    }

    public init(method: String, parametersJSON: String = "{}") {
        self.method = method
        self.parametersJSON = parametersJSON
    }
}

public struct NoraPluginResponse: Codable, Hashable, Sendable {
    public let resultJSON: String?
    public let error: String?

    public init<Result: Encodable>(result: Result) throws {
        self.init(resultJSON: String(decoding: try JSONEncoder().encode(result), as: UTF8.self))
    }

    public func result<Result: Decodable>(as type: Result.Type) throws -> Result {
        if let error { throw NoraPluginConnectionError.host(error) }
        guard let resultJSON else { throw NoraPluginConnectionError.unavailable }
        return try JSONDecoder().decode(type, from: Data(resultJSON.utf8))
    }

    public init(resultJSON: String? = nil, error: String? = nil) {
        self.resultJSON = resultJSON
        self.error = error
    }
}

@objc public protocol NoraPluginExtensionXPC {
    func invoke(_ request: Data, reply: @escaping @Sendable (Data) -> Void)
}

@objc public protocol NoraPluginHostXPC {
    func request(_ request: Data, reply: @escaping @Sendable (Data) -> Void)
}
