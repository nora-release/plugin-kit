import Foundation
import Testing
@testable import NoraPluginKit

@Suite("Native plugin timeline items")
struct NoraPluginTimelineItemTests {
    @Test("A comment preserves the original author, controls, Markdown, and multiline draft")
    func commentRoundTrip() throws {
        let author = NoraPluginView(type: .button, id: "author", text: "example[bot]", action: "open-author",
                                    disabled: true, appearance: .plain, avatarBase64: "AA==")
        let metadata = NoraPluginView(type: .row, children: [
            .init(type: .text, text: "commented today", textStyle: .caption),
            .init(type: .spacer), .init(type: .button, text: "More", action: "comment-menu"),
        ])
        let body = NoraPluginView(type: .column, children: [
            .init(type: .markdown, text: "A **comment** with a [source](https://example.com)."),
            .init(type: .textEditor, id: "draft", value: "A reply\n\n第二行"),
            .init(type: .button, text: "Reply", action: "reply"),
        ])
        let item = NoraPluginView(type: .timelineItem, id: "comment", children: [author, metadata, body])
        let decoded = try NoraPluginView.decode(JSONEncoder().encode(item))
        #expect(decoded == item)
        #expect(decoded.value == nil)
        #expect(decoded.action == nil)
        #expect(decoded.children[0] == author)
        #expect(decoded.children[0].disabled)
        #expect(decoded.children[1].children[2].action == "comment-menu")
        #expect(decoded.children[2].children[1].value == "A reply\n\n第二行")
        #expect(decoded.children[2].children[2].action == "reply")
    }

    @Test("Events and comments support an omitted body and preserve explicit layout modes")
    func layoutModes() throws {
        for value: String? in [nil, "comment", "event"] {
            let item = NoraPluginView(type: .timelineItem, id: "activity", value: value,
                                      children: minimalChildren, disabled: true)
            let decoded = try NoraPluginView.decode(JSONEncoder().encode(item))
            #expect(decoded == item)
            #expect(decoded.children.count == 2)
            #expect(decoded.disabled)
        }
        let omitted = try NoraPluginView.decode(Data(#"{"type":"timelineItem","children":[{"type":"button","text":"Author"},{"type":"row"}]}"#.utf8))
        let explicitNull = try NoraPluginView.decode(Data(#"{"type":"timelineItem","value":null,"action":null,"children":[{"type":"button"},{"type":"row"}]}"#.utf8))
        #expect(omitted.value == nil && !omitted.disabled)
        #expect(explicitNull.value == nil && explicitNull.action == nil)
    }

    @Test("Author buttons retain their existing optional action and placeholder rules")
    func authorCompatibility() throws {
        for action: String? in [nil, "", "open-author"] {
            let author = NoraPluginView(type: .button, text: "Unlinked Git author", action: action,
                                        symbol: "person.crop.circle")
            let item = NoraPluginView(type: .timelineItem, children: [author, .init(type: .row)])
            #expect(try NoraPluginView.decode(JSONEncoder().encode(item)).children[0] == author)
        }
    }

    @Test("Timeline slots cannot be omitted, reordered, or silently discarded")
    func malformedSlots() {
        let author = minimalChildren[0], metadata = minimalChildren[1], body = NoraPluginView(type: .column)
        let invalid: [[NoraPluginView]] = [
            [], [author], [metadata, author], [.init(type: .text, text: "Author"), metadata],
            [author, body], [author, metadata, .init(type: .markdown, text: "Body")],
            [author, metadata, body, body],
        ]
        for children in invalid {
            #expect(throws: NoraPluginValidationError.self) {
                try NoraPluginView(type: .timelineItem, children: children).validate()
            }
        }
    }

    @Test("Container actions, item arrays, and unknown layout modes are rejected")
    func invalidContainerState() {
        for action in ["", "open-author", "reply"] {
            #expect(throws: NoraPluginValidationError.self) {
                try NoraPluginView(type: .timelineItem, action: action, children: minimalChildren).validate()
            }
        }
        for value in ["", "selected", "Event", "arbitrary"] {
            #expect(throws: NoraPluginValidationError.self) {
                try NoraPluginView(type: .timelineItem, value: value, children: minimalChildren).validate()
            }
        }
        #expect(throws: NoraPluginValidationError.self) {
            try NoraPluginView(type: .timelineItem, items: [.init(type: .text, text: "Lost content")],
                               children: minimalChildren).validate()
        }
        #expect(throws: NoraPluginValidationError.self) {
            try NoraPluginView(type: .timelineItem, children: minimalChildren, avatarBase64: "AA==").validate()
        }
    }

    @Test("Timeline descendants retain the shared avatar, identifier, and input limits")
    func descendantValidation() {
        let oversizedAuthor = NoraPluginView(type: .button, text: "Author",
                                             avatarBase64: Data(repeating: 0, count: NoraPluginView.maximumAvatarBytes + 1).base64EncodedString())
        #expect(throws: NoraPluginValidationError.self) {
            try NoraPluginView(type: .timelineItem, children: [oversizedAuthor, .init(type: .row)]).validate()
        }
        #expect(throws: NoraPluginValidationError.self) {
            try NoraPluginView(type: .timelineItem, id: "duplicate", children: [
                .init(type: .button, id: "duplicate"), .init(type: .row),
            ]).validate()
        }
        #expect(throws: NoraPluginValidationError.self) {
            try NoraPluginView(type: .timelineItem, children: minimalChildren + [
                .init(type: .column, children: [.init(type: .textEditor, value: String(repeating: "x", count: 64 * 1024 + 1))]),
            ]).validate()
        }
    }

    private var minimalChildren: [NoraPluginView] {
        [.init(type: .button, text: "Author", action: "open-author"), .init(type: .row)]
    }
}
