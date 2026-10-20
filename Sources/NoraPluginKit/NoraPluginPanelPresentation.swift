import Foundation

/// Native window chrome shared with Nora; plugin content remains an ExtensionKit view.
public struct NoraPluginPanelPresentation: Codable, Hashable, Sendable {
    public static let presentMethod = "panel.present"
    public static let actionMethod = "panel.action"
    public static let maximumBytes = 32 * 1024

    public let panelID: String
    public let title: String?
    public let number: Int?
    public let status: NoraPluginPanelStatus?
    public let hasSidebar: Bool
    public let sidebarVisible: Bool
    public let sidebarWidth: Double
    public let actions: [NoraPluginPanelAction]

    public init(panelID: String, title: String? = nil, number: Int? = nil,
                status: NoraPluginPanelStatus? = nil, hasSidebar: Bool = true,
                sidebarVisible: Bool = true, sidebarWidth: Double = 268, actions: [NoraPluginPanelAction] = []) {
        self.panelID = panelID
        self.title = title
        self.number = number
        self.status = status
        self.hasSidebar = hasSidebar
        self.sidebarVisible = sidebarVisible
        self.sidebarWidth = sidebarWidth
        self.actions = actions
    }

    public init(from decoder: any Decoder) throws {
        let values = try decoder.container(keyedBy: CodingKeys.self)
        panelID = try values.decode(String.self, forKey: .panelID)
        title = try values.decodeIfPresent(String.self, forKey: .title)
        number = try values.decodeIfPresent(Int.self, forKey: .number)
        status = try values.decodeIfPresent(NoraPluginPanelStatus.self, forKey: .status)
        hasSidebar = try values.decodeIfPresent(Bool.self, forKey: .hasSidebar) ?? true
        sidebarVisible = try values.decodeIfPresent(Bool.self, forKey: .sidebarVisible) ?? true
        sidebarWidth = try values.decodeIfPresent(Double.self, forKey: .sidebarWidth) ?? 268
        actions = try values.decodeIfPresent([NoraPluginPanelAction].self, forKey: .actions) ?? []
    }

    public static func decode(_ data: Data) throws -> Self {
        guard data.count <= maximumBytes else {
            throw NoraPluginValidationError.invalid("panel presentation", "exceeds 32 KiB")
        }
        let value = try JSONDecoder().decode(Self.self, from: data)
        try value.validate()
        return value
    }

    public func validate() throws {
        try validatePanelIdentifier(panelID)
        guard sidebarWidth.isFinite, (180...560).contains(sidebarWidth) else {
            throw NoraPluginValidationError.invalid("panel sidebar width", "must be a finite number from 180 to 560")
        }
        if let title { try panelText(title, field: "panel title", maximum: 1024) }
        if let number, number <= 0 || number > Int(Int32.max) {
            throw NoraPluginValidationError.invalid("panel number", "must be a positive 32-bit integer")
        }
        if let status {
            try panelText(status.title, field: "panel status title", maximum: 128)
            try panelText(status.systemImage, field: "panel status systemImage", maximum: 128)
            try panelIcon(status.iconSVG)
        }
        guard actions.count <= 16, Set(actions.map(\.id)).count == actions.count,
              actions.filter(\.isPrimary).count <= 1 else {
            throw NoraPluginValidationError.invalid("panel actions", "requires at most 16 unique actions and one primary action")
        }
        for action in actions {
            try validatePanelActionIdentifier(action.id)
            try panelText(action.title, field: "panel action title", maximum: 128)
            try panelText(action.systemImage, field: "panel action systemImage", maximum: 128)
            try panelIcon(action.iconSVG)
        }
        guard try JSONEncoder().encode(self).count <= Self.maximumBytes else {
            throw NoraPluginValidationError.invalid("panel presentation", "exceeds 32 KiB")
        }
    }
}

public enum NoraPluginPanelTone: String, Codable, Hashable, Sendable, CaseIterable {
    case neutral, success, warning, danger, purple, muted
}

public struct NoraPluginPanelStatus: Codable, Hashable, Sendable {
    public let title: String
    public let systemImage: String
    public let tone: NoraPluginPanelTone
    public let iconSVG: String?

    public init(title: String, systemImage: String, tone: NoraPluginPanelTone = .neutral, iconSVG: String? = nil) {
        self.title = title
        self.systemImage = systemImage
        self.tone = tone
        self.iconSVG = iconSVG
    }
}

public struct NoraPluginPanelAction: Codable, Hashable, Sendable, Identifiable {
    public let id: String
    public let title: String
    public let systemImage: String
    public let isPrimary: Bool
    public let isEnabled: Bool
    public let iconSVG: String?

    public init(id: String, title: String, systemImage: String, isPrimary: Bool = false, isEnabled: Bool = true, iconSVG: String? = nil) {
        self.id = id
        self.title = title
        self.systemImage = systemImage
        self.isPrimary = isPrimary
        self.isEnabled = isEnabled
        self.iconSVG = iconSVG
    }
}

public struct NoraPluginPanelActionInvocation: Codable, Hashable, Sendable {
    public let panelID: String
    public let actionID: String

    public init(panelID: String, actionID: String) {
        self.panelID = panelID
        self.actionID = actionID
    }

    public func validate() throws {
        try validatePanelIdentifier(panelID)
        try validatePanelActionIdentifier(actionID)
    }
}

public enum NoraPluginPanelKeyboardAction {
    public static let focusSearch = "focusSearch"
    public static let toggleSidebar = "toggleSidebar"
    public static let closeDetails = "closeDetails"
}

private func validatePanelIdentifier(_ value: String) throws {
    guard value.utf8.count <= 255,
          value.range(of: #"^[A-Za-z0-9][A-Za-z0-9-]*(?:\.[A-Za-z0-9][A-Za-z0-9-]*)+\z"#, options: .regularExpression) != nil else {
        throw NoraPluginValidationError.invalid("panel id", "must be a dot-separated identifier of at most 255 bytes")
    }
}

private func validatePanelActionIdentifier(_ value: String) throws {
    guard value.utf8.count <= 128,
          value.range(of: #"^[A-Za-z][A-Za-z0-9._-]*\z"#, options: .regularExpression) != nil else {
        throw NoraPluginValidationError.invalid("panel action id", "must be a simple identifier of at most 128 bytes")
    }
}

private func panelText(_ value: String, field: String, maximum: Int) throws {
    guard !value.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty, value.utf8.count <= maximum,
          !value.unicodeScalars.contains(where: { CharacterSet.controlCharacters.contains($0) }) else {
        throw NoraPluginValidationError.invalid(field, "must be nonempty plain text of at most \(maximum) bytes")
    }
}

private func panelIcon(_ value: String?) throws {
    guard let value else { return }
    guard value.utf8.count <= 16 * 1024 else {
        throw NoraPluginValidationError.invalid("panel icon", "exceeds 16 KiB")
    }
    try NoraPluginIconAsset.validate(data: Data(value.utf8), path: "icon.svg")
}
