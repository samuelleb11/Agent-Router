import XCTest
@testable import AgentRouter

/// Two replies, read as the router sends them.
///
/// The probe of a profile account answers what `claude auth status --json` says of it, as
/// the router reads it and sends it on (src/accounts.rs, `AuthStatus`): `loggedIn`. The phone
/// read `logged_in`, which the router never sends, so a profile not yet signed in was never
/// said to be.
///
/// An answer to a question or a permission prompt that the router refuses says so with
/// `ok: false`. The phone read the status alone, so `ok: false` in a 200 read as success:
/// "Answered.", and the prompt put away while it was still waiting. It is read as an error
/// status is now (`Verdict`): the router's words, and the prompt still there.
///
/// Each call goes through the screens' own state and the app's own client to the stand-in
/// router `RefusalTests` uses, so nothing leaves the process. The replies are the router's,
/// word for word (`api::accounts_probe`, and `live::answer` with its `bad`).
final class ProbeAndAnswerTests: XCTestCase {
    @MainActor
    private func router() -> RouterState {
        StandInRouter.reset()
        let config = URLSessionConfiguration.ephemeral
        config.protocolClasses = [StandInRouter.self]
        return RouterState(
            router: Router(name: "Stand-in", host: "router.invalid", port: 443, scheme: .https),
            password: nil,
            calls: URLSession(configuration: config)
        )
    }

    /// An open session's feed, calling the stand-in.
    @MainActor
    private func feed() throws -> LiveFeed {
        let json = #"{"session_id":"s-1","cwd":"/code/p","project":"-code-p","git_branch":null,"title":null,"created_ms":0,"last_activity_ms":0,"active":true,"state":"ask","archived":false,"wrapped":true,"bytes":10}"#
        let session = try JSONDecoder().decode(LiveSession.self, from: Data(json.utf8))
        return LiveFeed(session: session, client: router().liveClient)
    }

    /// What the router answers when a prompt it was asked to settle is not there to settle.
    private let gone = "That prompt is no longer waiting"

    /// The router sends every field of `AuthStatus`, `null` for one the CLI did not say.
    @MainActor
    func testTheProbesLoggedInIsReadAsTheRouterSendsIt() async throws {
        let accounts = AccountsState(router: router())
        StandInRouter.answer("/api/accounts/probe", 200,
            #"{"loggedIn":false,"email":null,"orgName":null,"subscriptionType":null,"authMethod":null}"#)
        let waiting = await accounts.probe("side")
        XCTAssertNotNil(waiting, "the reply reads")
        XCTAssertEqual(waiting?.loggedIn, false, "not signed in yet, and said so")

        StandInRouter.answer("/api/accounts/probe", 200,
            #"{"loggedIn":true,"email":"me@example.com","orgName":"Example","subscriptionType":"max","authMethod":"claude.ai"}"#)
        let signedIn = await accounts.probe("side")
        XCTAssertEqual(signedIn?.loggedIn, true)
        XCTAssertEqual(StandInRouter.posts, ["/api/accounts/probe", "/api/accounts/probe"])
    }

    /// The form's own question is waiting, and named; the router refuses the answer, in a 200
    /// and in the 409 a current router sends. The same words either way, and the question is
    /// still the one waiting. Nothing was picked to go as a message instead, so nothing does.
    @MainActor
    func testAnAnswerToAQuestionTheRouterRefusesWithOkFalseIsShownAsARefusal() async throws {
        let feed = try feed()
        StandInRouter.answer("/api/live/prompt", 200,
            #"{"pending":true,"request_id":"req-Q","tool":"AskUserQuestion","question":true,"input":{"questions":[{"question":"Which database?"}]}}"#)
        for status in [200, 409] {
            StandInRouter.answer("/api/live/answer", status, #"{"ok":false,"reason":"\#(gone)"}"#)
            await feed.answer(["Which database?": "SQLite"], questions: ["Which database?"], fallback: [], row: UUID())
            XCTAssertEqual(feed.notice, LiveFeed.Notice(text: gone, ok: false), "\(status) is a refusal, not \"Answered.\"")
            XCTAssertEqual(feed.questionPrompt?.requestId, "req-Q", "\(status): still waiting")
        }
        XCTAssertEqual(StandInRouter.posts, ["/api/live/answer", "/api/live/answer"])
    }

    /// The card tapped is the one waiting; the router refuses the tap's answer, in a 200 and
    /// in a 409. The card stays up, saying why.
    @MainActor
    func testAnAnswerToAPermissionPromptTheRouterRefusesWithOkFalseIsShownAsARefusal() async throws {
        let feed = try feed()
        StandInRouter.answer("/api/live/prompt", 200,
            #"{"pending":true,"request_id":"req-P","tool":"Bash","question":false,"display_name":"Bash","description":"rm -r build"}"#)
        await feed.loadPrompt()
        let card = try XCTUnwrap(feed.pendingPrompt, "the card the reader taps")
        for status in [200, 409] {
            StandInRouter.answer("/api/live/answer", status, #"{"ok":false,"reason":"\#(gone)"}"#)
            await feed.answerPermission(card, allow: true, said: "Allowed.")
            XCTAssertEqual(feed.notice, LiveFeed.Notice(text: gone, ok: false), "\(status) is a refusal, not \"Allowed.\"")
            XCTAssertEqual(feed.pendingPrompt?.requestId, "req-P", "\(status): still waiting, so still shown")
        }
        XCTAssertEqual(StandInRouter.posts, ["/api/live/answer", "/api/live/answer"])
    }
}
