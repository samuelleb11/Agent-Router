// The providers the router serves, named as the console names them (ui/src/model/settings.ts,
// `providerName`, `ADD_PROVIDERS`), with each provider's rules for a credential and a base URL
// (ui/src/screens/Settings/rules.ts), so the phone refuses before sending what the router would.

import Foundation

enum Provider {
    /// The providers an account can be added for, in the order the console offers them.
    static let addable = ["anthropic", "kimi", "openai"]

    /// A provider by its name. An account from before providers has none, and is Anthropic's.
    static func name(_ id: String?) -> String {
        switch id {
        case nil, "anthropic": "Anthropic"
        case "kimi": "Kimi Code"
        case "openai": "OpenAI"
        case let other?: other
        }
    }

    /// Where a provider's requests go when its account names no base URL of its own.
    static func defaultBaseURL(_ id: String?) -> String? {
        switch id {
        case "kimi": "https://api.kimi.ai/coding"
        case "openai": "https://api.openai.com"
        default: nil
        }
    }

    /// Anthropic first, then the order the console adds them in, then anything else by name.
    static func rank(_ id: String?) -> Int {
        addable.firstIndex(of: id ?? "anthropic") ?? addable.count
    }

    /// A credential checked by its provider's rule (src/api.rs `accounts_edit`, src/accounts.rs
    /// `Provider::key_problem`): the `sk-ant-` rule is Anthropic's alone. Blank is nobody's problem
    /// here: an editor reads it as "keep", and the add form asks for a key on its own.
    static func keyProblem(_ provider: String, _ key: String) -> String? {
        let k = key.trimmingCharacters(in: .whitespacesAndNewlines)
        if k.isEmpty { return nil }
        switch provider {
        case "kimi":
            return k.hasPrefix("sk-ant-") ? "that is an Anthropic token — a Kimi Code account takes a key from the Kimi Code console" : nil
        case "openai":
            if k.hasPrefix("sk-ant-") { return "that is an Anthropic token — an OpenAI account takes an OpenAI API key" }
            return k.hasPrefix("sk-") ? nil : "expected an OpenAI API key starting sk-; use the Codex login source for a ChatGPT subscription"
        default:
            if k.hasPrefix("sk-ant-sid") { return "that is a claude.ai session key — it belongs in the session-key field, and cannot sign API requests" }
            return k.hasPrefix("sk-ant-") ? nil : "expected a token starting sk-ant- (from `claude setup-token`)"
        }
    }

    /// The router's rule for an account's base URL (src/api.rs `base_url_for`): an https address
    /// with a host, a trailing slash dropped, named as typed when it is not one. Blank means the
    /// provider's own.
    static func baseURLProblem(_ url: String) -> String? {
        let u = trimmedBaseURL(url)
        if u.isEmpty { return nil }
        if let parsed = URL(string: u), parsed.scheme == "https", let host = parsed.host, !host.isEmpty { return nil }
        return "\"\(u)\" is not an https:// address"
    }

    /// A base URL as it is sent: trimmed, its trailing slashes dropped.
    static func trimmedBaseURL(_ url: String) -> String {
        var u = url.trimmingCharacters(in: .whitespacesAndNewlines)
        while u.hasSuffix("/") { u.removeLast() }
        return u
    }
}
