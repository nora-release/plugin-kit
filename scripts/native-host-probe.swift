import AppKit
import CoreGraphics
import Darwin

@MainActor
private final class RegistrationObserver: NSObject {
    var processIdentifier: pid_t = 0
    var violations = Set<String>()
    var observations = 0
    var preview = false
    var visibleWindowObserved = false
    var accessoryPreviewObserved = false
    var executableURL: URL?

    func matches(_ application: NSRunningApplication) -> Bool {
        application.processIdentifier == processIdentifier || application.executableURL == executableURL
    }

    @objc func applicationActivated(_ notification: Notification) {
        guard !preview,
              let application = notification.userInfo?[NSWorkspace.applicationUserInfoKey] as? NSRunningApplication,
              matches(application) else { return }
        violations.insert("became frontmost")
    }

    @objc func applicationLaunched(_ notification: Notification) {
        guard let application = notification.userInfo?[NSWorkspace.applicationUserInfoKey] as? NSRunningApplication,
              matches(application) else { return }
        processIdentifier = application.processIdentifier
        sample()
    }

    func sample() {
        if processIdentifier == 0, let application = NSWorkspace.shared.runningApplications.first(where: matches) {
            processIdentifier = application.processIdentifier
        }
        var accessory = false
        if let application = NSRunningApplication(processIdentifier: processIdentifier) {
            observations += 1
            if application.activationPolicy == .regular { violations.insert("regular activation policy") }
            if !preview && application.isActive { violations.insert("became active") }
            accessory = application.activationPolicy == .accessory
        }
        if !preview && NSWorkspace.shared.frontmostApplication?.processIdentifier == processIdentifier {
            violations.insert("became frontmost")
        }
        let windows = CGWindowListCopyWindowInfo([.optionAll, .excludeDesktopElements], kCGNullWindowID) as? [[String: Any]] ?? []
        let ownWindows = windows.filter { ($0[kCGWindowOwnerPID as String] as? NSNumber)?.int32Value == processIdentifier }
        if !preview && !ownWindows.isEmpty { violations.insert("created a window") }
        if ownWindows.contains(where: {
            ($0[kCGWindowIsOnscreen as String] as? Bool) == true &&
            ($0[kCGWindowLayer as String] as? NSNumber)?.intValue == 0
        }) {
            visibleWindowObserved = true
            if accessory { accessoryPreviewObserved = true }
        }
    }
}

@MainActor
private final class WorkspaceLaunch {
    var completed = false
    var application: NSRunningApplication?
    var error: String?
}

@main
enum RegistrationProbe {
    @MainActor static func main() throws {
        guard CommandLine.arguments.count == 3,
              ["register", "preview", "workspace"].contains(CommandLine.arguments[2]) else {
            throw NSError(domain: "NativeHostProbe", code: 1, userInfo: [NSLocalizedDescriptionKey: "Expected an exact containing-host executable path and register/preview/workspace mode"])
        }
        let mode = CommandLine.arguments[2]
        let preview = mode == "preview"
        let workspace = mode == "workspace"
        let process = Process()
        process.executableURL = URL(fileURLWithPath: CommandLine.arguments[1])
        process.arguments = preview ? [] : ["--nora-register"]
        process.standardOutput = FileHandle.standardError
        process.standardError = FileHandle.standardError
        let observer = RegistrationObserver()
        observer.preview = preview
        observer.executableURL = process.executableURL
        guard !NSWorkspace.shared.runningApplications.contains(where: observer.matches) else {
            throw NSError(domain: "NativeHostProbe", code: 2, userInfo: [NSLocalizedDescriptionKey: "Refusing to inspect an already running containing host"])
        }
        let center = NSWorkspace.shared.notificationCenter
        center.addObserver(observer, selector: #selector(RegistrationObserver.applicationActivated(_:)),
                           name: NSWorkspace.didActivateApplicationNotification, object: nil)
        center.addObserver(observer, selector: #selector(RegistrationObserver.applicationLaunched(_:)),
                           name: NSWorkspace.didLaunchApplicationNotification, object: nil)
        defer { center.removeObserver(observer) }
        let launch = WorkspaceLaunch()
        let start = Date()
        if workspace {
            let configuration = NSWorkspace.OpenConfiguration()
            configuration.activates = false
            configuration.hides = true
            configuration.createsNewApplicationInstance = true
            configuration.arguments = ["--nora-register"]
            let appURL = process.executableURL!.deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
            NSWorkspace.shared.openApplication(at: appURL, configuration: configuration) { application, error in
                DispatchQueue.main.async {
                    launch.completed = true
                    launch.application = application
                    launch.error = error?.localizedDescription
                    if let application { observer.processIdentifier = application.processIdentifier }
                }
            }
        } else {
            try process.run()
            observer.processIdentifier = process.processIdentifier
        }
        func running() -> Bool {
            workspace ? (!launch.completed || launch.application?.isTerminated == false) : process.isRunning
        }
        defer {
            if workspace {
                if let application = NSRunningApplication(processIdentifier: observer.processIdentifier), !application.isTerminated {
                    application.forceTerminate()
                }
            } else if process.isRunning {
                kill(process.processIdentifier, SIGKILL)
            }
        }
        repeat {
            observer.sample()
            RunLoop.main.run(until: Date().addingTimeInterval(0.001))
        } while running() && Date().timeIntervalSince(start) < 3 && (!preview || !observer.accessoryPreviewObserved)
        let elapsed = Date().timeIntervalSince(start)
        if preview && !observer.visibleWindowObserved { observer.violations.insert("preview did not show a visible window within 3 seconds") }
        if preview && !observer.accessoryPreviewObserved { observer.violations.insert("preview window was never observed with accessory activation policy") }
        if workspace && (!launch.completed || launch.application == nil || launch.error != nil) {
            observer.violations.insert("workspace launch did not return an application without an error: \(launch.error ?? "no application")")
        }
        if running() {
            if preview {
                process.terminate()
                let cleanupDeadline = Date().addingTimeInterval(0.5)
                while process.isRunning && Date() < cleanupDeadline {
                    RunLoop.main.run(until: Date().addingTimeInterval(0.001))
                }
            } else {
                observer.violations.insert("did not exit within 3 seconds")
            }
            if !workspace && process.isRunning { kill(process.processIdentifier, SIGKILL) }
        }
        if !workspace { process.waitUntilExit() }
        RunLoop.main.run(until: Date().addingTimeInterval(0.02))
        if observer.observations == 0 { observer.violations.insert("process was never observable") }
        let report: [String: Any] = [
            "exitStatus": workspace ? NSNull() : process.terminationStatus as Any,
            "elapsedSeconds": elapsed,
            "observations": observer.observations,
            "mode": mode,
            "visibleWindowObserved": observer.visibleWindowObserved,
            "accessoryPreviewObserved": observer.accessoryPreviewObserved,
            "workspaceCompleted": launch.completed,
            "workspaceReturnedApplication": launch.application != nil,
            "violations": observer.violations.sorted(),
        ]
        let data = try JSONSerialization.data(withJSONObject: report, options: [.sortedKeys])
        print(String(decoding: data, as: UTF8.self))
        if (!preview && !workspace && process.terminationStatus != 0) || !observer.violations.isEmpty { exit(1) }
    }
}
