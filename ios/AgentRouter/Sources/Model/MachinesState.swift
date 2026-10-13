// A hosted router's machines, read for the Machines screen while it is open.

import Foundation
import Observation

@MainActor
@Observable
final class MachinesState {
    let router: RouterState
    /// What the router last listed. Nil until a read has succeeded, so a list that could not
    /// be read is never shown as one that is empty.
    var rows: [MachineRow]?
    /// Why the last read failed, when it did. Whatever was read before stays on screen.
    var failure: String?
    /// The router's health, read with the list for its build: a hosted router is upgraded on
    /// its own, and may be while the screen is open.
    var health: Health?

    init(router: RouterState) { self.router = router }

    private var client: RouterClient? { router.liveClient }

    /// The router's own build: nil until its health has answered, and from a router that
    /// names none.
    var routerBuild: String? { (health ?? router.health)?.build }

    /// The machines as the screen lists them, measured at `now`.
    func items(now: Date) -> [Machines.Item] {
        Machines.items(rows ?? [], routerBuild: routerBuild, now: now)
    }

    func load() async {
        guard let client else {
            failure = "That router has no usable address."
            return
        }
        async let list: [MachineRow] = client.get("/api/machines")
        async let h: Health = client.get("/_router/health")
        // Each awaited on its own: one that throws must not leave the other pending.
        do { rows = try await list; failure = nil } catch { failure = LiveFeed.describe(error) }
        if let read = try? await h { health = read }
    }

    /// What the last sync the screen asked for came to, by machine, in words.
    var told: [String: String] = [:]
    /// The machine being told to sync now, for its row to show it.
    var syncing: String?

    /// Tells one machine to sync its Library now, through the tunnel its agent holds open, and
    /// says what it answered; a refusal in the router's words.
    func sync(_ machine: String) async {
        guard let client else { return }
        syncing = machine
        defer { syncing = nil }
        do {
            let reply: LibrarySyncReply = try await client.post("/api/library/sync", LibrarySyncBody(machine: machine))
            for t in reply.machines { told[t.machine] = Machines.told(t) }
            if reply.machines.isEmpty { told[machine] = "The router told no machine." }
        } catch {
            told[machine] = LiveFeed.describe(error)
        }
        await load()
    }

    /// Every five seconds, as the session list is read, until the screen goes away. Each
    /// read draws the list again, so every "online for" is measured anew with it.
    func poll(every seconds: Double = 5) async {
        while !Task.isCancelled {
            await load()
            do { try await Task.sleep(for: .seconds(seconds)) } catch { return }
        }
    }
}
