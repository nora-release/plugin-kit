import Foundation
import Testing
@testable import NoraPluginKit

@Suite("Plugin conversation drafts")
struct NoraPluginChatDraftTests {
    @Test("A plugin can provide an unsent draft without reading existing conversations")
    func roundTrip() throws {
        let draft = NoraPluginChatDraft(title: "Issue #42", prompt: "Help me investigate this issue.",
                                        context: "Untrusted issue text\nwith **Markdown** and code.",
                                        sourceURL: "https://github.com/example/repo/issues/42#issuecomment-1")
        #expect(try NoraPluginChatDraft.decode(JSONEncoder().encode(draft)) == draft)
        #expect(NoraPluginChatDraft.openDraftMethod == "chat.openDraft")
        let keys = try #require(JSONSerialization.jsonObject(with: JSONEncoder().encode(draft)) as? [String: Any])
        #expect(Set(keys.keys) == ["title", "prompt", "context", "sourceURL"])
    }

    @Test("Optional source URLs and empty context remain compatible with generic drafts")
    func optionalContextSource() throws {
        let draft = NoraPluginChatDraft(title: "Draft", prompt: "Help me review this.", context: "")
        #expect(try NoraPluginChatDraft.decode(JSONEncoder().encode(draft)) == draft)
        #expect(draft.sourceURL == nil)
    }

    @Test("The chat permission is available to script plugins and does not change native permissions")
    func permission() throws {
        let script = NoraPluginManifest(schemaVersion: 2, id: "com.example.chat", name: "Chat", version: "1.0.0",
                                        summary: "Draft handoff", publisher: "Example", permissions: [.chat], runtime: .script, entryPoint: "plugin.js")
        #expect(try NoraPluginManifest.decode(JSONEncoder().encode(script)) == script)
        #expect(NoraPluginPermission.chat.rawValue == "chat")
        let native = NoraPluginManifest(id: "com.example.chat", name: "Chat", version: "1.0.0", summary: "Draft handoff",
                                        publisher: "Example", extensionBundleIdentifier: "com.example.chat.extension", permissions: [.chat])
        #expect(throws: NoraPluginValidationError.self) { try native.validate() }
    }

    @Test("Draft field limits count UTF-8 bytes rather than characters")
    func byteLimits() throws {
        try NoraPluginChatDraft(title: String(repeating: "x", count: 256), prompt: String(repeating: "x", count: 4096),
                                context: String(repeating: "x", count: 128 * 1024)).validate()
        for draft in [
            NoraPluginChatDraft(title: String(repeating: "界", count: 86), prompt: "Review", context: ""),
            NoraPluginChatDraft(title: "Draft", prompt: String(repeating: "界", count: 1366), context: ""),
            NoraPluginChatDraft(title: "Draft", prompt: "Review", context: String(repeating: "界", count: 43691)),
            NoraPluginChatDraft(title: "Draft", prompt: "Review", context: "", sourceURL: "https://example.com/" + String(repeating: "x", count: 8192)),
        ] {
            #expect(throws: NoraPluginValidationError.self) { try draft.validate() }
        }
        let escaped = NoraPluginChatDraft(title: "Draft", prompt: "Review", context: String(repeating: "\u{0}", count: 128 * 1024))
        #expect(try NoraPluginChatDraft.decode(JSONEncoder().encode(escaped)) == escaped)
    }

    @Test("Draft titles and prompts must be nonempty, and titles cannot contain controls", arguments: ["", " \n", "Title\n", "Title\u{0}"])
    func invalidTitle(_ title: String) {
        #expect(throws: NoraPluginValidationError.self) { try NoraPluginChatDraft(title: title, prompt: "Review", context: "").validate() }
        if title.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
            #expect(throws: NoraPluginValidationError.self) { try NoraPluginChatDraft(title: "Draft", prompt: title, context: "").validate() }
        }
    }

    @Test("Sources must be bounded HTTPS links without URL credentials", arguments: [
        "", "http://example.com/issue", "file:///tmp/issue", "javascript:alert(1)", "https:example.com", "https:///issue",
        "https://user:secret@example.com/issue", "https://user@example.com/issue", "https://@example.com/issue",
        "https://example.com/ issue", "https://example.com/issue\n", "https://example.com:65536/issue",
    ])
    func invalidSource(_ sourceURL: String) {
        #expect(throws: NoraPluginValidationError.self) { try NoraPluginChatDraft(title: "Draft", prompt: "Review", context: "", sourceURL: sourceURL).validate() }
    }

    @Test("Drafts do not fetch sources or require the source to be in networkDomains", arguments: [
        "https://github.com/example/repo/issues/42", "https://example.org:8443/item?q=source#details", "HTTPS://example.org/item",
    ])
    func validSource(_ sourceURL: String) throws {
        try NoraPluginChatDraft(title: "Draft", prompt: "Review", context: "", sourceURL: sourceURL).validate()
    }

    @Test("Malformed and oversized draft JSON cannot bypass validation")
    func invalidJSON() {
        for json in [#"{"title":"Draft","prompt":"Review"}"#, #"{"title":42,"prompt":"Review","context":""}"#,
                     #"{"title":"Draft","prompt":"Review","context":"","sourceURL":true}"#] {
            #expect(throws: DecodingError.self) { try NoraPluginChatDraft.decode(Data(json.utf8)) }
        }
        #expect(throws: NoraPluginValidationError.self) {
            try NoraPluginChatDraft.decode(Data(repeating: 32, count: NoraPluginChatDraft.maximumBytes + 1))
        }
    }
}
