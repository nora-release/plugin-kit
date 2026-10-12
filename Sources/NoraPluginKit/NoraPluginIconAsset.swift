import Foundation

public enum NoraPluginIconAsset {
    public static func validate(data: Data, path: String) throws {
        guard !data.isEmpty, data.count <= NoraPluginManifest.maximumIconBytes else {
            throw NoraPluginValidationError.invalid("icon", "must be a nonempty file of at most 512 KiB")
        }
        switch (path as NSString).pathExtension {
        case "png":
            guard data.starts(with: [137, 80, 78, 71, 13, 10, 26, 10]) else {
                throw NoraPluginValidationError.invalid("icon", "has an invalid PNG signature")
            }
        case "jpg", "jpeg":
            guard data.count >= 4, data.starts(with: [255, 216]), data.suffix(2) == Data([255, 217]) else {
                throw NoraPluginValidationError.invalid("icon", "has an invalid JPEG signature")
            }
        case "svg":
            try validateSVG(data)
        default:
            throw NoraPluginValidationError.invalid("icon", "must be an SVG, PNG, or JPEG file")
        }
    }

    private static func validateSVG(_ data: Data) throws {
        guard let svg = String(data: data, encoding: .utf8) else {
            throw NoraPluginValidationError.invalid("icon", "must contain UTF-8 SVG markup")
        }
        let markup = svg.replacingOccurrences(of: #"<!--[\s\S]*?-->"#, with: "", options: .regularExpression)
            .replacingOccurrences(of: #"^\uFEFF?\s*<\?xml\s[^?]*\?>"#, with: "", options: [.regularExpression, .caseInsensitive])
        guard markup.range(of: #"<[!?]"#, options: .regularExpression) == nil else {
            throw NoraPluginValidationError.invalid("icon", "SVG declarations, entities, and processing instructions are prohibited")
        }
        let delegate = StaticVectorDelegate()
        let parser = XMLParser(data: data)
        parser.shouldResolveExternalEntities = false
        parser.externalEntityResolvingPolicy = .never
        parser.delegate = delegate
        guard parser.parse(), delegate.valid, delegate.root == "svg" else {
            throw NoraPluginValidationError.invalid("icon", "must be static vector SVG without scripts or linked resources")
        }
    }
}

private final class StaticVectorDelegate: NSObject, XMLParserDelegate {
    private static let allowedElements: Set<String> = [
        "svg", "g", "path", "rect", "circle", "ellipse", "line", "polyline", "polygon", "defs", "title", "desc",
        "clipPath", "mask", "linearGradient", "radialGradient", "stop",
    ]
    private(set) var valid = true
    private(set) var root: String?

    func parser(_ parser: XMLParser, didStartElement elementName: String, namespaceURI: String?,
                qualifiedName qName: String?, attributes attributeDict: [String: String]) {
        if root == nil { root = elementName }
        guard Self.allowedElements.contains(elementName), attributeDict.allSatisfy({ name, value in
            let localName = name.split(separator: ":").last.map(String.init)?.lowercased() ?? ""
            return localName != "href" && localName != "style" && !localName.hasPrefix("on")
                && value.range(of: #"url\s*\("#, options: [.regularExpression, .caseInsensitive]) == nil
        }) else {
            valid = false
            parser.abortParsing()
            return
        }
    }
}
