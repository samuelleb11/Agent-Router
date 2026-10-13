import XCTest
@testable import AgentRouter

/// Settings → Forges and Settings → Library on the phone, as the console has them (AGEN-0185):
/// the forges, sources, publish tokens and machine-publish setting decoded, a forge's check and a
/// source's imports in words, and each of their requests.
final class LibrarySettingsTests: XCTestCase {
    private let forges = #"{"forges":[{"key":"team-gitlab","kind":"gitlab","base_url":"https://gitlab.example.com","token_present":true,"sources":2,"created_at":1791581336033},{"key":"public-hub","kind":"github","base_url":"https://github.com","token_present":false,"sources":0,"created_at":1791581336034}]}"#
    private let sources = #"""
    {"sources":[
     {"key":"team-skills","forge":"team-gitlab","repository":"team/skills","ref":"main","path":"skills","live":false,"head":"0123456789abcdef","checked_at":1791860508371,"imported_at":1791583811682,"last_result":{"source":"team-skills","commit":"0123456789abcdef","fetched":true,"items":[{"kind":"skill","name":"review","path":"skills/review","outcome":"new","version":2,"state":"pending"},{"kind":"skill","name":"bench","path":"skills/bench","outcome":"unchanged"},{"kind":"command","name":"x","path":"commands/x.md","outcome":"refused","why":"too big"}],"gone":["skill/old"]},"hook":true,"created_at":1791583808774},
     {"key":"lab","forge":"team-gitlab","repository":"team/lab","ref":"dev","path":"","live":true,"head":null,"checked_at":1791860508371,"imported_at":null,"last_result":{"ok":false,"error":"the forge could not be reached: client error (Connect)"},"hook":false,"created_at":1791583808775},
     {"key":"fresh","forge":"public-hub","repository":"owner/repo","ref":"main","path":"","live":false,"head":null,"checked_at":null,"imported_at":null,"last_result":null,"hook":false,"created_at":1791583808776}]}
    """#

    func testForgesSourcesTokensAndTheSettingDecode() throws {
        let f = try JSONDecoder().decode(ForgesReply.self, from: Data(forges.utf8)).forges
        XCTAssertEqual(f.map(\.key), ["team-gitlab", "public-hub"])
        XCTAssertEqual(f.map { LibrarySettings.kindLabel($0.kind) }, ["GitLab", "GitHub"])
        XCTAssertEqual(f.map(\.tokenPresent), [true, false])
        let s = try JSONDecoder().decode(SourcesReply.self, from: Data(sources.utf8)).sources
        XCTAssertEqual(s.map(\.hook), [true, false, false])
        let t = try JSONDecoder().decode(PublishTokensReply.self, from: Data(#"{"tokens":[{"id":"pt_1","name":"ci","kinds":["skill"],"items":[],"live":false,"created_at":1,"last_used_at":null,"revoked_at":null},{"id":"pt_2","name":"one","kinds":[],"items":["skill/bench","command/deploy"],"live":true,"created_at":1,"last_used_at":2,"revoked_at":3}]}"#.utf8)).tokens
        XCTAssertEqual(t.map(LibrarySettings.scopeWords), ["any skill", "skill/bench, command/deploy"])
        XCTAssertEqual(LibrarySettings.scopeWords(PublishToken(id: "x", name: "x", kinds: [], items: [], live: false, createdAt: 0, lastUsedAt: nil, revokedAt: nil)), "anything")
        let setting = try JSONDecoder().decode(LibrarySetting.self, from: Data(#"{"machine_publishes":"approval"}"#.utf8))
        XCTAssertEqual(setting.machinePublishes, "approval")
    }

    func testAForgesCheckAndASourcesImportAreSaidInWords() throws {
        XCTAssertEqual(LibrarySettings.checked(ForgeChecked(result: "unauthorised", words: "401 from GitLab")), "The forge refused the token: 401 from GitLab")
        XCTAssertEqual(LibrarySettings.checked(ForgeChecked(result: "ok", words: "read team/skills")), "The forge answered: read team/skills")
        let s = try JSONDecoder().decode(SourcesReply.self, from: Data(sources.utf8)).sources
        XCTAssertEqual(s.map(LibrarySettings.lastWords), [
            "Imported at 01234567: 1 new, 1 unchanged, 1 refused; no longer in the repository: skill/old.",
            "The forge said no: the forge could not be reached: client error (Connect)",
            "Never imported yet.",
        ])
        let still = Imported(source: "x", commit: "fedcba9876543210", fetched: false, items: [], gone: [])
        XCTAssertEqual(LibrarySettings.importWords(still), "Nothing to fetch: the ref is still at fedcba98.")
    }

    func testEachRequestIsBuiltAsTheConsoleSendsIt() throws {
        func json(_ v: some Encodable) throws -> String { String(decoding: try JSONEncoder().encode(v), as: UTF8.self) }
        let keep = try json(ForgeBody(kind: "gitlab", baseURL: "https://gitlab.example.com", token: .keep))
        XCTAssertFalse(keep.contains("token"), "an absent token keeps the kept one: \(keep)")
        XCTAssertTrue(try json(ForgeBody(kind: "gitlab", baseURL: "https://g", token: .remove)).contains(#""token":null"#), "null removes it")
        XCTAssertTrue(try json(ForgeBody(kind: "github", baseURL: "https://github.com", token: .set("ghp_x"))).contains(#""token":"ghp_x""#))
        XCTAssertEqual(LibrarySettings.forgePath("team-gitlab", "/check"), "/api/library/forges/team-gitlab/check")
        XCTAssertEqual(LibrarySettings.sourcePath("team-skills", "/import"), "/api/library/sources/team-skills/import")
        XCTAssertEqual(LibrarySettings.sourcePath("team-skills", "/secret"), "/api/library/sources/team-skills/secret")
        let source = try XCTUnwrap(JSONSerialization.jsonObject(with: JSONEncoder().encode(SourceBody(forge: "team-gitlab", repository: "team/skills", ref: "main", path: "", live: true))) as? [String: Any])
        XCTAssertEqual(source["repository"] as? String, "team/skills")
        XCTAssertEqual(source["live"] as? Bool, true)
        let token = try XCTUnwrap(JSONSerialization.jsonObject(with: JSONEncoder().encode(PublishTokenBody(name: "ci", kinds: ["skill"], items: LibrarySettings.itemList(" skill/bench, ,command/deploy "), live: false))) as? [String: Any])
        XCTAssertEqual(token["items"] as? [String], ["skill/bench", "command/deploy"])
        XCTAssertEqual(try json(LibrarySetting(machinePublishes: "live")), #"{"machine_publishes":"live"}"#)
        let secret = try JSONDecoder().decode(SecretReply.self, from: Data(#"{"secret":"s3cr3t","path":"/api/library/hooks/team-skills","url":null}"#.utf8))
        XCTAssertNil(secret.url, "the phone's own address stands in for a router that names none")
        XCTAssertTrue(LibrarySettings.isKey("team-skills"))
        XCTAssertFalse(LibrarySettings.isKey("Team Skills"))
        XCTAssertFalse(LibrarySettings.isKey("double--hyphen"))
    }
}
