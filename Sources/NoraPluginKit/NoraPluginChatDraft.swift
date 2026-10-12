import Foundation

/// Plugin-supplied content for a new, unsent conversation draft. This API grants no chat-reading access.
public struct NoraPluginChatDraft: Codable, Hashable, Sendable {
    public static let openDraftMethod = "chat.openDraft"
    public static let maximumTitleBytes = 256
    public static let maximumPromptBytes = 4 * 1024
    public static let maximumContextBytes = 128 * 1024
    public static let maximumSourceURLBytes = 8192
    public static let maximumBytes = 1024 * 1024

    public let title: String
    public let prompt: String
    public let context: String
    public let sourceURL: String?

    public init(title: String, prompt: String, context: String, sourceURL: String? = nil) {
        self.title = title
        self.prompt = prompt
        self.context = context
        self.sourceURL = sourceURL
    }

    public static func decode(_ data: Data) throws -> Self {
        guard data.count <= maximumBytes else {
            throw NoraPluginValidationError.invalid("chat draft", "exceeds 1 MiB")
        }
        let draft = try JSONDecoder().decode(Self.self, from: data)
        try draft.validate()
        return draft
    }

    public func validate() throws {
        guard !title.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty,
              title.utf8.count <= Self.maximumTitleBytes,
              !title.unicodeScalars.contains(where: { CharacterSet.controlCharacters.contains($0) }) else {
            throw NoraPluginValidationError.invalid("chat draft title", "must be nonempty plain text of at most 256 UTF-8 bytes")
        }
        guard !prompt.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty,
              prompt.utf8.count <= Self.maximumPromptBytes else {
            throw NoraPluginValidationError.invalid("chat draft prompt", "must be nonempty text of at most 4 KiB")
        }
        guard context.utf8.count <= Self.maximumContextBytes else {
            throw NoraPluginValidationError.invalid("chat draft context", "exceeds 128 KiB")
        }
        guard let sourceURL else { return }
        guard sourceURL.utf8.count <= Self.maximumSourceURLBytes,
              !sourceURL.unicodeScalars.contains(where: { CharacterSet.controlCharacters.contains($0) || CharacterSet.whitespacesAndNewlines.contains($0) }),
              let components = URLComponents(string: sourceURL), components.scheme?.lowercased() == "https",
              let host = components.host, !host.isEmpty, components.url != nil,
              components.user == nil, components.password == nil,
              components.port.map({ (1...65535).contains($0) }) ?? true else {
            throw NoraPluginValidationError.invalid("chat draft sourceURL", "must be an HTTPS URL without credentials of at most 8192 UTF-8 bytes")
        }
    }
}
