import XCTest
@testable import AgentRouter

/// Settings → Access tokens on the phone, as the console has it (AGEN-0180): each token with its
/// state, a new one made with a scope and an expiry and shown once, and one revoked.
final class AccessTokensTests: XCTestCase {
    private let now = Date(timeIntervalSince1970: 1_791_860_000)

    private let reply = #"""
    {"tokens":[
     {"id":"tok_a","name":"Codex on my laptop","prefix":"art_7Qx","scope":"client","created_at":1791800000000,"last_used_at":1791850000000,"expires_at":1799600000000,"revoked_at":null},
     {"id":"tok_b","name":"desk-pc","prefix":"art_k2P","scope":"machine","created_at":1791000000000,"last_used_at":null,"expires_at":1791500000000,"revoked_at":null},
     {"id":"tok_c","name":"old admin","prefix":"art_zz9","scope":"admin","created_at":1790000000000,"last_used_at":1790100000000,"expires_at":null,"revoked_at":1790200000000}]}
    """#

    func testEachTokenDecodesWithItsState() throws {
        let tokens = try JSONDecoder().decode(AccessTokensReply.self, from: Data(reply.utf8)).tokens
        XCTAssertEqual(tokens.map { AccessTokens.status($0, now: now) }, ["Active", "Expired", "Revoked"])
        XCTAssertEqual(tokens.map { AccessTokens.scopeLabel($0.scope) }, ["Client", "Machine", "Administrator"])
        XCTAssertEqual(tokens[0].prefix, "art_7Qx")
        XCTAssertNil(tokens[1].lastUsedAt, "never used")
        XCTAssertNil(tokens[2].expiresAt, "never expires")
    }

    func testANewTokenIsMadeWithEachScopeAndExpiry() throws {
        XCTAssertEqual(AccessTokens.scopes.map(\.value), ["client", "machine", "admin"])
        XCTAssertEqual(AccessTokens.lifetimes.map(\.label), ["30 days", "90 days", "1 year", "Never"])
        XCTAssertEqual(AccessTokens.expiry(days: 30, now: now), 1_791_860_000_000 + 30 * 86_400_000)
        XCTAssertNil(AccessTokens.expiry(days: nil, now: now))
        let body = CreateTokenBody(name: "desk-pc", scope: "machine", expiresAt: AccessTokens.expiry(days: 365, now: now))
        let fields = try XCTUnwrap(JSONSerialization.jsonObject(with: JSONEncoder().encode(body)) as? [String: Any])
        XCTAssertEqual(fields["name"] as? String, "desk-pc")
        XCTAssertEqual(fields["scope"] as? String, "machine")
        XCTAssertEqual(fields["expires_at"] as? Int, 1_791_860_000_000 + 365 * 86_400_000)
        let never = String(decoding: try JSONEncoder().encode(CreateTokenBody(name: "x", scope: "admin", expiresAt: nil)), as: UTF8.self)
        XCTAssertTrue(never.contains(#""expires_at":null"#), "never is sent as null: \(never)")
    }

    func testTheTokenMadeIsReadOnceWithItsTerms() throws {
        let made = try JSONDecoder().decode(CreatedToken.self, from: Data(#"{"ok":true,"id":"tok_d","name":"phone","prefix":"art_abc","scope":"client","created_at":1791860000000,"last_used_at":null,"expires_at":null,"revoked_at":null,"token":"art_abcdefghijklmnop"}"#.utf8))
        XCTAssertEqual(made.token, "art_abcdefghijklmnop")
        XCTAssertEqual(AccessTokens.createdWords(made), "Permissions: Client. It does not expire.")
        XCTAssertTrue(AccessTokens.shownOnce.hasPrefix("This is the only time the full token is shown."))
    }

    func testARevokeNamesItsTokenInThePath() {
        XCTAssertEqual("/api/auth/tokens/" + RouterClient.segment("tok_a"), "/api/auth/tokens/tok_a")
        XCTAssertEqual(RouterClient.segment("a/b c"), "a%2Fb%20c", "an id stays one segment")
        XCTAssertTrue(AccessTokens.revokeWhat.hasPrefix("This token will stop authenticating new requests immediately."))
    }
}
