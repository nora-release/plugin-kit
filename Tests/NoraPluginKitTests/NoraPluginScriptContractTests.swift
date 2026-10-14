import Foundation
import Testing
@testable import NoraPluginKit

@Suite("Script plugin contract")
struct NoraPluginScriptContractTests {
    @Test("Script plugins decode without an Apple application or extension identifier")
    func scriptManifest() throws {
        let data = Data(#"{"schemaVersion":2,"apiVersion":1,"runtime":"script","entryPoint":"plugin.js","id":"com.example.script","name":"Script","version":"1.0.0","summary":"Native script panel","publisher":"Example","permissions":["network","credentials","openExternal"],"networkDomains":["api.github.com","github.com"]}"#.utf8)
        let manifest = try NoraPluginManifest.decode(data)
        #expect(manifest.isScript)
        #expect(manifest.extensionBundleIdentifier.isEmpty)
        #expect(try NoraPluginManifest.decode(JSONEncoder().encode(manifest)) == manifest)
    }

    @Test("Entry points cannot leave their data package", arguments: ["", "../plugin.js", "/plugin.js", "assets/../plugin.js", "assets//plugin.js", "assets\\plugin.js", "plugin.swift", "plugin.js\n"])
    func rejectsEntryPoints(_ path: String) {
        #expect(throws: NoraPluginValidationError.self) { try manifest(entryPoint: path).validate() }
    }

    @Test("Network access requires exact declared hosts", arguments: ["https://api.github.com", "*.github.com", "github.com:443", "GitHub.com", "github.com/", "api..github.com"])
    func rejectsNetworkDomains(_ host: String) {
        #expect(throws: NoraPluginValidationError.self) { try manifest(networkDomains: [host]).validate() }
    }

    @Test("Declaring hosts requires network permission")
    func rejectsUnapprovedNetwork() {
        #expect(throws: NoraPluginValidationError.self) { try manifest(permissions: []).validate() }
    }

    @Test("Native plugins retain their original manifest contract")
    func nativeCompatibility() throws {
        let native = NoraPluginManifest(id: "com.example.native", name: "Native", version: "1.0.0", summary: "Native panel",
                                        publisher: "Example", extensionBundleIdentifier: "com.example.native.extension")
        try native.validate()
        #expect(!native.isScript)
        #expect(throws: NoraPluginValidationError.self) { try manifest(schemaVersion: 1).validate() }
    }

    @Test("Native view descriptors provide defaults for omitted JSON properties")
    func viewDefaults() throws {
        let view = try NoraPluginView.decode(Data(#"{"type":"column","children":[{"type":"text","text":"Hello"},{"type":"picker","id":"repo","value":"nora","options":[{"value":"nora","text":"Nora"}]}]}"#.utf8))
        #expect(view.children.count == 2)
        #expect(view.items.isEmpty)
        #expect(!view.disabled)
        #expect(view.children[1].options[0].value == "nora")
        #expect(try NoraPluginView.decode(JSONEncoder().encode(view)) == view)
    }

    @Test("Native view inputs have unique identifiers")
    func duplicateViewIdentifiers() {
        let view = NoraPluginView(type: .column, children: [
            .init(type: .textField, id: "value"), .init(type: .secureField, id: "value"),
        ])
        #expect(throws: NoraPluginValidationError.self) { try view.validate() }
    }

    @Test("Multiline editors preserve draft text and omitted submission actions")
    func multilineEditor() throws {
        let text = "First line\n\n第二行\r\n\t```swift\nlet value = 1\n```\n"
        let editor = NoraPluginView(type: .textEditor, id: "draft", text: "Comment", placeholder: "Write a comment", value: text)
        let decoded = try NoraPluginView.decode(JSONEncoder().encode(editor))
        #expect(decoded == editor)
        #expect(decoded.value == text)
        #expect(decoded.action == nil)
        #expect(!decoded.disabled)
        let empty = try NoraPluginView.decode(Data(#"{"type":"textEditor","id":"draft","value":"","disabled":true}"#.utf8))
        #expect(empty.value == "")
        #expect(empty.disabled)
        #expect(empty.action == nil)
    }

    @Test("Multiline drafts allow 64 KiB UTF-8 while single-line values retain their limit")
    func multilineEditorValueLimit() throws {
        let maximum = String(repeating: "界", count: 21_845) + "\n"
        #expect(maximum.utf8.count == NoraPluginView.maximumTextEditorValueBytes)
        try NoraPluginView(type: .textEditor, value: maximum).validate()
        #expect(throws: NoraPluginValidationError.self) {
            try NoraPluginView(type: .textEditor, value: maximum + "x").validate()
        }
        try NoraPluginView(type: .textField, value: String(repeating: "x", count: 4096)).validate()
        #expect(throws: NoraPluginValidationError.self) {
            try NoraPluginView(type: .textField, value: String(repeating: "x", count: 4097)).validate()
        }
    }

    @Test("UI depth and node limits are enforced")
    func viewLimits() {
        var view = NoraPluginView(type: .text, text: "Deep")
        for _ in 0..<NoraPluginView.maximumDepth { view = .init(type: .column, children: [view]) }
        #expect(throws: NoraPluginValidationError.self) { try view.validate() }
        let broad = NoraPluginView(type: .list, items: Array(repeating: .init(type: .text, text: "Row"), count: NoraPluginView.maximumNodes))
        #expect(throws: NoraPluginValidationError.self) { try broad.validate() }
    }

    @Test("SDK bootstrap is loaded from its shared JavaScript author implementation")
    func bootstrapResource() {
        #expect(NoraPluginScriptAPI.bootstrapSource.hasPrefix("(function createNoraPluginAPI("))
        #expect(NoraPluginScriptAPI.bootstrapSource.contains("registerPanel"))
        #expect(NoraPluginScriptAPI.bootstrapSource.contains("invokeCommand"))
        for kind in NoraPluginView.Kind.allCases { #expect(NoraPluginScriptAPI.bootstrapSource.contains("'\(kind.rawValue)'")) }
    }

    private func manifest(schemaVersion: Int = 2, entryPoint: String = "plugin.js",
                          networkDomains: [String] = ["api.github.com"], permissions: [NoraPluginPermission] = [.network]) -> NoraPluginManifest {
        NoraPluginManifest(schemaVersion: schemaVersion, id: "com.example.script", name: "Script", version: "1.0.0",
                           summary: "Script panel", publisher: "Example", permissions: permissions,
                           runtime: .script, entryPoint: entryPoint, networkDomains: networkDomains)
    }
}
