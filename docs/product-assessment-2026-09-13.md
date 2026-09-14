# Product assessment: better and uniform

Assessment, 2026-09-13, after the dashboard, the live page and the iPhone app reached
feature parity. Ordered by what to do first. Nothing here was done at the time of
writing except where noted.

1. **Commit the work.** Everything from the September 12–14 sessions was uncommitted:
   forecast, password, iOS app, push, model and effort, wrapper auto-management, queue
   and side question, key import and tap fix, packaging. Land it as feature commits.
2. **One source of truth for how the router runs.** The login item's plist did not set
   the network bind, so the phone only worked because the router was launched by hand.
   Bind and port belong in the database like the password and the wrapper switch, shown
   in Settings on all three surfaces. (`agent-router install --bind` now carries the bind
   into the service; the database setting is still open.) A deploy script in the repo
   for the copy-then-rename handover would remove a class of mistakes.
3. **One web app, not two pages.** The dashboard and the live page total 3,400 lines,
   share six of twenty-one theme tokens and no helpers, and have two check scripts. The
   iOS app has the right shape: one shell, sessions as a section. The web should mirror
   it. This is the biggest "uniform" gap.
4. **Identical vocabulary and controls across surfaces.** "Live" versus "view" sessions,
   model and effort labels, the side question ("btw" button with a browser prompt on the
   web, "Ask on the side…" with a proper field on the phone), the queue count in the
   status line.
5. **Notifications beyond prompts.** A long turn finishing, the forecast saying a window
   runs out today, the router going offline or restarting. Each opt-in, with the same
   settings row on web and phone.
6. **Onboarding on the phone.** Bonjour advertisement so the app lists routers on the
   LAN, or a QR code from the dashboard carrying address and password. The password
   crosses the LAN in the clear by accepted design; self-signed TLS with the certificate
   pinned by the app would close that without any portal step.
7. **Versioning.** Router and app both say 0.1.0 forever. A bump per commit series, the
   router version shown in the app and dashboard, and the app warning when the router is
   older than it expects. (The extension's version now follows `Cargo.toml`.)
8. **Self-contained tests.** The UI tests need a hand-started router on a spare port
   with a database copy. A script for that, plus one command that runs router tests,
   both web checks and the simulator suites.
9. **Bigger bets.** A lock-screen widget with headroom and time to limit. A real iPad
   layout. A retention policy for captured bodies, since the database is at 900 MB.
