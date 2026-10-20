import Foundation
import Testing
@testable import NoraPluginKit

@Suite("Native plugin manifest")
struct NoraPluginManifestTests {
    @Test("A complete manifest round trips without losing permissions or contributions")
    func roundTrip() throws {
        let manifest = fixture()
        try manifest.validate()
        #expect(try NoraPluginManifest.decode(JSONEncoder().encode(manifest)) == manifest)
    }

    @Test("The first schema and API version are defaults when absent")
    func defaultVersions() throws {
        let data = Data(#"{"id":"com.example.native","name":"Example","version":"1.0.0","summary":"Native example","publisher":"Example","extensionBundleIdentifier":"com.example.native.extension"}"#.utf8)
        let manifest = try NoraPluginManifest.decode(data)
        #expect(manifest.schemaVersion == 1)
        #expect(manifest.apiVersion == 1)
        #expect(manifest.permissions.isEmpty)
    }

    @Test("Semantic version syntax is strict", arguments: ["1", "v1.2.3", "01.2.3", "1.2.3-01", "1.2.3-", "1.2.3+", "1.2.3\n"])
    func rejectsVersions(_ version: String) {
        #expect(throws: NoraPluginValidationError.self) { try fixture(version: version).validate() }
    }

    @Test("Semantic version prereleases and build labels are accepted", arguments: ["0.0.0", "1.2.3", "1.2.3-0", "1.2.3-alpha.1+build.001"])
    func acceptsVersions(_ version: String) throws {
        try fixture(version: version).validate()
    }

    @Test("Contribution identifiers cannot claim another plugin namespace")
    func namespace() {
        #expect(throws: NoraPluginValidationError.self) {
            try fixture(panelID: "com.example.native-other.panel").validate()
        }
    }

    @Test("Panels and commands cannot reuse the same identifier")
    func duplicateContributions() {
        #expect(throws: NoraPluginValidationError.self) {
            try fixture(panelID: "com.example.native.echo").validate()
        }
    }

    @Test("Command parameters require an object schema", arguments: ["[]", "true", "{}", #"{"type":"array"}"#, "not JSON"])
    func rejectsSchemas(_ schema: String) {
        #expect(throws: NoraPluginValidationError.self) { try fixture(schema: schema).validate() }
    }

    @Test("Oversized command schemas are rejected before parsing")
    func schemaLimit() {
        let schema = #"{"type":"object","description":""# + String(repeating: "x", count: 65_536) + #""}"#
        #expect(throws: NoraPluginValidationError.self) { try fixture(schema: schema).validate() }
    }

    @Test("Only implemented permissions can be decoded")
    func unknownPermission() {
        #expect(throws: DecodingError.self) {
            try JSONDecoder().decode(NoraPluginPermission.self, from: Data(#""filesystem""#.utf8))
        }
    }

    @Test("An oversized manifest is rejected before decoding")
    func manifestLimit() {
        #expect(throws: NoraPluginValidationError.self) {
            try NoraPluginManifest.decode(Data(repeating: 0x20, count: NoraPluginManifest.maximumManifestBytes + 1))
        }
    }

    @Test("Native manifests reject script-era fields even when null", arguments: [
        #""runtime":"script""#, #""runtime":"nativeExtension""#, #""runtime":null"#,
        #""entryPoint":"plugin.js""#, #""entryPoint":null"#,
    ])
    func rejectsNonNativeFields(_ field: String) throws {
        let data = try JSONEncoder().encode(fixture())
        let json = String(decoding: data, as: UTF8.self)
        let changed = Data((String(json.dropLast()) + "," + field + "}").utf8)
        #expect(throws: NoraPluginValidationError.self) { try NoraPluginManifest.decode(changed) }
    }

    @Test("Only the native manifest schema and API version are accepted", arguments: [(2, 1), (1, 2), (0, 1)])
    func rejectsVersions(_ versions: (Int, Int)) throws {
        var value = try #require(JSONSerialization.jsonObject(with: JSONEncoder().encode(fixture())) as? [String: Any])
        value["schemaVersion"] = versions.0
        value["apiVersion"] = versions.1
        let data = try JSONSerialization.data(withJSONObject: value)
        #expect(throws: NoraPluginValidationError.self) { try NoraPluginManifest.decode(data) }
    }

    @Test("Native capabilities retain a bounded exact host allowlist")
    func nativeCapabilities() throws {
        func capable(_ domains: [String], permissions: [NoraPluginPermission] = NoraPluginPermission.allCases) -> NoraPluginManifest {
            NoraPluginManifest(id: "com.example.native", name: "Native", version: "1.0.0", summary: "Native capabilities",
                               publisher: "Example", extensionBundleIdentifier: "com.example.native.extension",
                               permissions: permissions, networkDomains: domains)
        }
        let valid = capable(["api.github.com", "github.com"])
        #expect(try NoraPluginManifest.decode(JSONEncoder().encode(valid)) == valid)
        for domains in [["*.github.com"], ["https://github.com"], ["GitHub.com"], ["github.com:443"], ["github.com\n"], ["github.com", "github.com"]] {
            #expect(throws: NoraPluginValidationError.self) { try capable(domains).validate() }
        }
        #expect(throws: NoraPluginValidationError.self) { try capable(["github.com"], permissions: [.chat]).validate() }
    }

    private func fixture(version: String = "1.0.0", panelID: String = "com.example.native.panel", schema: String = #"{"type":"object","properties":{"text":{"type":"string"}}}"#) -> NoraPluginManifest {
        NoraPluginManifest(id: "com.example.native", name: "Example", version: version,
                           summary: "Native example", publisher: "Example",
                           extensionBundleIdentifier: "com.example.native.extension", permissions: [.hostInfo],
                           panels: [NoraPluginPanel(id: panelID, title: "Example", systemImage: "puzzlepiece.extension")],
                           commands: [NoraPluginCommand(id: "com.example.native.echo", title: "Echo", description: "Returns text", inputSchemaJSON: schema, readOnly: true)])
    }
}
