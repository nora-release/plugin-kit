import Foundation
import Testing
@testable import NoraPluginKit

@Suite("Native plugin workspaces")
struct NoraPluginWorkspaceTests {
    @Test("A native workspace preserves selection, metadata, toolbar controls, and inspector content")
    func workspaceRoundTrip() throws {
        let workspace = NoraPluginView(type: .workspace, id: "workspace", action: "closeDetail", children: [
            .init(type: .sidebar, id: "repositories", value: "repo-1", items: [
                .init(type: .navigationItem, id: "repo-1", text: "Repository", action: "selectRepository", symbol: "folder", tone: .accent),
            ]),
            .init(type: .column, children: [
                .init(type: .toolbar, children: [
                    .init(type: .text, text: "Issues", textStyle: .headline),
                    .init(type: .textField, id: "search", placeholder: "Search issues", action: "search"),
                    .init(type: .menu, text: "More", children: [
                        .init(type: .button, text: "Refresh", action: "refresh", symbol: "arrow.clockwise", appearance: .plain),
                    ], symbol: "ellipsis"),
                ]),
                .init(type: .collection, id: "issues", action: "peekItem", value: "issue-42", items: [
                    .init(type: .collectionItem, id: "issue-42", text: "Fix keyboard navigation", action: "selectItem", children: [
                        .init(type: .text, text: "Author", textStyle: .caption, tone: .muted),
                        .init(type: .badge, text: "Bug", tone: .danger),
                    ], subtitle: "#42", symbol: "circle", tone: .success),
                ]),
            ]),
            .init(type: .column, children: [
                .init(type: .text, text: "Fix keyboard navigation", textStyle: .title),
                .init(type: .markdown, text: "## Description\nKeyboard selection should preserve **focus**."),
                .init(type: .button, text: "Open", action: "openItem", symbol: "arrow.up.right", appearance: .primary),
            ]),
        ])
        let decoded = try NoraPluginView.decode(JSONEncoder().encode(workspace))
        #expect(decoded == workspace)
        #expect(decoded.children[1].children[1].value == "issue-42")
        #expect(decoded.children[1].children[1].items[0].subtitle == "#42")
        #expect(decoded.children[2].children[1].type == .markdown)
    }

    @Test("Existing view JSON retains its original defaults")
    func backwardCompatibility() throws {
        let view = try NoraPluginView.decode(Data(#"{"type":"column","children":[{"type":"button","text":"Open","action":"open"}]}"#.utf8))
        #expect(view.subtitle == nil)
        #expect(view.symbol == nil)
        #expect(view.textStyle == nil)
        #expect(view.tone == nil)
        #expect(view.appearance == nil)
        #expect(view.avatarBase64 == nil)
        #expect(view.children[0].appearance == nil)
        #expect(try NoraPluginView.decode(JSONEncoder().encode(view)) == view)
    }

    @Test("Workspaces also support main content alone and an optional sidebar or inspector")
    func workspaceLayouts() throws {
        for children: [NoraPluginView] in [
            [.init(type: .column)],
            [.init(type: .sidebar), .init(type: .column)],
            [.init(type: .column), .init(type: .column)],
        ] {
            try NoraPluginView(type: .workspace, children: children).validate()
        }
    }

    @Test("Workspaces cannot ambiguously reorder panes or omit main content")
    func rejectsMalformedLayouts() {
        for children: [NoraPluginView] in [
            [], [.init(type: .sidebar)],
            [.init(type: .column), .init(type: .sidebar)],
            [.init(type: .sidebar), .init(type: .column), .init(type: .sidebar)],
            [.init(type: .column), .init(type: .column), .init(type: .column)],
        ] {
            #expect(throws: NoraPluginValidationError.self) { try NoraPluginView(type: .workspace, children: children).validate() }
        }
        #expect(throws: NoraPluginValidationError.self) {
            try NoraPluginView(type: .workspace, items: [.init(type: .text)], children: [.init(type: .column)]).validate()
        }
    }

    @Test("Visual styles are fixed native choices and do not decode arbitrary CSS")
    func typedStyles() throws {
        for style in NoraPluginView.TextStyle.allCases {
            try NoraPluginView(type: .text, text: "Text", textStyle: style).validate()
        }
        for tone in NoraPluginView.Tone.allCases {
            try NoraPluginView(type: .badge, text: "Status", tone: tone).validate()
        }
        for appearance in NoraPluginView.Appearance.allCases {
            try NoraPluginView(type: .button, text: "Button", appearance: appearance).validate()
        }
        for field in ["textStyle", "tone", "appearance"] {
            let json = #"{"type":"text","text":"Hello",""# + field + #"":"arbitrary-css"}"#
            #expect(throws: DecodingError.self) { try NoraPluginView.decode(Data(json.utf8)) }
        }
    }

    @Test("Purple is a native tone that round trips without changing descriptor content")
    func purpleToneRoundTrip() throws {
        for kind: NoraPluginView.Kind in [.text, .badge, .itemRow] {
            let view = NoraPluginView(type: kind, text: "Merged", symbol: "arrow.triangle.merge", tone: .purple)
            let encoded = try JSONEncoder().encode(view)
            let decoded = try NoraPluginView.decode(encoded)
            #expect(decoded == view)
            #expect(decoded.tone?.rawValue == "purple")
        }
        let json = Data(#"{"type":"badge","text":"Merged","tone":"purple"}"#.utf8)
        #expect(try NoraPluginView.decode(json).tone == .purple)
    }

    @Test("Additional text and symbols respect descriptor limits")
    func newPropertyLimits() {
        #expect(throws: NoraPluginValidationError.self) {
            try NoraPluginView(type: .collectionItem, subtitle: String(repeating: "界", count: 1366)).validate()
        }
        for symbol in ["", "folder\n", String(repeating: "x", count: 129)] {
            #expect(throws: NoraPluginValidationError.self) { try NoraPluginView(type: .navigationItem, symbol: symbol).validate() }
        }
    }
}
