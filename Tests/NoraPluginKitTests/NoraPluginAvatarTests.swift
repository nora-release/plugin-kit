import Foundation
import Testing
@testable import NoraPluginKit

@Suite("Native label avatars")
struct NoraPluginAvatarTests {
    @Test("Existing descriptors and explicit null avatars retain their original defaults")
    func backwardCompatibility() throws {
        for json in [#"{"type":"menu","text":"Account"}"#, #"{"type":"button","text":"Account","avatarBase64":null}"#] {
            let view = try NoraPluginView.decode(Data(json.utf8))
            #expect(view.avatarBase64 == nil)
            let data = try JSONEncoder().encode(view)
            let object = try #require(JSONSerialization.jsonObject(with: data) as? [String: Any])
            #expect(object["avatarBase64"] == nil)
            #expect(try NoraPluginView.decode(data) == view)
        }
    }

    @Test("Menu, button, and navigation avatars round trip with their original native label")
    func avatarRoundTrip() throws {
        let avatar = Data([137, 80, 78, 71, 13, 10, 26, 10]).base64EncodedString()
        for kind: NoraPluginView.Kind in [.menu, .button, .navigationItem] {
            let view = NoraPluginView(type: kind, text: "Account", action: "account", symbol: "person.crop.circle", avatarBase64: avatar)
            let decoded = try NoraPluginView.decode(JSONEncoder().encode(view))
            #expect(decoded == view)
            #expect(decoded.avatarBase64 == avatar)
            #expect(decoded.text == "Account")
            #expect(Data(base64Encoded: try #require(decoded.avatarBase64)) == Data([137, 80, 78, 71, 13, 10, 26, 10]))
        }
    }

    @Test("Avatars accept standard Base64 at the exact encoded and decoded boundaries")
    func avatarBoundaries() throws {
        #expect(NoraPluginView.maximumAvatarBase64Characters == 128 * 1024)
        #expect(NoraPluginView.maximumAvatarBytes == 96 * 1024)
        for size in [1, 2, 3, NoraPluginView.maximumAvatarBytes - 2, NoraPluginView.maximumAvatarBytes - 1, NoraPluginView.maximumAvatarBytes] {
            let avatar = Data(repeating: 255, count: size).base64EncodedString()
            #expect(avatar.utf8.count <= NoraPluginView.maximumAvatarBase64Characters)
            try NoraPluginView(type: .menu, avatarBase64: avatar).validate()
        }
        let oversized = Data(repeating: 0, count: NoraPluginView.maximumAvatarBytes + 1).base64EncodedString()
        #expect(throws: NoraPluginValidationError.self) { try NoraPluginView(type: .menu, avatarBase64: oversized).validate() }
        #expect(throws: NoraPluginValidationError.self) {
            try NoraPluginView(type: .button, avatarBase64: String(repeating: "A", count: NoraPluginView.maximumAvatarBase64Characters + 1)).validate()
        }
    }

    @Test("Avatars reject URLs, whitespace, nonstandard padding, and non-Base64 values", arguments: [
        "", " ", "data:image/png;base64,AA==", "https://example.com/avatar.png", "AA==\n", "A A==", "AA\t==", "界",
        "AA", "AAA", "A===", "====", "AAAA=", "AA===", "AA-_", "AA==AAAA", "Zh==", "Zm9=",
    ])
    func rejectsMalformedAvatar(_ avatar: String) {
        #expect(throws: NoraPluginValidationError.self) { try NoraPluginView(type: .menu, avatarBase64: avatar).validate() }
    }

    @Test("Avatar JSON must be a string and avatars only belong to supported native labels")
    func rejectsInvalidAvatarProperties() {
        for json in [#"{"type":"menu","avatarBase64":42}"#, #"{"type":"menu","avatarBase64":true}"#, #"{"type":"menu","avatarBase64":[]}"#] {
            #expect(throws: DecodingError.self) { try NoraPluginView.decode(Data(json.utf8)) }
        }
        for kind in NoraPluginView.Kind.allCases where ![.menu, .button, .navigationItem].contains(kind) {
            #expect(throws: NoraPluginValidationError.self) { try NoraPluginView(type: kind, avatarBase64: "AA==").validate() }
        }
    }

    @Test("Inline avatars remain subject to the complete descriptor byte budget")
    func totalDescriptorLimit() throws {
        let avatar = Data(repeating: 0, count: NoraPluginView.maximumAvatarBytes).base64EncodedString()
        let label = NoraPluginView(type: .button, text: "Account", avatarBase64: avatar)
        try NoraPluginView(type: .column, children: Array(repeating: label, count: 3)).validate()
        #expect(throws: NoraPluginValidationError.self) {
            try NoraPluginView(type: .column, children: Array(repeating: label, count: 4)).validate()
        }
    }
}
