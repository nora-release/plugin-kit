import Foundation

public struct NoraPluginView: Codable, Hashable, Sendable {
    public static let maximumBytes = 512 * 1024
    public static let maximumNodes = 1000
    public static let maximumDepth = 32

    public enum Kind: String, Codable, Hashable, Sendable, CaseIterable {
        case column, row, text, button, textField, secureField, picker, list
        case divider, spacer, badge, progress
        /// Children are an optional first sidebar, main content, and optional inspector.
        /// The action closes the inspector; native events clear values[workspace.id].
        case workspace
        /// Items are navigationItem nodes. value selects an item id; selection updates values[sidebar.id].
        case sidebar
        case toolbar, navigationItem
        /// Items are collectionItem nodes. value selects an item id; Space/Enter invokes this node's action.
        case collection
        /// The action selects the item by click or arrow key. children provide native metadata views.
        case collectionItem
        /// Children provide secondary native controls, such as buttons, dividers, and pickers.
        case menu
        case emptyState
        /// Native static Markdown text without HTML or automatic remote image requests.
        case markdown
    }

    public enum TextStyle: String, Codable, Hashable, Sendable, CaseIterable {
        case title, headline, body, caption, monospaced
    }

    public enum Tone: String, Codable, Hashable, Sendable, CaseIterable {
        case neutral, accent, success, warning, danger, muted
    }

    public enum Appearance: String, Codable, Hashable, Sendable, CaseIterable {
        case plain, primary, secondary, icon
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
    public let subtitle: String?
    public let symbol: String?
    public let textStyle: TextStyle?
    public let tone: Tone?
    public let appearance: Appearance?

    public init(type: Kind, id: String? = nil, text: String? = nil, placeholder: String? = nil,
                action: String? = nil, value: String? = nil, options: [Option] = [],
                items: [NoraPluginView] = [], children: [NoraPluginView] = [], disabled: Bool = false,
                subtitle: String? = nil, symbol: String? = nil, textStyle: TextStyle? = nil,
                tone: Tone? = nil, appearance: Appearance? = nil) {
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
        self.subtitle = subtitle
        self.symbol = symbol
        self.textStyle = textStyle
        self.tone = tone
        self.appearance = appearance
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
        subtitle = try values.decodeIfPresent(String.self, forKey: .subtitle)
        symbol = try values.decodeIfPresent(String.self, forKey: .symbol)
        textStyle = try values.decodeIfPresent(TextStyle.self, forKey: .textStyle)
        tone = try values.decodeIfPresent(Tone.self, forKey: .tone)
        appearance = try values.decodeIfPresent(Appearance.self, forKey: .appearance)
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
              subtitle?.utf8.count ?? 0 <= 4096,
              placeholder?.utf8.count ?? 0 <= 1024, action?.utf8.count ?? 0 <= 256,
              value?.utf8.count ?? 0 <= 4096, options.count <= 1000,
              options.allSatisfy({ $0.value.utf8.count <= 4096 && $0.text.utf8.count <= 4096 }),
              Set(options.map(\.value)).count == options.count else {
            throw NoraPluginValidationError.invalid("view", "contains oversized or duplicate values")
        }
        if let symbol {
            guard !symbol.isEmpty, symbol.utf8.count <= 128,
                  !symbol.unicodeScalars.contains(where: { CharacterSet.controlCharacters.contains($0) }) else {
                throw NoraPluginValidationError.invalid("view symbol", "must be a bounded SF Symbol name")
            }
        }
        if type == .workspace {
            let hasSidebar = children.first?.type == .sidebar
            let contentCount = children.count - (hasSidebar ? 1 : 0)
            guard (1...2).contains(contentCount), items.isEmpty,
                  children.dropFirst(hasSidebar ? 1 : 0).allSatisfy({ $0.type != .sidebar }) else {
                throw NoraPluginValidationError.invalid("workspace", "requires an optional first sidebar, main content, and optional inspector")
            }
        }
        for child in children + items { try child.validate(depth: depth + 1, count: &count, identifiers: &identifiers) }
    }
}
