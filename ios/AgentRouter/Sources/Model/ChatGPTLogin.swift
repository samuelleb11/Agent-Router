// A ChatGPT login pasted into the router as an OpenAI account (AGEN-0194): Codex's `auth.json`
// from a sign-in made for the router, which the router keeps, refreshes and serves to Codex
// alone. The steps and the check before sending are the router's (src/chatgpt.rs).

import Foundation

enum ChatGPTLogin {
    /// The way an OpenAI account is added from a ChatGPT login, beside an API key.
    static let source = "chatgpt"

    /// How to extract a login of the router's own, so it never shares a session with the
    /// machine's Codex. `CODEX_HOME` is exported, so the shell still names the folder when the
    /// login is printed and deleted.
    static let signIn = #"export CODEX_HOME=$(mktemp -d) && codex login -c cli_auth_credentials_store=file && cat "$CODEX_HOME/auth.json""#
    static let cleanUp = #"rm -rf "$CODEX_HOME" && unset CODEX_HOME"#
    static let steps = [
        "On a machine with Codex, run this and sign in with ChatGPT:",
        signIn,
        "Paste what it prints — that folder’s auth.json — here, then delete the folder:",
        cleanUp,
    ]

    /// What the row says of an account added this way.
    static let servesCodexOnly = "serves Codex only"

    /// Why a pasted text is not a ChatGPT sign-in the router can keep; nil when it is.
    static func problem(_ raw: String) -> String? {
        let text = raw.trimmingCharacters(in: .whitespacesAndNewlines)
        if text.isEmpty { return nil }
        guard let v = try? JSONSerialization.jsonObject(with: Data(text.utf8)) as? [String: Any] else {
            return "that is not Codex's auth.json: paste the whole file, which is JSON"
        }
        if v["auth_mode"] as? String == "apikey" || ((v["OPENAI_API_KEY"] as? String)?.isEmpty == false) {
            return "that login is an OpenAI API key, not a ChatGPT sign-in — add the key as an API-key account instead"
        }
        let tokens = v["tokens"] as? [String: Any]
        let has = { (key: String) in (tokens?[key] as? String)?.isEmpty == false }
        guard has("access_token"), has("refresh_token"), has("id_token") else {
            return "that auth.json holds no ChatGPT sign-in: sign in with ChatGPT in Codex first, then paste the file"
        }
        return nil
    }

    /// Who signed in and on which plan, from the id token, read as the router reads it: never
    /// verified, since OpenAI is what checks it.
    static func identity(_ raw: String) -> (email: String?, plan: String?) {
        guard let v = try? JSONSerialization.jsonObject(with: Data(raw.utf8)) as? [String: Any],
              let id = (v["tokens"] as? [String: Any])?["id_token"] as? String,
              let claims = claims(id) else { return (nil, nil) }
        let email = claims["email"] as? String ?? (claims["https://api.openai.com/profile"] as? [String: Any])?["email"] as? String
        let plan = (claims["https://api.openai.com/auth"] as? [String: Any])?["chatgpt_plan_type"] as? String
        return (email, plan)
    }

    static func claims(_ jwt: String) -> [String: Any]? {
        let parts = jwt.split(separator: ".")
        guard parts.count == 3 else { return nil }
        var payload = parts[1].replacingOccurrences(of: "-", with: "+").replacingOccurrences(of: "_", with: "/")
        while payload.count % 4 != 0 { payload += "=" }
        guard let data = Data(base64Encoded: payload) else { return nil }
        return try? JSONSerialization.jsonObject(with: data) as? [String: Any]
    }

    /// The request that adds it: an OpenAI account whose credential is the whole login.
    static func body(label: String, login: String) -> AddAccountBody {
        AddAccountBody(label: label, source: source, token: login.trimmingCharacters(in: .whitespacesAndNewlines), provider: "openai")
    }
}
