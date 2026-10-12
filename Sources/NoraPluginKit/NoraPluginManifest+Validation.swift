import Foundation

public enum NoraPluginValidationError: Error, LocalizedError, Equatable, Sendable {
    case invalid(String, String)

    public var errorDescription: String? {
        switch self {
        case let .invalid(field, reason): "Invalid plugin \(field): \(reason)."
        }
    }
}

extension NoraPluginManifest {
    public func validate() throws {
        guard apiVersion == 1,
              (schemaVersion == 1 && runtime == .nativeExtension) || (schemaVersion == 2 && runtime == .script) else {
            throw NoraPluginValidationError.invalid("version", "requires native schemaVersion 1 or script schemaVersion 2 with apiVersion 1")
        }
        try Self.identifier(id, field: "id")
        if let iconPath {
            try Self.relativeResourcePath(iconPath, field: "iconPath")
            guard ["svg", "png", "jpg", "jpeg"].contains((iconPath as NSString).pathExtension) else {
                throw NoraPluginValidationError.invalid("iconPath", "must name an SVG, PNG, or JPEG file inside the plugin")
            }
        }
        if isScript {
            guard extensionBundleIdentifier.isEmpty else {
                throw NoraPluginValidationError.invalid("extensionBundleIdentifier", "script plugins do not contain native extensions")
            }
            guard let entryPoint else { throw NoraPluginValidationError.invalid("entryPoint", "is required for script plugins") }
            try Self.scriptEntryPoint(entryPoint)
            try Self.networkHosts(networkDomains)
            guard networkDomains.isEmpty || permissions.contains(.network) else {
                throw NoraPluginValidationError.invalid("networkDomains", "requires the network permission")
            }
        } else {
            try Self.identifier(extensionBundleIdentifier, field: "extensionBundleIdentifier")
            guard entryPoint == nil, networkDomains.isEmpty,
                  permissions.allSatisfy({ [.hostInfo, .notifications].contains($0) }) else {
                throw NoraPluginValidationError.invalid("runtime", "script capabilities require a script plugin")
            }
        }
        try Self.text(name, field: "name", maximum: 128)
        try Self.text(publisher, field: "publisher", maximum: 128)
        try Self.text(summary, field: "summary", maximum: 2048)
        guard NoraPluginVersion(version) != nil else {
            throw NoraPluginValidationError.invalid("version", "must be a semantic version such as 1.0.0")
        }
        guard Set(permissions).count == permissions.count else {
            throw NoraPluginValidationError.invalid("permissions", "contains duplicates")
        }
        guard panels.count <= 32, commands.count <= 128 else {
            throw NoraPluginValidationError.invalid("contributions", "exceeds 32 panels or 128 commands")
        }
        var identifiers = Set<String>()
        for panel in panels {
            try validateContributionID(panel.id, seen: &identifiers)
            try Self.text(panel.title, field: "panel title", maximum: 128)
            try Self.text(panel.systemImage, field: "panel systemImage", maximum: 128)
        }
        for command in commands {
            try validateContributionID(command.id, seen: &identifiers)
            try Self.text(command.title, field: "command title", maximum: 128)
            try Self.text(command.description, field: "command description", maximum: 4096)
            guard command.inputSchemaJSON.utf8.count <= Self.maximumSchemaBytes else {
                throw NoraPluginValidationError.invalid("command schema", "exceeds 64 KiB")
            }
            guard let schema = try? JSONSerialization.jsonObject(with: Data(command.inputSchemaJSON.utf8)) as? [String: Any],
                  schema["type"] as? String == "object" else {
                throw NoraPluginValidationError.invalid("command schema", "must describe a JSON object")
            }
        }
        guard try JSONEncoder().encode(self).count <= Self.maximumManifestBytes else {
            throw NoraPluginValidationError.invalid("manifest", "exceeds 512 KiB")
        }
    }

    private static func scriptEntryPoint(_ value: String) throws {
        try relativeResourcePath(value, field: "entryPoint")
        guard value.hasSuffix(".js") else {
            throw NoraPluginValidationError.invalid("entryPoint", "must be a relative JavaScript file path inside the plugin")
        }
    }

    private static func relativeResourcePath(_ value: String, field: String) throws {
        let components = value.split(separator: "/", omittingEmptySubsequences: false)
        guard !value.isEmpty, value.utf8.count <= 1024,
              !value.contains("\\"), !value.unicodeScalars.contains(where: { CharacterSet.controlCharacters.contains($0) }),
              components.allSatisfy({ !$0.isEmpty && $0 != "." && $0 != ".." }) else {
            throw NoraPluginValidationError.invalid(field, "must be a relative resource path inside the plugin")
        }
    }

    private static func networkHosts(_ domains: [String]) throws {
        guard domains.count <= 64, Set(domains).count == domains.count else {
            throw NoraPluginValidationError.invalid("networkDomains", "contains duplicates or exceeds 64 hosts")
        }
        let pattern = #"^[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?(?:\.[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?)*$"#
        guard domains.allSatisfy({ $0.utf8.count <= 253 && $0.utf8.allSatisfy({ $0 == 45 || $0 == 46 || (48...57).contains($0) || (97...122).contains($0) })
            && $0.range(of: pattern, options: .regularExpression) != nil }) else {
            throw NoraPluginValidationError.invalid("networkDomains", "must contain exact lowercase hosts without schemes, ports, or wildcards")
        }
    }

    private func validateContributionID(_ value: String, seen: inout Set<String>) throws {
        try Self.identifier(value, field: "contribution id")
        guard value.hasPrefix(id + "."), seen.insert(value).inserted else {
            throw NoraPluginValidationError.invalid("contribution id", "must be unique and start with \(id).")
        }
    }

    private static func identifier(_ value: String, field: String) throws {
        let pattern = #"^[A-Za-z0-9][A-Za-z0-9-]*(?:\.[A-Za-z0-9][A-Za-z0-9-]*)+$"#
        guard value.utf8.count <= 255, value.range(of: pattern, options: .regularExpression) != nil,
              !value.contains(where: { $0.isWhitespace }) else {
            throw NoraPluginValidationError.invalid(field, "must be a dot-separated identifier")
        }
    }

    private static func text(_ value: String, field: String, maximum: Int) throws {
        guard !value.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty,
              value.utf8.count <= maximum, !value.unicodeScalars.contains(where: { CharacterSet.controlCharacters.contains($0) }) else {
            throw NoraPluginValidationError.invalid(field, "must be nonempty plain text of at most \(maximum) bytes")
        }
    }
}
