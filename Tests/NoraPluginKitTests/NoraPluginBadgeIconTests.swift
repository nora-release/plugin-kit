import Foundation
import Testing
@testable import NoraPluginKit

@Suite("Native badge, status row, button, and navigation SVG contract")
struct NoraPluginBadgeIconTests {
    @Test("Static SVG badge glyphs preserve path, circle, group, tone, and symbol fallback")
    func roundTrip() throws {
        for svg in badgeGlyphs {
            let badge = NoraPluginView(type: .badge, text: "Status", symbol: "arrow.triangle.pull", tone: .success, iconSVG: svg)
            let decoded = try NoraPluginView.decode(JSONEncoder().encode(badge))
            #expect(decoded == badge)
            #expect(decoded.iconSVG == svg)
            #expect(decoded.symbol == "arrow.triangle.pull")
        }
    }

    @Test("Legacy badge JSON and null icons preserve the original defaults")
    func defaults() throws {
        for json in [#"{"type":"badge","text":"Open","symbol":"circle"}"#,
                     #"{"type":"badge","text":"Open","symbol":"circle","iconSVG":null}"#] {
            let badge = try NoraPluginView.decode(Data(json.utf8))
            #expect(badge.iconSVG == nil)
            #expect(badge.symbol == "circle")
            let object = try #require(JSONSerialization.jsonObject(with: JSONEncoder().encode(badge)) as? [String: Any])
            #expect(object["iconSVG"] == nil)
        }
    }

    @Test("Status row SVGs preserve metadata actions tone and symbol fallback")
    func itemRowRoundTrip() throws {
        for svg in badgeGlyphs {
            let row = NoraPluginView(type: .itemRow, id: "commit", text: "3 commits", action: "show-commits", value: "selected",
                                     children: [.init(type: .text, text: "8m ago", textStyle: .caption)],
                                     disabled: true, subtitle: "main", symbol: "circle", tone: .muted, iconSVG: svg)
            let decoded = try NoraPluginView.decode(JSONEncoder().encode(row))
            #expect(decoded == row)
            #expect(decoded.iconSVG == svg)
            #expect(decoded.action == "show-commits")
            #expect(decoded.children[0].text == "8m ago")
        }
        for json in [#"{"type":"itemRow","text":"3 commits","symbol":"circle"}"#,
                     #"{"type":"itemRow","text":"3 commits","symbol":"circle","iconSVG":null}"#] {
            let row = try NoraPluginView.decode(Data(json.utf8))
            #expect(row.iconSVG == nil)
            #expect(row.symbol == "circle")
        }
    }

    @Test("Button and tab SVGs preserve action, selection, disabled state, count, and symbol fallback")
    func buttonRoundTrip() throws {
        for svg in buttonGlyphs {
            let button = NoraPluginView(type: .button, id: "conversation-tab", text: "Conversation", action: "select-conversation",
                                        value: "selected", disabled: true, subtitle: "0", symbol: "bubble.left.and.bubble.right",
                                        tone: .muted, appearance: .plain, iconSVG: svg)
            let tabs = NoraPluginView(type: .tabBar, children: [button])
            let decoded = try NoraPluginView.decode(JSONEncoder().encode(tabs))
            #expect(decoded == tabs)
            #expect(decoded.children[0].iconSVG == svg)
            #expect(decoded.children[0].symbol == "bubble.left.and.bubble.right")
        }
        for json in [#"{"type":"button","text":"Conversation","symbol":"bubble.left.and.bubble.right"}"#,
                     #"{"type":"button","text":"Conversation","symbol":"bubble.left.and.bubble.right","iconSVG":null}"#] {
            let button = try NoraPluginView.decode(Data(json.utf8))
            #expect(button.iconSVG == nil)
            #expect(button.symbol == "bubble.left.and.bubble.right")
        }
    }

    @Test("Navigation SVGs preserve sidebar selection, action, metadata, avatar, and symbol fallback")
    func navigationRoundTrip() throws {
        for svg in badgeGlyphs + buttonGlyphs {
            let item = NoraPluginView(type: .navigationItem, id: "view-issues", text: "Issues", action: "kind:issues",
                                      children: [.init(type: .badge, text: "Open", tone: .success)], disabled: true,
                                      symbol: "questionmark.bubble", tone: .muted, avatarBase64: "AA==", iconSVG: svg)
            let sidebar = NoraPluginView(type: .sidebar, id: "view-navigation", value: "view-issues", items: [item])
            let decoded = try NoraPluginView.decode(JSONEncoder().encode(sidebar))
            #expect(decoded == sidebar)
            #expect(decoded.items[0].iconSVG == svg)
            #expect(decoded.items[0].avatarBase64 == "AA==")
            #expect(decoded.items[0].symbol == "questionmark.bubble")
            #expect(decoded.items[0].action == "kind:issues")
        }
        for json in [#"{"type":"navigationItem","text":"Issues","symbol":"questionmark.bubble"}"#,
                     #"{"type":"navigationItem","text":"Issues","symbol":"questionmark.bubble","iconSVG":null}"#] {
            let item = try NoraPluginView.decode(Data(json.utf8))
            #expect(item.iconSVG == nil)
            #expect(item.symbol == "questionmark.bubble")
        }
    }

    @Test("Inline SVG glyphs are restricted to badge, itemRow, button, and navigationItem nodes")
    func rejectsOtherKinds() {
        for kind in NoraPluginView.Kind.allCases where kind != .badge && kind != .itemRow && kind != .button && kind != .navigationItem {
            let children: [NoraPluginView] = kind == .workspace ? [.init(type: .column)] : []
            #expect(throws: NoraPluginValidationError.self) {
                try NoraPluginView(type: kind, children: children, iconSVG: "<svg><path/></svg>").validate()
            }
        }
    }

    @Test("The exact 16 KiB boundary measures UTF-8 bytes before parsing")
    func byteLimits() throws {
        #expect(NoraPluginView.maximumIconSVGBytes == 16 * 1024)
        let prefix = "<svg><title>", suffix = "</title></svg>"
        let remaining = NoraPluginView.maximumIconSVGBytes - prefix.utf8.count - suffix.utf8.count
        let ascii = prefix + String(repeating: "x", count: remaining) + suffix
        let unicode = prefix + String(repeating: "界", count: remaining / 3) + String(repeating: "x", count: remaining % 3) + suffix
        for svg in [ascii, unicode] {
            #expect(svg.utf8.count == NoraPluginView.maximumIconSVGBytes)
            for kind: NoraPluginView.Kind in [.badge, .itemRow, .button, .navigationItem] {
                try NoraPluginView(type: kind, iconSVG: svg).validate()
                #expect(throws: NoraPluginValidationError.self) { try NoraPluginView(type: kind, iconSVG: svg + " ").validate() }
            }
        }
        for kind: NoraPluginView.Kind in [.badge, .itemRow, .button, .navigationItem] {
            #expect(throws: NoraPluginValidationError.self) { try NoraPluginView(type: kind, iconSVG: "").validate() }
        }
    }

    @Test("Badge, status row, button, and navigation SVG glyphs share the existing static asset validator", arguments: [
        #"<!DOCTYPE svg [<!ENTITY x SYSTEM "file:///tmp/private">]><svg>&x;</svg>"#,
        "<svg><script>alert(1)</script></svg>", #"<svg><image href="https://example.com/image.png"/></svg>"#,
        ##"<svg><use href="#linked"/></svg>"##, "<svg><foreignObject/></svg>", #"<svg><path onclick="run()"/></svg>"#,
        #"<svg><path style="fill:u&#114;l(https://example.com/x)"/></svg>"#,
        #"<svg><path fill="u&#114;l(https://example.com/x)"/></svg>"#, #"<svg><animate attributeName="href"/></svg>"#,
        "<svg><path></svg>", "<svg>&undefined;</svg>", "<html/>",
    ])
    func rejectsUnsafeMarkup(_ svg: String) {
        for kind: NoraPluginView.Kind in [.badge, .itemRow, .button, .navigationItem] {
            #expect(throws: NoraPluginValidationError.self) { try NoraPluginView(type: kind, iconSVG: svg).validate() }
        }
    }

    @Test("Wire SVG fields reject non-string values")
    func rejectsWrongWireTypes() {
        for value in ["42", "true", "[]", "{}"] {
            for kind in ["badge", "itemRow", "button", "navigationItem"] {
                let json = #"{"type":"# + "\"\(kind)\"" + #", "iconSVG":"# + value + "}"
                #expect(throws: DecodingError.self) { try NoraPluginView.decode(Data(json.utf8)) }
            }
        }
    }

    private var buttonGlyphs: [String] {
        [
            #"<svg xmlns="http://www.w3.org/2000/svg" width="32" height="32" viewBox="0 0 24 24"><!-- Icon from Huge Icons by Hugeicons - undefined --><g fill="none" stroke="currentColor" stroke-linejoin="round" stroke-width="1.5"><path stroke-linecap="round" d="M7.5 12h6m-6-4h3m-2 12c1.05.87 2.315 1.424 3.764 1.519c1.141.075 2.333.075 3.473 0a4 4 0 0 0 1.188-.268c.41-.167.614-.25.719-.237c.104.012.255.122.557.342c.533.388 1.204.666 2.2.643c.503-.012.755-.019.867-.208c.113-.19-.027-.452-.308-.977c-.39-.728-.636-1.561-.262-2.229c.643-.954 1.19-2.083 1.27-3.303c.043-.655.043-1.334 0-1.99A6.7 6.7 0 0 0 21.4 11"/><path d="M12.345 17.487c3.556-.234 6.388-3.08 6.62-6.653c.046-.699.046-1.423 0-2.122c-.232-3.572-3.064-6.418-6.62-6.652c-1.213-.08-2.48-.08-3.69 0c-3.556.234-6.388 3.08-6.62 6.652c-.046.7-.046 1.423 0 2.122c.084 1.302.665 2.506 1.349 3.524c.397.712.135 1.6-.279 2.377c-.298.56-.447.84-.327 1.042s.387.209.922.221c1.057.026 1.77-.271 2.336-.685c.321-.234.482-.351.593-.365c.11-.013.328.075.763.253c.392.16.846.258 1.263.286c1.21.08 2.477.08 3.69 0Z"/></g></svg>"#,
            #"<svg xmlns="http://www.w3.org/2000/svg" width="32" height="32" viewBox="0 0 24 24"><!-- Icon from Huge Icons by Hugeicons - undefined --><g fill="none" stroke="currentColor" stroke-linecap="round" stroke-linejoin="round" stroke-width="1.5"><circle cx="12" cy="12" r="3"/><path d="M9 12H3m12 0h6"/></g></svg>"#,
            #"<svg xmlns="http://www.w3.org/2000/svg" width="32" height="32" viewBox="0 0 24 24"><!-- Icon from Huge Icons by Hugeicons - undefined --><g fill="none" stroke="currentColor" stroke-linecap="round" stroke-width="1.5"><path d="M11 6h10m-10 6h10m-10 6h10"/><path stroke-linejoin="round" d="M3 7.393S4 8.045 4.5 9C4.5 9 6 5.25 8 4M3 18.393S4 19.045 4.5 20c0 0 1.5-3.75 3.5-5"/></g></svg>"#,
            #"<svg xmlns="http://www.w3.org/2000/svg" width="32" height="32" viewBox="0 0 24 24"><!-- Icon from Huge Icons by Hugeicons - undefined --><path fill="none" stroke="currentColor" stroke-linecap="round" stroke-linejoin="round" stroke-width="1.5" d="M5.993 12v6M9 14.992H3M3 22h6m4 0c3.771 0 5.657 0 6.828-1.172S21 17.771 21 14v-3.343c0-.818 0-1.226-.152-1.594c-.152-.367-.441-.657-1.02-1.235l-4.736-4.736c-.499-.499-.748-.748-1.058-.896a2 2 0 0 0-.197-.082C13.514 2 13.161 2 12.456 2c-3.245 0-4.868 0-5.967.886a4 4 0 0 0-.603.603C5.144 4.41 5.023 5.7 5.004 8M14 2.5V3c0 2.828 0 4.243.879 5.121C15.757 9 17.172 9 20 9h.5"/></svg>"#,
        ]
    }

    private var badgeGlyphs: [String] {
        [
            #"<svg xmlns="http://www.w3.org/2000/svg" width="32" height="32" viewBox="0 0 24 24"><!-- Icon from Huge Icons by Hugeicons - undefined --><g fill="none" stroke="currentColor" stroke-width="1.5"><path stroke-linecap="round" stroke-linejoin="round" d="M6 8v8m12 0v-4c0-2.828 0-4.243-.879-5.121C16.243 6 14.828 6 12 6h-1m0 0c0-.7 1.994-2.008 2.5-2.5M11 6c0 .7 1.994 2.008 2.5 2.5"/><circle cx="6" cy="18" r="2"/><circle cx="6" cy="6" r="2"/><circle cx="18" cy="18" r="2"/></g></svg>"#,
            #"<svg xmlns="http://www.w3.org/2000/svg" width="32" height="32" viewBox="0 0 24 24"><!-- Icon from Huge Icons by Hugeicons - undefined --><g fill="none" stroke="currentColor" stroke-width="1.5"><path stroke-linecap="round" stroke-linejoin="round" d="M6 8v8"/><circle cx="6" cy="18" r="2"/><circle cx="6" cy="6" r="2"/><circle cx="18" cy="18" r="2"/><path stroke-linecap="round" d="M18.125 11H18m.25 0a.25.25 0 1 1-.5 0a.25.25 0 0 1 .5 0Zm-.125-5H18m.25 0a.25.25 0 1 1-.5 0a.25.25 0 0 1 .5 0Z"/></g></svg>"#,
            #"<svg xmlns="http://www.w3.org/2000/svg" width="32" height="32" viewBox="0 0 24 24"><!-- Icon from Huge Icons by Hugeicons - undefined --><g fill="none" stroke="currentColor" stroke-width="1.5"><path stroke-linecap="round" stroke-linejoin="round" d="M6 8v8m12-5v5"/><circle cx="6" cy="18" r="2"/><circle cx="6" cy="6" r="2"/><circle cx="18" cy="18" r="2"/><path stroke-linecap="round" stroke-linejoin="round" d="m20 4l-2 2m0 0l-2 2m2-2l2 2m-2-2l-2-2"/></g></svg>"#,
        ]
    }
}
