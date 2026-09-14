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
    var status: LiveStatus?
    var connected = false
    var notice: Notice?
    var stats: SessionStats?
    var project: ProjectStats?
    var options: LiveOptions?
    var switching = false
    var sending = false
    var stopping = false

    init(session: LiveSession, client: RouterClient?) {
        self.session = session
        self.client = client
    }

    /// Only a wrapped session can be stopped; an unwrapped one keeps Send and finds
    /// out on the click, as the web view does.
    var canStop: Bool { session.wrapped && (status?.isBusy ?? false) }

    /// The stream, for as long as the page is open. A dropped connection is retried,
    /// and the feed rebuilt from the router's backfill rather than appended to twice.
    func run() async {
        guard let client else {
            notice = Notice(text: "That router has no usable address.", ok: false)
            return
        }
        let path = "/api/live/stream?session=\(session.sessionId)"
        while !Task.isCancelled {
            rows.removeAll()
            do {
                for try await ev in client.events(path) {
                    connected = true
                    switch ev.event {
                    case nil:
                        if let item = try? JSONDecoder().decode(LiveItem.self, from: Data(ev.data.utf8)) {
                            rows.append(Row(id: UUID(), item: item, pending: false, answered: false))
                            if rows.count > 600 { rows.removeFirst(rows.count - 600) }
                        }
                    case "status":
                        status = try? JSONDecoder().decode(LiveStatus.self, from: Data(ev.data.utf8))
                    case "error":
                        notice = Notice(text: ev.data, ok: false)
                    default:
                        break
                    }
                }
            } catch let e as RouterClient.Failure where e == .unauthorized || e == .noPasswordSet {
                connected = false
                notice = Notice(text: e.errorDescription ?? "Refused.", ok: false)
                return
            } catch {
                connected = false
            }
            if Task.isCancelled { return }
            try? await Task.sleep(for: .seconds(2))
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
            } else if status?.state == "ask" {
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

    /// The editor's Submit, from the phone. When the prompt cannot be answered directly
    /// — an unwrapped session, or a prompt already gone — the choices are still worth
    /// sending as a message, and saying so beats losing what was picked.
    func answer(_ answers: [String: String], fallback: [String], row id: UUID) async {
        guard let client else { return }
        do {
            let _: LiveReply = try await client.post("/api/live/answer", AnswerBody(session: session.sessionId, answers: answers))
            if let i = rows.firstIndex(where: { $0.id == id }) { rows[i].answered = true }
            notice = Notice(text: "Answered.", ok: true)
        } catch {
            notice = Notice(text: Self.describe(error), ok: false)
            await send(fallback.joined(separator: "\n"))
        }
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
struct AnswerBody: Encodable, Sendable { let session: String; let answers: [String: String] }
struct ArchiveBody: Encodable, Sendable { let session: String; let archived: Bool }
