import Foundation

public struct NoraPluginView: Codable, Hashable, Sendable {
    public static let maximumBytes = 512 * 1024
    public static let maximumNodes = 1000
    public static let maximumDepth = 32

    public enum Kind: String, Codable, Hashable, Sendable, CaseIterable {
        case column, row, text, button, textField, secureField, picker, list
        case divider, spacer, badge, progress
    }

    public struct Option: Codable, Hashable, Sendable {
        public let value: String
        public let text: String

        public init(value: String, text: String) {
            self.value = value
            self.text = text
        }
    }

    public let type: Kind
    public let id: String?
    public let text: String?
    public let placeholder: String?
    public let action: String?
    public let value: String?
    public let options: [Option]
    public let items: [NoraPluginView]
    public let children: [NoraPluginView]
    public let disabled: Bool

    public init(type: Kind, id: String? = nil, text: String? = nil, placeholder: String? = nil,
                action: String? = nil, value: String? = nil, options: [Option] = [],
                items: [NoraPluginView] = [], children: [NoraPluginView] = [], disabled: Bool = false) {
        self.type = type
        self.id = id
        self.text = text
        self.placeholder = placeholder
        self.action = action
        self.value = value
        self.options = options
        self.items = items
        self.children = children
        self.disabled = disabled
    }

    public init(from decoder: any Decoder) throws {
        let values = try decoder.container(keyedBy: CodingKeys.self)
        type = try values.decode(Kind.self, forKey: .type)
        id = try values.decodeIfPresent(String.self, forKey: .id)
        text = try values.decodeIfPresent(String.self, forKey: .text)
        placeholder = try values.decodeIfPresent(String.self, forKey: .placeholder)
        action = try values.decodeIfPresent(String.self, forKey: .action)
        value = try values.decodeIfPresent(String.self, forKey: .value)
        options = try values.decodeIfPresent([Option].self, forKey: .options) ?? []
        items = try values.decodeIfPresent([Self].self, forKey: .items) ?? []
        children = try values.decodeIfPresent([Self].self, forKey: .children) ?? []
        disabled = try values.decodeIfPresent(Bool.self, forKey: .disabled) ?? false
    }

    public static func decode(_ data: Data) throws -> Self {
        guard data.count <= maximumBytes else { throw NoraPluginValidationError.invalid("view", "exceeds 512 KiB") }
        let view = try JSONDecoder().decode(Self.self, from: data)
        try view.validate()
        return view
    }

    public func validate() throws {
        var count = 0
        var identifiers = Set<String>()
        try validate(depth: 0, count: &count, identifiers: &identifiers)
        guard try JSONEncoder().encode(self).count <= Self.maximumBytes else {
            throw NoraPluginValidationError.invalid("view", "exceeds 512 KiB")
        }
    }

    private func validate(depth: Int, count: inout Int, identifiers: inout Set<String>) throws {
        count += 1
        guard depth < Self.maximumDepth, count <= Self.maximumNodes else {
            throw NoraPluginValidationError.invalid("view", "exceeds its depth or node limit")
        }
        if let id {
            guard !id.isEmpty, id.utf8.count <= 256, identifiers.insert(id).inserted else {
                throw NoraPluginValidationError.invalid("view id", "must be nonempty, bounded, and unique")
            }
        }
        guard text?.utf8.count ?? 0 <= 64 * 1024,
              placeholder?.utf8.count ?? 0 <= 1024, action?.utf8.count ?? 0 <= 256,
              value?.utf8.count ?? 0 <= 4096, options.count <= 1000,
              options.allSatisfy({ $0.value.utf8.count <= 4096 && $0.text.utf8.count <= 4096 }),
              Set(options.map(\.value)).count == options.count else {
            throw NoraPluginValidationError.invalid("view", "contains oversized or duplicate values")
        }
        for child in children + items { try child.validate(depth: depth + 1, count: &count, identifiers: &identifiers) }
    }
}
