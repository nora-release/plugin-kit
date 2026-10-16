import Foundation
import Testing
@testable import NoraPluginKit

@Suite("Native plugin detail summary asides")
struct NoraPluginDetailAsideTests {
    @Test("A summary aside preserves content, avatars, state, and exact actions through the codec")
    func asideRoundTrip() throws {
        let aside = NoraPluginView(type: .column, id: "summary", children: [
            .init(type: .text, text: "Reviewers", textStyle: .caption, tone: .muted),
            .init(type: .button, id: "reviewer", text: "Reviewer", action: "open-reviewer", disabled: true,
                  appearance: .plain, avatarBase64: "AA=="),
            .init(type: .divider),
            .init(type: .itemRow, id: "build", text: "Build", action: "show-checks", children: [
                .init(type: .badge, text: "2m 14s"),
            ], symbol: "checkmark.circle", tone: .success),
            .init(type: .menu, text: "Merge options", children: [
                .init(type: .button, text: "Squash", action: "prepare-squash"),
            ]),
            .init(type: .picker, id: "strategy", value: "squash", options: [.init(value: "squash", text: "Squash")]),
            .init(type: .row, children: [.init(type: .spacer), .init(type: .progress, text: "Loading")]),
            .init(type: .emptyState, text: "No labels"),
            .init(type: .markdown, text: "**2** resolved threads"),
        ])
        let view = detail(items: [aside])
        let decoded = try NoraPluginView.decode(JSONEncoder().encode(view))
        #expect(decoded == view)
        #expect(decoded.items.count == 1)
        #expect(decoded.items[0].children[1].avatarBase64 == "AA==")
        #expect(decoded.items[0].children[1].disabled)
        #expect(decoded.items[0].children[3].action == "show-checks")
        #expect(decoded.items[0].children[4].children[0].action == "prepare-squash")
        #expect(decoded.children[2].children[0].action == "show-discussion")
    }

    @Test("Existing details with absent or empty items keep the original children contract")
    func existingDetailCompatibility() throws {
        let source = #"{"type":"detail","children":[{"type":"toolbar"},{"type":"column"},{"type":"tabBar","children":[{"type":"button","action":"show-discussion"}]},{"type":"column"}]}"#
        let decoded = try NoraPluginView.decode(Data(source.utf8))
        #expect(decoded.items.isEmpty)
        #expect(try NoraPluginView.decode(JSONEncoder().encode(decoded)) == decoded)
        try detail(items: []).validate()
        try detail(items: [.init(type: .column)]).validate()
    }

    @Test("An aside is at most one column and rejects full-size layouts or input controls anywhere")
    func boundedAsideShapes() {
        #expect(throws: NoraPluginValidationError.self) {
            try detail(items: [.init(type: .column), .init(type: .column)]).validate()
        }
        for type in NoraPluginView.Kind.allCases where type != .column {
            #expect(throws: NoraPluginValidationError.self) { try detail(items: [.init(type: type)]).validate() }
        }
        let allowed: Set<NoraPluginView.Kind> = [.column, .row, .text, .button, .picker, .menu, .divider,
                                                .spacer, .badge, .progress, .emptyState, .markdown, .itemRow]
        for type in NoraPluginView.Kind.allCases where !allowed.contains(type) {
            let aside = NoraPluginView(type: .column, children: [.init(type: .row, children: [.init(type: type)])])
            #expect(throws: NoraPluginValidationError.self) { try detail(items: [aside]).validate() }
        }
    }

    @Test("No summary descendant can hide content in an ignored item array")
    func nestedItems() {
        for type: NoraPluginView.Kind in [.column, .row, .text, .button, .picker, .menu, .divider,
                                         .spacer, .badge, .progress, .emptyState, .markdown, .itemRow] {
            let aside = NoraPluginView(type: .column, children: [.init(type: type, items: [.init(type: .text, text: "Hidden")])])
            #expect(throws: NoraPluginValidationError.self) { try detail(items: [aside]).validate() }
        }
    }

    @Test("Normal recursive validation still applies to summary metadata and labels")
    func recursiveValidation() {
        for child in [NoraPluginView(type: .itemRow, children: [.init(type: .button, action: "hidden")]),
                      .init(type: .button, avatarBase64: "not base64"),
                      .init(type: .text, text: String(repeating: "x", count: 64 * 1024 + 1))] {
            #expect(throws: NoraPluginValidationError.self) {
                try detail(items: [.init(type: .column, children: [child])]).validate()
            }
        }
        let duplicate = NoraPluginView(type: .column, id: "main", children: [.init(type: .button, id: "summary-control")])
        #expect(throws: NoraPluginValidationError.self) { try detail(items: [duplicate]).validate() }
        #expect(throws: NoraPluginValidationError.self) {
            try detail(items: [.init(type: .column, children: [
                .init(type: .button, id: "summary-control"), .init(type: .button, id: "summary-control"),
            ])]).validate()
        }
    }

    @Test("Summary descendants share the complete detail's node depth and byte budgets")
    func sharedBudgets() throws {
        let atLimit = NoraPluginView(type: .column, children: Array(repeating: .init(type: .text), count: 993))
        try detail(items: [atLimit]).validate()
        let tooBroad = NoraPluginView(type: .column, children: Array(repeating: .init(type: .text), count: 994))
        #expect(throws: NoraPluginValidationError.self) { try detail(items: [tooBroad]).validate() }
        var nested = NoraPluginView(type: .text, text: "Deep")
        for _ in 0..<NoraPluginView.maximumDepth { nested = .init(type: .column, children: [nested]) }
        #expect(throws: NoraPluginValidationError.self) { try detail(items: [.init(type: .column, children: [nested])]).validate() }
        let tooLarge = NoraPluginView(type: .column, children: Array(repeating: .init(type: .text, text: String(repeating: "x", count: 64 * 1024)), count: 9))
        #expect(throws: NoraPluginValidationError.self) { try detail(items: [tooLarge]).validate() }
    }

    private func detail(items: [NoraPluginView]) -> NoraPluginView {
        .init(type: .detail, items: items, children: [
            .init(type: .toolbar), .init(type: .column),
            .init(type: .tabBar, children: [.init(type: .button, action: "show-discussion")]),
            .init(type: .column, id: "main"),
        ])
    }
}
