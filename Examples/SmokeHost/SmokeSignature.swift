import Foundation
import Security

enum SmokeSignature {
    static func verify(processIdentifier: pid_t, extensionURL: URL) throws -> String {
        var expectedCode: SecStaticCode?
        try check(SecStaticCodeCreateWithPath(extensionURL as CFURL, [], &expectedCode))
        guard let expectedCode else { throw SignatureError(message: "Missing installed extension signature") }
        let expected = try information(expectedCode)
        guard let hash = expected[kSecCodeInfoUnique as String] as? Data,
              let identifier = expected[kSecCodeInfoIdentifier as String] as? String,
              let expectedExecutable = expected[kSecCodeInfoMainExecutable as String] as? URL else {
            throw SignatureError(message: "Incomplete installed extension signature")
        }
        var guest: SecCode?
        try check(SecCodeCopyGuestWithAttributes(nil, [kSecGuestAttributePid as String: NSNumber(value: processIdentifier)] as CFDictionary, [], &guest))
        guard let guest else { throw SignatureError(message: "Missing running extension signature") }
        let digest = hash.map { String(format: "%02x", $0) }.joined()
        var requirement: SecRequirement?
        try check(SecRequirementCreateWithString("identifier \"\(identifier)\" and cdhash H\"\(digest)\"" as CFString, [], &requirement))
        try check(SecCodeCheckValidity(guest, [], requirement))
        var actualCode: SecStaticCode?
        try check(SecCodeCopyStaticCode(guest, [], &actualCode))
        guard let actualCode else { throw SignatureError(message: "Missing running extension code") }
        let actual = try information(actualCode)
        guard let actualExecutable = actual[kSecCodeInfoMainExecutable as String] as? URL,
              actualExecutable.resolvingSymlinksInPath().standardizedFileURL == expectedExecutable.resolvingSymlinksInPath().standardizedFileURL,
              actual[kSecCodeInfoUnique as String] as? Data == hash else {
            throw SignatureError(message: "Running extension path or signature differs from its installed bundle")
        }
        return "PASS peer PID \(processIdentifier), cdhash \(digest), executable \(actualExecutable.path)"
    }

    private static func information(_ code: SecStaticCode) throws -> [String: Any] {
        var information: CFDictionary?
        try check(SecCodeCopySigningInformation(code, SecCSFlags(rawValue: kSecCSSigningInformation), &information))
        guard let information = information as? [String: Any] else { throw SignatureError(message: "Missing signing information") }
        return information
    }

    private static func check(_ status: OSStatus) throws {
        guard status == errSecSuccess else {
            throw SignatureError(message: "Security error \(status): \(SecCopyErrorMessageString(status, nil) as String? ?? "Unknown error")")
        }
    }

    private struct SignatureError: Error, CustomStringConvertible {
        let message: String
        var description: String { message }
    }
}
