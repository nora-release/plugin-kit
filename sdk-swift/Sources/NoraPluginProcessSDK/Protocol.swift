import Foundation

/// The version a host speaks unless the plugin's manifest names another
/// (`runtime.protocol`); `pluginStreamsProtocolVersion` adds streams.
public let pluginProtocolVersion: UInt32 = 1

public indirect enum JSONValue: Codable, Sendable, Equatable {
    case null
    case bool(Bool)
    case integer(Int64)
    case unsigned(UInt64)
    case number(Double)
    case string(String)
    case array([JSONValue])
    case object([String: JSONValue])

    public init(from decoder: any Decoder) throws {
        let container = try decoder.singleValueContainer()
        if container.decodeNil() {
            self = .null
        } else if let value = try? container.decode(Bool.self) {
            self = .bool(value)
        } else if let value = try? container.decode(Int64.self) {
            self = .integer(value)
        } else if let value = try? container.decode(UInt64.self) {
            self = .unsigned(value)
        } else if let value = try? container.decode(Double.self) {
            self = .number(value)
        } else if let value = try? container.decode(String.self) {
            self = .string(value)
        } else if let value = try? container.decode([JSONValue].self) {
            self = .array(value)
        } else {
            self = try .object(container.decode([String: JSONValue].self))
        }
    }

    public func encode(to encoder: any Encoder) throws {
        var container = encoder.singleValueContainer()
        switch self {
        case .null: try container.encodeNil()
        case let .bool(value): try container.encode(value)
        case let .integer(value): try container.encode(value)
        case let .unsigned(value): try container.encode(value)
        case let .number(value): try container.encode(value)
        case let .string(value): try container.encode(value)
        case let .array(value): try container.encode(value)
        case let .object(value): try container.encode(value)
        }
    }

    public subscript(key: String) -> JSONValue? {
        guard case let .object(value) = self else { return nil }
        return value[key]
    }

    public var unsignedValue: UInt64? {
        switch self {
        case let .unsigned(value): value
        case let .integer(value) where value >= 0: UInt64(value)
        default: nil
        }
    }
}

public struct InitializeParameters: Codable, Sendable {
    public let protocolVersion: UInt32
    public let pluginId: String
    public let config: JSONValue
    public let grantedServices: [String]
    /// Protocol 2: the host's credit for each stream it reads.
    public let streamWindow: UInt64?
}

/// The server answers the version the host chose, and its own stream
/// window, whatever these say.
public struct InitializeResult: Codable, Sendable {
    public let protocolVersion: UInt32
    public let tools: [ToolDefinition]
    public let services: [String]
    public let streamWindow: UInt64?

    public init(tools: [ToolDefinition], services: [String] = []) {
        protocolVersion = pluginProtocolVersion
        self.tools = tools
        self.services = services
        streamWindow = nil
    }
}

public struct ToolDefinition: Codable, Sendable, Equatable {
    public let name: String
    public let description: String
    public let inputSchema: JSONValue

    public init(name: String, description: String, inputSchema: JSONValue) {
        self.name = name
        self.description = description
        self.inputSchema = inputSchema
    }
}

public struct ToolCallParameters: Codable, Sendable {
    public let name: String
    public let arguments: JSONValue
}

public struct ServiceCallParameters: Codable, Sendable {
    public let service: String
    public let method: String
    public let arguments: JSONValue
}

public enum ContentBlock: Codable, Sendable, Equatable {
    case text(String)
    case image(data: String, mimeType: String)

    private enum CodingKeys: String, CodingKey { case type, text, data, mimeType }

    public init(from decoder: any Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        switch try container.decode(String.self, forKey: .type) {
        case "text": self = try .text(container.decode(String.self, forKey: .text))
        case "image":
            self = try .image(data: container.decode(String.self, forKey: .data),
                              mimeType: container.decode(String.self, forKey: .mimeType))
        default:
            throw DecodingError.dataCorruptedError(forKey: .type, in: container, debugDescription: "Unsupported content block type")
        }
    }

    public func encode(to encoder: any Encoder) throws {
        var container = encoder.container(keyedBy: CodingKeys.self)
        switch self {
        case let .text(text):
            try container.encode("text", forKey: .type)
            try container.encode(text, forKey: .text)
        case let .image(data, mimeType):
            try container.encode("image", forKey: .type)
            try container.encode(data, forKey: .data)
            try container.encode(mimeType, forKey: .mimeType)
        }
    }
}

public struct ToolResult: Codable, Sendable, Equatable {
    public let content: [ContentBlock]
    public let isError: Bool
    public let details: JSONValue?

    public init(content: [ContentBlock], isError: Bool = false, details: JSONValue? = nil) {
        self.content = content
        self.isError = isError
        self.details = details
    }

    public static func text(_ text: String) -> Self {
        .init(content: [.text(text)])
    }
}

public struct RPCError: Error, Codable, Sendable, LocalizedError {
    public let code: Int32
    public let message: String
    public let data: JSONValue?
    public var errorDescription: String? {
        message
    }

    public init(code: Int32, message: String, data: JSONValue? = nil) {
        self.code = code
        self.message = message
        self.data = data
    }
}

public protocol ProcessPlugin: Sendable {
    func initialize(_ parameters: InitializeParameters) async throws -> InitializeResult
    func callTool(_ parameters: ToolCallParameters) async throws -> ToolResult
    func callService(_ parameters: ServiceCallParameters) async throws -> JSONValue
}

public extension ProcessPlugin {
    func callService(_: ServiceCallParameters) async throws -> JSONValue {
        throw RPCError(code: -32601, message: "The plugin service is not supported")
    }
}
