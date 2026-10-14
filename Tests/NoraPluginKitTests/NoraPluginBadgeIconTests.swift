import Foundation
import Testing
@testable import NoraPluginKit

@Suite("Native badge SVG contract")
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

    @Test("Inline SVG glyphs are restricted to badge nodes")
    func rejectsOtherKinds() {
        for kind in NoraPluginView.Kind.allCases where kind != .badge {
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
            try NoraPluginView(type: .badge, iconSVG: svg).validate()
            #expect(throws: NoraPluginValidationError.self) { try NoraPluginView(type: .badge, iconSVG: svg + " ").validate() }
        }
        #expect(throws: NoraPluginValidationError.self) { try NoraPluginView(type: .badge, iconSVG: "").validate() }
    }

    @Test("Badge SVG glyphs share the existing static asset validator", arguments: [
        #"<!DOCTYPE svg [<!ENTITY x SYSTEM "file:///tmp/private">]><svg>&x;</svg>"#,
        "<svg><script>alert(1)</script></svg>", #"<svg><image href="https://example.com/image.png"/></svg>"#,
        ##"<svg><use href="#linked"/></svg>"##, "<svg><foreignObject/></svg>", #"<svg><path onclick="run()"/></svg>"#,
        #"<svg><path style="fill:u&#114;l(https://example.com/x)"/></svg>"#,
        #"<svg><path fill="u&#114;l(https://example.com/x)"/></svg>"#, #"<svg><animate attributeName="href"/></svg>"#,
        "<svg><path></svg>", "<svg>&undefined;</svg>", "<html/>",
    ])
    func rejectsUnsafeMarkup(_ svg: String) {
        #expect(throws: NoraPluginValidationError.self) { try NoraPluginView(type: .badge, iconSVG: svg).validate() }
    }

    @Test("Wire SVG fields reject non-string values")
    func rejectsWrongWireTypes() {
        for value in ["42", "true", "[]", "{}"] {
            let json = #"{"type":"badge","iconSVG":"# + value + "}"
            #expect(throws: DecodingError.self) { try NoraPluginView.decode(Data(json.utf8)) }
        }
    }

    private var badgeGlyphs: [String] {
        [
            #"<svg xmlns="http://www.w3.org/2000/svg" width="32" height="32" viewBox="0 0 24 24"><!-- Icon from Huge Icons by Hugeicons - undefined --><g fill="none" stroke="currentColor" stroke-width="1.5"><path stroke-linecap="round" stroke-linejoin="round" d="M6 8v8m12 0v-4c0-2.828 0-4.243-.879-5.121C16.243 6 14.828 6 12 6h-1m0 0c0-.7 1.994-2.008 2.5-2.5M11 6c0 .7 1.994 2.008 2.5 2.5"/><circle cx="6" cy="18" r="2"/><circle cx="6" cy="6" r="2"/><circle cx="18" cy="18" r="2"/></g></svg>"#,
            #"<svg xmlns="http://www.w3.org/2000/svg" width="32" height="32" viewBox="0 0 24 24"><!-- Icon from Huge Icons by Hugeicons - undefined --><g fill="none" stroke="currentColor" stroke-width="1.5"><path stroke-linecap="round" stroke-linejoin="round" d="M6 8v8"/><circle cx="6" cy="18" r="2"/><circle cx="6" cy="6" r="2"/><circle cx="18" cy="18" r="2"/><path stroke-linecap="round" d="M18.125 11H18m.25 0a.25.25 0 1 1-.5 0a.25.25 0 0 1 .5 0Zm-.125-5H18m.25 0a.25.25 0 1 1-.5 0a.25.25 0 0 1 .5 0Z"/></g></svg>"#,
            #"<svg xmlns="http://www.w3.org/2000/svg" width="32" height="32" viewBox="0 0 24 24"><!-- Icon from Huge Icons by Hugeicons - undefined --><g fill="none" stroke="currentColor" stroke-width="1.5"><path stroke-linecap="round" stroke-linejoin="round" d="M6 8v8m12-5v5"/><circle cx="6" cy="18" r="2"/><circle cx="6" cy="6" r="2"/><circle cx="18" cy="18" r="2"/><path stroke-linecap="round" stroke-linejoin="round" d="m20 4l-2 2m0 0l-2 2m2-2l2 2m-2-2l-2-2"/></g></svg>"#,
        ]
    }
}
