import Foundation
import Testing
@testable import NoraPluginKit

@Suite("Native plugin detail views")
struct NoraPluginDetailTests {
    @Test("Details preserve a fixed heading, tab actions, and split body selection")
    func detailRoundTrip() throws {
        let view = NoraPluginView(type: .detail, id: "detail", children: [
            .init(type: .toolbar, children: [.init(type: .text, text: "example/repo · PR #42")]),
            .init(type: .column, children: [
                .init(type: .text, text: "Fix keyboard navigation", textStyle: .title),
                .init(type: .text, text: "author wants to merge 2 commits", textStyle: .caption),
            ]),
            tabs,
            .init(type: .splitView, action: "back", value: "detail", children: [
                .init(type: .column, children: [
                    .init(type: .itemRow, id: "check", text: "Build", action: "selectCheck", value: "selected",
                          children: [.init(type: .badge, text: "2m 14s")], subtitle: "macOS", symbol: "checkmark.circle", tone: .success),
                ]),
                .init(type: .column, children: [.init(type: .markdown, text: "## Build\nAll steps passed.")]),
            ]),
        ])
        let decoded = try NoraPluginView.decode(JSONEncoder().encode(view))
        #expect(decoded == view)
        #expect(decoded.children[2].children[0].id == "discussion")
        #expect(decoded.children[2].children[0].action == "showDiscussion")
        #expect(decoded.children[2].children[0].subtitle == "4")
        #expect(decoded.children[2].children[1].disabled)
        #expect(decoded.children[3].value == "detail")
        #expect(decoded.children[3].action == "back")
    }

    @Test("Details support multiple scrolling body blocks after the fixed header")
    func scrollingBody() throws {
        try NoraPluginView(type: .detail, children: [
            .init(type: .toolbar), .init(type: .column), tabs,
            .init(type: .markdown, text: "Description"),
            .init(type: .column, children: [.init(type: .textEditor, id: "comment")]),
        ]).validate()
    }

    @Test("Detail headers cannot be omitted or reordered")
    func malformedDetails() {
        for children: [NoraPluginView] in [
            [], [.init(type: .toolbar), .init(type: .column), tabs],
            [.init(type: .column), .init(type: .toolbar), tabs, .init(type: .column)],
            [.init(type: .toolbar), .init(type: .text), tabs, .init(type: .column)],
            [.init(type: .toolbar), .init(type: .column), .init(type: .row), .init(type: .column)],
        ] {
            #expect(throws: NoraPluginValidationError.self) { try NoraPluginView(type: .detail, children: children).validate() }
        }
    }

    @Test("Tab bars contain buttons with at most one selected tab")
    func tabShapes() throws {
        try tabs.validate()
        try NoraPluginView(type: .tabBar, children: [.init(type: .button, text: "Discussion", disabled: true)]).validate()
        for children: [NoraPluginView] in [
            [], [.init(type: .text, text: "Discussion")],
            [.init(type: .button, value: "arbitrary")],
            [.init(type: .button, value: "selected"), .init(type: .button, value: "selected")],
        ] {
            #expect(throws: NoraPluginValidationError.self) { try NoraPluginView(type: .tabBar, children: children).validate() }
        }
    }

    @Test("Split views require exactly two panes and explicit compact selection values")
    func splitShapes() throws {
        for value: String? in [nil, "list", "detail"] {
            try NoraPluginView(type: .splitView, text: "Back to files", action: "back", value: value,
                               children: [.init(type: .column), .init(type: .column)]).validate()
        }
        for children: [NoraPluginView] in [[], [.init(type: .column)], Array(repeating: .init(type: .column), count: 3)] {
            #expect(throws: NoraPluginValidationError.self) { try NoraPluginView(type: .splitView, children: children).validate() }
        }
        #expect(throws: NoraPluginValidationError.self) {
            try NoraPluginView(type: .splitView, value: "arbitrary", children: [.init(type: .column), .init(type: .column)]).validate()
        }
    }

    @Test("Read-only steps omit actions while selectable rows retain their exact action")
    func itemRowActions() throws {
        let row = NoraPluginView(type: .itemRow, text: "Install dependencies", children: [
            .init(type: .row, children: [.init(type: .text, text: "14s"), .init(type: .badge, text: "Passed")]),
        ], subtitle: "pnpm install", symbol: "checkmark.circle", tone: .success)
        let decoded = try NoraPluginView.decode(JSONEncoder().encode(row))
        #expect(decoded.action == nil)
        #expect(decoded == row)
        let selected = NoraPluginView(type: .itemRow, id: "job", text: "Build", action: "selectJob", value: "selected")
        #expect(try NoraPluginView.decode(JSONEncoder().encode(selected)).action == "selectJob")
        for action in ["", "  \n", String(repeating: "界", count: 86)] {
            for view in [NoraPluginView(type: .itemRow, action: action),
                         NoraPluginView(type: .splitView, action: action, children: [.init(type: .column), .init(type: .column)]),
                         NoraPluginView(type: .tabBar, children: [.init(type: .button, action: action)])] {
                #expect(throws: NoraPluginValidationError.self) { try view.validate() }
            }
        }
        try NoraPluginView(type: .itemRow, action: String(repeating: "界", count: 85) + "x").validate()
    }

    @Test("Item row metadata cannot create nested controls or hidden actions")
    func staticMetadata() {
        let invalid: [NoraPluginView] = [
            .init(type: .button, action: "post"), .init(type: .textField, id: "hiddenInput"), .init(type: .menu),
            .init(type: .itemRow, action: "nestedRow"), .init(type: .text, action: "hiddenAction"),
            .init(type: .row, children: [.init(type: .badge, action: "hiddenBadgeAction")]),
            .init(type: .column, items: [.init(type: .text)]),
            .init(type: .text, options: [.init(value: "hidden", text: "Hidden")]),
        ]
        for child in invalid {
            #expect(throws: NoraPluginValidationError.self) { try NoraPluginView(type: .itemRow, children: [child]).validate() }
        }
        #expect(throws: NoraPluginValidationError.self) { try NoraPluginView(type: .itemRow, value: "arbitrary").validate() }
        #expect(throws: NoraPluginValidationError.self) { try NoraPluginView(type: .itemRow, avatarBase64: "AA==").validate() }
    }

    @Test("New layouts reject item arrays rather than silently dropping content")
    func rejectItemArrays() {
        for view in [NoraPluginView(type: .detail, items: [.init(type: .text)], children: [
            .init(type: .toolbar), .init(type: .column), tabs, .init(type: .column),
        ]), NoraPluginView(type: .tabBar, items: [.init(type: .text)], children: [.init(type: .button)]),
        NoraPluginView(type: .splitView, items: [.init(type: .text)], children: [.init(type: .column), .init(type: .column)]),
        NoraPluginView(type: .itemRow, items: [.init(type: .text)])] {
            #expect(throws: NoraPluginValidationError.self) { try view.validate() }
        }
    }

    private var tabs: NoraPluginView {
        .init(type: .tabBar, children: [
            .init(type: .button, id: "discussion", text: "Discussion", action: "showDiscussion", value: "selected", subtitle: "4"),
            .init(type: .button, id: "checks", text: "Checks", action: "showChecks", disabled: true, subtitle: "8"),
        ])
    }
}
