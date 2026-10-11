import AppKit
import ExtensionFoundation
import ExtensionKit
import Foundation
import NoraPluginKit

@main
enum SmokeHost {
    @MainActor static func main() {
        let app = NSApplication.shared
        let delegate = SmokeDelegate()
        app.delegate = delegate
        app.setActivationPolicy(.regular)
        withExtendedLifetime(delegate) { app.run() }
    }
}

@MainActor
private final class SmokeDelegate: NSObject, NSApplicationDelegate, EXHostViewControllerDelegate {
    private let reportURL = URL(fileURLWithPath: CommandLine.arguments[1])
    private let holdSeconds = min(120, max(0, Double(CommandLine.arguments.dropFirst(2).first ?? "0") ?? 0))
    private var events: [String] = []
    private var window: NSWindow?
    private var controller: EXHostViewController?
    private var process: AppExtensionProcess?
    private var connections: [NSXPCConnection] = []
    private var discoveryTask: Task<Void, Never>?
    private var timeoutTask: Task<Void, Never>?
    private var finished = false
    private var sceneConnected = false
    private var identity: AppExtensionIdentity?

    func applicationDidFinishLaunching(_ notification: Notification) {
        record("Host launched: \(Bundle.main.bundleIdentifier ?? "unknown")")
        timeoutTask = Task { [weak self] in
            do { try await Task.sleep(for: .seconds(35)) } catch { return }
            self?.finish(error: "Timed out waiting for ExtensionKit discovery or activation.")
        }
        discoveryTask = Task { [weak self] in await self?.discover() }
    }

    private func discover() async {
        do {
            let identities = try AppExtensionIdentity.matching(appExtensionPointIDs: NoraPluginManifest.extensionPointIdentifier)
            for await available in identities {
                record("Discovered: \(available.map(\.bundleIdentifier).joined(separator: ", "))")
                guard let identity = available.first(where: { $0.bundleIdentifier == "com.nora.example.native.extension" }) else { continue }
                self.identity = identity
                presentScene(identity)
                return
            }
            finish(error: "Extension discovery ended without finding the example.")
        } catch {
            finish(error: "Discovery or headless command: \(error)")
        }
    }

    private func launchProcess(_ identity: AppExtensionIdentity) throws -> AppExtensionProcess {
        try AppExtensionProcess(configuration: .init(appExtensionIdentity: identity))
    }

    private func verifyHeadless(_ identity: AppExtensionIdentity) async throws {
        let process = try launchProcess(identity)
        self.process = process
        let connection = try process.makeXPCConnection()
        configure(connection)
        try await verifyHandshake(connection)
        let response = try await invokeEcho(connection, text: "headless-native-success")
        guard response.resultJSON == #"{"text":"headless-native-success"}"# else {
            throw SmokeError("Unexpected headless response: \(response)")
        }
        record("PASS headless AppExtensionProcess commands.execute echo")
        try await verifyHostInfo(connection)
        record("PASS headless plugin → host.info → plugin → host")
    }

    private func presentScene(_ identity: AppExtensionIdentity) {
        let controller = EXHostViewController()
        self.controller = controller
        controller.delegate = self
        controller.placeholderView = NSTextField(labelWithString: "Waiting for native extension…")
        controller.view.isHidden = true
        controller.configuration = .init(appExtension: identity, sceneID: "com.nora.example.native.panel")
        let window = NSWindow(contentRect: NSRect(x: 180, y: 180, width: 560, height: 360),
                              styleMask: [.titled, .closable, .resizable], backing: .buffered, defer: false)
        window.title = "Nora Native Extension Smoke Test"
        let container = NSViewController()
        container.view = NSView(frame: NSRect(x: 0, y: 0, width: 560, height: 360))
        container.preferredContentSize = NSSize(width: 560, height: 360)
        container.addChild(controller)
        let hostedView = controller.view
        hostedView.translatesAutoresizingMaskIntoConstraints = false
        container.view.addSubview(hostedView)
        NSLayoutConstraint.activate([
            hostedView.leadingAnchor.constraint(equalTo: container.view.leadingAnchor),
            hostedView.trailingAnchor.constraint(equalTo: container.view.trailingAnchor),
            hostedView.topAnchor.constraint(equalTo: container.view.topAnchor),
            hostedView.bottomAnchor.constraint(equalTo: container.view.bottomAnchor),
        ])
        window.contentViewController = container
        window.contentMinSize = NSSize(width: 560, height: 360)
        window.setContentSize(NSSize(width: 560, height: 360))
        self.window = window
        window.makeKeyAndOrderFront(nil)
        NSApp.activate(ignoringOtherApps: true)
        record("Requested SwiftUI scene")
    }

    func hostViewControllerDidActivate(_ viewController: EXHostViewController) {
        guard !sceneConnected else { return }
        sceneConnected = true
        record("PASS native scene activated")
        Task {
            do {
                let connection = try viewController.makeXPCConnection()
                configure(connection)
                try await verifyHandshake(connection)
                viewController.view.isHidden = false
                let response = try await invokeEcho(connection, text: "scene-native-success")
                guard response.resultJSON == #"{"text":"scene-native-success"}"# else {
                    throw SmokeError("Unexpected scene response: \(response)")
                }
                record("PASS scene EXHostViewController commands.execute echo")
                try await verifyHostInfo(connection)
                record("PASS scene plugin → host.info → plugin → host")
                if let identity { try await verifyHeadless(identity) }
                try await Task.sleep(for: .seconds(1))
                record("Host content bounds: \(String(describing: window?.contentView?.bounds)); window frame: \(String(describing: window?.frame))")
                if holdSeconds > 0 {
                    timeoutTask?.cancel()
                    record("Holding native window for \(holdSeconds) seconds")
                    writeReport(error: nil)
                    try await Task.sleep(for: .seconds(holdSeconds))
                }
                finish(error: nil)
            } catch {
                finish(error: "Scene command: \(error)")
            }
        }
    }

    func hostViewControllerWillDeactivate(_ viewController: EXHostViewController, error: (any Error)?) {
        guard !finished else { return }
        guard sceneConnected || error != nil else { return }
        finish(error: "Native scene deactivated: \(String(describing: error))")
    }

    private func configure(_ connection: NSXPCConnection) {
        connection.remoteObjectInterface = NSXPCInterface(with: NoraPluginExtensionXPC.self)
        connection.exportedInterface = NSXPCInterface(with: NoraPluginHostXPC.self)
        connection.exportedObject = SmokeHostAPI()
        connection.resume()
        connections.append(connection)
    }

    private func invokeEcho(_ connection: NSXPCConnection, text: String) async throws -> NoraPluginResponse {
        let parameters = try JSONSerialization.data(withJSONObject: ["commandID": "com.nora.example.native.echo", "arguments": ["text": text]])
        let request = NoraPluginRequest(method: "commands.execute", parametersJSON: String(decoding: parameters, as: UTF8.self))
        return try await invoke(connection, request: request)
    }

    private func verifyHandshake(_ connection: NSXPCConnection) async throws {
        let response = try await invoke(connection, request: NoraPluginRequest(method: "plugin.handshake"))
        guard response.resultJSON == #"{"apiVersion":1}"# else { throw SmokeError("Invalid SDK handshake") }
        record("PASS SDK handshake")
        let extensionURL = Bundle.main.bundleURL.appendingPathComponent("Contents/Extensions/NativePlugin.appex")
        record(try SmokeSignature.verify(processIdentifier: connection.processIdentifier, extensionURL: extensionURL))
    }

    private func verifyHostInfo(_ connection: NSXPCConnection) async throws {
        let request = NoraPluginRequest(method: "commands.execute", parametersJSON: #"{"commandID":"com.nora.example.native.hostInfo","arguments":{}}"#)
        let response = try await invoke(connection, request: request)
        guard response.resultJSON == #"{"name":"Nora Smoke Host"}"# else { throw SmokeError("Invalid host.info response: \(response)") }
    }

    private func invoke(_ connection: NSXPCConnection, request: NoraPluginRequest) async throws -> NoraPluginResponse {
        let data = try NoraPluginWire.encode(request)
        let reply = SmokeReply()
        let response = try await withCheckedThrowingContinuation { continuation in
            reply.continuation = continuation
            let proxy = connection.remoteObjectProxyWithErrorHandler { reply.finish(.failure($0)) } as? any NoraPluginExtensionXPC
            guard let proxy else { reply.finish(.failure(SmokeError("Missing extension proxy"))); return }
            proxy.invoke(data) { reply.finish(.success($0)) }
        }
        return try NoraPluginWire.decodeResponse(response)
    }

    private func record(_ event: String) {
        events.append(event)
        print(event)
        fflush(stdout)
    }

    private func finish(error: String?) {
        guard !finished else { return }
        finished = true
        if let error { record("FAIL \(error)") }
        writeReport(error: error)
        timeoutTask?.cancel()
        discoveryTask?.cancel()
        connections.forEach { $0.invalidate() }
        process?.invalidate()
        NSApp.terminate(nil)
    }

    private func writeReport(error: String?) {
        let report: [String: Any] = ["success": error == nil, "events": events, "error": error ?? ""]
        if let data = try? JSONSerialization.data(withJSONObject: report, options: [.prettyPrinted, .sortedKeys]) {
            try? data.write(to: reportURL, options: .atomic)
        }
    }
}

private struct SmokeError: Error, CustomStringConvertible { let description: String; init(_ description: String) { self.description = description } }

private final class SmokeReply: @unchecked Sendable {
    private let lock = NSLock()
    var continuation: CheckedContinuation<Data, any Error>?

    func finish(_ result: Result<Data, any Error>) {
        lock.lock()
        let continuation = self.continuation
        self.continuation = nil
        lock.unlock()
        continuation?.resume(with: result)
    }
}

private final class SmokeHostAPI: NSObject, NoraPluginHostXPC, Sendable {
    func request(_ request: Data, reply: @escaping @Sendable (Data) -> Void) {
        guard (try? NoraPluginWire.decodeRequest(request).method) == "host.info" else {
            reply(Data(#"{"error":"Unsupported host request"}"#.utf8))
            return
        }
        let result = NoraPluginResponse(resultJSON: #"{"name":"Nora Smoke Host"}"#)
        reply((try? NoraPluginWire.encode(result)) ?? Data())
    }
}
