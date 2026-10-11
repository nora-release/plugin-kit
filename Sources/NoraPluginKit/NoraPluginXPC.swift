import Foundation

public struct NoraPluginRequest: Codable, Hashable, Sendable {
    public let method: String
    public let parametersJSON: String

    public init(method: String, parametersJSON: String = "{}") {
        self.method = method
        self.parametersJSON = parametersJSON
    }
}

public struct NoraPluginResponse: Codable, Hashable, Sendable {
    public let resultJSON: String?
    public let error: String?

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
