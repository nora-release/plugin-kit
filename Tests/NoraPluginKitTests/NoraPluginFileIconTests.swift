import Foundation
import Testing
@testable import NoraPluginKit

@Suite("Native file icon hints")
struct NoraPluginFileIconTests {
    @Test("File rows preserve Unicode paths, fallback icons, state, metadata, and exact actions")
    func roundTrip() throws {
        for path in ["package.json", "pnpm-lock.yaml", ".gitignore", "src/设置/🦊.swift", "unknown.ext",
                     "/private/not-a-real-file.json", "../package.json", "https://example.invalid/file.yaml"] {
            let row = NoraPluginView(type: .itemRow, id: "file", text: "Filename", action: "select-file", value: "selected",
                                     children: [.init(type: .badge, text: "+1")], disabled: true, subtitle: "Nested path",
                                     symbol: "doc", tone: .muted, iconSVG: "<svg/>", fileIconPath: path)
            let decoded = try NoraPluginView.decode(JSONEncoder().encode(row))
            #expect(decoded == row)
            #expect(decoded.fileIconPath == path)
            #expect(decoded.action == "select-file")
            #expect(decoded.symbol == "doc")
            #expect(decoded.children[0].text == "+1")
        }
    }

    @Test("Legacy or null fields retain the original icon behavior")
    func legacyDefaults() throws {
        for json in [#"{"type":"itemRow","symbol":"doc"}"#,
                     #"{"type":"itemRow","symbol":"doc","fileIconPath":null}"#] {
            let row = try NoraPluginView.decode(Data(json.utf8))
            #expect(row.fileIconPath == nil)
            #expect(row.symbol == "doc")
            let encoded = try #require(JSONSerialization.jsonObject(with: JSONEncoder().encode(row)) as? [String: Any])
            #expect(encoded["fileIconPath"] == nil)
        }
    }

    @Test("File icon hints measure the exact 4 KiB UTF-8 boundary")
    func byteLimits() throws {
        #expect(NoraPluginView.maximumFileIconPathBytes == 4096)
        for path in [String(repeating: "a", count: 4096), String(repeating: "界", count: 1365) + "x",
                     String(repeating: "🦊", count: 1024)] {
            #expect(path.utf8.count == NoraPluginView.maximumFileIconPathBytes)
            try NoraPluginView(type: .itemRow, fileIconPath: path).validate()
            #expect(throws: NoraPluginValidationError.self) {
                try NoraPluginView(type: .itemRow, fileIconPath: path + "x").validate()
            }
        }
        #expect(throws: NoraPluginValidationError.self) { try NoraPluginView(type: .itemRow, fileIconPath: "").validate() }
    }

    @Test("Filename hints reject controls, invisible formatting, and line separators", arguments: [
        "\n", "\r", "\t", "\0", "\u{7f}", "\u{85}", "\u{200b}", "\u{200d}", "\u{202e}", "\u{2066}", "\u{2028}", "\u{2029}",
    ])
    func singleLine(_ scalar: String) {
        #expect(throws: NoraPluginValidationError.self) {
            try NoraPluginView(type: .itemRow, fileIconPath: "file" + scalar + ".json").validate()
        }
    }

    @Test("Only itemRow accepts file icon hints")
    func rejectsOtherKinds() {
        for kind in NoraPluginView.Kind.allCases where kind != .itemRow {
            #expect(throws: NoraPluginValidationError.self) {
                try NoraPluginView(type: kind, fileIconPath: "package.json").validate()
            }
        }
    }

    @Test("Wire hints reject non-string values", arguments: ["42", "true", "[]", "{}"])
    func wireType(_ value: String) {
        let json = #"{"type":"itemRow","fileIconPath":"# + value + "}"
        #expect(throws: DecodingError.self) { try NoraPluginView.decode(Data(json.utf8)) }
    }

    @Test("File icons keep native row and aside content restrictions")
    func layoutRestrictions() throws {
        let row = NoraPluginView(type: .itemRow, fileIconPath: "package.json")
        let aside = NoraPluginView(type: .column, children: [row])
        let tabs = NoraPluginView(type: .tabBar, children: [.init(type: .button, action: "discussion")])
        try NoraPluginView(type: .detail, items: [aside], children: [
            .init(type: .toolbar), .init(type: .column), tabs, .init(type: .column),
        ]).validate()
        #expect(throws: NoraPluginValidationError.self) {
            try NoraPluginView(type: .itemRow, children: [.init(type: .button, action: "hidden")], fileIconPath: "package.json").validate()
        }
        #expect(throws: NoraPluginValidationError.self) {
            try NoraPluginView(type: .tabBar, items: [.init(type: .row, children: [row])], children: tabs.children).validate()
        }
        #expect(throws: NoraPluginValidationError.self) {
            try NoraPluginView(type: .detail, items: [.init(type: .column, children: [.init(type: .itemRow, items: [row])])],
                               children: [.init(type: .toolbar), .init(type: .column), tabs, .init(type: .column)]).validate()
        }
    }
}
