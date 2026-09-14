// This phone's notification centre, as this app uses it. Banners go up when a session
// blocks on a person; they come down when the prompt behind one is answered — wherever
// it was answered, which is usually not here.
//
// Two ways down, because neither alone is enough. The router sends a background push
// naming what was answered, which is immediate and silent but which Apple is free to
// drop, and always drops while the app is force quit. So whenever the app comes to the
// front it also asks each router what is still waiting and clears whatever is left over.

import Foundation
import UserNotifications

/// Deliberately not main-actor: a `UNNotification` is not `Sendable`, so the
/// notifications themselves stay on this side and only strings cross over.
enum Notifications {
    /// One banner on screen, reduced to what can be reasoned about elsewhere.
    struct OnScreen: Sendable {
        /// iOS's own handle for the banner — what removing it takes.
        let id: String
        /// The router that raised it, as it named itself in the payload.
        let router: String?
        /// The prompt behind it: `req:…` from a wrapped session, `ask:…` from a bare one.
        let prompt: String
    }

    /// Banners raised by a prompt, and only those. A test push or the UI test's own
    /// notification is left alone — nothing here can say whether it is still wanted.
    static func onScreen() async -> [OnScreen] {
        await UNUserNotificationCenter.current().deliveredNotifications().compactMap { n in
            let info = n.request.content.userInfo
            guard let prompt = info["request_id"] as? String,
                  prompt.hasPrefix("req:") || prompt.hasPrefix("ask:") else { return nil }
            return OnScreen(id: n.request.identifier, router: info["router_ref"] as? String, prompt: prompt)
        }
    }

    /// Take down the banners raised by these prompts.
    static func takeDown(answered: Set<String>) async {
        guard !answered.isEmpty else { return }
        let stale = await onScreen().filter { answered.contains($0.prompt) }.map(\.id)
        remove(stale)
    }

    static func remove(_ ids: [String]) {
        guard !ids.isEmpty else { return }
        UNUserNotificationCenter.current().removeDeliveredNotifications(withIdentifiers: ids)
    }
}

extension Notifications {
    /// Ask every router that raised a banner whether its prompt is still waiting, and
    /// take down the ones that are not. A router that cannot be reached keeps its
    /// banners: a phone off the LAN knows nothing, and guessing would hide real prompts.
    @MainActor
    static func reconcile(store: RouterStore) async {
        let showing = await onScreen()
        guard !showing.isEmpty else { return }
        for router in store.routers {
            let mine = showing.filter { $0.router == router.id.uuidString }
            guard !mine.isEmpty, let client = store.state(for: router).liveClient else { continue }
            let reply: PendingReply? = try? await client.get("/api/push/pending")
            guard let waiting = reply.map({ Set($0.waiting) }) else { continue }
            remove(mine.filter { !waiting.contains($0.prompt) }.map(\.id))
        }
    }
}
