// One router, one password. Plain HTTP on the LAN — the router's own trust model — or
// HTTPS to a hosted router, which is added by its https URL.

import Foundation

struct RouterClient: Sendable {
    let baseURL: URL
    let password: String?
    /// Where `get` and `post` go: the app's one session, unless a test hands over one that
    /// answers from a table instead of the network.
    var calls: URLSession = RouterClient.session
    /// Where `events` goes, likewise: the session made for streams, unless a test hands
    /// over one of its own.
    var streams: URLSession = RouterClient.streamSession

    enum Failure: Error, LocalizedError, Sendable, Equatable {
        case unreachable(String)
        /// 401: the router has a password and this is not it.
        case unauthorized
        /// 403 with the router's own message: nobody has set a password on it yet.
        case noPasswordSet
        /// An error status that says why in the router's own words — `error`, or the live
        /// endpoints' `reason` — which are shown as they are. A refused change answers this
        /// way: a password too short, a key that does not sign, a sign-in already running.
        case refused(Int, String)
        /// An error status with nothing to say for itself: a guard's plain text, a proxy's page.
        case status(Int, String)
        case decoding(String)

        var errorDescription: String? {
            switch self {
            case .unreachable(let why): "Can't reach the router — \(why)"
            case .unauthorized: "Wrong password for this router"
            case .noPasswordSet: "This router has no password yet — set one on its dashboard, on the Mac"
            case .refused(_, let words): words
            case .status(let code, let body): "Router returned \(code)\(body.isEmpty ? "" : ": \(body)")"
            case .decoding(let why): "Unexpected reply from the router — \(why)"
            }
        }

        /// An error status the router sent, said in its words or not: what a refused
        /// stream ends with, as against a dropped one.
        var isErrorStatus: Bool {
            switch self {
            case .refused, .status: true
            default: false
            }
        }
    }

    static let session: URLSession = {
        let c = URLSessionConfiguration.ephemeral
        c.timeoutIntervalForRequest = 12
        c.timeoutIntervalForResource = 30
        c.waitsForConnectivity = false
        return URLSession(configuration: c)
    }()

    /// For event streams, which live as long as the page that opened them. The router
    /// sends a status line at least every few seconds while quiet, so the idle timeout
    /// only fires when something is actually wrong.
    static let streamSession: URLSession = {
        let c = URLSessionConfiguration.ephemeral
        c.timeoutIntervalForRequest = 45
        c.timeoutIntervalForResource = 7 * 24 * 3600
        c.waitsForConnectivity = false
        return URLSession(configuration: c)
    }()

    static func authorization(password: String) -> String {
        // Any username satisfies the router; only the password counts.
        "Basic " + Data("app:\(password)".utf8).base64EncodedString()
    }

    func get<T: Decodable>(_ path: String) async throws -> T {
        try await send(request(path, method: "GET", body: nil))
    }

    /// `get` with a query string; nil and empty values are left out, as the dashboard does.
    func get<T: Decodable>(_ path: String, query: [String: String?]) async throws -> T {
        var items = query.compactMap { k, v -> URLQueryItem? in
            guard let v, !v.isEmpty else { return nil }
            return URLQueryItem(name: k, value: v)
        }
        items.sort { $0.name < $1.name }
        var c = URLComponents()
        c.queryItems = items.isEmpty ? nil : items
        return try await get(path + (c.percentEncodedQuery.map { "?\($0)" } ?? ""))
    }

    /// A query value encoded as the web console's `encodeURIComponent` encodes it: everything
    /// but ASCII letters, digits and `-_.!~*'()`, so a `/`, `&`, `+` or `#` in a path or a
    /// repository reaches the router as written.
    static func queryValue(_ value: String) -> String {
        value.addingPercentEncoding(withAllowedCharacters: componentAllowed) ?? value
    }

    private static let componentAllowed = CharacterSet(
        charactersIn: "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_.!~*'()")

    func post<T: Decodable, B: Encodable & Sendable>(_ path: String, _ body: B) async throws -> T {
        try await send(request(path, method: "POST", body: try JSONEncoder().encode(body)))
    }

    private func request(_ path: String, method: String, body: Data?) throws -> URLRequest {
        guard let url = URL(string: path, relativeTo: baseURL)?.absoluteURL else {
            throw Failure.unreachable("bad address")
        }
        var r = URLRequest(url: url)
        r.httpMethod = method
        r.setValue("application/json", forHTTPHeaderField: "accept")
        if let password, !password.isEmpty {
            r.setValue(Self.authorization(password: password), forHTTPHeaderField: "authorization")
        }
        if let body {
            r.httpBody = body
            r.setValue("application/json", forHTTPHeaderField: "content-type")
        }
        return r
    }

    private func send<T: Decodable>(_ req: URLRequest) async throws -> T {
        let data: Data
        let response: URLResponse
        do {
            (data, response) = try await calls.data(for: req)
        } catch let e as URLError {
            throw Failure.unreachable(e.localizedDescription)
        } catch {
            throw Failure.unreachable(error.localizedDescription)
        }
        guard let http = response as? HTTPURLResponse else { throw Failure.unreachable("not HTTP") }
        guard (200..<300).contains(http.statusCode) else { throw Self.failure(status: http.statusCode, body: data) }
        let reply: T
        do { reply = try JSONDecoder().decode(T.self, from: data) }
        catch { throw Failure.decoding(String(describing: error).prefix(160).description) }
        // A refusal all the same, and read as one: see `Verdict`.
        if let verdict = reply as? any Verdict, !verdict.ok { throw Self.failure(status: http.statusCode, body: data) }
        return reply
    }

    /// What an error status stands for, read the same way from a call's reply and from a
    /// stream's: the router's own words when the body carries them (`refused`), and
    /// otherwise the status it sent with what little the body says. A stream once threw the
    /// bare status without reading the body, so a hosted router's "the machine running this
    /// session (…) is offline" could only have read "Router returned 503".
    ///
    /// A refusal arrives as an error status with its reason inside — a router from before
    /// that answered the same refusal as a 200 carrying `error`, which is why
    /// `OkReply.problem` still looks there. Either way the screen shows those words.
    static func failure(status: Int, body data: Data) -> Failure {
        let text = String(decoding: data, as: UTF8.self).trimmingCharacters(in: .whitespacesAndNewlines)
        switch status {
        case 401:
            return .unauthorized
        case 403 where text.contains("no password set"):
            return .noPasswordSet
        default:
            if let words = reason(in: data), !words.isEmpty { return .refused(status, words) }
            return .status(status, text.prefix(160).description)
        }
    }

    /// More than any refusal's few words, and all of an error body a stream reads.
    private static let errorBodyMax = 16_384

    /// The live endpoints refuse with `{"ok":false,"reason":…}` and the rest with
    /// `{"error":…}` (the wrapper switches send both); either is better than the raw body.
    static func reason(in data: Data) -> String? {
        guard let obj = try? JSONSerialization.jsonObject(with: data) as? [String: Any] else { return nil }
        return (obj["reason"] as? String) ?? (obj["error"] as? String)
    }

    /// Server-sent events from `path`, until the connection ends or the consumer stops.
    func events(_ path: String) -> AsyncThrowingStream<SSEEvent, Error> {
        AsyncThrowingStream { continuation in
            let task = Task {
                do {
                    var req = try request(path, method: "GET", body: nil)
                    req.setValue("text/event-stream", forHTTPHeaderField: "accept")
                    let (bytes, response) = try await streams.bytes(for: req)
                    guard let http = response as? HTTPURLResponse else { throw Failure.unreachable("not HTTP") }
                    guard (200..<300).contains(http.statusCode) else {
                        // Refused: the body says why, as any call's does, in a few words. A body
                        // that cannot be read still leaves the status that was sent.
                        var body = Data()
                        do {
                            for try await byte in bytes {
                                body.append(byte)
                                if body.count >= Self.errorBodyMax { break }
                            }
                        } catch {}
                        throw Self.failure(status: http.statusCode, body: body)
                    }
                    // Not `bytes.lines`: AsyncLineSequence drops empty lines, and an
                    // SSE frame is terminated *by* an empty line — through `.lines` no
                    // frame ever ends and the feed stays blank forever.
                    var parser = SSEParser()
                    var splitter = SSELineSplitter()
                    for try await byte in bytes {
                        if let line = splitter.feed(byte: byte), let event = parser.feed(line: line) {
                            continuation.yield(event)
                        }
                    }
                    continuation.finish()
                } catch let e as URLError {
                    continuation.finish(throwing: Failure.unreachable(e.localizedDescription))
                } catch {
                    continuation.finish(throwing: error)
                }
            }
            continuation.onTermination = { _ in task.cancel() }
        }
    }
}

/// A reply that says whether the change was made: `ok`, beside the reason when it was not,
/// as the live endpoints answer. `ok: false` is a refusal whatever status comes with it, so a
/// call reads it as it reads an error status (`RouterClient.failure`): in the router's words,
/// and never as done. Answering a question or a permission prompt once read the status
/// alone, and a 200 saying `ok: false` said "Answered." and put away a prompt still waiting.
protocol Verdict {
    var ok: Bool { get }
}

/// Bytes into lines, every line — including the empty ones the framing depends on.
struct SSELineSplitter: Sendable {
    private var buf: [UInt8] = []

    mutating func feed(byte: UInt8) -> String? {
        guard byte == UInt8(ascii: "\n") else {
            buf.append(byte)
            return nil
        }
        if buf.last == UInt8(ascii: "\r") { buf.removeLast() }
        let line = String(decoding: buf, as: UTF8.self)
        buf.removeAll(keepingCapacity: true)
        return line
    }
}

struct SSEEvent: Equatable, Sendable {
    /// Nil for an unnamed `data:` message.
    var event: String?
    var data: String
}

/// The text/event-stream framing, one line at a time: `event:` and `data:` fields
/// accumulate, a blank line delivers, `:` lines are keep-alives.
struct SSEParser: Sendable {
    private var event: String?
    private var data: [String] = []

    mutating func feed(line: String) -> SSEEvent? {
        if line.isEmpty {
            defer { event = nil; data = [] }
            guard !data.isEmpty else { return nil }
            return SSEEvent(event: event, data: data.joined(separator: "\n"))
        }
        if line.hasPrefix(":") { return nil }
        let field: Substring
        var value: Substring
        if let colon = line.firstIndex(of: ":") {
            field = line[..<colon]
            value = line[line.index(after: colon)...]
            if value.hasPrefix(" ") { value = value.dropFirst() }
        } else {
            field = line[...]
            value = ""
        }
        switch field {
        case "event": event = String(value)
        case "data": data.append(String(value))
        default: break
        }
        return nil
    }
}
