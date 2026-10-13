import XCTest
@testable import AgentRouter

/// A ChatGPT login pasted into the router from the iPhone (AGEN-0194): the add request it
/// builds, a paste that is not a ChatGPT sign-in refused before sending, the steps to extract
/// one, and the account's identity and plan as its row says them.
final class ChatGPTLoginTests: XCTestCase {
    /// An unsigned JWT with these claims, as the router reads one: never verified.
    private func jwt(_ claims: [String: Any]) -> String {
        let enc = { (d: Data) in d.base64EncodedString().replacingOccurrences(of: "=", with: "").replacingOccurrences(of: "+", with: "-").replacingOccurrences(of: "/", with: "_") }
        return [enc(Data(#"{"alg":"RS256"}"#.utf8)), enc(try! JSONSerialization.data(withJSONObject: claims)), enc(Data("sig".utf8))].joined(separator: ".")
    }

    private var login: String {
        let id = jwt(["email": "you@example.com", "https://api.openai.com/auth": ["chatgpt_plan_type": "pro"]])
        let access = jwt(["exp": 1_900_000_000, "client_id": "app_test"])
        let json: [String: Any] = ["auth_mode": "chatgpt", "OPENAI_API_KEY": NSNull(),
                                   "tokens": ["id_token": id, "access_token": access, "refresh_token": "rt", "account_id": "ws"],
                                   "last_refresh": "2026-10-13T00:00:00Z"]
        return String(decoding: try! JSONSerialization.data(withJSONObject: json), as: UTF8.self)
    }

    func testTheAddRequestIsAnOpenAIAccountCarryingTheWholeLogin() throws {
        let login = self.login
        let body = ChatGPTLogin.body(label: "plan", login: "  \(login)\n")
        let fields = try XCTUnwrap(JSONSerialization.jsonObject(with: JSONEncoder().encode(body)) as? [String: String])
        XCTAssertEqual(fields["source"], "chatgpt")
        XCTAssertEqual(fields["provider"], "openai")
        XCTAssertEqual(fields["label"], "plan")
        XCTAssertEqual(fields["token"], login, "the whole auth.json, trimmed")
        XCTAssertNil(fields["base_url"], "a ChatGPT login goes to ChatGPT's own backend")
    }

    func testAPasteThatIsNotAChatGPTSignInIsRefusedBeforeSending() {
        XCTAssertNil(ChatGPTLogin.problem(login))
        XCTAssertNil(ChatGPTLogin.problem("   "), "blank is the form's to ask for")
        XCTAssertEqual(ChatGPTLogin.problem("sk-proj-abc"), "that is not Codex's auth.json: paste the whole file, which is JSON")
        XCTAssertTrue(ChatGPTLogin.problem(#"{"auth_mode":"apikey","OPENAI_API_KEY":"sk-x"}"#)?.contains("API key") == true)
        XCTAssertTrue(ChatGPTLogin.problem(#"{"auth_mode":"chatgpt","tokens":{"access_token":"a.b.c"}}"#)?.contains("no ChatGPT sign-in") == true)
    }

    func testTheStepsMakeALoginOfTheRoutersOwn() {
        let steps = ChatGPTLogin.steps.joined(separator: "\n")
        XCTAssertTrue(steps.contains("CODEX_HOME=$(mktemp -d) codex login -c cli_auth_credentials_store=file"), "a sign-in of its own, kept in a file")
        XCTAssertTrue(steps.contains("$CODEX_HOME/auth.json"))
        XCTAssertTrue(steps.contains("Delete that folder"))
    }

    func testTheLoginAndItsAccountSayWhoSignedInAndOnWhichPlan() throws {
        let who = ChatGPTLogin.identity(login)
        XCTAssertEqual(who.email, "you@example.com")
        XCTAssertEqual(who.plan, "pro")
        let row = #"{"id":"plan","label":"plan","email":"you@example.com","subscription_type":"pro","active":1,"source":"chatgpt","provider":"openai","ready":true,"detail":"Codex ChatGPT login"}"#
        let a = try JSONDecoder().decode(Account.self, from: Data(row.utf8))
        XCTAssertTrue(a.isChatGPTLogin)
        XCTAssertEqual(a.providerName, "OpenAI")
        XCTAssertEqual(a.subscriptionType, "pro")
        XCTAssertEqual(a.sourceWords, "ChatGPT login · serves Codex only")
        XCTAssertFalse(a.takesUsageKey, "a claude.ai session key reads Anthropic's usage alone")
        let codex = try JSONDecoder().decode(Account.self, from: Data(#"{"id":"codex-default","label":"Codex login","active":1,"source":"codex","provider":"openai"}"#.utf8))
        XCTAssertEqual(codex.sourceWords, "Codex login")
    }
}
