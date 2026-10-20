import Foundation

public enum NoraPluginWire {
    public static let maximumRequestBytes = 1024 * 1024
    public static let maximumMessageBytes = 8 * 1024 * 1024

    public static func encode(_ request: NoraPluginRequest) throws -> Data {
        try validate(request)
        return try boundedEncode(request, maximumBytes: maximumRequestBytes)
    }

    public static func decodeRequest(_ data: Data) throws -> NoraPluginRequest {
        try checkSize(data, maximumBytes: maximumRequestBytes)
        let request = try JSONDecoder().decode(NoraPluginRequest.self, from: data)
        try validate(request)
        return request
    }

    public static func encode(_ response: NoraPluginResponse) throws -> Data {
        try validate(response)
        return try boundedEncode(response, maximumBytes: maximumMessageBytes)
    }

    public static func decodeResponse(_ data: Data) throws -> NoraPluginResponse {
        try checkSize(data)
        let response = try JSONDecoder().decode(NoraPluginResponse.self, from: data)
        try validate(response)
        return response
    }

    private static func validate(_ request: NoraPluginRequest) throws {
        guard !request.method.isEmpty, request.method.utf8.count <= 128,
              !request.method.unicodeScalars.contains(where: { CharacterSet.controlCharacters.contains($0) }) else {
            throw NoraPluginValidationError.invalid("request method", "must contain 1–128 bytes without control characters")
        }
        let parameters = Data(request.parametersJSON.utf8)
        try checkSize(parameters, maximumBytes: maximumRequestBytes)
        guard (try? JSONSerialization.jsonObject(with: parameters)) is [String: Any] else {
            throw NoraPluginValidationError.invalid("request parameters", "must be a JSON object")
        }
    }

    private static func validate(_ response: NoraPluginResponse) throws {
        guard (response.resultJSON == nil) != (response.error == nil) else {
            throw NoraPluginValidationError.invalid("response", "must contain exactly one result or error")
        }
        if let result = response.resultJSON {
            let data = Data(result.utf8)
            try checkSize(data)
            _ = try JSONSerialization.jsonObject(with: data, options: [.fragmentsAllowed])
        }
    }

    private static func boundedEncode(_ value: some Encodable, maximumBytes: Int) throws -> Data {
        let data = try JSONEncoder().encode(value)
        try checkSize(data, maximumBytes: maximumBytes)
        return data
    }

    private static func checkSize(_ data: Data, maximumBytes: Int = maximumMessageBytes) throws {
        guard data.count <= maximumBytes else {
            throw NoraPluginValidationError.invalid("message", "exceeds \(maximumBytes / (1024 * 1024)) MiB")
        }
    }
}
