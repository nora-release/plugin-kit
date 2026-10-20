import Foundation

/// Nora provides this context only after authenticating the native connection.
public struct NoraPluginConnectionContext: Codable, Hashable, Sendable {
    public let panelID: String?

    public init(panelID: String? = nil) {
        self.panelID = panelID
    }

    public func validate() throws {
        guard let panelID else { return }
        try NoraPluginPanelPresentation(panelID: panelID).validate()
    }
}
