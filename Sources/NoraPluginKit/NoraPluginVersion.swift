import Foundation

public struct NoraPluginVersion: Hashable, Sendable, Comparable {
    private let components: [String]
    private let prerelease: [String]

    public init?(_ value: String) {
        let pattern = #"^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)(?:-((?:0|[1-9][0-9]*|[0-9]*[A-Za-z-][0-9A-Za-z-]*)(?:\.(?:0|[1-9][0-9]*|[0-9]*[A-Za-z-][0-9A-Za-z-]*))*))?(?:\+([0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*))?$"#
        guard value.utf8.count <= 128, value.range(of: pattern, options: .regularExpression) != nil,
              !value.contains(where: { $0.isWhitespace }) else { return nil }
        let precedence = value.split(separator: "+", maxSplits: 1)[0]
        let parts = precedence.split(separator: "-", maxSplits: 1)
        components = parts[0].split(separator: ".").map(String.init)
        prerelease = parts.count == 2 ? parts[1].split(separator: ".").map(String.init) : []
    }

    public static func isNewer(_ candidate: String, than current: String) -> Bool {
        guard let candidate = Self(candidate), let current = Self(current) else { return false }
        return candidate > current
    }

    public static func < (lhs: Self, rhs: Self) -> Bool {
        for (left, right) in zip(lhs.components, rhs.components) where left != right {
            return numericLess(left, right)
        }
        if lhs.prerelease.isEmpty || rhs.prerelease.isEmpty {
            return !lhs.prerelease.isEmpty && rhs.prerelease.isEmpty
        }
        for (left, right) in zip(lhs.prerelease, rhs.prerelease) where left != right {
            let leftNumeric = left.allSatisfy(\.isNumber)
            let rightNumeric = right.allSatisfy(\.isNumber)
            if leftNumeric != rightNumeric { return leftNumeric }
            return leftNumeric ? numericLess(left, right) : left < right
        }
        return lhs.prerelease.count < rhs.prerelease.count
    }

    private static func numericLess(_ lhs: String, _ rhs: String) -> Bool {
        lhs.count == rhs.count ? lhs < rhs : lhs.count < rhs.count
    }
}
