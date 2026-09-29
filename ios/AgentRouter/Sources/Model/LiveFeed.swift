// One open session: its feed as the router streams it, and the ways back into it.

import Foundation
import Observation

@MainActor
@Observable
final class LiveFeed {
    struct Row: Identifiable, Hashable {
        let id: UUID
        var item: LiveItem
        /// Echoed the moment a send succeeds; the real row arrives when the session
        /// writes it to its transcript, which on a busy session can be minutes.
        var pending: Bool
        var answered: Bool
    }

    struct Notice: Equatable {
        let text: String
        let ok: Bool
    }

    let session: LiveSession
    @ObservationIgnored private let client: RouterClient?
    private(set) var rows: [Row] = []
    /// What the stream last said the session is doing. Kept across a drop, so the header
    /// can say it is reconnecting rather than connecting for the first time — but only
    /// shown while `connected` says it is current.
    var status: LiveStatus?
    /// True from the first status on a connection until that connection ends, fails or
    /// goes silent. While false, the last status is history, not news.
    var connected = false
    var notice: Notice?
    var stats: SessionStats?
    var project: ProjectStats?
    var options: LiveOptions?
    /// Set only while the session is blocked on a permission request; a question is
    /// left to the feed, which has it from the transcript.
    var pendingPrompt: LivePrompt?
    /// The question the session is blocked on, when the router has one waiting under the
    /// wrapper. Nothing draws it — the feed has the question — but its questions say whether
    /// a submitted form is this one's, and its request id is what the answer names.
    @ObservationIgnored var questionPrompt: LivePrompt?
    var switching = false
    var sending = false
    var stopping = false

    /// How long the stream may say nothing before what it last said counts as stale — ten
    /// seconds, the same on the web view and on both phones, so no client goes on vouching
    /// for a status another has already dropped. A current router sends a status every
    /// second, so this is ten missed in a row: the link is gone, and a phone that has left
    /// the LAN otherwise hears no error until the request times out, 45 s later. An older
    /// router speaks only on a change or once the transcript has been quiet for three
    /// seconds, and bookkeeping lines can keep it from being quiet: over a week of real
    /// transcripts its silence passed ten seconds once, at about eleven. There the cost is
    /// one needless reconnect, whose backfill rebuilds the same feed.
    @ObservationIgnored var staleAfter: Duration = .seconds(10)
    /// When the stream last delivered anything at all.
    @ObservationIgnored private var heard = ContinuousClock.now
    /// Set as a connection opens, cleared by the first thing it delivers — which is when
    /// the old rows give way to the router's backfill. Not before: while a dead link is
    /// being retried, the rows already read stay readable.
    @ObservationIgnored private var fresh = false
    /// Prompts answered from here: never offered again, even if a poll lands before the
    /// router has seen the answer go through.
    @ObservationIgnored private var answeredPrompts: Set<String> = []

    init(session: LiveSession, client: RouterClient?) {
        self.session = session
        self.client = client
    }

    /// Only a wrapped session can be stopped; an unwrapped one keeps Send and finds
    /// out on the click, as the web view does. Only while a turn is running — not for
    /// background work alone, whose results an interrupt could discard — and only while
    /// the stream is current: a stale status would offer Stop for a turn long over.
    var canStop: Bool { session.wrapped && connected && (status?.isBusy ?? false) }

    /// The stream, for as long as the page is open. A dropped connection is retried,
    /// and the feed rebuilt from the router's backfill rather than appended to twice.
    func run() async {
        guard let client else {
            notice = Notice(text: "That router has no usable address.", ok: false)
            return
        }
        let path = "/api/live/stream?session=\(session.sessionId)"
        while !Task.isCancelled {
            do {
                try await follow(client.events(path))
            } catch let e as RouterClient.Failure where e == .unauthorized || e == .noPasswordSet {
                notice = Notice(text: e.errorDescription ?? "Refused.", ok: false)
                return
            } catch {
                // Dropped, refused, or gone silent: all retried the same way.
            }
            if Task.isCancelled { return }
            try? await Task.sleep(for: .seconds(2))
        }
    }

    /// One connection's worth of the stream: until it ends, fails, or goes silent for
    /// longer than the router ever is. Silence ends it as surely as an error does, so the
    /// retry opens a fresh connection instead of waiting out a dead one. However it ends,
    /// `connected` is false afterwards — the header must not go on showing the last line.
    func follow(_ events: AsyncThrowingStream<SSEEvent, Error>) async throws {
        heard = .now
        fresh = true
        defer { connected = false }
        try await withThrowingTaskGroup(of: Void.self) { group in
            // Plain children that hop to the main actor per call: a child closure marked
            // `@MainActor` here is a shape Swift 6's isolation checker rejects outright.
            group.addTask {
                for try await ev in events { await self.take(ev) }
            }
            group.addTask {
                try await self.untilSilent()
            }
            // Whichever finishes first ends the connection; the other is cancelled, and a
            // cancelled stream closes its request.
            _ = try await group.next()
            group.cancelAll()
        }
    }

    struct Silence: Error {}

    /// Returns only by throwing: once nothing has arrived for `staleAfter`.
    private func untilSilent() async throws {
        while true {
            let due = heard.advanced(by: staleAfter)
            if ContinuousClock.now >= due { throw Silence() }
            try await Task.sleep(until: due, clock: .continuous)
        }
    }

    private func take(_ ev: SSEEvent) {
        heard = .now
        if fresh {
            // A new connection replays its backfill: start the feed over rather than
            // append the same rows twice.
            fresh = false
            rows.removeAll()
        }
        switch ev.event {
        case nil:
            if let item = try? JSONDecoder().decode(LiveItem.self, from: Data(ev.data.utf8)) {
                rows.append(Row(id: UUID(), item: item, pending: false, answered: false))
                if rows.count > 600 { rows.removeFirst(rows.count - 600) }
            }
        case "status":
            status = try? JSONDecoder().decode(LiveStatus.self, from: Data(ev.data.utf8))
            connected = true
        case "error":
            notice = Notice(text: ev.data, ok: false)
        default:
            break
        }
    }

    func send(_ raw: String) async {
        let text = raw.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !text.isEmpty, let client else { return }
        sending = true
        defer { sending = false }
        do {
            let reply: LiveReply = try await client.post("/api/live/send", SendBody(session: session.sessionId, text: text))
            rows.append(Row(id: UUID(),
                            item: LiveItem(t: nil, role: "user", kind: "text", name: nil, text: text, ask: nil, durMs: nil, sidechain: false),
                            pending: true, answered: false))
            if reply.framed == true {
                notice = Notice(text: "Sent, but as a peer message — the session will see it as coming from another Claude, not from you. Install the wrapper on the Mac and restart the session for it to arrive as yours.", ok: false)
            } else if status?.needsYou == true {
                // Measured on the web view: a queued turn does not satisfy a pending
                // question. Saying "delivered" would be true and misleading.
                notice = Notice(text: "Sent as a message — it may not dismiss the prompt.", ok: false)
            } else {
                notice = Notice(text: "Delivered as your message.", ok: true)
            }
        } catch {
            notice = Notice(text: Self.describe(error), ok: false)
        }
    }

    func interrupt() async {
        guard let client else { return }
        stopping = true
        defer { stopping = false }
        do {
            let _: LiveReply = try await client.post("/api/live/interrupt", InterruptBody(session: session.sessionId))
            notice = Notice(text: "Stopped.", ok: true)
        } catch {
            notice = Notice(text: Self.describe(error), ok: false)
        }
    }

    /// The editor's Submit, from the phone. `questions` are the texts the form was drawn
    /// for, in order: the form comes from the transcript, which carries no prompt id, so
    /// they are what say whether the question waiting now is the one it answers. When the
    /// prompt cannot be answered directly — an unwrapped session, or a prompt already gone
    /// — the choices are still worth sending as a message, and saying so beats losing what
    /// was picked.
    func answer(_ answers: [String: String], questions: [String], fallback: [String], row id: UUID) async {
        guard let client else { return }
        // What is waiting now, not what the last poll saw up to two seconds ago: a form
        // stays on screen after its question was answered in the editor, and the model may
        // have asked another since. A failed look changes nothing and the last poll stands.
        await loadPrompt()
        let question: String?
        switch Self.target(for: questions, question: questionPrompt, permission: pendingPrompt) {
        case .refused(let why):
            // Nothing is sent, not even as a message, and the choices stay picked.
            notice = Notice(text: why, ok: false)
            return
        case .named(let id):
            question = id
        case .unnamed:
            question = nil
        }
        do {
            let _: LiveReply = try await client.post("/api/live/answer",
                AnswerBody(session: session.sessionId, answers: answers, requestId: question))
            if let question { answeredPrompts.insert(question) }
            questionPrompt = nil
            if let i = rows.firstIndex(where: { $0.id == id }) { rows[i].answered = true }
            notice = Notice(text: "Answered.", ok: true)
        } catch {
            notice = Notice(text: Self.describe(error), ok: false)
            await send(fallback.joined(separator: "\n"))
        }
    }

    /// Where a submitted form's answers may go, given what is waiting now.
    enum AnswerTarget: Equatable, Sendable {
        /// The question waiting is this form's: name it, so the router answers exactly it
        /// or says it has gone.
        case named(String)
        /// Nothing under the wrapper to name — an unwrapped session, or no question the
        /// router reports. Sent unnamed, a current router answers only a lone waiting prompt
        /// and says why not otherwise, and the choices then go as a message instead.
        case unnamed
        /// Sending would settle something the form was not drawn for. Nothing goes.
        case refused(String)
    }

    /// The web view's rule, so a form one client refuses to send the others refuse too.
    /// A waiting question is named only when it asks what the form asks (`LivePrompt.asks`):
    /// otherwise the router, told to, would put these answers on a question they were never
    /// given for, and the editor's dialog for it would close unseen. A waiting permission
    /// request refuses as well — an answer that named no prompt would land on it, and to
    /// answer it is to allow it. The router offers the newest prompt first, so the question
    /// comes up once that one is settled, the choices still picked in the form.
    ///
    /// The status's `waiting_on` is not consulted: it names the same prompt the prompt
    /// endpoint returns, but without its questions, so it cannot say whether that prompt is
    /// this form's.
    nonisolated static func target(for questions: [String], question: LivePrompt?, permission: LivePrompt?) -> AnswerTarget {
        if let question {
            guard question.asks(questions) else { return .refused("Not sent: a different question is waiting now.") }
            if let id = question.requestId { return .named(id) }
        }
        if permission != nil { return .refused("A permission request is waiting too — answer it first, then submit these again.") }
        return .unnamed
    }

    /// A tab's dot. The open session's tab says what its stream says, like the header
    /// beside it — the list row under it can be five seconds old — for as long as the
    /// stream vouches for it. Every other tab, and this one while the stream is not
    /// current, is its list row.
    func look(of s: LiveSession) -> StateLook {
        if s.sessionId == session.sessionId, connected, let status { return status.look }
        return s.look
    }

    /// A side question: answered with the session's context in a fork the session never
    /// sees. Both the question and the answer are shown here only.
    func aside(_ raw: String) async {
        let text = raw.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !text.isEmpty, let client else { return }
        switching = true
        defer { switching = false }
        rows.append(Row(id: UUID(), item: LiveItem(t: nil, role: "user", kind: "text", name: nil, text: text, ask: nil, durMs: nil, sidechain: false), pending: true, answered: false))
        notice = Notice(text: "Asking on the side — this replays the session once, usually under a minute.", ok: true)
        do {
            let r: AsideReply = try await client.post("/api/live/aside", AsideBody(session: session.sessionId, text: text))
            rows.append(Row(id: UUID(), item: LiveItem(t: nil, role: "assistant", kind: "aside", name: nil, text: r.answer ?? "(no answer)", ask: nil, durMs: r.tookMs, sidechain: false), pending: false, answered: false))
            notice = Notice(text: "Answered in \(Fmt.duration(TimeInterval(r.tookMs ?? 0) / 1000))" + (r.cost.map { " · \(Fmt.usd($0))" } ?? ""), ok: true)
        } catch {
            notice = Notice(text: Self.describe(error), ok: false)
        }
    }

    /// The model is the editor's picker over the control channel; the CLI confirms or
    /// refuses and that is what the notice says. Effort is `/effort <level>` typed in.
    func setModel(_ model: String) async {
        guard let client else { return }
        switching = true
        defer { switching = false }
        do {
            let r: SwitchReply = try await client.post("/api/live/model", ModelBody(session: session.sessionId, model: model))
            let said = r.notices?.joined(separator: " ")
            notice = Notice(text: said?.isEmpty == false ? said! : (r.pending == true ? "Model sent — the session has not confirmed yet." : "Model set to \(model) — takes effect on the next turn."), ok: true)
        } catch {
            notice = Notice(text: Self.describe(error), ok: false)
        }
        await loadFacts()
    }

    /// Change what the session stops to ask about. The CLI can refuse — `bypassPermissions`
    /// needs the session to have been launched with `--dangerously-skip-permissions`, and
    /// `auto` can be gated — so its own sentence is what gets shown.
    func setMode(_ mode: String) async {
        guard let client else { return }
        switching = true
        defer { switching = false }
        do {
            let r: SwitchReply = try await client.post("/api/live/mode", ModeBody(session: session.sessionId, mode: mode))
            let took = r.mode ?? mode
            notice = Notice(text: r.pending == true ? "Mode sent — the session has not confirmed yet."
                                                    : "Now \(took).", ok: true)
        } catch {
            notice = Notice(text: Self.describe(error), ok: false)
        }
        await loadFacts()
    }

    /// What the session is blocked on, when it is a permission request rather than a
    /// question. It arrives as a control frame between the CLI and its editor, so it is
    /// never in the transcript the feed is built from and has to be asked for.
    func loadPrompt() async {
        guard let client else { return }
        guard let p: LivePrompt = try? await client.get("/api/live/prompt?session=\(session.sessionId)") else { return }
        let done = p.requestId.map { answeredPrompts.contains($0) } ?? false
        pendingPrompt = p.isPermission && !done ? p : nil
        questionPrompt = p.isQuestion && !done ? p : nil
    }

    /// Allow the waiting tool, optionally taking one of the prompt's own standing
    /// offers, or refuse it. The answer names the prompt the card showed: another may
    /// have arrived since, and a tap on one must never settle the other — the router
    /// refuses instead, and the card is refreshed to show what is waiting now.
    func answerPermission(_ prompt: LivePrompt, allow: Bool, suggestion: Int? = nil, said: String) async {
        guard let client else { return }
        switching = true
        defer { switching = false }
        do {
            let body = AnswerBody(session: session.sessionId, allow: allow, suggestion: suggestion,
                                  message: allow ? nil : "Declined from the phone.",
                                  requestId: prompt.requestId)
            let _: LiveReply = try await client.post("/api/live/answer", body)
            if let id = prompt.requestId { answeredPrompts.insert(id) }
            pendingPrompt = nil
            notice = Notice(text: said, ok: true)
        } catch {
            notice = Notice(text: Self.describe(error), ok: false)
            await loadPrompt()
        }
    }

    func setEffort(_ level: String) async {
        guard let client else { return }
        switching = true
        defer { switching = false }
        do {
            let _: SwitchReply = try await client.post("/api/live/effort", EffortBody(session: session.sessionId, effort: level))
            notice = Notice(text: "Effort set to \(level) — takes effect on the next turn.", ok: true)
        } catch {
            notice = Notice(text: Self.describe(error), ok: false)
        }
        await loadFacts()
    }

    /// Totals need a whole-file scan, so they are read once per open rather than
    /// pushed with every event.
    func loadFacts() async {
        guard let client else { return }
        if options == nil { options = try? await client.get("/api/live/options") }
        stats = try? await client.get("/api/live/stats?session=\(session.sessionId)")
        if let cwd = session.cwd,
           let q = cwd.addingPercentEncoding(withAllowedCharacters: .urlQueryAllowed) {
            project = try? await client.get("/api/live/project?cwd=\(q)")
        }
    }

    static func describe(_ error: Error) -> String {
        (error as? RouterClient.Failure)?.errorDescription ?? error.localizedDescription
    }
}

struct SendBody: Encodable, Sendable { let session: String; let text: String }
struct InterruptBody: Encodable, Sendable { let session: String }
/// `answers` alone is how a question is answered, and how this app always spoke. The
/// rest is a permission request: let it through, optionally applying one of the offers
/// the prompt itself made, or refuse it with a reason.
struct AnswerBody: Encodable, Sendable {
    let session: String
    var answers: [String: String] = [:]
    var allow: Bool? = nil
    var suggestion: Int? = nil
    var message: String? = nil
    /// The prompt this answers. A router that knows the field answers exactly that prompt
    /// or refuses with 409; an older one ignores it. Left out when unknown, which a
    /// current router accepts only while a single prompt is waiting.
    var requestId: String? = nil

    enum CodingKeys: String, CodingKey {
        case session, answers, allow, suggestion, message, requestId = "request_id"
    }
}
struct ArchiveBody: Encodable, Sendable { let session: String; let archived: Bool }
