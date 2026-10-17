import Foundation
import Testing
@testable import NoraPluginKit

@Suite("Continuous native content")
struct NoraPluginContinuousContentTests {
    @Test("Complete Markdown exceeds the ordinary text limit without splitting its syntax")
    func completeDocument() throws {
        let body = "# Release notes\n\n```swift\n" + String(repeating: "let 中文 = \"🦉\"\n", count: 35_000) + "```\n"
        let view = NoraPluginView(type: .document, id: "description", text: body)
        let data = try JSONEncoder().encode(view)
        #expect(data.count > NoraPluginView.maximumBytes)
        #expect(try NoraPluginView.decodeDocument(data).text == body)
        #expect(throws: NoraPluginValidationError.self) { try NoraPluginView.decode(data) }
        #expect(throws: NoraPluginValidationError.self) {
            try NoraPluginView(type: .markdown, text: body).validateDocument()
        }
    }

    @Test("Continuous columns retain all loaded entries and automatic loading")
    func continuousRows() throws {
        let rows = (0..<2_000).map { NoraPluginView(type: .text, id: "row-\($0)", text: "Event \($0)") }
        let view = NoraPluginView(type: .lazyColumn, id: "history", children: rows,
                                  loadMoreAction: "load-next", loadMoreToken: "page-67")
        let decoded = try NoraPluginView.decodeDocument(JSONEncoder().encode(view))
        #expect(decoded.children.count == rows.count)
        #expect(decoded.children.last?.id == "row-1999")
        #expect(decoded.loadMoreToken == "page-67")
        #expect(throws: NoraPluginValidationError.self) { try view.validate() }
    }

    @Test("Document validation measures slashes without doubling their transport size")
    func slashRichDocuments() throws {
        let body = String(repeating: "/", count: NoraPluginView.maximumDocumentTextBytes)
        let view = NoraPluginView(type: .lazyColumn, children: (0..<5).map {
            .init(type: .document, id: "body-\($0)", text: body)
        })
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.withoutEscapingSlashes]
        let data = try encoder.encode(view)
        #expect(data.count < NoraPluginView.maximumDocumentBytes)
        let decoded = try NoraPluginView.decodeDocument(data)
        #expect(decoded.children.count == 5)
        #expect(decoded.children.last?.text?.utf8.count == body.utf8.count)
    }

    @Test("Large documents preserve safety and identity validation")
    func validatesCompleteTree() throws {
        #expect(throws: NoraPluginValidationError.self) {
            try NoraPluginView(type: .lazyColumn, children: [.init(type: .text, id: "same"), .init(type: .text, id: "same")]).validateDocument()
        }
        #expect(throws: NoraPluginValidationError.self) {
            try NoraPluginView(type: .document, text: String(repeating: "x", count: NoraPluginView.maximumDocumentTextBytes + 1)).validateDocument()
        }
        #expect(throws: NoraPluginValidationError.self) {
            try NoraPluginView(type: .document, text: "Markdown", action: "send-comment").validateDocument()
        }
        #expect(throws: NoraPluginValidationError.self) {
            try NoraPluginView(type: .lazyColumn, loadMoreAction: "next").validateDocument()
        }
        #expect(throws: NoraPluginValidationError.self) {
            try NoraPluginView(type: .lazyColumn, items: [.init(type: .text, text: "Unsupported")]).validateDocument()
        }
        #expect(throws: NoraPluginValidationError.self) {
            try NoraPluginView.decodeDocument(Data(repeating: 32, count: NoraPluginView.maximumDocumentBytes + 1))
        }
        #expect(throws: NoraPluginValidationError.self) {
            try NoraPluginView(type: .lazyColumn, children: Array(repeating: .init(type: .text), count: NoraPluginView.maximumDocumentNodes)).validateDocument()
        }
    }
}
