// A router's Settings as the console's pages (ui/src/model/settings.ts, `settingsPages`,
// `connectSnippet`, the notes; ui/src/lib/routes.ts, `SECTIONS`): named and ordered as the
// console names and orders them, for the router's mode.

import Foundation

enum SettingsPage: String, Hashable, Sendable, CaseIterable, Identifiable {
    case accounts, models, clients
    case claudeCode = "claude-code"
    case network
    case accessTokens = "access-tokens"
    case password, notifications, router, forges, library

    var id: String { rawValue }

    var title: String {
        switch self {
        case .accounts: "Accounts"
        case .models: "Models"
        case .clients: "Clients"
        case .claudeCode: "Claude Code"
        case .network: "Network"
        case .accessTokens: "Access tokens"
        case .password: "Password"
        case .notifications: "Notifications"
        case .router: "Router"
        case .forges: "Forges"
        case .library: "Library"
        }
    }

    var icon: String {
        switch self {
        case .accounts: "person.crop.circle"
        case .models: "cpu"
        case .clients: "terminal"
        case .claudeCode: "laptopcomputer"
        case .network: "network"
        case .accessTokens: "key"
        case .password: "lock"
        case .notifications: "bell"
        case .router: "server.rack"
        case .forges: "shippingbox"
        case .library: "books.vertical"
        }
    }
}

/// `POST /api/network`: listen beyond this Mac, or stop.
struct NetworkBody: Encodable, Sendable { let enable: Bool }

enum SettingsPages {
    /// The pages offered: every router has Models after Accounts; a hosted router has no Claude
    /// Code and no Network page, and alone has Forges and Library; a router that has not said
    /// yet what it is has none of those.
    static func pages(hosted: Bool?) -> [SettingsPage] {
        switch hosted {
        case true?: [.accounts, .models, .clients, .accessTokens, .password, .notifications, .router, .forges, .library]
        case false?: [.accounts, .models, .clients, .claudeCode, .network, .accessTokens, .password, .notifications, .router]
        case nil: [.accounts, .models, .clients, .accessTokens, .password, .notifications, .router]
        }
    }

    /// Whether the setup and the wrapper are read at all: never on a hosted router, which would
    /// refuse, nor before the router has said which it is.
    static func readsSetup(hosted: Bool?) -> Bool { hosted == false }

    /// What a project's `.claude/settings.local.json` takes to route through this router: the
    /// base URL, and a credential line when the router is hosted or off loopback.
    static func connectSnippet(origin: String, hosted: Bool, fromLoopback: Bool?) -> String {
        let url = "\"ANTHROPIC_BASE_URL\": \"\(origin)\""
        return hosted || fromLoopback == false
            ? "{ \"env\": {\n  \(url),\n  \"ANTHROPIC_AUTH_TOKEN\": \"<Client access token or router password>\"\n} }"
            : "{ \"env\": { \(url) } }"
    }

    /// Where the router keeps what its pages set.
    static func storeNote(hosted: Bool) -> String {
        hosted ? "Everything above is kept in the router’s data directory. Its own settings (port, database location, upstream, body cap) are set by the deployment and read at startup."
            : "Everything above is stored locally. The router's own settings (port, database location, upstream, body cap) are environment variables read at startup — see the README."
    }

    static let networkExplain = "Off, the router answers only on this Mac. On, it also listens on your local network, so the phone apps — and a Claude Code on another machine — can reach it; every one of them must present an access token or the router password. It takes effect at once, with nothing restarted and no session dropped."
    static let networkOff = "The router answers only on this Mac from then on: the phones and any Claude Code on another machine lose it until it is turned on again here."
}
