import Testing
import NoraPluginKit

struct NoraPluginVersionTests {
    @Test("SemVer precedence orders numeric identifiers, prereleases, and releases")
    func precedence() {
        let ordered = ["1.0.0-alpha", "1.0.0-alpha.1", "1.0.0-alpha.beta", "1.0.0-beta", "1.0.0-beta.2", "1.0.0-beta.11", "1.0.0-rc.1", "1.0.0", "1.0.1", "1.1.0", "2.0.0"]
        for (lower, upper) in zip(ordered, ordered.dropFirst()) {
            #expect(NoraPluginVersion.isNewer(upper, than: lower))
            #expect(!NoraPluginVersion.isNewer(lower, than: upper))
        }
    }

    @Test("Build metadata does not affect precedence and large numeric components do not overflow")
    func metadataAndLargeNumbers() {
        #expect(NoraPluginVersion("1.0.0+one") == NoraPluginVersion("1.0.0+two"))
        #expect(!NoraPluginVersion.isNewer("1.0.0+two", than: "1.0.0+one"))
        #expect(NoraPluginVersion.isNewer("999999999999999999999999999.0.0", than: "99999999999999999999999999.0.0"))
        #expect(!NoraPluginVersion.isNewer("bad", than: "1.0.0"))
    }
}
