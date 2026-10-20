import Foundation
import Testing
@testable import NoraPluginKit

@Suite("Native plugin panel presentation")
struct NoraPluginPanelPresentationTests {
    private let panelID = "com.example.native.panel"
    private let icon = #"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><circle cx="12" cy="12" r="3"/></svg>"#

    @Test("Native chrome round trips through the same typed XPC contract")
    func roundTrip() throws {
        let presentation = NoraPluginPanelPresentation(
            panelID: panelID, title: "Review pull request", number: 42,
            status: NoraPluginPanelStatus(title: "Merged", systemImage: "arrow.triangle.merge", tone: .purple, iconSVG: icon),
            sidebarVisible: false, sidebarWidth: 320,
            actions: [NoraPluginPanelAction(id: "refresh", title: "Refresh", systemImage: "arrow.clockwise", iconSVG: icon)])
        let request = try NoraPluginRequest(method: NoraPluginPanelPresentation.presentMethod, parameters: presentation)
        let decoded = try NoraPluginWire.decodeRequest(NoraPluginWire.encode(request))
        let value = try NoraPluginPanelPresentation.decode(Data(decoded.parametersJSON.utf8))
        #expect(value == presentation)
        #expect(try decoded.parameters(as: NoraPluginPanelPresentation.self) == presentation)
        let response = try NoraPluginResponse(result: true)
        #expect(try NoraPluginWire.decodeResponse(NoraPluginWire.encode(response)).result(as: Bool.self))
        #expect(throws: NoraPluginConnectionError.self) { try NoraPluginResponse(error: "Permission denied").result(as: Bool.self) }
    }

    @Test("Absent optional chrome uses native defaults")
    func defaults() throws {
        let value = try NoraPluginPanelPresentation.decode(Data(#"{"panelID":"com.example.native.panel"}"#.utf8))
        #expect(value.title == nil && value.number == nil && value.status == nil)
        #expect(value.hasSidebar && value.sidebarVisible && value.actions.isEmpty)
        #expect(value.sidebarWidth == 268)
    }

    @Test("Native sidebar geometry remains finite and bounded")
    func sidebarGeometry() throws {
        for width: Double in [180, 268, 320.5, 560] {
            let presentation = NoraPluginPanelPresentation(panelID: panelID, sidebarWidth: width)
            #expect(try NoraPluginPanelPresentation.decode(JSONEncoder().encode(presentation)).sidebarWidth == width)
        }
        for width: Double in [179.9, 560.1, .nan, .infinity, -.infinity] {
            #expect(throws: NoraPluginValidationError.self) {
                try NoraPluginPanelPresentation(panelID: panelID, sidebarWidth: width).validate()
            }
        }
    }

    @Test("Panel identity and issue numbers cannot carry malformed values")
    func identityAndNumbers() throws {
        for identifier in ["", "panel", "com.example.panel\n", "com.example..panel", String(repeating: "x", count: 256) + ".panel"] {
            #expect(throws: NoraPluginValidationError.self) { try NoraPluginPanelPresentation(panelID: identifier).validate() }
        }
        for number in [0, -1, Int(Int32.max) + 1] {
            #expect(throws: NoraPluginValidationError.self) { try NoraPluginPanelPresentation(panelID: panelID, number: number).validate() }
        }
        try NoraPluginPanelPresentation(panelID: panelID, number: Int(Int32.max)).validate()
    }

    @Test("Actions are bounded, unique, and have at most one primary action")
    func actions() {
        func action(_ id: String, primary: Bool = false) -> NoraPluginPanelAction {
            NoraPluginPanelAction(id: id, title: "Refresh", systemImage: "arrow.clockwise", isPrimary: primary)
        }
        for actions in [[action("refresh"), action("refresh")], [action("one", primary: true), action("two", primary: true)],
                        [action("refresh\n")], [action("")], (0..<17).map { action("action\($0)") }] {
            #expect(throws: NoraPluginValidationError.self) {
                try NoraPluginPanelPresentation(panelID: panelID, actions: actions).validate()
            }
        }
    }

    @Test("Presentation text and icons are validated before host rendering")
    func renderingBoundaries() {
        for title in ["", " ", "title\n", String(repeating: "x", count: 1025)] {
            #expect(throws: NoraPluginValidationError.self) { try NoraPluginPanelPresentation(panelID: panelID, title: title).validate() }
        }
        for icon in ["<svg><script/></svg>", "<svg><image href=\"https://example.com/image\"/></svg>", String(repeating: "x", count: 16 * 1024 + 1)] {
            #expect(throws: NoraPluginValidationError.self) {
                try NoraPluginPanelPresentation(panelID: panelID, status: .init(title: "Open", systemImage: "circle", iconSVG: icon)).validate()
            }
        }
        #expect(throws: NoraPluginValidationError.self) {
            try NoraPluginPanelPresentation.decode(Data(repeating: 0x20, count: NoraPluginPanelPresentation.maximumBytes + 1))
        }
    }

    @Test("Native keyboard actions use the same explicit panel callback")
    func keyboardActions() throws {
        for action in [NoraPluginPanelKeyboardAction.focusSearch, NoraPluginPanelKeyboardAction.toggleSidebar, NoraPluginPanelKeyboardAction.closeDetails] {
            let invocation = NoraPluginPanelActionInvocation(panelID: panelID, actionID: action)
            try invocation.validate()
            let request = try NoraPluginRequest(method: NoraPluginPanelPresentation.actionMethod, parameters: invocation)
            #expect(try request.parameters(as: NoraPluginPanelActionInvocation.self) == invocation)
        }
    }
}
