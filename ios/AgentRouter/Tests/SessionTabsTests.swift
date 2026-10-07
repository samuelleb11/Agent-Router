import XCTest
@testable import AgentRouter

/// A hosted router's session list a machine at a time: All, then a tab for each machine,
/// each saying what is at work there and what waits on you — and no tabs at all for one
/// machine or a local router, whose list stays exactly as it was. The web console tabs its
/// list the same way (SE-C226 to SE-C229).
final class SessionTabsTests: XCTestCase {
    /// A row as the router lists it, as LiveTests builds one. `extra` is spliced in as
    /// further keys: an activity, the machine.
    private func session(_ id: String, project: String, agoHours: Double = 0.1, active: Bool = false,
                         state: String = "idle", archived: Bool = false, extra: String = "", now: Date) -> LiveSession {
        let json = """
        {"session_id":"\(id)","cwd":"/code/\(project)","project":"-code-\(project)","git_branch":null,"title":null,
         "created_ms":0,"last_activity_ms":\(Int(now.addingTimeInterval(-agoHours * 3600).timeIntervalSince1970 * 1000)),
         "active":\(active),"state":"\(state)","archived":\(archived),"wrapped":false,"bytes":10\(extra)}
        """
        return try! JSONDecoder().decode(LiveSession.self, from: Data(json.utf8))
    }

    private func on(_ id: String, _ label: String) -> String { #","machine":{"id":"\#(id)","label":"\#(label)"}"# }
    private let working = #","activity":"generating","tone":"busy""#
    private let tool = #","activity":"tool","tone":"busy""#
    private let waiting = #","activity":"waiting","tone":"attention""#
    private let idle = #","activity":"idle","tone":"idle""#

    /// Sessions on three named machines and one the router named no machine for.
    private func threeMachines(_ now: Date) -> [LiveSession] {
        [
            session("s-gen", project: "agent-router", active: true, state: "working", extra: working + on("5f0c2a8e", "studio"), now: now),
            session("s-ask", project: "northstar", active: true, state: "ask", extra: waiting + on("5f0c2a8e", "studio"), now: now),
            session("s-old", project: "northstar", agoHours: 30, extra: idle + on("5f0c2a8e", "studio"), now: now),
            session("l-tool", project: "agent-router", active: true, state: "working", extra: tool + on("9b3e1d47", "Laptop"), now: now),
            session("l-arc", project: "agent-router", agoHours: 3, archived: true, extra: idle + on("9b3e1d47", "Laptop"), now: now),
            session("b-idle", project: "infra", extra: idle + on("0d1e2f3a", "build-box"), now: now),
            session("u-wait", project: "misc", active: true, state: "ask", extra: waiting, now: now),
        ]
    }

    /// A local router names no machine, and one machine's sessions are not headed by it:
    /// either way there is no strip, and whatever was chosen before, the list is drawn as it
    /// always was.
    func testOneMachineOrALocalRouterHasNoTabs() {
        let now = Date()
        let local = [
            session("aaaa", project: "p", active: true, state: "working", extra: working, now: now),
            session("bbbb", project: "q", active: true, state: "ask", extra: waiting, now: now),
        ]
        let one = [
            session("aaaa", project: "p", active: true, state: "working", extra: working + on("5f0c2a8e", "studio"), now: now),
            session("bbbb", project: "q", extra: idle + on("5f0c2a8e", "studio"), now: now),
        ]
        for (what, rows) in [("a local router", local), ("one machine", one), ("an empty list", [])] {
            let parts = SessionGroups.byMachine(rows, now: now)
            XCTAssertEqual(SessionTabs.tabs(parts), [], what)
            XCTAssertEqual(SessionTabs.parts(parts, showing: nil), parts, what)
            XCTAssertEqual(SessionTabs.parts(parts, showing: "machine:studio"), parts, "\(what): a choice changes nothing")
        }
        XCTAssertFalse(SessionTabs.isUnlisted("machine:studio", rows: one),
                       "the chosen machine, alone in the list, is still listed")
    }

    /// All first, then each machine in the order the list heads them, the unnamed part last;
    /// each tab counts what is at work there and what waits on you in the routers list's
    /// words, and carries the attention dot when anything waits — so a prompt on a machine
    /// that is not on screen is not missed. An archived or older session counts as it does
    /// everywhere else: by what it is doing.
    func testTabsAreAllThenEachMachineWithItsCounts() {
        let now = Date()
        let rows = threeMachines(now)
        let tabs = SessionTabs.tabs(SessionGroups.byMachine(rows, now: now))
        XCTAssertEqual(tabs.map(\.label), ["All", "build-box", "Laptop", "studio", "Unnamed machine"])
        XCTAssertEqual(tabs.map(\.machine), [nil, "machine:build-box", "machine:Laptop", "machine:studio", "unnamed"])
        XCTAssertEqual(tabs.map(\.counts), ["2 working · 2 waiting", "", "1 working", "1 working · 1 waiting", "1 waiting"])
        XCTAssertEqual(tabs.map(\.waiting), [true, false, false, true, true])
        XCTAssertEqual(tabs[0].counts, rows.liveSummary, "All says what the routers list says of the whole router")
        XCTAssertEqual(Set(tabs.map(\.id)).count, tabs.count, "one identity per tab")
        XCTAssertEqual(tabs.map(\.spoken), [
            "All, 2 working · 2 waiting", "build-box, idle", "Laptop, 1 working",
            "studio, 1 working · 1 waiting", "Unnamed machine, 1 waiting",
        ])

        // Two machines, with nothing waiting anywhere: no tab carries the dot.
        let two = rows.filter { $0.machineName == "Laptop" || $0.machineName == "build-box" }
        let twoTabs = SessionTabs.tabs(SessionGroups.byMachine(two, now: now))
        XCTAssertEqual(twoTabs.map(\.label), ["All", "build-box", "Laptop"])
        XCTAssertEqual(twoTabs.map(\.counts), ["1 working", "", "1 working"])
        XCTAssertEqual(twoTabs.map(\.waiting), [false, false, false])

        // An older router names no activity: its word is "active", and nothing is said to wait.
        let older = [
            session("o-1", project: "p", active: true, state: "working", extra: on("5f0c2a8e", "studio"), now: now),
            session("o-2", project: "p", active: true, state: "ask", extra: on("9b3e1d47", "Laptop"), now: now),
            session("o-3", project: "p", extra: on("9b3e1d47", "Laptop"), now: now),
        ]
        let olderTabs = SessionTabs.tabs(SessionGroups.byMachine(older, now: now))
        XCTAssertEqual(olderTabs.map(\.counts), ["2 active", "1 active", "1 active"])
        XCTAssertEqual(olderTabs.map(\.waiting), [false, false, false])
    }

    /// A machine's tab shows that machine's part alone, without its heading, and exactly as
    /// the list headed it: its own projects, its older sessions and its shelf. It keeps the
    /// part's id, so its folds are the same folds on All.
    func testAMachineTabShowsOnlyThatMachine() {
        let now = Date()
        let rows = threeMachines(now)
        let parts = SessionGroups.byMachine(rows, now: now)
        let studio = SessionTabs.parts(parts, showing: "machine:studio")
        XCTAssertEqual(studio.count, 1)
        XCTAssertEqual(studio.first?.id, "machine:studio")
        XCTAssertNil(studio.first?.name, "no heading: the tab names the machine")
        XCTAssertEqual(studio.first?.groups, SessionGroups.build(rows.filter { $0.machineName == "studio" }, now: now))
        XCTAssertEqual(studio.first?.groups.older.flatMap { $0.sessions.map(\.shortId) }, ["s-old"])

        let laptop = SessionTabs.parts(parts, showing: "machine:Laptop")
        XCTAssertEqual(laptop.first?.groups.archived.map(\.shortId), ["l-arc"], "a machine's shelf is on its tab")
        XCTAssertEqual(laptop.first?.groups.rows.map(\.shortId).sorted(), ["l-arc", "l-tool"])

        let unnamed = SessionTabs.parts(parts, showing: "unnamed")
        XCTAssertEqual(unnamed.first?.groups.rows.map(\.shortId), ["u-wait"])
    }

    /// All shows every part under its heading, exactly as the list was before it had tabs.
    func testAllKeepsEveryPart() {
        let now = Date()
        let parts = SessionGroups.byMachine(threeMachines(now), now: now)
        let all = SessionTabs.parts(parts, showing: nil)
        XCTAssertEqual(all, parts)
        XCTAssertEqual(all.map(\.name), ["build-box", "Laptop", "studio", "Unnamed machine"])
        XCTAssertEqual(all.flatMap(\.groups.rows).count, 7, "every session is somewhere")
    }

    /// A chosen machine that has left the list — gone offline, or its last session archived
    /// away to nothing — gives way to All, and the list says so; one still listed does not.
    func testAnUnknownSelectionFallsBackToAll() {
        let now = Date()
        let rows = threeMachines(now)
        let parts = SessionGroups.byMachine(rows, now: now)
        XCTAssertEqual(SessionTabs.parts(parts, showing: "machine:gone"), parts, "an unknown machine shows All")
        XCTAssertTrue(SessionTabs.isUnlisted("machine:gone", rows: rows))
        XCTAssertFalse(SessionTabs.isUnlisted("machine:studio", rows: rows))
        XCTAssertFalse(SessionTabs.isUnlisted("unnamed", rows: rows))
        XCTAssertFalse(SessionTabs.isUnlisted(nil, rows: rows), "All is never unlisted")

        let studioGone = rows.filter { $0.machineName != "studio" }
        XCTAssertTrue(SessionTabs.isUnlisted("machine:studio", rows: studioGone))
        XCTAssertEqual(SessionTabs.parts(SessionGroups.byMachine(studioGone, now: now), showing: "machine:studio").map(\.name),
                       ["build-box", "Laptop", "Unnamed machine"])
        XCTAssertTrue(SessionTabs.isUnlisted("unnamed", rows: rows.filter { $0.machineName != nil }))
        XCTAssertTrue(SessionTabs.isUnlisted("machine:studio", rows: []), "an emptied list holds no machine")
        XCTAssertEqual(SessionTabs.unlisted, "That machine has no sessions listed now, so this is all of them.")
    }
}
