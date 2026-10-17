import Foundation

public struct NoraPluginView: Codable, Hashable, Sendable {
    public static let maximumBytes = 512 * 1024
    public static let maximumNodes = 1000
    public static let maximumDepth = 32
    public static let maximumAvatarBase64Characters = 128 * 1024
    public static let maximumAvatarBytes = 96 * 1024
    public static let maximumIconSVGBytes = 16 * 1024
    public static let maximumTextEditorValueBytes = 64 * 1024

    public enum Kind: String, Codable, Hashable, Sendable, CaseIterable {
        case column, row, text, button, textField, secureField, picker, list
        /// Native multiline input bound through values[id]. Return inserts a newline without invoking action.
        case textEditor
        case divider, spacer, badge, progress
        /// Children are an optional first sidebar, main content, and optional inspector.
        /// The action closes the inspector; native events clear values[workspace.id].
        case workspace
        /// Items are navigationItem nodes. value selects an item id; selection updates values[sidebar.id].
        /// loadMoreAction and loadMoreToken opt into native loading when the list reaches its bottom.
        case sidebar
        case toolbar
        /// Children are a toolbar, a column containing the heading and summary, a tabBar, and body content.
        /// A sole splitView body fills the remaining height; other body content scrolls beneath the fixed heading.
        /// Items may contain one column for a separate summary aside. It accepts bounded content and controls,
        /// without nested workspaces, scrolling layouts, input fields, timeline items, or item arrays.
        case detail
        /// Children are an author button, metadata row, and optional body column. The host places the author's
        /// existing avatar in a leading gutter and its name beside the metadata. value is "comment" (default) or "event".
        case timelineItem
        /// Children are native tab buttons. A button with value "selected" marks the active tab; subtitle is its count.
        /// Items may contain one row of static trailing metadata. Gate it with nora.features?.tabBarSummary === true.
        case tabBar
        /// Two independently scrolling panes: a 248-point list and flexible detail. At narrow widths value "detail"
        /// shows the detail pane; other values show the list. action and text provide the optional back button.
        case splitView
        /// Compact status row. value "selected" marks selection; action is optional for read-only rows.
        /// tone colors only the leading SVG or symbol; children contain noninteractive trailing metadata.
        case itemRow
        /// The action selects a row. children can provide native metadata, such as a status badge.
        case navigationItem
        /// Items are collectionItem nodes. value selects an item id; Space/Enter invokes this node's action.
        case collection
        /// The action selects the item by click or arrow key. children provide native metadata views.
        case collectionItem
        /// Children provide secondary native controls, such as buttons, dividers, and pickers.
        case menu
        case emptyState
        /// Native static Markdown text without HTML or automatic remote image requests.
        case markdown
        /// Native read-only unified patch in text, with a filename hint in subtitle and optional unified/split layout in value.
        /// The filename is display metadata only and never grants access to local files.
        case diff
    }

    public enum TextStyle: String, Codable, Hashable, Sendable, CaseIterable {
        case title, headline, body, caption, monospaced
    }

    public enum Tone: String, Codable, Hashable, Sendable, CaseIterable {
        case neutral, accent, success, warning, danger, muted, purple
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
    /// Sidebar-only action invoked when its list reaches the bottom. Pair with loadMoreToken.
    public let loadMoreAction: String?
    /// An opaque pagination revision, at most 256 UTF-8 bytes. Change after loading each page.
    public let loadMoreToken: String?
    public let options: [Option]
    public let items: [NoraPluginView]
    public let children: [NoraPluginView]
    public let disabled: Bool
    public let subtitle: String?
    public let symbol: String?
    public let textStyle: TextStyle?
    public let tone: Tone?
    public let appearance: Appearance?
    /// Inline standard Base64 avatar data for native menu, button, or navigation labels.
    public let avatarBase64: String?
    /// Static SVG glyph for a badge, itemRow, or button. Native template rendering applies tone; symbol remains a fallback.
    /// Gate button SVGs with nora.features?.buttonIcon === true on SDK 0.17 or newer.
    public let iconSVG: String?

    public init(type: Kind, id: String? = nil, text: String? = nil, placeholder: String? = nil,
                action: String? = nil, value: String? = nil, options: [Option] = [],
                items: [NoraPluginView] = [], children: [NoraPluginView] = [], disabled: Bool = false,
                subtitle: String? = nil, symbol: String? = nil, textStyle: TextStyle? = nil,
                tone: Tone? = nil, appearance: Appearance? = nil, avatarBase64: String? = nil,
                loadMoreAction: String? = nil, loadMoreToken: String? = nil, iconSVG: String? = nil) {
        self.type = type
        self.id = id
        self.text = text
        self.placeholder = placeholder
        self.action = action
        self.value = value
        self.loadMoreAction = loadMoreAction
        self.loadMoreToken = loadMoreToken
        self.options = options
        self.items = items
        self.children = children
        self.disabled = disabled
        self.subtitle = subtitle
        self.symbol = symbol
        self.textStyle = textStyle
        self.tone = tone
        self.appearance = appearance
        self.avatarBase64 = avatarBase64
        self.iconSVG = iconSVG
    }

    public init(from decoder: any Decoder) throws {
        let values = try decoder.container(keyedBy: CodingKeys.self)
        type = try values.decode(Kind.self, forKey: .type)
        id = try values.decodeIfPresent(String.self, forKey: .id)
        text = try values.decodeIfPresent(String.self, forKey: .text)
        placeholder = try values.decodeIfPresent(String.self, forKey: .placeholder)
        action = try values.decodeIfPresent(String.self, forKey: .action)
        value = try values.decodeIfPresent(String.self, forKey: .value)
        loadMoreAction = try values.decodeIfPresent(String.self, forKey: .loadMoreAction)
        loadMoreToken = try values.decodeIfPresent(String.self, forKey: .loadMoreToken)
        options = try values.decodeIfPresent([Option].self, forKey: .options) ?? []
        items = try values.decodeIfPresent([Self].self, forKey: .items) ?? []
        children = try values.decodeIfPresent([Self].self, forKey: .children) ?? []
        disabled = try values.decodeIfPresent(Bool.self, forKey: .disabled) ?? false
        subtitle = try values.decodeIfPresent(String.self, forKey: .subtitle)
        symbol = try values.decodeIfPresent(String.self, forKey: .symbol)
        textStyle = try values.decodeIfPresent(TextStyle.self, forKey: .textStyle)
        tone = try values.decodeIfPresent(Tone.self, forKey: .tone)
        appearance = try values.decodeIfPresent(Appearance.self, forKey: .appearance)
        avatarBase64 = try values.decodeIfPresent(String.self, forKey: .avatarBase64)
        iconSVG = try values.decodeIfPresent(String.self, forKey: .iconSVG)
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
              value?.utf8.count ?? 0 <= (type == .textEditor ? Self.maximumTextEditorValueBytes : 4096), options.count <= 1000,
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
        if let avatarBase64 {
            guard [.menu, .button, .navigationItem].contains(type),
                  !avatarBase64.isEmpty, avatarBase64.utf8.count <= Self.maximumAvatarBase64Characters,
                  let data = Data(base64Encoded: avatarBase64), !data.isEmpty,
                  data.count <= Self.maximumAvatarBytes, data.base64EncodedString() == avatarBase64 else {
                throw NoraPluginValidationError.invalid("view avatar", "must contain bounded standard Base64 data on a native label")
            }
        }
        if let iconSVG {
            let data = Data(iconSVG.utf8)
            guard [.badge, .itemRow, .button].contains(type), !data.isEmpty, data.count <= Self.maximumIconSVGBytes else {
                throw NoraPluginValidationError.invalid("view icon SVG", "must be nonempty static SVG of at most 16 KiB UTF-8 on a badge, itemRow, or button")
            }
            try NoraPluginIconAsset.validate(data: data, path: "icon.svg")
        }
        if loadMoreAction != nil || loadMoreToken != nil {
            guard type == .sidebar,
                  let loadMoreAction, !loadMoreAction.isEmpty, loadMoreAction.utf8.count <= 256,
                  let loadMoreToken, !loadMoreToken.isEmpty, loadMoreToken.utf8.count <= 256 else {
                throw NoraPluginValidationError.invalid("sidebar pagination", "requires paired nonempty action and token strings of at most 256 UTF-8 bytes on a sidebar")
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
        try validateDetailShape()
        try validateDiffShape()
        for child in children + items { try child.validate(depth: depth + 1, count: &count, identifiers: &identifiers) }
    }

    private func validateDiffShape() throws {
        guard type == .diff else { return }
        guard let text, !text.isEmpty,
              let subtitle, !subtitle.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty,
              !subtitle.unicodeScalars.contains(where: {
                  [.control, .format, .lineSeparator, .paragraphSeparator].contains($0.properties.generalCategory)
              }), value == nil || value == "unified" || value == "split",
              children.isEmpty, items.isEmpty, options.isEmpty,
              action == nil, placeholder == nil, symbol == nil, textStyle == nil,
              tone == nil, appearance == nil, avatarBase64 == nil, iconSVG == nil,
              loadMoreAction == nil, loadMoreToken == nil else {
            throw NoraPluginValidationError.invalid("diff", "requires a nonempty patch, a single-line filename hint, an optional unified/split layout, and no interactive or styled content")
        }
    }

    private func validateDetailShape() throws {
        switch type {
        case .detail:
            guard children.count >= 4,
                  children[0].type == .toolbar, children[1].type == .column, children[2].type == .tabBar else {
                throw NoraPluginValidationError.invalid("detail", "requires a toolbar, heading column, tab bar, and body content in that order")
            }
            guard items.count <= 1, items.first.map({ $0.type == .column && $0.hasOnlyDetailAsideContent }) ?? true else {
                throw NoraPluginValidationError.invalid("detail aside", "accepts at most one column of bounded content and controls, without nested layouts, inputs, timeline items, or item arrays")
            }
        case .timelineItem:
            guard items.isEmpty, (2...3).contains(children.count),
                  children[0].type == .button, children[1].type == .row,
                  children.count == 2 || children[2].type == .column,
                  value == nil || value == "comment" || value == "event", action == nil else {
                throw NoraPluginValidationError.invalid("timeline item", "requires an author button, metadata row, optional body column, an optional comment or event value, and no container action or items")
            }
        case .tabBar:
            guard !children.isEmpty,
                  children.allSatisfy({ $0.type == .button && ($0.value == nil || $0.value == "selected") }),
                  children.filter({ $0.value == "selected" }).count <= 1 else {
                throw NoraPluginValidationError.invalid("tab bar", "requires button children with at most one selected value")
            }
            guard items.count <= 1, items.first.map({ $0.type == .row }) ?? true,
                  Self.hasOnlyStaticMetadata(items) else {
                throw NoraPluginValidationError.invalid("tab bar summary", "accepts at most one row of bounded static metadata, without actions, controls, nested layouts, item arrays, or options")
            }
            for child in children { try child.validateOptionalRowAction() }
        case .splitView:
            guard items.isEmpty, children.count == 2, value == nil || value == "list" || value == "detail" else {
                throw NoraPluginValidationError.invalid("split view", "requires two child panes and an optional list or detail value")
            }
            try validateOptionalRowAction()
        case .itemRow:
            guard items.isEmpty, value == nil || value == "selected", Self.hasOnlyStaticMetadata(children) else {
                throw NoraPluginValidationError.invalid("item row", "requires static metadata children and an optional selected value")
            }
            try validateOptionalRowAction()
        default:
            break
        }
    }

    private func validateOptionalRowAction() throws {
        guard let action else { return }
        guard !action.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty, action.utf8.count <= 256 else {
            throw NoraPluginValidationError.invalid("native row action", "must be omitted for read-only content or a nonempty string of at most 256 UTF-8 bytes")
        }
    }

    private static func hasOnlyStaticMetadata(_ nodes: [Self]) -> Bool {
        var pending = nodes
        var count = 0
        while let node = pending.popLast() {
            count += 1
            guard count <= Self.maximumNodes,
                  [.row, .column, .text, .badge, .progress, .spacer, .divider].contains(node.type),
                  node.action == nil, node.items.isEmpty, node.options.isEmpty else { return false }
            pending.append(contentsOf: node.children)
        }
        return true
    }

    private var hasOnlyDetailAsideContent: Bool {
        var pending = [self]
        var count = 0
        while let node = pending.popLast() {
            count += 1
            guard count <= Self.maximumNodes,
                  [.column, .row, .text, .button, .picker, .menu, .divider, .spacer,
                   .badge, .progress, .emptyState, .markdown, .itemRow].contains(node.type),
                  node.items.isEmpty else { return false }
            pending.append(contentsOf: node.children)
        }
        return true
    }
}
