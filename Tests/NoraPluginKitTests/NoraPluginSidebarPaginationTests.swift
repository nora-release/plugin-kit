import Foundation
import Testing
@testable import NoraPluginKit

@Suite("Native sidebar pagination contract")
struct NoraPluginSidebarPaginationTests {
    @Test("Pagination fields and item status metadata survive native view encoding")
    func roundTrip() throws {
        let sidebar = NoraPluginView(type: .sidebar, id: "items", value: "issue-42", items: [
            .init(type: .navigationItem, id: "issue-42", text: "Fix keyboard navigation", action: "selectIssue",
                  children: [.init(type: .badge, text: "Open", tone: .success)], subtitle: "example/repo #42"),
        ], loadMoreAction: "nextPage", loadMoreToken: "issues:all:2")
        let workspace = NoraPluginView(type: .workspace, children: [sidebar, .init(type: .column)])
        let decoded = try NoraPluginView.decode(JSONEncoder().encode(workspace))
        #expect(decoded == workspace)
        #expect(decoded.children[0].loadMoreAction == "nextPage")
        #expect(decoded.children[0].loadMoreToken == "issues:all:2")
        #expect(decoded.children[0].items[0].children[0].type == .badge)
    }

    @Test("Legacy sidebars omit pagination and null fields retain the same defaults")
    func defaults() throws {
        for json in [#"{"type":"sidebar","items":[]}"#,
                     #"{"type":"sidebar","loadMoreAction":null,"loadMoreToken":null}"#] {
            let sidebar = try NoraPluginView.decode(Data(json.utf8))
            #expect(sidebar.loadMoreAction == nil)
            #expect(sidebar.loadMoreToken == nil)
            let encoded = try JSONEncoder().encode(sidebar)
            let object = try #require(JSONSerialization.jsonObject(with: encoded) as? [String: Any])
            #expect(object["loadMoreAction"] == nil)
            #expect(object["loadMoreToken"] == nil)
        }
    }

    @Test("Both pagination fields are required together")
    func rejectsUnpairedFields() {
        for (action, token): (String?, String?) in [("next", nil), (nil, "2"), ("", "2"), ("next", ""), ("", "")] {
            #expect(throws: NoraPluginValidationError.self) {
                try NoraPluginView(type: .sidebar, loadMoreAction: action, loadMoreToken: token).validate()
            }
        }
        for json in [#"{"type":"sidebar","loadMoreAction":"next","loadMoreToken":null}"#,
                     #"{"type":"sidebar","loadMoreAction":null,"loadMoreToken":"2"}"#] {
            #expect(throws: NoraPluginValidationError.self) { try NoraPluginView.decode(Data(json.utf8)) }
        }
    }

    @Test("Pagination is rejected on every other native view kind")
    func rejectsOtherKinds() {
        for kind in NoraPluginView.Kind.allCases where kind != .sidebar {
            let children: [NoraPluginView] = kind == .workspace ? [.init(type: .column)] : []
            #expect(throws: NoraPluginValidationError.self) {
                try NoraPluginView(type: kind, children: children, loadMoreAction: "next", loadMoreToken: "2").validate()
            }
        }
    }

    @Test("Both fields measure their limits in UTF-8 bytes")
    func byteLimits() throws {
        for value in [String(repeating: "x", count: 256), String(repeating: "é", count: 128),
                      String(repeating: "界", count: 85) + "x", String(repeating: "😀", count: 64)] {
            #expect(value.utf8.count == 256)
            try NoraPluginView(type: .sidebar, loadMoreAction: value, loadMoreToken: value).validate()
        }
        for value in [String(repeating: "x", count: 257), String(repeating: "é", count: 129),
                      String(repeating: "界", count: 86), String(repeating: "😀", count: 65)] {
            #expect(throws: NoraPluginValidationError.self) {
                try NoraPluginView(type: .sidebar, loadMoreAction: value, loadMoreToken: "2").validate()
            }
            #expect(throws: NoraPluginValidationError.self) {
                try NoraPluginView(type: .sidebar, loadMoreAction: "next", loadMoreToken: value).validate()
            }
        }
    }

    @Test("Wire fields reject non-string values")
    func rejectsWrongWireTypes() {
        for field in ["loadMoreAction", "loadMoreToken"] {
            for value in ["42", "true", "[]", "{}"] {
                let other = field == "loadMoreAction" ? #""loadMoreToken":"2""# : #""loadMoreAction":"next""#
                let json = "{\"type\":\"sidebar\",\(other),\"\(field)\":\(value)}"
                #expect(throws: DecodingError.self) { try NoraPluginView.decode(Data(json.utf8)) }
            }
        }
    }
}
