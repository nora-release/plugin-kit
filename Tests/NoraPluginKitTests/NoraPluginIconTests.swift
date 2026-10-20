import Foundation
import Testing
@testable import NoraPluginKit

@Suite("Plugin brand icons")
struct NoraPluginIconTests {
    @Test("Brand icons are optional in native manifests")
    func optionalIcon() throws {
        let native = manifest(iconPath: nil)
        #expect(native.iconPath == nil)
        #expect(try NoraPluginManifest.decode(JSONEncoder().encode(native)) == native)
    }

    @Test("Icons use safe relative image paths", arguments: ["", "../brand.svg", "/brand.svg", "a/../brand.svg", "a/./brand.svg", "a//brand.svg", "a\\brand.svg", "brand.svg\n", "brand.js", "brand.SVG", "Contents/Resources/brand.svg", "contents/resources/brand.svg", String(repeating: "a/", count: 513) + "brand.svg"])
    func rejectsPaths(_ path: String) {
        #expect(throws: NoraPluginValidationError.self) { try manifest(iconPath: path).validate() }
    }

    @Test("Native plugins can declare static images", arguments: ["assets/brand.svg", "brand.png", "images/brand.jpg", "images/brand.jpeg"])
    func acceptsPaths(_ path: String) throws {
        try manifest(iconPath: path).validate()
    }

    @Test("Static SVG markup is accepted without external resource loading")
    func staticSVG() throws {
        try NoraPluginIconAsset.validate(data: Data(#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path d="M0 0h24v24H0z"/></svg>"#.utf8), path: "assets/brand.svg")
        try NoraPluginIconAsset.validate(data: Data(#"<?xml version="1.0"?><svg><title>GitHub &amp; Nora</title><path/></svg>"#.utf8), path: "brand.svg")
    }

    @Test("SVG icons cannot run code, resolve entities, or reference external content", arguments: [
        #"<!DOCTYPE svg [<!ENTITY x SYSTEM "file:///tmp/private">]><svg>&x;</svg>"#,
        "<svg><script>alert(1)</script></svg>", #"<svg><image href="https://example.com/image.png"/></svg>"#,
        ##"<svg><use href="#linked"/></svg>"##, "<svg><foreignObject/></svg>", #"<svg><path onclick="run()"/></svg>"#,
        #"<svg><path style="fill:u&#114;l(https://example.com/x)"/></svg>"#,
        #"<svg><path fill="u&#114;l(https://example.com/x)"/></svg>"#, #"<svg><animate attributeName="href"/></svg>"#,
        "<svg><path></svg>", "<svg>&undefined;</svg>", "<html/>",
    ])
    func rejectsSVG(_ svg: String) {
        #expect(throws: NoraPluginValidationError.self) { try NoraPluginIconAsset.validate(data: Data(svg.utf8), path: "brand.svg") }
    }

    @Test("Icon byte limits and raster signatures are checked before decoding")
    func resourceLimits() throws {
        #expect(NoraPluginManifest.maximumIconBytes == 512 * 1024)
        #expect(throws: NoraPluginValidationError.self) { try NoraPluginIconAsset.validate(data: Data(), path: "brand.svg") }
        #expect(throws: NoraPluginValidationError.self) {
            try NoraPluginIconAsset.validate(data: Data(repeating: 0, count: NoraPluginManifest.maximumIconBytes + 1), path: "brand.png")
        }
        #expect(throws: NoraPluginValidationError.self) { try NoraPluginIconAsset.validate(data: Data("not PNG".utf8), path: "brand.png") }
        try NoraPluginIconAsset.validate(data: Data([137, 80, 78, 71, 13, 10, 26, 10]), path: "brand.png")
        try NoraPluginIconAsset.validate(data: Data([255, 216, 255, 217]), path: "brand.jpg")
    }

    private func manifest(iconPath: String?) -> NoraPluginManifest {
        NoraPluginManifest(id: "com.example.icon", name: "Icon", version: "1.0.0",
                           summary: "Plugin icon", publisher: "Example", extensionBundleIdentifier: "com.example.icon.extension",
                           iconPath: iconPath)
    }
}
