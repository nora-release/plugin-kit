import Foundation
import Testing
@testable import NoraPluginKit

@Suite("Native sidebar content transition contract")
struct NoraPluginSidebarContentTransitionTests {
    @Test("Committed list category survives encoding without changing selection, header controls, or pagination")
    func roundTrip() throws {
        let sidebar = NoraPluginView(type: .sidebar, id: "items", value: "item-42", items: [
            .init(type: .navigationItem, id: "item-42", text: "Keyboard navigation", action: "selectItem"),
        ], children: [
            .init(type: .picker, id: "category", value: "reviews", options: [
                .init(value: "tasks", text: "Tasks"), .init(value: "reviews", text: "Reviews"),
            ]),
        ], loadMoreAction: "nextPage", loadMoreToken: "reviews:2", contentTransitionKey: "reviews")
        let workspace = NoraPluginView(type: .workspace, children: [sidebar, .init(type: .column)])
        let decoded = try NoraPluginView.decode(JSONEncoder().encode(workspace))
        #expect(decoded == workspace)
        #expect(decoded.children[0].id == "items")
        #expect(decoded.children[0].value == "item-42")
        #expect(decoded.children[0].contentTransitionKey == "reviews")
        #expect(decoded.children[0].children[0].value == "reviews")
        #expect(decoded.children[0].loadMoreToken == "reviews:2")
        #expect(decoded.children[0].items[0].action == "selectItem")
    }

    @Test("Omitted or null transition keys keep the legacy sidebar representation")
    func defaults() throws {
        for json in [#"{"type":"sidebar","items":[]}"#,
                     #"{"type":"sidebar","contentTransitionKey":null}"#] {
            let sidebar = try NoraPluginView.decode(Data(json.utf8))
            #expect(sidebar.contentTransitionKey == nil)
            let object = try #require(JSONSerialization.jsonObject(with: JSONEncoder().encode(sidebar)) as? [String: Any])
            #expect(object["contentTransitionKey"] == nil)
        }
    }

    @Test("Transition keys are bounded by UTF-8 bytes")
    func byteLimits() throws {
        for key in [String(repeating: "x", count: 256), String(repeating: "é", count: 128),
                    String(repeating: "界", count: 85) + "x", String(repeating: "😀", count: 64)] {
            #expect(key.utf8.count == NoraPluginView.maximumSidebarContentTransitionKeyBytes)
            let sidebar = NoraPluginView(type: .sidebar, contentTransitionKey: key)
            #expect(try NoraPluginView.decode(JSONEncoder().encode(sidebar)).contentTransitionKey == key)
        }
        for key in ["", String(repeating: "x", count: 257), String(repeating: "é", count: 129),
                    String(repeating: "界", count: 86), String(repeating: "😀", count: 65)] {
            #expect(throws: NoraPluginValidationError.self) {
                try NoraPluginView(type: .sidebar, contentTransitionKey: key).validate()
            }
        }
    }

    @Test("Transition keys are rejected specifically on every other view kind")
    func rejectsOtherKinds() {
        let expected = NoraPluginValidationError.invalid(
            "sidebar content transition key", "must be a nonempty string of at most 256 UTF-8 bytes on a sidebar"
        )
        for kind in NoraPluginView.Kind.allCases where kind != .sidebar {
            #expect(throws: expected) {
                try NoraPluginView(type: kind, contentTransitionKey: "reviews").validate()
            }
        }
    }

    @Test("Wire transition keys reject non-string values")
    func rejectsWrongWireTypes() {
        for value in ["42", "true", "[]", "{}"] {
            let json = "{\"type\":\"sidebar\",\"contentTransitionKey\":\(value)}"
            #expect(throws: DecodingError.self) { try NoraPluginView.decode(Data(json.utf8)) }
        }
    }
}
