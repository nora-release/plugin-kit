import Foundation

public enum NoraPluginScriptAPI {
    public static let bootstrapSource: String = {
        guard let url = Bundle.module.url(forResource: "script-api", withExtension: "js"),
              let source = try? String(contentsOf: url, encoding: .utf8),
              source.hasPrefix("export default function ") else {
            return "(function(){throw new Error('Nora script SDK resources are unavailable.');})"
        }
        return "(" + source.dropFirst("export default ".count) + ")"
    }()
}
