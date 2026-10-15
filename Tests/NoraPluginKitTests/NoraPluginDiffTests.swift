import Foundation
import Testing
@testable import NoraPluginKit

@Suite("Native read-only plugin diffs")
struct NoraPluginDiffTests {
    private let patch = "@@ -1,2 +1,2 @@\n-let value = 1\n+let value = 2\n // Unchanged\n"

    @Test("Patch descriptors preserve raw content, filename hints, and both layouts")
    func roundTrip() throws {
        for value: String? in [nil, "unified", "split"] {
            let view = NoraPluginView(type: .diff, id: "selected-file", text: patch, value: value,
                                      disabled: true, subtitle: "Sources/示例.swift")
            let decoded = try NoraPluginView.decode(JSONEncoder().encode(view))
            #expect(decoded == view)
            #expect(decoded.action == nil)
            #expect(decoded.children.isEmpty)
        }
    }

    @Test("Patches and filename hints use UTF-8 byte limits")
    func boundedPayload() throws {
        let maximumPatch = String(repeating: "界", count: 21_845) + "\n"
        let maximumHint = String(repeating: "界", count: 1365) + "x"
        #expect(maximumPatch.utf8.count == 64 * 1024)
        #expect(maximumHint.utf8.count == 4096)
        try NoraPluginView(type: .diff, text: maximumPatch, subtitle: maximumHint).validate()
        for view in [NoraPluginView(type: .diff, text: maximumPatch + "x", subtitle: "main.swift"),
                     NoraPluginView(type: .diff, text: patch, subtitle: maximumHint + "x")] {
            #expect(throws: NoraPluginValidationError.self) { try view.validate() }
        }
    }

    @Test("Filename hints cannot inject patch headers or invisible control text")
    func filenameHints() throws {
        for hint in ["Sources/main.swift\n--- /etc/passwd", "Sources/main.swift\r+++ /etc/passwd", "file\u{0}.swift",
                     "file\u{85}.swift", "file\u{2028}.swift", "file\u{2029}.swift", "file\u{202E}.swift", "\tfile.swift", "  "] {
            #expect(throws: NoraPluginValidationError.self) {
                try NoraPluginView(type: .diff, text: patch, subtitle: hint).validate()
            }
        }
        // Path-shaped labels remain inert metadata. They do not request any filesystem operation.
        for hint in ["../Sources/main.swift", "/tmp/main.swift", "src/file with spaces.swift"] {
            let view = NoraPluginView(type: .diff, text: patch, subtitle: hint)
            #expect(try NoraPluginView.decode(JSONEncoder().encode(view)).subtitle == hint)
        }
    }

    @Test("Diffs reject missing content, interactive controls, and unsupported layout values")
    func readOnlyShape() throws {
        let base: [String: Any] = ["type": "diff", "text": patch, "subtitle": "main.swift"]
        let invalid: [[String: Any]] = [
            ["text": NSNull()], ["text": ""], ["subtitle": NSNull()], ["subtitle": ""],
            ["value": ""], ["value": "detail"], ["value": "arbitrary"],
            ["action": "writeFile"], ["action": ""], ["placeholder": "Edit"], ["symbol": "folder"],
            ["textStyle": "body"], ["tone": "accent"], ["appearance": "plain"],
            ["avatarBase64": "AA=="], ["iconSVG": "<svg/>"],
            ["loadMoreAction": "readFile", "loadMoreToken": "1"],
            ["children": [["type": "button", "action": "writeFile"]]],
            ["items": [["type": "textField", "id": "input"]]],
            ["options": [["value": "edit", "text": "Edit"]]],
        ]
        for fields in invalid {
            let descriptor = base.merging(fields) { _, replacement in replacement }
            #expect(throws: (any Error).self) {
                try NoraPluginView.decode(JSONSerialization.data(withJSONObject: descriptor))
            }
        }
        let emptyCollections = base.merging(["children": [], "items": [], "options": []]) { _, replacement in replacement }
        #expect(try NoraPluginView.decode(JSONSerialization.data(withJSONObject: emptyCollections)).type == .diff)
    }
}
