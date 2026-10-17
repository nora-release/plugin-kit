import Foundation
import Testing
@testable import NoraPluginKit

@Suite("Native plugin tab bar summaries")
struct NoraPluginTabBarSummaryTests {
    @Test("A static trailing row round-trips without changing tab identity or actions")
    func staticSummaryRoundTrip() throws {
        let summary = NoraPluginView(type: .row, id: "change-summary", children: [
            .init(type: .text, text: "45 files", textStyle: .caption),
            .init(type: .column, children: [
                .init(type: .row, children: [.init(type: .badge, text: "+3363", tone: .success)]),
                .init(type: .text, text: "−1373", tone: .danger),
            ]),
            .init(type: .spacer), .init(type: .divider), .init(type: .progress, text: "Loading"),
        ])
        let view = NoraPluginView(type: .tabBar, id: "tabs", items: [summary], children: buttons)
        let decoded = try NoraPluginView.decode(JSONEncoder().encode(view))
        #expect(decoded == view)
        #expect(decoded.items == [summary])
        #expect(decoded.children == buttons)
        #expect(decoded.children[0].value == "selected")
        #expect(decoded.children[1].disabled)
        #expect(decoded.children[1].action == "show-files")
    }

    @Test("Existing tab bars without a summary keep the original wire shape")
    func noSummary() throws {
        let data = Data(#"{"type":"tabBar","children":[{"type":"button","id":"discussion","action":"show-discussion","value":"selected"}]}"#.utf8)
        let decoded = try NoraPluginView.decode(data)
        #expect(decoded.items.isEmpty)
        #expect(decoded.children[0].action == "show-discussion")
        #expect(decoded.children[0].value == "selected")
        try tabBar(items: []).validate()
        try tabBar(items: [.init(type: .row)]).validate()
    }

    @Test("Only one root summary row is accepted")
    func summaryRootShape() {
        for items: [NoraPluginView] in [
            [.init(type: .text, text: "45 files")], [.init(type: .column)],
            [.init(type: .row), .init(type: .row)],
            [.init(type: .row, action: "hidden-action")],
            [.init(type: .row, options: [.init(value: "hidden", text: "Hidden")])],
            [.init(type: .row, items: [.init(type: .text, text: "Hidden")])],
        ] {
            #expect(throws: NoraPluginValidationError.self) { try tabBar(items: items).validate() }
        }
    }

    @Test("Summaries reject controls, actions, item arrays, and options at every depth")
    func summaryRejectsInteractiveContent() {
        let allowed: Set<NoraPluginView.Kind> = [.row, .column, .text, .badge, .progress, .spacer, .divider]
        for kind in NoraPluginView.Kind.allCases where !allowed.contains(kind) {
            let content = NoraPluginView(type: .row, children: [.init(type: .column, children: [.init(type: kind)])])
            #expect(throws: NoraPluginValidationError.self) { try tabBar(items: [content]).validate() }
        }
        for kind in allowed {
            for node in [NoraPluginView(type: kind, action: "hidden-action"),
                         NoraPluginView(type: kind, options: [.init(value: "hidden", text: "Hidden")]),
                         NoraPluginView(type: kind, items: [.init(type: .text, text: "Hidden")]),
                         NoraPluginView(type: kind, loadMoreAction: "load", loadMoreToken: "page")] {
                let content = NoraPluginView(type: .row, children: [.init(type: .column, children: [node])])
                #expect(throws: NoraPluginValidationError.self) { try tabBar(items: [content]).validate() }
            }
        }
    }

    @Test("Summary nodes retain the global text, identifier, node, depth, and byte limits")
    func summaryBounds() {
        var deep = NoraPluginView(type: .text, text: "Nested")
        for _ in 0..<NoraPluginView.maximumDepth { deep = .init(type: .row, children: [deep]) }
        let invalid = [
            NoraPluginView(type: .row, children: [.init(type: .text, text: String(repeating: "x", count: 64 * 1024 + 1))]),
            .init(type: .row, id: "discussion"),
            .init(type: .row, children: Array(repeating: .init(type: .text), count: NoraPluginView.maximumNodes)),
            deep,
            .init(type: .row, children: Array(repeating: .init(type: .text, text: String(repeating: "x", count: 64 * 1024)), count: 9)),
        ]
        for row in invalid {
            #expect(throws: NoraPluginValidationError.self) { try tabBar(items: [row]).validate() }
        }
    }

    @Test("A summary cannot weaken the original tab selection and action validation")
    func tabValidation() {
        let summary = NoraPluginView(type: .row, children: [.init(type: .text, text: "+1")])
        for children: [NoraPluginView] in [
            [], [.init(type: .text)], [.init(type: .button, value: "arbitrary")],
            [.init(type: .button, value: "selected"), .init(type: .button, value: "selected")],
            [.init(type: .button, action: "  \n")],
        ] {
            #expect(throws: NoraPluginValidationError.self) {
                try NoraPluginView(type: .tabBar, items: [summary], children: children).validate()
            }
        }
    }

    private func tabBar(items: [NoraPluginView]) -> NoraPluginView {
        .init(type: .tabBar, items: items, children: buttons)
    }

    private var buttons: [NoraPluginView] {
        [
            .init(type: .button, id: "discussion", text: "Discussion", action: "show-discussion", value: "selected", subtitle: "0"),
            .init(type: .button, id: "files", text: "Files", action: "show-files", disabled: true, subtitle: "45"),
        ]
    }
}
