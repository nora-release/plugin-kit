import Foundation

public struct NoraPluginManifest: Codable, Hashable, Sendable, Identifiable {
    public static let extensionPointIdentifier = "com.nora.plugin.native"
    public static let relativeManifestPath = "Contents/Resources/nora-plugin.json"
    public static let maximumManifestBytes = 512 * 1024
    public static let maximumSchemaBytes = 64 * 1024
    public static let maximumIconBytes = 512 * 1024

    public let schemaVersion: Int
    public let id: String
    public let name: String
    public let version: String
    public let summary: String
    public let publisher: String
    public let extensionBundleIdentifier: String
    public let permissions: [NoraPluginPermission]
    public let panels: [NoraPluginPanel]
    public let commands: [NoraPluginCommand]
    public let apiVersion: Int
    public let networkDomains: [String]
    public let iconPath: String?

    public init(schemaVersion: Int = 1, id: String, name: String, version: String, summary: String,
                publisher: String, extensionBundleIdentifier: String,
                permissions: [NoraPluginPermission] = [], panels: [NoraPluginPanel] = [],
                commands: [NoraPluginCommand] = [], apiVersion: Int = 1,
                networkDomains: [String] = [], iconPath: String? = nil) {
        self.schemaVersion = schemaVersion
        self.id = id
        self.name = name
        self.version = version
        self.summary = summary
        self.publisher = publisher
        self.extensionBundleIdentifier = extensionBundleIdentifier
        self.permissions = permissions
        self.panels = panels
        self.commands = commands
        self.apiVersion = apiVersion
        self.networkDomains = networkDomains
        self.iconPath = iconPath
    }

    public init(from decoder: any Decoder) throws {
        let fields = try decoder.container(keyedBy: ManifestField.self)
        if let field = fields.allKeys.first(where: { ["runtime", "entryPoint"].contains($0.stringValue) }) {
            throw NoraPluginValidationError.invalid(field.stringValue, "is not supported by native plugins")
        }
        let values = try decoder.container(keyedBy: CodingKeys.self)
        schemaVersion = try values.decodeIfPresent(Int.self, forKey: .schemaVersion) ?? 1
        id = try values.decode(String.self, forKey: .id)
        name = try values.decode(String.self, forKey: .name)
        version = try values.decode(String.self, forKey: .version)
        summary = try values.decode(String.self, forKey: .summary)
        publisher = try values.decode(String.self, forKey: .publisher)
        extensionBundleIdentifier = try values.decode(String.self, forKey: .extensionBundleIdentifier)
        permissions = try values.decodeIfPresent([NoraPluginPermission].self, forKey: .permissions) ?? []
        panels = try values.decodeIfPresent([NoraPluginPanel].self, forKey: .panels) ?? []
        commands = try values.decodeIfPresent([NoraPluginCommand].self, forKey: .commands) ?? []
        apiVersion = try values.decodeIfPresent(Int.self, forKey: .apiVersion) ?? 1
        networkDomains = try values.decodeIfPresent([String].self, forKey: .networkDomains) ?? []
        iconPath = try values.decodeIfPresent(String.self, forKey: .iconPath)
    }

    public static func decode(_ data: Data) throws -> Self {
        guard data.count <= maximumManifestBytes else {
            throw NoraPluginValidationError.invalid("manifest", "exceeds 512 KiB")
        }
        let manifest = try JSONDecoder().decode(Self.self, from: data)
        try manifest.validate()
        return manifest
    }
}

private struct ManifestField: CodingKey {
    var stringValue: String
    var intValue: Int? { nil }
    init?(stringValue: String) { self.stringValue = stringValue }
    init?(intValue: Int) { return nil }
}

public enum NoraPluginPermission: String, Codable, Hashable, Sendable, CaseIterable {
    case hostInfo
    case notifications
    case network
    case credentials
    case openExternal
    case chat
}

public struct NoraPluginPanel: Codable, Hashable, Sendable, Identifiable {
    public let id: String
    public let title: String
    public let systemImage: String

    public init(id: String, title: String, systemImage: String) {
        self.id = id
        self.title = title
        self.systemImage = systemImage
    }
}

public struct NoraPluginCommand: Codable, Hashable, Sendable, Identifiable {
    public let id: String
    public let title: String
    public let description: String
    public let inputSchemaJSON: String
    public let readOnly: Bool

    public init(id: String, title: String, description: String, inputSchemaJSON: String, readOnly: Bool) {
        self.id = id
        self.title = title
        self.description = description
        self.inputSchemaJSON = inputSchemaJSON
        self.readOnly = readOnly
    }
}
